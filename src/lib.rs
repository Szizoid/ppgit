use std::env;
use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::{self, ErrorKind, Read, Write};
use std::process::{Command, ExitCode};

#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;

enum Builtin {
    Help,
    Version,
    Init,
}

fn recognize(args: &[OsString]) -> Option<Builtin> {
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

fn cmd_help() {
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
}

fn cmd_version() {
    println!("ppgit {}", env!("CARGO_PKG_VERSION"));
}

/// Runs `git <args>`, inheriting stdio. `Ok(())` only if git itself was
/// launched *and* exited successfully — any other outcome is turned into the
/// `ExitCode` that `cmd_init` should return.
fn run_git_checked(args: &[&str]) -> Result<(), ExitCode> {
    match Command::new("git").args(args).status() {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(match status.code() {
            Some(code) => ExitCode::from(code as u8),
            None => ExitCode::FAILURE,
        }),
        Err(e) => {
            eprintln!("ppgit: failed to launch git: {e}");
            Err(ExitCode::FAILURE)
        }
    }
}

const PPGITIGNORE_TEMPLATE: &str = "# List the files or directories below that should be excluded from the\n\
                                     # public repository and kept only in the private one.\n";

/// Creates `.ppgitignore` with a template comment, unless it already exists
/// (in which case it's left untouched — `ppgit init` must be idempotent).
fn create_ppgitignore_template() -> io::Result<()> {
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(".ppgitignore")
    {
        Ok(mut file) => file.write_all(PPGITIGNORE_TEMPLATE.as_bytes()),
        Err(e) if e.kind() == ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(e),
    }
}

const PPGIT_GITIGNORE_MARKER: &str = "# ppgit";

fn exclusions_exists(file: &mut File) -> io::Result<bool> {
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;
    Ok(contents.contains(PPGIT_GITIGNORE_MARKER))
}

fn write_ppgit_exclusions(file: &mut File) -> io::Result<()> {
    file.write_all(format!("\n{PPGIT_GITIGNORE_MARKER}\n/.ppgit/\n/.ppgitignore\n").as_bytes())
}

fn cmd_init() -> ExitCode {
    if let Err(code) = run_git_checked(&["init"]) {
        return code;
    }

    if let Err(e) = create_ppgitignore_template() {
        eprintln!("ppgit: failed to create .ppgitignore: {e}");
        return ExitCode::FAILURE;
    }

    if let Err(code) = run_git_checked(&["init", "--bare", ".ppgit"]) {
        return code;
    }

    // .read(true) so exclusions_exists can check the current content,
    // .append(true) so any write always lands at the end regardless of
    // where the read cursor ended up, .create(true) so this same call
    // handles both "already exists" and "doesn't exist yet".
    let mut gitignore = match OpenOptions::new()
        .read(true)
        .append(true)
        .create(true)
        .open(".gitignore")
    {
        Ok(file) => file,
        Err(e) => {
            eprintln!("ppgit: failed to open .gitignore: {e}");
            return ExitCode::FAILURE;
        }
    };

    match exclusions_exists(&mut gitignore) {
        Ok(true) => {}
        Ok(false) => {
            if let Err(e) = write_ppgit_exclusions(&mut gitignore) {
                eprintln!("ppgit: failed to write to .gitignore: {e}");
                return ExitCode::FAILURE;
            }
        }
        Err(e) => {
            eprintln!("ppgit: failed to read .gitignore: {e}");
            return ExitCode::FAILURE;
        }
    }

    ExitCode::SUCCESS
}

fn to_git(args: Vec<OsString>) -> ExitCode {
    let mut git = Command::new("git");

    match git.args(args).status() {
        Ok(status) => {
            if !status.success() {
                if let Some(code) = status.code() {
                    return ExitCode::from(code as u8);
                }

                #[cfg(unix)]
                if let Some(signal) = status.signal() {
                    eprintln!("ppgit: git was terminated by signal {signal}");
                    return ExitCode::from(128 + signal as u8);
                }

                eprintln!("ppgit: git terminated abnormally, no exit code available");
                return ExitCode::FAILURE;
            } else {
                return ExitCode::SUCCESS;
            }
        }
        Err(e) => {
            eprintln!("ppgit: failed to launch git: {e}");
            return ExitCode::FAILURE;
        }
    }
}

pub fn run() -> ExitCode {
    let args: Vec<OsString> = env::args_os().skip(1).collect();

    match recognize(&args) {
        Some(builtin) => match builtin {
            Builtin::Help => {
                cmd_help();
                return ExitCode::SUCCESS;
            }
            Builtin::Version => {
                cmd_version();
                return ExitCode::SUCCESS;
            }
            Builtin::Init => cmd_init(),
        },
        None => to_git(args),
    }
}
