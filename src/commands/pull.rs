//! Dual `pull`. Running plain `git pull` on both halves cannot work:
//! whichever half pulls first updates the shared working tree, and the
//! other half's merge then refuses to "overwrite local changes" — git
//! judges dirtiness against its *index*, so the file already holding
//! exactly the merge result does not help (verified in scratch, both
//! orders). So the private (superset) half pulls for real — its merge
//! result is what the shared tree should hold — and the public half is
//! then advanced without touching the tree at all: fetch, and iff the
//! local branch is strictly behind its upstream, a fast-forward of ref
//! and index via `reset --mixed`. The ancestor guard means nothing is
//! ever discarded; a genuine divergence is reported, not resolved.

use std::ffi::OsString;
use std::path::Path;
use std::process::ExitCode;

use crate::announce;
use crate::cli::Scope;
use crate::exec::{
    PRIVATE_GIT_PREFIX, PUBLIC_GIT_PREFIX, run_loud_checked, run_quiet_ok, run_quiet_stdout, to_git,
};
use crate::ppgitignore::{PRIVATE_GIT_DIR, PUBLIC_GIT_DIR};

pub fn cmd_pull(explicit: Option<Scope>, args: &[OsString]) -> ExitCode {
    if !Path::new(PRIVATE_GIT_DIR).is_dir() {
        return to_git(PUBLIC_GIT_PREFIX, args);
    }

    match explicit {
        // A deliberately narrow pull is passthrough, exactly as before —
        // including `--public`, which is how the superset invariant gets
        // broken, but that's the user's explicit call and doctor's find.
        Some(Scope::Public) => to_git(PUBLIC_GIT_PREFIX, args),
        Some(Scope::Private) => to_git(PRIVATE_GIT_PREFIX, args),
        Some(Scope::Both) | None => pull_both(args),
    }
}

fn pull_both(args: &[OsString]) -> ExitCode {
    announce(PRIVATE_GIT_DIR);
    let private = to_git(PRIVATE_GIT_PREFIX, args);
    if private != ExitCode::SUCCESS {
        // A conflicted merge stops here with the private index mid-merge
        // and the public half untouched; once it's resolved and
        // committed, a re-run finds the private half up to date and
        // advances the public one.
        return private;
    }

    announce(PUBLIC_GIT_DIR);
    match advance_public() {
        Ok(()) => private,
        Err(code) => code,
    }
}

/// Brings the public half up to its upstream without touching the
/// working tree, which the private pull has already settled. Any pull
/// arguments deliberately don't apply here: this side is not a merge,
/// only a catch-up, and a state it can't fast-forward through is
/// reported rather than guessed at.
fn advance_public() -> Result<(), ExitCode> {
    run_loud_checked("git", ["fetch"])?;

    let Ok(branch) = run_quiet_stdout("git", &["symbolic-ref", "--short", "HEAD"]) else {
        eprintln!("ppgit: public half is not on a branch; nothing to advance");
        return Ok(());
    };
    let Ok(upstream) = run_quiet_stdout("git", &["rev-parse", "--abbrev-ref", "@{upstream}"])
    else {
        if run_quiet_ok(
            "git",
            &["rev-parse", "--verify", "-q", &format!("origin/{branch}")],
        ) {
            eprintln!("ppgit: public {branch} has no upstream though origin/{branch} exists");
            eprintln!("  `pp doctor --fix` repairs that; then pull again.");
            return Err(ExitCode::FAILURE);
        }
        println!("ppgit: public {branch} is not on the remote yet; nothing to pull");
        return Ok(());
    };

    let head = run_quiet_stdout("git", &["rev-parse", "HEAD"]);
    let target = run_quiet_stdout("git", &["rev-parse", &upstream]);
    match (head, target) {
        (Ok(head), Ok(target)) if head == target => {
            println!("ppgit: public half already up to date");
            Ok(())
        }
        (Ok(_), Ok(target)) => {
            if !run_quiet_ok("git", &["merge-base", "--is-ancestor", "HEAD", &target]) {
                eprintln!("ppgit: public {branch} has diverged from {upstream}");
                eprintln!("  The private half pulled fine, but the public one has local commits");
                eprintln!("  its remote doesn't know. Nothing was changed on the public side —");
                eprintln!("  see `pp doctor` for the state and the ways out.");
                return Err(ExitCode::FAILURE);
            }
            // Ref and index only — the tree already holds the private
            // merge, which is a superset of this content.
            run_loud_checked("git", ["reset", "-q", "--mixed", &target])?;
            println!("ppgit: public {branch} fast-forwarded to {upstream}");
            Ok(())
        }
        _ => {
            eprintln!("ppgit: could not compare the public half with {upstream}");
            Err(ExitCode::FAILURE)
        }
    }
}
