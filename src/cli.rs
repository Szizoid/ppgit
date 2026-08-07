use std::ffi::OsString;
use std::process::ExitCode;

/// Commands ppgit handles itself instead of just forwarding. Some are its
/// own (`init`), some are git's but need orchestrating across the two
/// repositories (`commit`, whose editor must open once, not once each).
pub enum Builtin {
    Help,
    Version,
    Init,
    Commit,
}

pub fn recognize(args: &[OsString]) -> Option<Builtin> {
    if args.is_empty() {
        return Some(Builtin::Help);
    }
    match args[0].to_str() {
        Some("help") | Some("--help") | Some("-h") => Some(Builtin::Help),
        Some("--version") | Some("-V") => Some(Builtin::Version),
        Some("init") => Some(Builtin::Init),
        Some("commit") => Some(Builtin::Commit),
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
const DUAL_BY_DEFAULT: &[&str] = &[
    "add", "commit", "status", "rm", "mv", "restore", "push", "pull", "fetch",
];

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
/// was given for a branch command, which must not be narrowed.
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

    Ok(explicit.unwrap_or(match subcommand(args) {
        Some(name) if DUAL_BY_DEFAULT.contains(&name) => Scope::Both,
        _ => Scope::Public,
    }))
}

/// Whether this is a `push`. Not a `Builtin` — push is still forwarded to
/// git untouched — but it's the one command that has to be stopped while
/// private files are still tracked publicly, since it's the point of no
/// return: everything else stays on this machine.
pub fn is_push(args: &[OsString]) -> bool {
    args.first().is_some_and(|arg| arg.to_str() == Some("push"))
}
