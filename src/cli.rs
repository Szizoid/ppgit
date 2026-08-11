use std::ffi::OsString;
use std::process::ExitCode;

/// Commands ppgit handles itself instead of just forwarding. Some are its
/// own (`init`), some are git's but need orchestrating across the two
/// repositories (`commit`, whose editor must open once, not once each).
pub enum Builtin {
    Help,
    Version,
    Init,
    Clone,
    Commit,
    Doctor,
    Privatize,
    Publicize,
    Stash,
    Clean,
    Pull,
    Push,
    Reset,
    CherryPick,
    Checkout,
    Rebase,
}

pub fn recognize(args: &[OsString]) -> Option<Builtin> {
    if args.is_empty() {
        return Some(Builtin::Help);
    }
    match args[0].to_str() {
        Some("help") | Some("--help") | Some("-h") => Some(Builtin::Help),
        Some("--version") | Some("-V") => Some(Builtin::Version),
        Some("init") => Some(Builtin::Init),
        Some("clone") => Some(Builtin::Clone),
        Some("commit") => Some(Builtin::Commit),
        Some("doctor") => Some(Builtin::Doctor),
        Some("privatize") => Some(Builtin::Privatize),
        Some("publicize") => Some(Builtin::Publicize),
        // Working-tree commands that must not follow the usual routing:
        // both concern the whole shared tree, which only the private
        // half sees in full, and both have a mode that would eat the
        // private git-dir itself. See `commands::tree`.
        Some("stash") => Some(Builtin::Stash),
        Some("clean") => Some(Builtin::Clean),
        // Pull needs orchestrating rather than mirroring: two plain
        // pulls fight over the shared working tree, whichever order
        // they run in. See `commands::pull`.
        Some("pull") => Some(Builtin::Pull),
        // A plain push never sends the pairing-notes ref along with it —
        // git doesn't fetch or push refs/notes/* by default. See
        // `commands::push`.
        Some("push") => Some(Builtin::Push),
        // Addresses a specific commit, which the two repositories don't
        // agree on by default — routed by whether the target is shared
        // or private-only. See `commands::reset`.
        Some("reset") => Some(Builtin::Reset),
        // Same routing as `reset`, applying the change rather than
        // moving a ref. See `commands::cherry_pick`.
        Some("cherry-pick") => Some(Builtin::CherryPick),
        // Branch names are shared between the two repositories (unlike
        // arbitrary commit SHAs), so this needs no shared/private-only
        // classification — just a fix for the same "would overwrite
        // local changes" wall pull and cherry-pick hit. Still a
        // `BRANCH_COMMAND` below, for the scope-refusal and path-form
        // rules — this only changes *how* the Both case runs.
        // See `commands::checkout`.
        Some("checkout") => Some(Builtin::Checkout),
        // Same routing as `reset`/`cherry-pick`, but for a whole range —
        // see `commands::rebase`.
        Some("rebase") => Some(Builtin::Rebase),
        _ => None,
    }
}

/// Which of the two repositories a command should run against.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Public,
    Private,
    Both,
}

/// Peels off a leading `--public`/`--private`/`--both`, returning it along
/// with the rest of the command line. Only recognised in first position,
/// before the git subcommand — everything after that belongs to git, and
/// ppgit has no business reinterpreting it.
pub fn split_scope(args: &[OsString]) -> (Option<Scope>, &[OsString]) {
    let scope = match args.first().and_then(|arg| arg.to_str()) {
        Some("--public") => Scope::Public,
        Some("--private") => Scope::Private,
        Some("--both") => Scope::Both,
        _ => return (None, args),
    };
    (Some(scope), &args[1..])
}

/// Commands that touch the working tree or the index the same way in both
/// repositories, and so are worth running twice by default. Everything
/// else describes *history*, which the two repositories have every right
/// to disagree about, so it goes to the public one unless asked otherwise
/// — that way a bare `ppgit log` shows what a bare `git log` would.
///
/// `commit` isn't here even though it also defaults to both: it has its
/// own carve-out in `resolve_scope` (`commit_must_run_on_both`) alongside
/// the branch commands below, since — unlike everything in this list — it
/// must refuse being narrowed, not just default away from it.
const DUAL_BY_DEFAULT: &[&str] = &["add", "status", "rm", "mv", "restore", "push", "fetch"];

/// Commands that work on branch *names*. The two repositories must always
/// agree about which branches exist and which one is checked out — they
/// share a working tree, so a branch that exists in only one of them means
/// the next commit lands somewhere its counterpart can't follow. These are
/// always run against both, and asking for a single one is refused rather
/// than quietly allowed to drift them apart.
///
/// This is about names, not history: the two repositories hold genuinely
/// different commits (a private-only change makes one where the other has
/// none), so anything addressing a specific commit — `rebase`,
/// `cherry-pick`, `reset <sha>` — can't be mirrored and isn't listed here.
const BRANCH_COMMANDS: &[&str] = &["branch", "checkout", "switch", "merge"];

fn subcommand(args: &[OsString]) -> Option<&str> {
    args.first().and_then(|arg| arg.to_str())
}

/// `checkout` is two commands wearing one name: it switches branches, but
/// with a `--` it restores paths instead, which is an ordinary file
/// operation and none of the branch rule's business. Nothing else in
/// `BRANCH_COMMANDS` has such a form.
fn is_path_checkout(args: &[OsString]) -> bool {
    subcommand(args) == Some("checkout") && args.iter().any(|arg| arg.to_str() == Some("--"))
}

fn manipulates_branches(args: &[OsString]) -> bool {
    subcommand(args).is_some_and(|name| BRANCH_COMMANDS.contains(&name)) && !is_path_checkout(args)
}

/// Settles which repositories a command runs against: the explicit flag if
/// there was one, the command itself otherwise. Fails when a scope flag
/// was given for a branch command, which must not be narrowed, and equally
/// for `commit` — see `commit_must_run_on_both` below.
pub fn resolve_scope(explicit: Option<Scope>, args: &[OsString]) -> Result<Scope, ExitCode> {
    if manipulates_branches(args) {
        if matches!(explicit, Some(Scope::Public) | Some(Scope::Private)) {
            let name = subcommand(args).unwrap_or("that");
            eprintln!("ppgit: `{name}` can only run on both repositories at once");
            eprintln!(
                "  They share one working tree, so their branches have to match. A branch\n  \
                 in only one of them — or the two sitting on different branches — means\n  \
                 the next commit goes somewhere the other cannot follow.\n  \
                 Re-run without --public/--private."
            );
            return Err(ExitCode::FAILURE);
        }
        return Ok(Scope::Both);
    }

    if commit_must_run_on_both(args) {
        if matches!(explicit, Some(Scope::Public) | Some(Scope::Private)) {
            eprintln!("ppgit: `commit` can only run on both repositories at once");
            eprintln!(
                "  A commit scoped to one half can never be paired with its counterpart,\n  \
                 and that pairing is what later lets ppgit route reset/cherry-pick/rebase\n  \
                 by shared vs. private-only commits. Re-run without --public/--private —\n  \
                 a plain `ppgit commit` already does nothing on a half with nothing staged,\n  \
                 which covers the case a narrow commit was ever used for."
            );
            return Err(ExitCode::FAILURE);
        }
        return Ok(Scope::Both);
    }

    Ok(explicit.unwrap_or(match subcommand(args) {
        Some(name) if DUAL_BY_DEFAULT.contains(&name) => Scope::Both,
        _ => Scope::Public,
    }))
}

/// `commit` is meant to be the only door through which a *paired* commit
/// is created — one made of both halves committing together, in the same
/// invocation. A commit made instead by two separate narrow invocations —
/// `--private commit` now, `--public commit` later — has no single moment
/// where the two could be tied together, no matter how related they
/// actually are, so scoping is refused here the same way branch commands
/// refuse it above.
fn commit_must_run_on_both(args: &[OsString]) -> bool {
    subcommand(args) == Some("commit")
}

/// Whether this is a `push` — the one command that has to be stopped
/// while private files are still tracked publicly, since it's the point
/// of no return: everything else stays on this machine. Checked here
/// rather than inside `commands::push` because this gate runs before
/// `Builtin` dispatch even happens (see `lib::run`).
pub fn is_push(args: &[OsString]) -> bool {
    args.first().is_some_and(|arg| arg.to_str() == Some("push"))
}
