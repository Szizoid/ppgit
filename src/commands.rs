use std::process::ExitCode;

pub mod clone;
pub mod commit;
pub mod doctor;
pub mod init;

pub fn cmd_help() -> ExitCode {
    println!(
        "ppgit {version} — passthrough wrapper around git

Usage:
ppgit <git-command> [args...]
pp <git-command> [args...]      (alias for ppgit)

Every command is forwarded to git as-is — `ppgit status` is exactly
`git status`. Run `git --help` for git's own commands and flags.

ppgit's own commands:
ppgit init                      set up both halves of a project here
ppgit clone <repo> [<dir>]      set up both halves from GitHub
ppgit doctor                    check the two halves are in step

{repo}",
        version = env!("CARGO_PKG_VERSION"),
        repo = env!("CARGO_PKG_REPOSITORY"),
    );
    ExitCode::SUCCESS
}

pub fn cmd_version() -> ExitCode {
    println!("ppgit {}", env!("CARGO_PKG_VERSION"));
    ExitCode::SUCCESS
}
