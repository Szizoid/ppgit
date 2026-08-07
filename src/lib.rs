mod cli;
mod commands;
mod exec;
mod gh;
mod ppgitignore;

use std::env;
use std::ffi::OsString;
use std::process::ExitCode;

use cli::{Builtin, Scope, is_push, recognize, resolve_scope, split_scope};
use commands::{cmd_help, cmd_version, clone::cmd_clone, commit::cmd_commit, init::cmd_init};
use exec::{PRIVATE_GIT_PREFIX, PUBLIC_GIT_PREFIX, to_git};
use ppgitignore::{
    PPGITIGNORE, PRIVATE_GIT_DIR, PUBLIC_GIT_DIR, sync_excludes, tracked_but_ignored,
    warn_tracked_but_ignored,
};

pub fn run() -> ExitCode {
    let args: Vec<OsString> = env::args_os().skip(1).collect();

    // Before anything else, so an edit to .ppgitignore takes effect on the
    // very next command. Hard failure on purpose: if the private paths
    // can't be hidden, letting a `git add` run anyway risks leaking them.
    if let Err(e) = sync_excludes() {
        eprintln!("ppgit: failed to sync exclude files: {e}");
        return ExitCode::FAILURE;
    }

    let (explicit_scope, rest) = split_scope(&args);

    match tracked_but_ignored() {
        Ok(conflicts) if !conflicts.is_empty() => {
            warn_tracked_but_ignored(&conflicts);
            if is_push(rest) {
                eprintln!("ppgit: refusing to push until the above is resolved");
                return ExitCode::FAILURE;
            }
        }
        Ok(_) => {}
        // A failed *check* only costs a warning — unlike a failed sync,
        // it doesn't itself make a leak more likely, and dying here would
        // block ordinary work for no gain.
        Err(e) => eprintln!("ppgit: warning: could not check for {PPGITIGNORE} conflicts: {e}"),
    }

    let scope = match resolve_scope(explicit_scope, rest) {
        Ok(scope) => scope,
        Err(code) => return code,
    };

    match recognize(rest) {
        // ppgit's own commands aren't per-repository, so a scope flag has
        // nothing to select between and is simply not consulted for them.
        Some(Builtin::Help) => cmd_help(),
        Some(Builtin::Version) => cmd_version(),
        Some(Builtin::Init) => cmd_init(),
        Some(Builtin::Clone) => cmd_clone(rest),
        Some(Builtin::Commit) => cmd_commit(scope, rest),
        None => match scope {
            Scope::Public => to_git(PUBLIC_GIT_PREFIX, rest),
            Scope::Private => to_git(PRIVATE_GIT_PREFIX, rest),
            Scope::Both => run_on_both(rest),
        },
    }
}

/// Marks whose output follows, on stderr so it annotates the run without
/// getting into anything piping ppgit's stdout.
pub(crate) fn announce(git_dir: &str) {
    let half = if git_dir == PRIVATE_GIT_DIR {
        "private"
    } else {
        "public"
    };
    eprintln!("== {half} ({git_dir}) ==");
}

/// Runs `args` against both repositories, private first: it's the superset,
/// so it's the run that must succeed, and going first means the working
/// tree is settled before the public half looks at it.
///
/// Only the private result decides the exit code. The public repository
/// deliberately can't see every file, so it failing where the private one
/// succeeded is an ordinary outcome — `ppgit add CLAUDE.md` on a private
/// path, or a commit with nothing public to record — not something to
/// report as the command having failed. Its output is still shown, so
/// nothing is hidden from the user.
pub(crate) fn run_on_both(args: &[OsString]) -> ExitCode {
    announce(PRIVATE_GIT_DIR);
    let private = to_git(PRIVATE_GIT_PREFIX, args);
    if private != ExitCode::SUCCESS {
        return private;
    }

    announce(PUBLIC_GIT_DIR);
    to_git(PUBLIC_GIT_PREFIX, args);
    private
}
