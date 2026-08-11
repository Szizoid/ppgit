//! Shared plumbing for the git-notes commit-pairing mechanism
//! (`commands::commit` creates the pairs) so `refs/notes/commits`
//! actually travels between machines: neither a fetch nor a push brings
//! notes over by default, verified in scratch — a plain `git clone`
//! (bare or not) and a plain `git fetch`/`git push` all leave the ref
//! behind, silently.
use std::process::ExitCode;

use crate::exec::{
    PRIVATE_GIT_PREFIX, PUBLIC_GIT_PREFIX, run_loud_checked, run_quiet_ok, run_quiet_stdout,
};

/// Added — never set — to `remote.origin.fetch`, so it sits alongside
/// whatever branch refspec is already configured instead of replacing
/// it. Safe even when nothing has been noted yet, or the remote has no
/// notes ref at all: a fetch matching nothing is a quiet no-op, verified
/// in scratch.
pub const FETCH_REFSPEC: &str = "+refs/notes/*:refs/notes/*";

/// The matching push-side spec, used as a standalone `git push` refspec
/// rather than through `remote.origin.push`: that config key doesn't
/// add to what `push.default` would otherwise push, it *replaces* it
/// outright (verified in scratch — configuring it to this refspec alone
/// makes a bare `git push` push only notes, dropping the branch), so it
/// can't be set once and forgotten the way the fetch refspec can.
const PUSH_REFSPEC: &str = "refs/notes/*:refs/notes/*";

/// Registers the notes fetch refspec for the git-dir `prefix` addresses,
/// unless it's already there — idempotent, so `init` and `clone` can
/// both call it without needing to know whether the other already has.
pub fn ensure_fetch_refspec(prefix: &[&str]) -> Result<(), ExitCode> {
    let mut list = prefix.to_vec();
    list.extend(["config", "--get-all", "remote.origin.fetch"]);
    if run_quiet_stdout("git", &list).is_ok_and(|out| out.lines().any(|line| line == FETCH_REFSPEC))
    {
        return Ok(());
    }

    let mut add = prefix.to_vec();
    add.extend(["config", "--add", "remote.origin.fetch", FETCH_REFSPEC]);
    run_loud_checked("git", &add)
}

/// Pushes only the pairing notes ref to `origin`, after `prefix`'s
/// ordinary push already succeeded. Best-effort and quiet: nothing here
/// should turn an otherwise-successful `push` into a failure, or clutter
/// its output over an internal bookkeeping ref the user didn't ask
/// about. A failure is still worth a line on stderr, though — it means
/// the *next* machine won't see this pairing until it's retried.
pub fn push_best_effort(prefix: &[&str]) {
    let mut args = prefix.to_vec();
    args.extend(["push", "--quiet", "origin", PUSH_REFSPEC]);
    if !run_quiet_ok("git", &args) {
        eprintln!("ppgit: warning: could not push the pairing notes (branch push still succeeded)");
    }
}

/// Whether `sha` — a commit that exists in `prefix`'s repository — is
/// shared: paired, via `commands::commit`'s notes, with a commit in the
/// other repository. `Unpaired` covers everything else by construction:
/// genuinely private-only content, a commit older than this mechanism,
/// or one made by raw `git commit` bypassing ppgit entirely — none of
/// those have a note, and none of them should look shared.
pub enum Pairing {
    Shared(String),
    Unpaired,
}

/// Looks up `sha`'s pairing note in `prefix`'s repository. `sha` has to
/// be a real object there for this to find anything — unlike a note's
/// *content* (the counterpart's SHA), which never needs to resolve
/// locally at all (verified in scratch, see CLAUDE.md).
pub fn pairing(prefix: &[&str], sha: &str) -> Pairing {
    let mut args = prefix.to_vec();
    args.extend(["notes", "show", sha]);
    match run_quiet_stdout("git", &args) {
        Ok(counterpart) if !counterpart.is_empty() => Pairing::Shared(counterpart),
        _ => Pairing::Unpaired,
    }
}

/// `HEAD` in `prefix`'s repository, or `None` before the first commit —
/// `rev-parse HEAD` fails outright there (not just `--verify`, see
/// CLAUDE.md's facts), which is exactly "no commit yet" rather than
/// something worth reporting as an error.
pub fn read_head(prefix: &[&str]) -> Option<String> {
    let mut args = prefix.to_vec();
    args.extend(["rev-parse", "HEAD"]);
    run_quiet_stdout("git", &args).ok()
}

/// Attaches the pairing notes, but only once both halves have genuinely
/// landed something new at `HEAD` — the only case a "pair" means
/// anything. Used both right after a dual `commit` and after a dual
/// `cherry-pick` of a shared target: in either case, a half whose `HEAD`
/// didn't move (the public repository naturally having nothing to do,
/// the ordinary shape of a private-only change) leaves no note anywhere
/// — an unpaired result is, by design, not a shared one, and doesn't
/// need to look like one.
pub fn pair_if_both_moved(before_private: Option<String>, before_public: Option<String>) {
    let Some(private) = read_head(PRIVATE_GIT_PREFIX) else {
        return;
    };
    let Some(public) = read_head(PUBLIC_GIT_PREFIX) else {
        return;
    };
    if before_private.as_deref() == Some(private.as_str()) {
        return;
    }
    if before_public.as_deref() == Some(public.as_str()) {
        return;
    }

    attach_note(PRIVATE_GIT_PREFIX, &private, &public);
    attach_note(PUBLIC_GIT_PREFIX, &public, &private);
}

/// Records that `on` — a commit that just landed at `HEAD` in `prefix`'s
/// repository — is paired with `pointing_at`, its counterpart's SHA in
/// the other repository. The note's key never needs to resolve to an
/// object `git` has locally to be written or read back (verified in a
/// scratch directory: `git notes add`/`show` don't check the target
/// exists at all), which is what makes this safe even though the two
/// repositories' object stores never actually contain each other's
/// objects.
///
/// Best-effort: failing to attach it doesn't unwind an already-successful
/// commit, just warns — the note is metadata for later commands
/// (`reset`, `cherry-pick`, `rebase` — all built on it), not something
/// the operation that produced it depends on. `pub(crate)`: `rebase`
/// calls it directly for each commit it reconstructs, since it already
/// knows both SHAs of every pair as it builds them rather than having to
/// diff `HEAD` before and after like `commit`/`cherry-pick` do.
pub(crate) fn attach_note(prefix: &[&str], on: &str, pointing_at: &str) {
    let mut args = prefix.to_vec();
    args.extend(["notes", "add", "-m", pointing_at, on]);
    if let Err(e) = run_quiet_stdout("git", &args) {
        eprintln!("ppgit: warning: could not record the pairing note on {on}: {e}");
    }
}
