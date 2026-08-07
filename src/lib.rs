mod cli;
mod commands;
mod exec;
mod gh;
mod ppgitignore;

use std::env;
use std::ffi::OsString;
use std::process::ExitCode;

use cli::{Builtin, is_push, recognize};
use commands::{cmd_help, cmd_version, init::cmd_init};
use exec::to_git;
use ppgitignore::{PPGITIGNORE, sync_excludes, tracked_but_ignored, warn_tracked_but_ignored};

pub fn run() -> ExitCode {
    let args: Vec<OsString> = env::args_os().skip(1).collect();

    // Before anything else, so an edit to .ppgitignore takes effect on the
    // very next command. Hard failure on purpose: if the private paths
    // can't be hidden, letting a `git add` run anyway risks leaking them.
    if let Err(e) = sync_excludes() {
        eprintln!("ppgit: failed to sync exclude files: {e}");
        return ExitCode::FAILURE;
    }

    match tracked_but_ignored() {
        Ok(conflicts) if !conflicts.is_empty() => {
            warn_tracked_but_ignored(&conflicts);
            if is_push(&args) {
                eprintln!("ppgit: refusing to push until the above is resolved");
                return ExitCode::FAILURE;
            }
        }
        Ok(_) => {}
        // A failed *check* only costs a warning — unlike a failed sync,
        // it doesn't itself make a leak more likely, and dying here would
        // block ordinary work for no gain.
        Err(e) => eprintln!("ppgit: warning: could not check for {PPGITIGNORE} conflicts: {e}"),
    }

    match recognize(&args) {
        Some(builtin) => match builtin {
            Builtin::Help => cmd_help(),
            Builtin::Version => cmd_version(),
            Builtin::Init => cmd_init(),
        },
        None => to_git(&args),
    }
}
