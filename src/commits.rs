//! Resolves a commit-ish argument (as `reset`/`cherry-pick`/`rebase`
//! take one) against whichever repository actually has it, classifies it
//! shared vs. private-only via `notes::pairing`, and — for a
//! private-only target — finds the nearest shared ancestor. Shared by
//! `commands::reset` and `commands::cherry_pick`.

use std::ffi::OsString;

use crate::exec::{PRIVATE_GIT_PREFIX, PUBLIC_GIT_PREFIX, run_quiet_stdout};
use crate::notes::{self, Pairing};

/// Where a resolved commit-ish argument was actually found. Finding it
/// in one repository says nothing by itself about whether it's shared —
/// a shared commit's *private*-side SHA still won't resolve publicly,
/// since the two repositories never hold each other's objects even for
/// paired commits — so callers still need `notes::pairing` on whichever
/// side this names.
pub enum Found {
    Public(String),
    Private(String),
}

/// Resolves `text` against both repositories, public first — matching
/// the project's general rule that anything addressing history defaults
/// to the public repository's own ref space (branch names, `HEAD~N`,
/// ...), which is why this essentially always lands here for anything
/// but a literal private-only SHA typed by hand.
pub fn resolve(text: &str) -> Option<Found> {
    if let Some(sha) = rev_parse(PUBLIC_GIT_PREFIX, text) {
        return Some(Found::Public(sha));
    }
    rev_parse(PRIVATE_GIT_PREFIX, text).map(Found::Private)
}

/// Resolves `text` to a concrete SHA, or `None` if it names nothing that
/// actually exists. `--verify` alone isn't enough for this — verified in
/// scratch that `rev-parse --verify -q <40 hex chars>` happily echoes
/// back a syntactically valid SHA and exits 0 even when no such object
/// exists anywhere in the repository; it only checks that the *text*
/// parses as a single revision, not that it resolves to a real one. The
/// `^{commit}` suffix makes it dereference for real (still works for
/// symbolic refs — `HEAD`, branch names, `HEAD~2` all peel to a commit
/// exactly as before), which is what a raw, possibly bogus SHA typed by
/// hand actually needs checked.
fn rev_parse(prefix: &[&str], text: &str) -> Option<String> {
    let mut args = prefix.to_vec();
    let target = format!("{text}^{{commit}}");
    args.extend(["rev-parse", "--verify", "-q", &target]);
    run_quiet_stdout("git", &args).ok()
}

/// For a private-only `target`, the public counterpart of its nearest
/// shared ancestor — what the public half should be pointed at instead,
/// since `target` itself has nothing there. `None` if no ancestor of
/// `target` is shared at all (a project with no paired commits in its
/// reachable history, e.g. everything so far predates this mechanism).
pub fn last_shared_ancestor(target: &str) -> Option<String> {
    let mut args = PRIVATE_GIT_PREFIX.to_vec();
    args.extend(["log", "--format=%H", target]);
    let history = run_quiet_stdout("git", &args).ok()?;

    // Skip `target` itself: a shared `target` is handled by the caller
    // before this is ever called (see `commands::reset`), so by the time
    // we're here it's already known to be unpaired.
    history
        .lines()
        .skip(1)
        .find_map(|sha| match notes::pairing(PRIVATE_GIT_PREFIX, sha) {
            Pairing::Shared(public_sha) => Some(public_sha),
            Pairing::Unpaired => None,
        })
}

/// What a resolved, classified target means for dual routing.
pub enum Classified {
    /// Paired — the private and public SHAs of the pair, regardless of
    /// which side `target` was actually named from.
    Shared { private: String, public: String },
    /// No pairing note, found in the *private* repository — the only
    /// shape that legitimately means "private-only content". An
    /// unpaired commit found in the *public* repository is a different
    /// case (`ClassifyError::UnpairedPublic`), not folded in here.
    PrivateOnly(String),
}

pub enum ClassifyError {
    /// `target` resolves in neither repository.
    NotFound,
    /// `target` resolves publicly but carries no pairing note — it
    /// either predates the pairing mechanism, or is the orphan `doctor`
    /// already detects as a broken superset invariant. Neither case is
    /// "private-only" in the sense that matters here, and guessing which
    /// one it is isn't this module's call to make.
    UnpairedPublic,
}

/// Resolves and classifies `target` in one step — the shared first half
/// of `reset` and `cherry-pick`'s routing, which diverge only in what
/// they *do* with the result.
pub fn classify(target: &str) -> Result<Classified, ClassifyError> {
    let (prefix, sha) = match resolve(target) {
        Some(Found::Public(sha)) => (PUBLIC_GIT_PREFIX, sha),
        Some(Found::Private(sha)) => (PRIVATE_GIT_PREFIX, sha),
        None => return Err(ClassifyError::NotFound),
    };

    match notes::pairing(prefix, &sha) {
        Pairing::Shared(counterpart) => Ok(if prefix == PRIVATE_GIT_PREFIX {
            Classified::Shared {
                private: sha,
                public: counterpart,
            }
        } else {
            Classified::Shared {
                private: counterpart,
                public: sha,
            }
        }),
        Pairing::Unpaired if prefix == PUBLIC_GIT_PREFIX => Err(ClassifyError::UnpairedPublic),
        Pairing::Unpaired => Ok(Classified::PrivateOnly(sha)),
    }
}

/// Why `resolve_targets` couldn't produce a `(private, public)` pair.
pub enum TargetError {
    /// `target` resolves in neither repository.
    NotFound,
    /// `target` resolves publicly but carries no pairing note.
    UnpairedPublic,
    /// `target` is private-only, and nothing in its history is shared
    /// either — there's no public commit to anchor the public half to
    /// at all.
    NoSharedAncestor,
}

/// Resolves `target` all the way down to a `(private_sha, public_sha)`
/// pair to act on — the shared first step of both `reset` and `rebase`'s
/// routing. A shared target's own pair; a private-only target's own SHA
/// paired with the public SHA of its nearest shared ancestor instead,
/// since `target` itself has nothing on the public side to move to.
/// (`cherry-pick` needs the same classification but not this last step —
/// it acts on the original pair directly rather than needing a *target*
/// for the public half to move to, so it calls `classify` on its own.)
pub fn resolve_targets(target: &str) -> Result<(String, String), TargetError> {
    match classify(target) {
        Ok(Classified::Shared { private, public }) => Ok((private, public)),
        Ok(Classified::PrivateOnly(private)) => match last_shared_ancestor(&private) {
            Some(public) => Ok((private, public)),
            None => Err(TargetError::NoSharedAncestor),
        },
        Err(ClassifyError::NotFound) => Err(TargetError::NotFound),
        Err(ClassifyError::UnpairedPublic) => Err(TargetError::UnpairedPublic),
    }
}

/// The index of `args`' sole commit-ish argument, if it has exactly one
/// — `None` for a no-target form, a `-- <path>...` form, or more than one
/// bare argument, none of which name a single commit to classify.
///
/// `args[0]` is always the git subcommand itself (`"reset"`,
/// `"cherry-pick"`, ...) — every `Builtin` gets the whole command line
/// that way, since that's what forwarding it to git needs — so the scan
/// starts at index 1. Skipping it isn't optional: the subcommand name
/// doesn't start with `-` either, so without the skip it reads as the
/// command's own positional argument and everything downstream
/// misclassifies.
pub fn single_target_index(args: &[OsString]) -> Option<usize> {
    if args.iter().skip(1).any(|arg| arg.to_str() == Some("--")) {
        return None;
    }
    let mut found = None;
    for (i, arg) in args.iter().enumerate().skip(1) {
        let text = arg.to_str()?;
        if !text.starts_with('-') {
            if found.is_some() {
                return None;
            }
            found = Some(i);
        }
    }
    found
}
