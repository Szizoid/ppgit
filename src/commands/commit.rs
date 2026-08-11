use std::ffi::OsString;
use std::fs;
use std::process::ExitCode;

use crate::exec::{PRIVATE_GIT_PREFIX, PUBLIC_GIT_PREFIX, run_quiet_ok, run_quiet_stdout, to_git};
use crate::notes::{pair_if_both_moved, read_head};
use crate::ppgitignore::{PRIVATE_GIT_DIR, PUBLIC_GIT_DIR};
use crate::{announce, run_on_both};

/// `commit` always runs against both repositories — `resolve_scope`
/// refuses a narrowing `--public`/`--private` before this is ever called,
/// since a scoped commit could never be paired with its counterpart (see
/// `cli::commit_must_run_on_both`). What's left to decide is only *how*
/// the two commits are produced: reusing one editor session, or not, and
/// whether `--private <text>` asked for a message the private commit
/// alone gets.
pub fn cmd_commit(args: &[OsString]) -> ExitCode {
    let (args, private_extra) = match extract_private_flag(args) {
        Ok(parsed) => parsed,
        Err(code) => return code,
    };
    let args = &args[..];

    if private_extra.is_some() && uses_unsupported_message_source(args) {
        eprintln!("ppgit: --private only knows how to combine with -m/--message or -F/--file");
        eprintln!("  (not --reuse-message, --fixup, --squash, --template, --no-edit, or -C/-c/-t)");
        return ExitCode::FAILURE;
    }

    // The superset half having nothing staged is not the failure it looks
    // like at first: it's the ordinary shape of a change that only
    // touches the public repository's own bookkeeping (`privatize`'s
    // `git rm --cached`, say) with nothing new for the private half to
    // record. Committing there alone, rather than letting the private
    // half's "nothing to commit" abort the whole thing, is what makes a
    // plain unscoped `pp commit` cover that case without a scope flag
    // that `commit` no longer accepts.
    if !has_staged_changes(PRIVATE_GIT_PREFIX) && has_staged_changes(PUBLIC_GIT_PREFIX) {
        if private_extra.is_some() {
            eprintln!("ppgit: --private has nothing to attach to: nothing is staged privately");
            return ExitCode::FAILURE;
        }
        announce(PUBLIC_GIT_DIR);
        return to_git(PUBLIC_GIT_PREFIX, args);
    }

    let before_private = read_head(PRIVATE_GIT_PREFIX);
    let before_public = read_head(PUBLIC_GIT_PREFIX);

    let result = if opens_an_editor(args) {
        commit_on_both(args, private_extra.as_deref())
    } else {
        run_on_both_with_private(args, private_extra.as_deref())
    };

    if result == ExitCode::SUCCESS {
        pair_if_both_moved(before_private, before_public);
    }
    result
}

/// Pulls a `--private <text>`/`--private=<text>` out of the argument
/// list, if there is one — it's `commit`'s own flag, not git's, so
/// neither half's invocation should ever see it. Kept separate from the
/// `--public`/`--private` *scope* flags: those are only ever recognised
/// in first position, before the subcommand (`cli::split_scope`), so
/// there's no actual collision with this one appearing after `commit` —
/// just a name worth not confusing with that one.
fn extract_private_flag(args: &[OsString]) -> Result<(Vec<OsString>, Option<String>), ExitCode> {
    fn as_text(arg: &OsString) -> Result<&str, ExitCode> {
        arg.to_str().ok_or_else(|| {
            eprintln!("ppgit: --private's message is not valid UTF-8");
            ExitCode::FAILURE
        })
    }

    let mut rest = Vec::with_capacity(args.len());
    let mut found: Option<String> = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let text = if arg.to_str() == Some("--private") {
            let Some(value) = iter.next() else {
                eprintln!("ppgit: --private needs a message");
                return Err(ExitCode::FAILURE);
            };
            Some(as_text(value)?)
        } else {
            match arg.to_str() {
                Some(s) => s.strip_prefix("--private="),
                None => None,
            }
        };

        match text {
            None => rest.push(arg.clone()),
            Some(_) if found.is_some() => {
                eprintln!("ppgit: --private given more than once");
                return Err(ExitCode::FAILURE);
            }
            Some(text) => found = Some(text.to_string()),
        }
    }
    Ok((rest, found))
}

/// Whether `args` gets its message some way `--private` doesn't know how
/// to layer onto — everything `opens_an_editor` treats as "a message was
/// given" except `-m`/`--message` and `-F`/`--file`, the two modes the
/// design actually covers.
fn uses_unsupported_message_source(args: &[OsString]) -> bool {
    args.iter().any(|arg| match arg.to_str() {
        Some(s) if s.starts_with("--") => matches!(
            s.split('=').next(),
            Some(
                "--reuse-message"
                    | "--reedit-message"
                    | "--template"
                    | "--fixup"
                    | "--squash"
                    | "--no-edit"
            )
        ),
        Some(s) if s.starts_with('-') => s.chars().skip(1).any(|c| "Cct".contains(c)),
        _ => false,
    })
}

/// Whether `prefix`'s index currently differs from `HEAD` — including
/// before the first commit, when `HEAD` doesn't exist yet: `git diff
/// --cached` compares against the empty tree in that case, verified in a
/// scratch repository, so this needs no special-casing for a fresh
/// project.
fn has_staged_changes(prefix: &[&str]) -> bool {
    let mut args = prefix.to_vec();
    args.extend(["diff", "--cached", "--quiet"]);
    !run_quiet_ok("git", &args)
}

/// Kept inside the private git-dir rather than the working tree: that's
/// excluded from both repositories, so a stray message file can never be
/// picked up by a commit or shown by `status`.
const MESSAGE_FILE: &str = ".ppgit/PPGIT_COMMIT_MSG";

/// Same rationale as `MESSAGE_FILE`, for the combined message
/// `--private` builds — the two are never in use at once (one belongs to
/// the editor path, the other to the `-m`/`-F` path), so one name covers
/// both.
const PRIVATE_EXTRA_FILE: &str = ".ppgit/PPGIT_PRIVATE_EXTRA_MSG";

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
///
/// `extra` is `--private <text>`'s payload, if any: appended to the
/// private commit only, by amending it right after it's made — in the
/// same breath, so nothing about this touches history that existed
/// before this call. The public commit reuses the *pre-amend* message,
/// read back before the amend ever runs.
fn commit_on_both(args: &[OsString], extra: Option<&str>) -> ExitCode {
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

    if let Some(extra) = extra {
        if !amend_private_with_extra(&message, extra) {
            eprintln!("ppgit: warning: could not add the --private text to the private commit");
            eprintln!(
                "  the commit itself is fine; amend it by hand if you still want that text there"
            );
        }
    }

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

/// Folds `extra` into `base_message` and amends the private commit *just
/// made* to hold the result — allowed under the "never rewrite existing
/// history" rule this project otherwise holds to, since this is the
/// commit from the line above, not history that existed before this
/// call. Quiet and best-effort: the underlying commit already succeeded
/// either way, so a failure here is reported by the caller as a warning,
/// not treated as the command having failed.
fn amend_private_with_extra(base_message: &str, extra: &str) -> bool {
    let combined = format!("{}\n\nprivate: {extra}\n", base_message.trim_end());
    if fs::write(PRIVATE_EXTRA_FILE, &combined).is_err() {
        return false;
    }

    let mut args = PRIVATE_GIT_PREFIX.to_vec();
    args.extend(["commit", "--amend", "--quiet", "-F", PRIVATE_EXTRA_FILE]);
    let ok = run_quiet_ok("git", &args);

    let _ = fs::remove_file(PRIVATE_EXTRA_FILE);
    ok
}

/// The non-editor path (`-m`/`-F` already say where the message comes
/// from) with `extra` folded into the private half's message, if
/// `--private` asked for that. Without `extra` this is exactly
/// `run_on_both` — kept as a thin wrapper so the ordinary case doesn't
/// pay for logic it never uses.
fn run_on_both_with_private(args: &[OsString], extra: Option<&str>) -> ExitCode {
    let Some(extra) = extra else {
        return run_on_both(args);
    };

    let private_args = match with_private_extra(args, extra) {
        Ok(args) => args,
        Err(code) => return code,
    };

    announce(PRIVATE_GIT_DIR);
    let private = to_git(PRIVATE_GIT_PREFIX, &private_args);
    let _ = fs::remove_file(PRIVATE_EXTRA_FILE);
    if private != ExitCode::SUCCESS {
        return private;
    }

    announce(PUBLIC_GIT_DIR);
    to_git(PUBLIC_GIT_PREFIX, args);
    private
}

/// Builds the private half's own argument list for the `-m`/`-F` path,
/// folding `extra` into whichever message source `args` already uses.
/// `-m`: git already joins repeated `-m` into one message with a blank
/// line between, so appending another is enough. `-F <file>`: can't add
/// a second `-m` or a second `-F` (git refuses both combinations), so a
/// temp file holding the original content plus `extra` takes over
/// instead — the public half keeps the original `-F <file>` untouched.
fn with_private_extra(args: &[OsString], extra: &str) -> Result<Vec<OsString>, ExitCode> {
    let Some(file) = file_message_source(args) else {
        let mut private_args = args.to_vec();
        private_args.push(OsString::from("-m"));
        // Prefixed the same way the -F and editor paths label it, so the
        // private commit reads the same regardless of how the message
        // was supplied.
        private_args.push(OsString::from(format!("private: {extra}")));
        return Ok(private_args);
    };

    let original = fs::read_to_string(&file).map_err(|e| {
        eprintln!("ppgit: could not read {file} for --private: {e}");
        ExitCode::FAILURE
    })?;
    let combined = format!("{}\n\nprivate: {extra}\n", original.trim_end());
    fs::write(PRIVATE_EXTRA_FILE, &combined).map_err(|e| {
        eprintln!("ppgit: could not stage the combined message for --private: {e}");
        ExitCode::FAILURE
    })?;

    let mut private_args = without_file_flag(args);
    private_args.push(OsString::from("-F"));
    private_args.push(OsString::from(PRIVATE_EXTRA_FILE));
    Ok(private_args)
}

/// The file named by a standalone `-F <file>`, `--file <file>` or
/// `--file=<file>` in `args`. Doesn't look inside a short cluster like
/// `-aF` — `-F`'s value has to be its own argument (or the `=` form) for
/// this to find it at all, which covers how the flag is actually used in
/// practice.
fn file_message_source(args: &[OsString]) -> Option<String> {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.to_str() {
            Some("-F") | Some("--file") => {
                return iter.next().and_then(|v| v.to_str()).map(str::to_string);
            }
            Some(s) => {
                if let Some(file) = s.strip_prefix("--file=") {
                    return Some(file.to_string());
                }
            }
            None => {}
        }
    }
    None
}

/// `args` with a standalone `-F <file>`/`--file <file>`/`--file=<file>`
/// removed — the counterpart to `file_message_source`, used to build the
/// private half's replacement `-F` without also keeping the original one.
fn without_file_flag(args: &[OsString]) -> Vec<OsString> {
    let mut out = Vec::with_capacity(args.len());
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.to_str() {
            Some("-F") | Some("--file") => {
                iter.next();
            }
            Some(s) if s.starts_with("--file=") => {}
            _ => out.push(arg.clone()),
        }
    }
    out
}
