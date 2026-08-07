use std::ffi::OsString;

pub enum Builtin {
    Help,
    Version,
    Init,
}

pub fn recognize(args: &[OsString]) -> Option<Builtin> {
    if args.is_empty() {
        return Some(Builtin::Help);
    }
    match args[0].to_str() {
        Some("help") | Some("--help") | Some("-h") => Some(Builtin::Help),
        Some("--version") | Some("-V") => Some(Builtin::Version),
        Some("init") => Some(Builtin::Init),
        _ => None,
    }
}

/// Whether this is a `push`. Not a `Builtin` — push is still forwarded to
/// git untouched — but it's the one command that has to be stopped while
/// private files are still tracked publicly, since it's the point of no
/// return: everything else stays on this machine.
pub fn is_push(args: &[OsString]) -> bool {
    args.first().is_some_and(|arg| arg.to_str() == Some("push"))
}
