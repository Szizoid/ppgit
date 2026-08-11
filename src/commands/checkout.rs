//! Dual `checkout`, for the one shape that actually needs it: switching
//! to a branch both halves already have. A plain double invocation (what
//! `checkout` did before this) hits exactly the "would overwrite local
//! changes" wall `pull` (0.13.0) and `cherry-pick` (0.20.0) already ran
//! into — verified in scratch: the private checkout runs first and
//! rewrites the shared working tree, so the public checkout's own index
//! (still reflecting whatever branch it *was* on) disagrees with the
//! tree that's now sitting there, and git refuses. Worse than pull's
//! version of the bug: `run_on_both` only looks at the private result,
//! so the command claimed success while quietly leaving the public half
//! on the old branch.
//!
//! The fix follows the same shape as `pull`: a real checkout only ever
//! runs on the private (superset) half. The public half is brought in
//! line without a second real checkout — `symbolic-ref` to switch which
//! branch is current, then `reset --mixed` to sync the index, neither of
//! which touches the working tree (already correct by the time the
//! public half's turn comes) or performs the check that trips on it.
//!
//! Scoped narrowly on purpose: only `checkout <branch>` — a single bare
//! argument naming a branch both halves already have. Anything else
//! (`-b`, multiple arguments, a path form, a raw commit-ish for a
//! detached checkout) passes through exactly as before — not fixed, but
//! not worse, and `-b` in particular was never affected in the first
//! place (a brand new branch points at the current commit, so there's no
//! working-tree change for the bug to trip on).

use std::ffi::OsString;
use std::process::ExitCode;

use crate::announce;
use crate::cli::Scope;
use crate::exec::{PRIVATE_GIT_PREFIX, PUBLIC_GIT_PREFIX, run_loud_checked, run_quiet_ok, to_git};
use crate::ppgitignore::{PRIVATE_GIT_DIR, PUBLIC_GIT_DIR};
use crate::run_on_both;

pub fn cmd_checkout(scope: Scope, args: &[OsString]) -> ExitCode {
    match scope {
        Scope::Public => to_git(PUBLIC_GIT_PREFIX, args),
        Scope::Private => to_git(PRIVATE_GIT_PREFIX, args),
        Scope::Both => match simple_branch_target(args) {
            Some(branch) => checkout_branch(args, &branch),
            // The path form (`checkout -- <path>`) is scoped narrowly by
            // `resolve_scope` before this is ever reached — `Both` here
            // only happens for a genuine branch switch this module
            // doesn't specifically handle (`-b`, several arguments,
            // ...), which is safe to fall back to the old behaviour for.
            None => run_on_both(args),
        },
    }
}

/// `args`' sole branch name, if `checkout` was given exactly one bare
/// argument that's a real branch in *both* repositories — `args[0]` is
/// always `"checkout"` itself, so this needs exactly two elements total.
/// Anything else (flags, a second argument, a name only one half has)
/// isn't this module's business.
fn simple_branch_target(args: &[OsString]) -> Option<String> {
    let [_, target] = args else {
        return None;
    };
    let name = target.to_str()?;
    if name.starts_with('-') {
        return None;
    }
    if is_branch(PRIVATE_GIT_PREFIX, name) && is_branch(PUBLIC_GIT_PREFIX, name) {
        Some(name.to_string())
    } else {
        None
    }
}

fn is_branch(prefix: &[&str], name: &str) -> bool {
    let refname = format!("refs/heads/{name}");
    let mut args = prefix.to_vec();
    args.extend(["show-ref", "--verify", "--quiet", &refname]);
    run_quiet_ok("git", &args)
}

/// A real checkout on the private half, then the public half is brought
/// in line rather than checked out a second time — see the module doc
/// comment for why. Deliberately *not* "private decides the outcome",
/// unlike everywhere else in dual mode: the whole point of this module
/// is that a public-side problem must never again be swallowed the way
/// it silently was before.
fn checkout_branch(args: &[OsString], branch: &str) -> ExitCode {
    announce(PRIVATE_GIT_DIR);
    let private = to_git(PRIVATE_GIT_PREFIX, args);
    if private != ExitCode::SUCCESS {
        return private;
    }

    announce(PUBLIC_GIT_DIR);
    let public = catch_up_public(branch);
    if public != ExitCode::SUCCESS {
        eprintln!("ppgit: the private half switched to {branch}, but the public half could not");
        eprintln!("  be brought in line — the two are now out of step. `pp doctor` can confirm.");
    }
    public
}

/// Switches which branch is current, then syncs the index to match —
/// verified in scratch to leave the working tree and `git status` alone
/// either way, since neither `symbolic-ref` nor `reset --mixed` performs
/// the working-tree-vs-index check `checkout`/`merge`/`pull` do. No
/// `--git-dir` prefix needed: this always addresses the public half,
/// which is what plain `git` already resolves to.
fn catch_up_public(branch: &str) -> ExitCode {
    if !run_quiet_ok(
        "git",
        &["symbolic-ref", "HEAD", &format!("refs/heads/{branch}")],
    ) {
        return ExitCode::FAILURE;
    }
    match run_loud_checked("git", ["reset", "--quiet", "--mixed", "HEAD"]) {
        Ok(()) => ExitCode::SUCCESS,
        Err(code) => code,
    }
}
