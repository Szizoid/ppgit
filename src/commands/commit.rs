use std::ffi::OsString;
use std::fs;
use std::process::ExitCode;

use crate::cli::Scope;
use crate::exec::{PRIVATE_GIT_PREFIX, PUBLIC_GIT_PREFIX, run_quiet_stdout, to_git};
use crate::ppgitignore::{PRIVATE_GIT_DIR, PUBLIC_GIT_DIR};
use crate::{announce, run_on_both};

/// Commit is ordinary passthrough for a single repository; it only needs
/// ppgit's help when both are involved, and even then only when git would
/// have opened an editor — with an explicit `-m` the same arguments
/// already produce the same message in both.
pub fn cmd_commit(scope: Scope, args: &[OsString]) -> ExitCode {
    match scope {
        Scope::Public => to_git(PUBLIC_GIT_PREFIX, args),
        Scope::Private => to_git(PRIVATE_GIT_PREFIX, args),
        Scope::Both if opens_an_editor(args) => commit_on_both(args),
        Scope::Both => run_on_both(args),
    }
}

/// Kept inside the private git-dir rather than the working tree: that's
/// excluded from both repositories, so a stray message file can never be
/// picked up by a commit or shown by `status`.
const MESSAGE_FILE: &str = ".ppgit/PPGIT_COMMIT_MSG";

/// Whether git would open an editor for this commit — i.e. whether the
/// user has *not* already said where the message comes from. When they
/// have, both repositories can simply be handed the same arguments; the
/// message is identical either way and no editor is involved.
fn opens_an_editor(args: &[OsString]) -> bool {
    !args.iter().any(|arg| match arg.to_str() {
        Some(arg) if arg.starts_with("--") => matches!(
            arg.split('=').next(),
            Some(
                "--message"
                    | "--file"
                    | "--reuse-message"
                    | "--reedit-message"
                    | "--template"
                    | "--fixup"
                    | "--squash"
                    | "--no-edit"
            )
        ),
        // A short cluster like `-am` is `-a -m`, so look at every letter
        // rather than just the first.
        Some(arg) if arg.starts_with('-') => arg.chars().skip(1).any(|c| "mFCct".contains(c)),
        _ => false,
    })
}

/// Commits to both repositories from a single editor session: the private
/// (superset) commit goes first and interactively, then its message is
/// reused verbatim for the public one.
///
/// Only for the case `opens_an_editor` reports — with an explicit `-m`
/// and friends there's nothing to share, and appending our own `-F` would
/// collide with the user's flag ("options '-m' and '-F' cannot be used
/// together").
fn commit_on_both(args: &[OsString]) -> ExitCode {
    announce(PRIVATE_GIT_DIR);
    let private = to_git(PRIVATE_GIT_PREFIX, args);
    if private != ExitCode::SUCCESS {
        // Nothing committed privately — an aborted editor, an empty
        // message, nothing staged. There's no message to carry over, and
        // committing publicly alone would leave the two out of step.
        return private;
    }

    let message = match run_quiet_stdout("git", &["--git-dir=.ppgit", "log", "-1", "--format=%B"]) {
        Ok(message) => message,
        Err(e) => {
            eprintln!("ppgit: committed privately, but could not read the message back: {e}");
            eprintln!("ppgit: commit the public half yourself with the same message");
            return ExitCode::FAILURE;
        }
    };

    if let Err(e) = fs::write(MESSAGE_FILE, &message) {
        eprintln!("ppgit: committed privately, but could not stage the message for reuse: {e}");
        eprintln!("ppgit: commit the public half yourself with the same message");
        return ExitCode::FAILURE;
    }

    announce(PUBLIC_GIT_DIR);
    let mut public_args = args.to_vec();
    public_args.push(OsString::from("-F"));
    public_args.push(OsString::from(MESSAGE_FILE));
    to_git(PUBLIC_GIT_PREFIX, &public_args);

    let _ = fs::remove_file(MESSAGE_FILE);

    // As everywhere in dual mode, only the private (superset) result
    // decides: the public repository having nothing to commit is the
    // normal outcome of a private-only change, not a failure.
    private
}
