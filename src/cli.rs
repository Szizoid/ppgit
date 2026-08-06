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
