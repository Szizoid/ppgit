//! `reset <target>`: the first of the three commit-addressing commands
//! that used to go to the public repository only because the two
//! repositories hold genuinely different commits. Routed now by whether
//! `target` is *shared* (paired via `commands::commit`'s notes) or
//! *private-only*: a shared target resets both repositories to their
//! respective halves of the pair; a private-only one resets the private
//! repository to the target itself and the public repository to the
//! nearest shared commit before it — the closest the public half can
//! get to "the same point in history".
//!
//! Only the unambiguous "reset to a commit" form is routed this way:
//! exactly one non-flag argument, no `--`. Anything else (no target,
//! `reset -- <path>`, multiple paths) is none of this module's business
//! and passes through exactly as it always has.

use std::ffi::OsString;
use std::path::Path;
use std::process::ExitCode;

use crate::cli::Scope;
use crate::commits::{TargetError, resolve_targets, single_target_index};
use crate::exec::{PRIVATE_GIT_PREFIX, PUBLIC_GIT_PREFIX, to_git};
use crate::ppgitignore::{PRIVATE_GIT_DIR, PUBLIC_GIT_DIR};
use crate::{announce, run_on_both};

fn in_project() -> bool {
    Path::new(PRIVATE_GIT_DIR).is_dir()
}

pub fn cmd_reset(explicit: Option<Scope>, args: &[OsString]) -> ExitCode {
    if !in_project() {
        return passthrough(explicit, args);
    }

    let Some(index) = single_target_index(args) else {
        return passthrough(explicit, args);
    };

    match explicit {
        Some(Scope::Public) => to_git(PUBLIC_GIT_PREFIX, args),
        Some(Scope::Private) => to_git(PRIVATE_GIT_PREFIX, args),
        // `--both` on a command that addresses a specific commit used to
        // be meaningless (mirroring the same SHA into two repositories
        // that hold different commits) — the routing below *is* what
        // "sensibly run this against both" means now, so it applies the
        // same whether `--both` was asked for or is just the default.
        Some(Scope::Both) | None => route(args, index),
    }
}

/// Today's behaviour for anything this module doesn't intercept: default
/// public, an explicit scope honoured as-is. Exactly what `resolve_scope`
/// already computes for `reset` — replicated here because a `Builtin`
/// bypasses that generic path entirely.
fn passthrough(explicit: Option<Scope>, args: &[OsString]) -> ExitCode {
    match explicit.unwrap_or(Scope::Public) {
        Scope::Public => to_git(PUBLIC_GIT_PREFIX, args),
        Scope::Private => to_git(PRIVATE_GIT_PREFIX, args),
        Scope::Both => run_on_both(args),
    }
}

fn route(args: &[OsString], index: usize) -> ExitCode {
    let text = target_text(args, index);
    match resolve_targets(text) {
        Ok((private, public)) => reset_to(args, index, &private, &public),
        Err(TargetError::NotFound) => {
            eprintln!("ppgit: {text} does not resolve to a commit in either repository");
            ExitCode::FAILURE
        }
        Err(TargetError::UnpairedPublic) => {
            // An unpaired *public* commit is the superset invariant's
            // business, not this command's: either it predates the
            // pairing mechanism, or it's the orphan `doctor` already
            // detects and `pp add . && pp commit` repairs.
            eprintln!("ppgit: {text} has no pairing note");
            eprintln!(
                "  It may predate `ppgit commit`'s pairing, or be an orphan `pp doctor` should"
            );
            eprintln!("  know about. Resetting a single half explicitly is still available with");
            eprintln!("  --public/--private.");
            ExitCode::FAILURE
        }
        Err(TargetError::NoSharedAncestor) => {
            eprintln!("ppgit: {text} has no shared ancestor to point the public half at");
            eprintln!("  Nothing in its history was ever committed with `pp commit`. Resetting a");
            eprintln!("  single half explicitly is still available with --public/--private.");
            ExitCode::FAILURE
        }
    }
}

fn target_text(args: &[OsString], index: usize) -> &str {
    // `single_target_index` only returns an index whose argument is
    // valid UTF-8 — it checked with the same `to_str` call.
    args[index].to_str().unwrap_or("<target>")
}

/// Runs `reset` against both repositories, private first as everywhere
/// else in dual mode, each with its own SHA substituted in place of
/// whatever the user actually typed — so both halves reset to a
/// concrete, already-resolved commit rather than re-interpreting the
/// original text (which, for a bare SHA, wouldn't even resolve on the
/// other side).
fn reset_to(args: &[OsString], index: usize, private_sha: &str, public_sha: &str) -> ExitCode {
    announce(PRIVATE_GIT_DIR);
    let mut private_args = args.to_vec();
    private_args[index] = OsString::from(private_sha);
    let private = to_git(PRIVATE_GIT_PREFIX, &private_args);
    if private != ExitCode::SUCCESS {
        return private;
    }

    announce(PUBLIC_GIT_DIR);
    let mut public_args = args.to_vec();
    public_args[index] = OsString::from(public_sha);
    to_git(PUBLIC_GIT_PREFIX, &public_args);
    private
}
