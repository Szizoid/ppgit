//! `stash` and `clean`: the two working-tree commands the dual-routing
//! audit found misrouted. Both concern the *whole* shared tree, which
//! only the private (superset) half can see in full — and both have a
//! mode that would eat the private git-dir itself, verified in a
//! scratch project before writing this.

use std::ffi::OsString;
use std::path::Path;
use std::process::ExitCode;

use crate::cli::Scope;
use crate::exec::{PRIVATE_GIT_PREFIX, PUBLIC_GIT_PREFIX, to_git};
use crate::ppgitignore::PRIVATE_GIT_DIR;

/// Outside a ppgit project both commands are none of ppgit's business
/// and pass through untouched, like any other git command.
fn in_project() -> bool {
    Path::new(PRIVATE_GIT_DIR).is_dir()
}

/// Whether a stash command would take ignored files with it (`--all`).
/// The short cluster check mirrors `opens_an_editor`'s: `-au` is `-a
/// -u`, so every letter counts.
fn stash_takes_ignored(args: &[OsString]) -> bool {
    args.iter().any(|arg| match arg.to_str() {
        Some("--all") => true,
        Some(arg) if arg.starts_with('-') && !arg.starts_with("--") => arg.contains('a'),
        _ => false,
    })
}

/// `stash` defaults to the private half: it's the one that sees every
/// change in the shared tree, so its stash is complete — a public stash
/// would quietly leave the private files' modifications in place. An
/// explicit `--public` still works (a deliberately narrow stash), but
/// `--both` is refused: whichever half stashed first would sweep away
/// the very changes the second was about to save.
pub fn cmd_stash(explicit: Option<Scope>, args: &[OsString]) -> ExitCode {
    if !in_project() {
        return to_git(PUBLIC_GIT_PREFIX, args);
    }

    if stash_takes_ignored(args) {
        eprintln!("ppgit: refusing `stash` with `-a`/`--all`");
        eprintln!(
            "  --all stashes ignored files, and in a ppgit project the private git-dir\n  \
             ({PRIVATE_GIT_DIR}) is itself ignored: git stashes the repository into itself\n  \
             and deletes it, stash and all. Use -u for untracked files."
        );
        return ExitCode::FAILURE;
    }

    match explicit {
        Some(Scope::Both) => {
            eprintln!("ppgit: `stash` cannot run on both halves");
            eprintln!(
                "  They share one working tree: whichever half stashed first would sweep\n  \
                 away the changes the second was about to save. It runs on the private\n  \
                 (superset) half, which sees every change. Re-run without --both."
            );
            ExitCode::FAILURE
        }
        Some(Scope::Public) => to_git(PUBLIC_GIT_PREFIX, args),
        Some(Scope::Private) | None => to_git(PRIVATE_GIT_PREFIX, args),
    }
}

/// `clean` runs on the private half only. Its untracked set is the true
/// one; to the public half every private file is merely "ignored", so a
/// public `clean -x` would delete them all. `-e` is injected because it
/// survives `-x` (verified), keeping the private git-dir from being
/// treated as removable rubbish.
pub fn cmd_clean(explicit: Option<Scope>, args: &[OsString]) -> ExitCode {
    if !in_project() {
        return to_git(PUBLIC_GIT_PREFIX, args);
    }

    if matches!(explicit, Some(Scope::Public) | Some(Scope::Both)) {
        eprintln!("ppgit: `clean` runs on the private repository only");
        eprintln!(
            "  The public half cannot tell a private file from rubbish — every private\n  \
             path is just 'ignored' to it, and `clean -x` would delete them all. The\n  \
             private half sees the true untracked set. Re-run without the scope flag."
        );
        return ExitCode::FAILURE;
    }

    let mut full = vec![
        OsString::from("clean"),
        OsString::from("-e"),
        OsString::from(format!("/{PRIVATE_GIT_DIR}")),
    ];
    full.extend(args[1..].iter().cloned());
    to_git(PRIVATE_GIT_PREFIX, &full)
}
