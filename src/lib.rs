use std::env;
use std::ffi::OsString;
use std::process::Command;

#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;

const HELP_FLAGS: &[&str] = &["help", "--help", "-h"];
const VERSION_FLAGS: &[&str] = &["--version", "-V"];

fn is_help(arg: &OsString) -> bool {
    arg.to_str().is_some_and(|s| HELP_FLAGS.contains(&s))
}

fn is_version(arg: &OsString) -> bool {
    arg.to_str().is_some_and(|s| VERSION_FLAGS.contains(&s))
}

pub fn run() -> i32 {
    let args: Vec<OsString> = env::args_os().skip(1).collect();
    let mut git = Command::new("git");

    if args.is_empty() || is_help(&args[0]) {
        println!(
            "ppgit {version} — passthrough wrapper around git

Usage:
    ppgit <git-command> [args...]
    pp <git-command> [args...]      (alias for ppgit)

Every command is forwarded to git as-is — `ppgit status` is exactly
`git status`. Run `git --help` for git's own commands and flags.

{repo}",
            version = env!("CARGO_PKG_VERSION"),
            repo = env!("CARGO_PKG_REPOSITORY"),
        );
        return 0;
    }

    if is_version(&args[0]) {
        println!("ppgit {}", env!("CARGO_PKG_VERSION"));
        return 0;
    }

    match git.args(args).status() {
        Ok(status) => {
            if !status.success() {
                if let Some(code) = status.code() {
                    return code;
                }

                #[cfg(unix)]
                if let Some(signal) = status.signal() {
                    eprintln!("ppgit: git was terminated by signal {signal}");
                    return 128 + signal;
                }

                eprintln!("ppgit: git terminated abnormally, no exit code available");
                return 1;
            } else {
                return 0;
            }
        }
        Err(e) => {
            eprintln!("ppgit: failed to launch git: {e}");
            return 1;
        }
    }
}
