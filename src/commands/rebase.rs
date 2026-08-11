//! `rebase <upstream>`: the last of the three commit-addressing commands,
//! and the one whose public half can't be reconstructed the same way
//! `commands::reset`/`commands::cherry_pick` do. Those two only ever
//! reconstruct *one* commit; a rebase replays a whole range, and the
//! shared working tree only ever holds the *final* state after a real
//! rebase, never each intermediate one along the way.
//!
//! The shape, exactly as the author designed it (a transaction with a
//! rollback, not either of the two narrower alternatives first
//! proposed — see QUESTIONS.md):
//!
//! 1. Resolve `<upstream>` to a `(private_target, public_target)` pair,
//!    the same way `reset` does.
//! 2. Capture the private branch's current tip (`private_before`), and
//!    the ordered list of commits about to be replayed, each noted as
//!    shared (with its public SHA) or private-only.
//! 3. Run one real rebase — on the private half only. It replays
//!    *everything*, shared and private-only alike; the private half is
//!    always the superset.
//! 4. Compare the replayed range's length to what was captured in step
//!    2. They should match — rebase preserves order and count — *unless*
//!    a commit became empty and was silently dropped (verified in
//!    scratch: `git rebase` does this by default, see CLAUDE.md). If
//!    they don't match, the positions from step 2 can no longer be
//!    trusted to line up with the new commits.
//! 5. If they matched: reconstruct the public half from *only* the
//!    positions that were shared, entirely in plumbing, never touching
//!    the shared working tree (which the private rebase already left in
//!    the right place) — see `reconstruct_public`.
//! 6. If they didn't: **roll the private half back** to `private_before`
//!    (`reset --hard` — exact and safe, since the abandoned commits
//!    aren't deleted, just unreferenced until GC) and report the
//!    mismatch, pointing at `doctor`. Nothing on the public half is ever
//!    touched in this case. `--force` (ppgit's own flag, stripped before
//!    git ever sees it — distinct from git's own `-f`/`--force-rebase`)
//!    skips the rollback instead: the private half stays rebased, the
//!    public half is left wherever it was, a named "out of step" state
//!    for the user to repair via `doctor` rather than an automatic
//!    recovery attempt.
//!
//! Scoped narrowly on purpose, matching `reset`/`cherry-pick`: only
//! `rebase <upstream>`, the current branch, no `--onto`, no `-i`, no
//! `--continue`/`--abort`/`--skip`. Anything else passes through
//! exactly as before — not fixed, but not worse.
//!
//! A conflict during the real private rebase is left for the user to
//! resolve directly (`git --git-dir=.ppgit --work-tree=. rebase
//! --continue`/`--abort`) — the public half was never touched, and
//! reconciling it afterward is an ordinary `pp doctor` /
//! `pp add . && pp commit` job, the same as any other case where the
//! private half moved and the public one didn't yet.

use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::process::{Command, ExitCode};

use crate::announce;
use crate::cli::Scope;
use crate::commits::{TargetError, resolve_targets, single_target_index};
use crate::exec::{
    PRIVATE_GIT_PREFIX, PUBLIC_GIT_PREFIX, run_loud_checked, run_quiet_ok, run_quiet_stdout, to_git,
};
use crate::notes::{self, Pairing, read_head};
use crate::ppgitignore::{PRIVATE_GIT_DIR, PUBLIC_GIT_DIR};
use crate::run_on_both;

fn in_project() -> bool {
    Path::new(PRIVATE_GIT_DIR).is_dir()
}

pub fn cmd_rebase(explicit: Option<Scope>, args: &[OsString]) -> ExitCode {
    if !in_project() {
        return passthrough(explicit, args);
    }

    let (args, force) = extract_force_flag(args);
    let args = &args[..];

    let Some(index) = single_target_index(args) else {
        return passthrough(explicit, args);
    };

    match explicit {
        Some(Scope::Public) => to_git(PUBLIC_GIT_PREFIX, args),
        Some(Scope::Private) => to_git(PRIVATE_GIT_PREFIX, args),
        Some(Scope::Both) | None => route(args, index, force),
    }
}

/// Today's behaviour for anything this module doesn't intercept: default
/// public, an explicit scope honoured as-is — same replication of
/// `resolve_scope`'s generic default `reset`/`cherry-pick` also need,
/// since a `Builtin` bypasses that path entirely.
fn passthrough(explicit: Option<Scope>, args: &[OsString]) -> ExitCode {
    match explicit.unwrap_or(Scope::Public) {
        Scope::Public => to_git(PUBLIC_GIT_PREFIX, args),
        Scope::Private => to_git(PRIVATE_GIT_PREFIX, args),
        Scope::Both => run_on_both(args),
    }
}

/// Pulls ppgit's own `--force` out of the argument list. Not a real
/// git-rebase flag — that's `-f`/`--force-rebase`, an unrelated meaning
/// left completely alone, since this only ever matches the exact string
/// `--force`.
fn extract_force_flag(args: &[OsString]) -> (Vec<OsString>, bool) {
    let mut rest = Vec::with_capacity(args.len());
    let mut force = false;
    for arg in args {
        if arg.to_str() == Some("--force") {
            force = true;
        } else {
            rest.push(arg.clone());
        }
    }
    (rest, force)
}

fn target_text(args: &[OsString], index: usize) -> &str {
    // `single_target_index` only returns an index whose argument is
    // valid UTF-8 — it checked with the same `to_str` call.
    args[index].to_str().unwrap_or("<upstream>")
}

fn route(args: &[OsString], index: usize, force: bool) -> ExitCode {
    let text = target_text(args, index);
    let (private_target, public_target) = match resolve_targets(text) {
        Ok(pair) => pair,
        Err(TargetError::NotFound) => {
            eprintln!("ppgit: {text} does not resolve to a commit in either repository");
            return ExitCode::FAILURE;
        }
        Err(TargetError::UnpairedPublic) => {
            eprintln!("ppgit: {text} has no pairing note");
            eprintln!(
                "  It may predate `ppgit commit`'s pairing, or be an orphan `pp doctor` should"
            );
            eprintln!("  know about. Rebasing a single half explicitly is still available with");
            eprintln!("  --public/--private.");
            return ExitCode::FAILURE;
        }
        Err(TargetError::NoSharedAncestor) => {
            eprintln!("ppgit: {text} has no shared ancestor to point the public half at");
            eprintln!("  Nothing in its history was ever committed with `pp commit`. Rebasing a");
            eprintln!("  single half explicitly is still available with --public/--private.");
            return ExitCode::FAILURE;
        }
    };

    let Some(private_before) = read_head(PRIVATE_GIT_PREFIX) else {
        eprintln!("ppgit: nothing to rebase — the private half has no commits yet");
        return ExitCode::FAILURE;
    };
    let Some(branch) = current_branch(PRIVATE_GIT_PREFIX) else {
        eprintln!("ppgit: not on a branch — rebase needs one to move");
        return ExitCode::FAILURE;
    };

    let Some(before_range) = ordered_range(&private_target) else {
        eprintln!("ppgit: could not read the range being rebased");
        return ExitCode::FAILURE;
    };
    let before_pairings: Vec<Option<String>> = before_range
        .iter()
        .map(|sha| match notes::pairing(PRIVATE_GIT_PREFIX, sha) {
            Pairing::Shared(public_sha) => Some(public_sha),
            Pairing::Unpaired => None,
        })
        .collect();

    announce(PRIVATE_GIT_DIR);
    let mut private_args = args.to_vec();
    private_args[index] = OsString::from(&private_target);
    let private = to_git(PRIVATE_GIT_PREFIX, &private_args);
    if private != ExitCode::SUCCESS {
        eprintln!("ppgit: the private rebase did not complete — resolve it directly:");
        eprintln!("  git --git-dir=.ppgit --work-tree=. rebase --continue   (or --abort)");
        eprintln!("  The public half was never touched. Once you're done, `pp doctor` and a dual");
        eprintln!("  `pp add . && pp commit` bring it back in step by hand.");
        return private;
    }

    let Some(after_range) = ordered_range(&private_target) else {
        eprintln!("ppgit: could not read the rebased range back");
        return ExitCode::FAILURE;
    };

    if after_range.len() != before_range.len() {
        eprintln!(
            "ppgit: the rebase changed the number of commits ({} \u{2192} {}) — most likely one",
            before_range.len(),
            after_range.len()
        );
        eprintln!("  became empty and was dropped. The public half can no longer be matched up");
        eprintln!("  commit-for-commit with confidence.");
        return give_up(&private_before, force);
    }

    announce(PUBLIC_GIT_DIR);
    match reconstruct_public(&public_target, &after_range, &before_pairings, &branch) {
        Ok(()) => private,
        Err(e) => {
            eprintln!("ppgit: {e}");
            give_up(&private_before, force)
        }
    }
}

/// What happens once the public half can no longer be trusted to
/// reconstruct correctly — either the range mismatch above, or a real
/// failure partway through `reconstruct_public`. Rolls the private
/// rebase back by default (exact and safe — see the module doc comment);
/// `--force` leaves it rebased instead, a named mismatch for the user to
/// repair through `doctor` on their own terms.
fn give_up(private_before: &str, force: bool) -> ExitCode {
    if force {
        eprintln!("  --force: leaving the private half rebased and the public half untouched.");
        eprintln!("  `pp doctor` can confirm the mismatch; repair it by hand from there.");
        return ExitCode::FAILURE;
    }
    eprintln!("  Rolling the private half back to before the rebase. Re-run with --force to");
    eprintln!("  keep the private rebase and repair the public half by hand instead.");
    let mut args = PRIVATE_GIT_PREFIX.to_vec();
    args.extend(["reset", "--quiet", "--hard", private_before]);
    if !run_quiet_ok("git", &args) {
        eprintln!("ppgit: warning: could not roll the private half back to {private_before}");
        eprintln!(
            "  fix it by hand: git --git-dir=.ppgit --work-tree=. reset --hard {private_before}"
        );
    }
    ExitCode::FAILURE
}

fn current_branch(prefix: &[&str]) -> Option<String> {
    let mut args = prefix.to_vec();
    args.extend(["symbolic-ref", "--short", "HEAD"]);
    run_quiet_stdout("git", &args).ok()
}

/// The commits between `target` and the private half's current `HEAD`,
/// oldest first — the range a rebase onto `target` would replay.
fn ordered_range(target: &str) -> Option<Vec<String>> {
    let spec = format!("{target}..HEAD");
    let mut args = PRIVATE_GIT_PREFIX.to_vec();
    args.extend(["log", "--reverse", "--format=%H", &spec]);
    let list = run_quiet_stdout("git", &args).ok()?;
    Some(list.lines().map(str::to_string).collect())
}

/// Scratch paths for the reconstruction below — kept inside the private
/// git-dir, like `commit`'s `MESSAGE_FILE`, so they're excluded from
/// both repositories and never picked up by `add` or shown by `status`.
/// Resolved to *absolute* paths before use and passed down explicitly,
/// rather than read as relative constants at each call site: verified in
/// scratch that git resolves a relative `GIT_INDEX_FILE` (and a relative
/// `--prefix`) against whatever `--work-tree` is in effect for that
/// invocation, not the process's cwd — and the staging step's work-tree
/// *is* the scratch directory itself, so a relative path there resolves
/// to nonsense (a path nested inside the scratch directory looking for
/// itself) instead of the sibling location it actually needs.
struct Scratch {
    tree: String,
    index: String,
}

fn scratch_paths() -> Result<Scratch, String> {
    let cwd = std::env::current_dir()
        .map_err(|e| format!("could not resolve the current directory: {e}"))?;
    let as_str = |rel: &str| {
        cwd.join(rel)
            .to_str()
            .map(str::to_string)
            .ok_or_else(|| "scratch path is not valid UTF-8".to_string())
    };
    Ok(Scratch {
        tree: as_str(".ppgit/PPGIT_REBASE_TREE")?,
        index: as_str(".ppgit/PPGIT_REBASE_INDEX")?,
    })
}

impl Scratch {
    fn cleanup(&self) {
        let _ = fs::remove_dir_all(&self.tree);
        let _ = fs::remove_file(&self.index);
    }
}

/// Builds the public half's reconstructed history and lands it, without
/// ever touching the shared working tree — the private rebase already
/// left it exactly right, and the caller (`route`) never checks it out
/// to anywhere else in between.
///
/// For each position in `after_range` that `before_pairings` says was
/// shared, materializes that (new) private commit's tree into a scratch
/// directory (`git read-tree` + `checkout-index`, entirely in the
/// private half's own object store, verified in scratch not to touch the
/// real index or working tree), stages *that* using the public
/// git-dir's own exclude rules into a scratch index (same trick, exactly
/// what filters private-only content out of a real `add`), and
/// `commit-tree`s the result onto the growing public chain with the
/// original public commit's own message and authorship. Once the chain
/// is built, the public branch ref moves to its tip and `reset --mixed`
/// syncs the index — the working tree is already correct, so nothing
/// else needs to happen.
fn reconstruct_public(
    public_target: &str,
    after_range: &[String],
    before_pairings: &[Option<String>],
    branch: &str,
) -> Result<(), String> {
    let scratch = scratch_paths()?;
    scratch.cleanup();

    let mut parent = public_target.to_string();
    for (new_private_sha, old_public_sha) in after_range.iter().zip(before_pairings) {
        let Some(old_public_sha) = old_public_sha else {
            continue; // a private-only position — nothing to reconstruct
        };
        match reconstruct_one(&scratch, new_private_sha, old_public_sha, &parent) {
            Ok(next) => {
                // Pairs the *new* commits, not the old ones the rebase
                // just made unreachable — notes don't survive a rebase
                // (verified in scratch, see CLAUDE.md), so without this
                // every commit this loop just built would be unpaired,
                // and everything downstream (another `reset`/
                // `cherry-pick`/`rebase` over this same history) would
                // wrongly treat it as private-only.
                notes::attach_note(PRIVATE_GIT_PREFIX, new_private_sha, &next);
                notes::attach_note(PUBLIC_GIT_PREFIX, &next, new_private_sha);
                parent = next;
            }
            Err(e) => {
                scratch.cleanup();
                return Err(e);
            }
        }
    }

    let refname = format!("refs/heads/{branch}");
    let mut update_ref = PUBLIC_GIT_PREFIX.to_vec();
    update_ref.extend(["update-ref", &refname, &parent]);
    if !run_quiet_ok("git", &update_ref) {
        scratch.cleanup();
        return Err(format!("could not move the public branch to {parent}"));
    }

    let synced = run_loud_checked("git", ["reset", "--quiet", "--mixed", "HEAD"]).is_ok();
    scratch.cleanup();
    if !synced {
        return Err("could not sync the public index after the rebase".into());
    }
    Ok(())
}

/// Reconstructs one public commit: `new_private_sha`'s tree, filtered to
/// what the public repository tracks, on top of `parent`, with
/// `old_public_sha`'s own message and authorship (what a real replay
/// there would have produced). Returns the new commit's SHA.
fn reconstruct_one(
    scratch: &Scratch,
    new_private_sha: &str,
    old_public_sha: &str,
    parent: &str,
) -> Result<String, String> {
    materialize(scratch, new_private_sha)?;
    let tree = filtered_tree(scratch)?;

    let message = read_commit_field(old_public_sha, "%B")
        .ok_or_else(|| format!("could not read {old_public_sha}'s message"))?;
    let author_name = read_commit_field(old_public_sha, "%an")
        .ok_or_else(|| format!("could not read {old_public_sha}'s author"))?;
    let author_email = read_commit_field(old_public_sha, "%ae")
        .ok_or_else(|| format!("could not read {old_public_sha}'s author"))?;
    let author_date = read_commit_field(old_public_sha, "%aI")
        .ok_or_else(|| format!("could not read {old_public_sha}'s author date"))?;

    commit_tree(
        &tree,
        parent,
        &message,
        &author_name,
        &author_email,
        &author_date,
    )
}

/// Materializes `sha` — a commit in the *private* half's object store —
/// into the scratch directory, via a scratch index of its own so the
/// real one is never touched. Verified in scratch: `checkout-index`
/// needs a work-tree in context even though `--prefix` redirects every
/// write away from it, so `--work-tree=.` (the real one, via
/// `PRIVATE_GIT_PREFIX`) is along for the ride without anything actually
/// landing there.
fn materialize(scratch: &Scratch, sha: &str) -> Result<(), String> {
    let _ = fs::remove_dir_all(&scratch.tree);
    fs::create_dir_all(&scratch.tree)
        .map_err(|e| format!("could not prepare a scratch directory: {e}"))?;

    let read_tree = Command::new("git")
        .args(PRIVATE_GIT_PREFIX)
        .args([
            "read-tree",
            &format!("--index-output={}", scratch.index),
            sha,
        ])
        .output();
    if !read_tree.is_ok_and(|out| out.status.success()) {
        return Err(format!("could not read {sha}'s tree"));
    }

    let prefix = format!("{}/", scratch.tree);
    let checkout = Command::new("git")
        .args(PRIVATE_GIT_PREFIX)
        .args(["checkout-index", "-a", &format!("--prefix={prefix}")])
        .env("GIT_INDEX_FILE", &scratch.index)
        .output();
    if !checkout.is_ok_and(|out| out.status.success()) {
        return Err(format!("could not materialize {sha}"));
    }
    Ok(())
}

/// Stages the scratch directory using the *public* git-dir's own exclude
/// rules — into a scratch index, never the real one — and returns the
/// resulting tree's SHA. This is what filters private-only content back
/// out, the same way it's kept out of any ordinary `add`.
fn filtered_tree(scratch: &Scratch) -> Result<String, String> {
    let _ = fs::remove_file(&scratch.index);
    let add = Command::new("git")
        .args(["--work-tree", &scratch.tree, "add", "-A"])
        .env("GIT_INDEX_FILE", &scratch.index)
        .output();
    if !add.is_ok_and(|out| out.status.success()) {
        return Err("could not stage the reconstructed tree".into());
    }

    let write_tree = Command::new("git")
        .args(["write-tree"])
        .env("GIT_INDEX_FILE", &scratch.index)
        .output()
        .map_err(|e| format!("could not run git write-tree: {e}"))?;
    if !write_tree.status.success() {
        return Err("could not write the reconstructed tree".into());
    }
    Ok(String::from_utf8_lossy(&write_tree.stdout)
        .trim()
        .to_string())
}

/// `commit-tree` takes authorship through the environment, not flags
/// (verified in scratch — unlike `git commit`, it has no `--author`/
/// `--date`), so both author and committer are set the same way here;
/// the committer becomes whoever's reconstructing this, same as any
/// ordinary commit, real cherry-picks included.
fn commit_tree(
    tree: &str,
    parent: &str,
    message: &str,
    author_name: &str,
    author_email: &str,
    author_date: &str,
) -> Result<String, String> {
    let output = Command::new("git")
        .args(PUBLIC_GIT_PREFIX)
        .args(["commit-tree", tree, "-p", parent, "-F", "-"])
        .env("GIT_AUTHOR_NAME", author_name)
        .env("GIT_AUTHOR_EMAIL", author_email)
        .env("GIT_AUTHOR_DATE", author_date)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            if let Some(stdin) = child.stdin.take() {
                let mut stdin = stdin;
                let _ = stdin.write_all(message.as_bytes());
            }
            child.wait_with_output()
        });

    match output {
        Ok(out) if out.status.success() => {
            Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
        }
        _ => Err(format!("could not build a commit on top of {parent}")),
    }
}

fn read_commit_field(sha: &str, format: &str) -> Option<String> {
    let format_arg = format!("--format={format}");
    let mut args = PUBLIC_GIT_PREFIX.to_vec();
    args.extend(["log", "-1", &format_arg, sha]);
    run_quiet_stdout("git", &args).ok()
}
