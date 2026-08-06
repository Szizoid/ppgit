mod cli;
mod commands;
mod exec;
mod gh;

use std::env;
use std::ffi::OsString;
use std::process::ExitCode;

use cli::{Builtin, recognize};
use commands::{cmd_help, cmd_version, init::cmd_init};
use exec::to_git;

pub fn run() -> ExitCode {
    let args: Vec<OsString> = env::args_os().skip(1).collect();

    match recognize(&args) {
        Some(builtin) => match builtin {
            Builtin::Help => cmd_help(),
            Builtin::Version => cmd_version(),
            Builtin::Init => cmd_init(),
        },
        None => to_git(&args),
    }
}
