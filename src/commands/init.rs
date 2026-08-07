use std::fs::OpenOptions;
use std::io::{self, ErrorKind, Write};
use std::path::Path;
use std::process::ExitCode;

use crate::exec::{PRIVATE_GIT_ARG, WORK_TREE_ARG, io_checked, run_loud_checked, run_quiet_ok};
use crate::gh::{PRIVATE_NAME_PREFIX, ensure_gh_ready, repo_create, repo_exists, repo_url};
use crate::ppgitignore::{PPGITIGNORE, PRIVATE_GIT_DIR, PUBLIC_GIT_DIR, sync_excludes};

const PPGITIGNORE_TEMPLATE: &str = "# List the files or directories below that should be excluded from the\n\
                                     # public repository and kept only in the private one.\n";

/// Creates `.ppgitignore` with a template comment, unless it already exists
/// (in which case it's left untouched — `ppgit init` must be idempotent).
fn create_ppgitignore_template() -> io::Result<()> {
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(PPGITIGNORE)
    {
        Ok(mut file) => file.write_all(PPGITIGNORE_TEMPLATE.as_bytes()),
        Err(e) if e.kind() == ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(e),
    }
}

/// Creates the private git-dir if it isn't there yet.
///
/// When the public repository already has history, the private one starts
/// as a bare clone of it rather than empty. It's meant to be a superset,
/// and starting from nothing would leave every already-committed file
/// looking brand new to it: the first private commit would duplicate the
/// entire project with none of the history that produced it, and the two
/// halves would share no ancestry at all.
fn ensure_private_repo() -> Result<(), ExitCode> {
    if Path::new(PRIVATE_GIT_DIR).is_dir() {
        return Ok(());
    }

    // Nothing to inherit from a public repository without commits, and
    // cloning one only produces a puzzling "you cloned an empty
    // repository" warning.
    if !run_quiet_ok("git", &["rev-parse", "--verify", "HEAD"]) {
        return run_loud_checked("git", ["init", "--bare", PRIVATE_GIT_DIR]);
    }

    run_loud_checked("git", ["clone", "--bare", PUBLIC_GIT_DIR, PRIVATE_GIT_DIR])?;

    // A bare clone points `origin` at the local git-dir it copied; the
    // private repository's origin belongs to its own GitHub remote, which
    // `ensure_remote` sets further down.
    run_loud_checked("git", [PRIVATE_GIT_ARG, "remote", "remove", "origin"])?;

    // It also has no index, which would leave every tracked file looking
    // deleted and untracked at the same time. Filling it from HEAD settles
    // the private half into a clean working state.
    run_loud_checked("git", [PRIVATE_GIT_ARG, WORK_TREE_ARG, "reset", "-q"])
}

/// The public repo's name defaults to the current directory's name — the
/// private one is always that with a `pp-` prefix.
fn project_name() -> io::Result<String> {
    let cwd = std::env::current_dir()?;
    cwd.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .ok_or_else(|| {
            io::Error::other("could not determine a project name from the current directory")
        })
}

/// Creates the GitHub repository `name`, unless it already exists.
fn ensure_repo(name: &str, private: bool) -> Result<(), ExitCode> {
    if repo_exists(name) {
        return Ok(());
    }
    repo_create(name, private)
}

/// Points `origin` in the given git-dir at `url`, unless it's already
/// configured — `git_dir_args` is `&[]` for the public repo (plain `.git`)
/// or `&["--git-dir=.ppgit"]` for the private one.
fn ensure_remote(git_dir_args: &[&str], url: &str) -> Result<(), ExitCode> {
    let mut check = git_dir_args.to_vec();
    check.extend(["remote", "get-url", "origin"]);
    if run_quiet_ok("git", &check) {
        return Ok(());
    }

    let mut add = git_dir_args.to_vec();
    add.extend(["remote", "add", "origin", url]);
    run_loud_checked("git", &add)
}

pub fn cmd_init() -> ExitCode {
    match try_init() {
        Ok(()) => ExitCode::SUCCESS,
        Err(code) => code,
    }
}

/// Spares the user a `--set-upstream` on the first push of every branch.
/// Both repositories have exactly one remote each, so there's never any
/// ambiguity about what a new branch should track.
pub(crate) fn ensure_auto_upstream(git_dir_args: &[&str]) -> Result<(), ExitCode> {
    let mut args = git_dir_args.to_vec();
    args.extend(["config", "push.autoSetupRemote", "true"]);
    run_loud_checked("git", &args)
}

fn try_init() -> Result<(), ExitCode> {
    run_loud_checked("git", ["init"])?;
    io_checked(create_ppgitignore_template(), "create .ppgitignore")?;
    ensure_private_repo()?;
    io_checked(sync_excludes(), "sync exclude files")?;

    ensure_auto_upstream(&[])?;
    ensure_auto_upstream(&[PRIVATE_GIT_ARG])?;

    let public_name = io_checked(project_name(), "determine project name")?;
    let private_name = format!("{PRIVATE_NAME_PREFIX}{public_name}");

    ensure_gh_ready()?;
    ensure_repo(&public_name, false)?;
    ensure_repo(&private_name, true)?;

    let public_url = io_checked(repo_url(&public_name), "get public repo URL")?;
    let private_url = io_checked(repo_url(&private_name), "get private repo URL")?;

    ensure_remote(&[], &public_url)?;
    ensure_remote(&[PRIVATE_GIT_ARG], &private_url)?;

    Ok(())
}
