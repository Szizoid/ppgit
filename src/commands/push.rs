use std::ffi::OsString;
use std::process::ExitCode;

use crate::announce;
use crate::cli::Scope;
use crate::exec::{PRIVATE_GIT_PREFIX, PUBLIC_GIT_PREFIX, to_git};
use crate::notes;
use crate::ppgitignore::{PRIVATE_GIT_DIR, PUBLIC_GIT_DIR};

/// `push` is ordinary passthrough, plus one thing a plain `git push`
/// never does on its own: once a half's push succeeds, its pairing-notes
/// ref goes along too. Verified in scratch that git never includes
/// `refs/notes/*` in a default push, so without this a note made here
/// would simply never reach a second machine — the same gap `clone`
/// closes for the fetch direction via the refspec.
pub fn cmd_push(scope: Scope, args: &[OsString]) -> ExitCode {
    match scope {
        Scope::Public => push_one(PUBLIC_GIT_PREFIX, args),
        Scope::Private => push_one(PRIVATE_GIT_PREFIX, args),
        Scope::Both => push_both(args),
    }
}

/// Dual push, private first: same rule as everywhere else in dual mode —
/// the superset half decides the outcome, the public half's own result
/// (which can differ, e.g. nothing new to push there) is shown but never
/// overrides it.
fn push_both(args: &[OsString]) -> ExitCode {
    announce(PRIVATE_GIT_DIR);
    let private = push_one(PRIVATE_GIT_PREFIX, args);
    if private != ExitCode::SUCCESS {
        return private;
    }

    announce(PUBLIC_GIT_DIR);
    push_one(PUBLIC_GIT_PREFIX, args);
    private
}

fn push_one(prefix: &[&str], args: &[OsString]) -> ExitCode {
    let result = to_git(prefix, args);
    if result == ExitCode::SUCCESS {
        notes::push_best_effort(prefix);
    }
    result
}
