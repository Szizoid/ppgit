//! `cherry-pick <target>`: the second of the three commit-addressing
//! commands routed by whether `target` is paired (see `commands::reset`
//! for the first, and the shared classification in `src/commits.rs`). A
//! shared target is cherry-picked into both repositories — private using
//! its own SHA of the pair, which may carry private-only content the
//! public diff never had, public using its; the two new commits are then
//! paired the same way a dual `commit` pairs its own two, since this is
//! the same logical change landing in both places again. A private-only
//! target has nothing meaningful to do on the public side, so it's
//! applied to the private repository alone. An unpaired *public* target
//! is refused rather than guessed at, exactly like `reset`.
//!
//! Only the unambiguous single-target form is routed this way, matching
//! `reset`: anything else (no target, several targets, a range,
//! `--abort`/`--continue`/`--quit`, `--`) passes through untouched.

use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::process::ExitCode;

use crate::cli::Scope;
use crate::commits::{Classified, ClassifyError, classify, single_target_index};
use crate::exec::{PRIVATE_GIT_PREFIX, PUBLIC_GIT_PREFIX, run_quiet_ok, run_quiet_stdout, to_git};
use crate::notes::{pair_if_both_moved, read_head};
use crate::ppgitignore::{PRIVATE_GIT_DIR, PUBLIC_GIT_DIR};
use crate::{announce, run_on_both};

fn in_project() -> bool {
    Path::new(PRIVATE_GIT_DIR).is_dir()
}

pub fn cmd_cherry_pick(explicit: Option<Scope>, args: &[OsString]) -> ExitCode {
    if !in_project() {
        return passthrough(explicit, args);
    }

    let Some(index) = single_target_index(args) else {
        return passthrough(explicit, args);
    };

    match explicit {
        Some(Scope::Public) => to_git(PUBLIC_GIT_PREFIX, args),
        Some(Scope::Private) => to_git(PRIVATE_GIT_PREFIX, args),
        // Same reasoning as `reset`: `--both` on a command addressing a
        // specific commit used to be meaningless, since the two
        // repositories hold different commits — the routing below is
        // what "sensibly run this against both" means now.
        Some(Scope::Both) | None => route(args, index),
    }
}

/// Today's behaviour for anything this module doesn't intercept: default
/// public, an explicit scope honoured as-is — the same replication of
/// `resolve_scope`'s generic default that `reset` needs, for the same
/// reason (a `Builtin` bypasses that path entirely).
fn passthrough(explicit: Option<Scope>, args: &[OsString]) -> ExitCode {
    match explicit.unwrap_or(Scope::Public) {
        Scope::Public => to_git(PUBLIC_GIT_PREFIX, args),
        Scope::Private => to_git(PRIVATE_GIT_PREFIX, args),
        Scope::Both => run_on_both(args),
    }
}

fn route(args: &[OsString], index: usize) -> ExitCode {
    let text = target_text(args, index);
    match classify(text) {
        Ok(Classified::Shared { private, public }) => {
            cherry_pick_both(args, index, &private, &public)
        }
        Ok(Classified::PrivateOnly(private)) => cherry_pick_private_only(args, index, &private),
        Err(ClassifyError::NotFound) => {
            eprintln!("ppgit: {text} does not resolve to a commit in either repository");
            ExitCode::FAILURE
        }
        Err(ClassifyError::UnpairedPublic) => {
            eprintln!("ppgit: {text} has no pairing note");
            eprintln!(
                "  It may predate `ppgit commit`'s pairing, or be an orphan `pp doctor` should"
            );
            eprintln!(
                "  know about. Cherry-picking onto a single half explicitly is still available"
            );
            eprintln!("  with --public/--private.");
            ExitCode::FAILURE
        }
    }
}

fn target_text(args: &[OsString], index: usize) -> &str {
    // `single_target_index` only returns an index whose argument is
    // valid UTF-8 — it checked with the same `to_str` call.
    args[index].to_str().unwrap_or("<target>")
}

/// Cherry-picks a shared target into both repositories, private first as
/// everywhere else in dual mode, using each half's own SHA of the pair.
/// If both succeed, the two resulting commits are paired — reusing the
/// exact mechanism `commands::commit` uses, since a cherry-pick of a
/// shared commit is the same kind of event: the same logical change
/// landing in both repositories together.
///
/// The public half is deliberately *not* done with a real
/// `git cherry-pick`: see `apply_public_side`.
fn cherry_pick_both(
    args: &[OsString],
    index: usize,
    private_sha: &str,
    public_sha: &str,
) -> ExitCode {
    // Checked before touching either half: `apply_public_side` needs a
    // clean public index to safely fold the result in, and finding that
    // out only after the private cherry-pick already landed would leave
    // the pair half-done for a failure that was entirely predictable in
    // advance.
    if has_staged_changes(PUBLIC_GIT_PREFIX) {
        eprintln!("ppgit: the public index already has staged changes; refusing to fold a");
        eprintln!("  cherry-pick's result into them. Commit or restore them first, then retry.");
        return ExitCode::FAILURE;
    }

    let before_private = read_head(PRIVATE_GIT_PREFIX);
    let before_public = read_head(PUBLIC_GIT_PREFIX);

    announce(PRIVATE_GIT_DIR);
    let mut private_args = args.to_vec();
    private_args[index] = OsString::from(private_sha);
    let private = to_git(PRIVATE_GIT_PREFIX, &private_args);
    if private != ExitCode::SUCCESS {
        // A conflict, most likely — nothing committed privately, so
        // there's no message to carry any pairing on, and the public
        // half is left untouched for the user to resolve and retry.
        return private;
    }

    announce(PUBLIC_GIT_DIR);
    let public = apply_public_side(public_sha);

    if public == ExitCode::SUCCESS {
        pair_if_both_moved(before_private, before_public);
    }

    // As everywhere in dual mode, only the private (superset) result
    // decides: a public-side problem here is surfaced (its own message
    // is printed either way), but doesn't turn an otherwise successful
    // private cherry-pick into a failure.
    private
}

/// Kept inside the private git-dir, like `commit`'s `MESSAGE_FILE` —
/// excluded from both repositories, so it's never picked up by `add` or
/// shown by `status`.
const CHERRY_PICK_MSG_FILE: &str = ".ppgit/PPGIT_CHERRY_PICK_MSG";

/// Reproduces `public_sha`'s cherry-pick effect on the public half
/// *without* running a real `git cherry-pick` there. A real one would
/// hit exactly the bug `pull` and `checkout` already ran into (see
/// CLAUDE.md/QUESTIONS.md): git's "would overwrite local changes" check
/// for a merge-like operation compares the working tree against *that
/// repository's own index*, and by this point the private cherry-pick
/// above has already rewritten the shared working tree — verified in
/// scratch that a real public-side `cherry-pick` fails there even though
/// the tree already holds exactly the right content.
///
/// Since the tree is already correct (the superset invariant means
/// `private_sha`'s diff, just applied, is a superset of `public_sha`'s),
/// all that's actually needed is: stage whatever the tree now holds for
/// the paths the public repository tracks (`add -A` — the public
/// git-dir's own exclude rules keep private-only files out of this
/// automatically, same as any ordinary `add`), and commit it with
/// `public_sha`'s own message and authorship, which is what a real
/// cherry-pick would have produced anyway. `git add`/`git commit` don't
/// perform the working-tree-vs-index check that trips up checkout/merge,
/// verified the same way.
///
/// Assumes the public index is already clean — `cherry_pick_both` checks
/// that before calling this, early enough that a dirty index refuses the
/// whole operation before the private half is ever touched.
fn apply_public_side(public_sha: &str) -> ExitCode {
    let log_field = |format: &str| {
        let mut args = PUBLIC_GIT_PREFIX.to_vec();
        args.extend(["log", "-1", format, public_sha]);
        run_quiet_stdout("git", &args)
    };
    let Ok(message) = log_field("--format=%B") else {
        eprintln!("ppgit: could not read {public_sha}'s message");
        return ExitCode::FAILURE;
    };
    let Ok(author) = log_field("--format=%an <%ae>") else {
        eprintln!("ppgit: could not read {public_sha}'s author");
        return ExitCode::FAILURE;
    };
    let Ok(date) = log_field("--format=%aI") else {
        eprintln!("ppgit: could not read {public_sha}'s author date");
        return ExitCode::FAILURE;
    };

    let mut add_args = PUBLIC_GIT_PREFIX.to_vec();
    add_args.extend(["add", "-A"]);
    if !run_quiet_ok("git", &add_args) {
        eprintln!("ppgit: could not stage the public half's tree");
        return ExitCode::FAILURE;
    }

    if let Err(e) = fs::write(CHERRY_PICK_MSG_FILE, &message) {
        eprintln!("ppgit: could not stage the commit message: {e}");
        return ExitCode::FAILURE;
    }
    let author_arg = format!("--author={author}");
    let date_arg = format!("--date={date}");
    let result = to_git(
        PUBLIC_GIT_PREFIX,
        &[
            OsString::from("commit"),
            OsString::from("--quiet"),
            OsString::from(author_arg),
            OsString::from(date_arg),
            OsString::from("-F"),
            OsString::from(CHERRY_PICK_MSG_FILE),
        ],
    );
    let _ = fs::remove_file(CHERRY_PICK_MSG_FILE);
    result
}

/// Whether `prefix`'s index currently differs from `HEAD` — same check,
/// and same "safe before the first commit too" reasoning, as
/// `commands::commit::has_staged_changes`; not shared from there since
/// that one is private to its own module.
fn has_staged_changes(prefix: &[&str]) -> bool {
    let mut args = prefix.to_vec();
    args.extend(["diff", "--cached", "--quiet"]);
    !run_quiet_ok("git", &args)
}

/// Cherry-picks a private-only target into the private repository alone
/// — the public repository has nothing meaningful to receive, since the
/// target was never shared with it in the first place.
fn cherry_pick_private_only(args: &[OsString], index: usize, private_sha: &str) -> ExitCode {
    announce(PRIVATE_GIT_DIR);
    let mut private_args = args.to_vec();
    private_args[index] = OsString::from(private_sha);
    to_git(PRIVATE_GIT_PREFIX, &private_args)
}
