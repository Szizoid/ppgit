use std::env;
use std::ffi::{OsStr, OsString};
use std::io;
use std::path::Path;
use std::process::ExitCode;

use crate::announce;
use crate::commands::init::ensure_auto_upstream;
use crate::exec::{PRIVATE_GIT_ARG, WORK_TREE_ARG, io_checked, run_loud_checked, run_quiet_stdout};
use crate::gh::{
    PRIVATE_NAME_PREFIX, RepoIdentity, ensure_gh_ready, repo_exists, repo_identity, repo_url,
};
use crate::ppgitignore::{PRIVATE_GIT_DIR, PUBLIC_GIT_DIR, sync_excludes};

/// Works out the two repositories a ppgit project is made of, given either
/// one of them. A *private* repository whose name starts with `pp-` is
/// taken to be the private half; anything else is taken for the public
/// one, whose counterpart is that name with `pp-` in front.
///
/// The visibility check is what makes this safe: a public repository
/// legitimately called `pp-something` is its own project's public half,
/// not somebody's private half, and going by the name alone would send
/// `clone` hunting for a `pp-pp-something` that was never meant to exist.
fn halves(identity: &RepoIdentity) -> io::Result<(String, String)> {
    let (owner, name) = identity.name_with_owner.rsplit_once('/').ok_or_else(|| {
        io::Error::other(format!(
            "gh returned a repository name without an owner: {}",
            identity.name_with_owner
        ))
    })?;

    match name.strip_prefix(PRIVATE_NAME_PREFIX) {
        Some(public_name) if identity.is_private => Ok((
            format!("{owner}/{public_name}"),
            identity.name_with_owner.clone(),
        )),
        _ => Ok((
            identity.name_with_owner.clone(),
            format!("{owner}/{PRIVATE_NAME_PREFIX}{name}"),
        )),
    }
}

/// Splits the command line into the repository and the optional
/// destination directory. Options are refused rather than forwarded:
/// `git clone`'s flags mostly concern a single repository's layout
/// (`--bare`, `--separate-git-dir`, `--depth`), and ppgit has definite
/// plans for that layout in both halves.
fn parse(args: &[OsString]) -> Option<(&OsString, Option<&OsString>)> {
    if args
        .iter()
        .any(|arg| arg.as_encoded_bytes().starts_with(b"-"))
    {
        return None;
    }

    match args {
        [repo] => Some((repo, None)),
        [repo, directory] => Some((repo, Some(directory))),
        _ => None,
    }
}

fn usage() -> ExitCode {
    eprintln!(
        "usage: ppgit clone <repository> [<directory>]

Sets up both halves of a ppgit project in one working directory. The
repository can be named any way `gh` accepts one — `name`, `owner/name`
or a URL — and can be either half: ppgit finds the other itself.

Takes none of `git clone`'s options."
    );
    ExitCode::FAILURE
}

/// Where the clone lands: the directory the user named, or the public
/// repository's own name. Whether that already exists is left for git to
/// judge — it refuses a non-empty destination itself, and says so better
/// than ppgit could.
fn destination_dir(explicit: Option<&OsString>, public: &str) -> OsString {
    match explicit {
        Some(directory) => directory.clone(),
        None => OsString::from(public.rsplit('/').next().unwrap_or(public)),
    }
}

/// The branch a git-dir has checked out.
fn current_branch(git_dir_args: &[&str]) -> io::Result<String> {
    let mut args = git_dir_args.to_vec();
    args.extend(["symbolic-ref", "--short", "HEAD"]);
    run_quiet_stdout("git", &args)
}

/// Clones the private half into the bare `.ppgit` git-dir and settles it
/// into a working state. `git clone --bare` gets a repository that is
/// only *nearly* right, and two things have to be put back by hand:
///
/// - it configures no fetch refspec at all (the whole local config is one
///   `remote.origin.url` line), so nothing would ever land in
///   `refs/remotes/origin/*` and no branch would have an upstream: `pull`
///   is left guessing and `status` can never report ahead/behind;
/// - it leaves no index, so every tracked file reads as deleted, and the
///   first `pull` refuses to run over the "local changes" that implies.
fn clone_private(url: &str) -> Result<(), ExitCode> {
    run_loud_checked("git", ["clone", "--bare", url, PRIVATE_GIT_DIR])?;

    run_loud_checked(
        "git",
        [
            PRIVATE_GIT_ARG,
            "config",
            "remote.origin.fetch",
            "+refs/heads/*:refs/remotes/origin/*",
        ],
    )?;
    run_loud_checked("git", [PRIVATE_GIT_ARG, "fetch", "--quiet", "origin"])?;

    let branch = io_checked(
        current_branch(&[PRIVATE_GIT_ARG]),
        "read the private repository's branch",
    )?;
    run_loud_checked(
        "git",
        [
            PRIVATE_GIT_ARG,
            "branch",
            "--quiet",
            &format!("--set-upstream-to=origin/{branch}"),
            &branch,
        ],
    )?;

    // Fills the index *and* the work tree from the private HEAD. Safe
    // despite the `--hard`: the public clone made this directory moments
    // ago, so there is nothing of the user's here to overwrite. The
    // private half is the superset, so it is the one entitled to say what
    // the tree holds — this lays the private-only files on top of the
    // subset the public clone just checked out.
    run_loud_checked(
        "git",
        [PRIVATE_GIT_ARG, WORK_TREE_ARG, "reset", "--quiet", "--hard"],
    )
}

/// The two halves are cloned separately, each landing on whichever branch
/// GitHub calls its default. ppgit needs them to agree — they share one
/// working tree — and nothing local can settle a disagreement that arrived
/// from the remotes, so this reports it instead of guessing which was
/// meant.
fn warn_if_branches_differ() {
    let (Ok(public), Ok(private)) = (current_branch(&[]), current_branch(&[PRIVATE_GIT_ARG]))
    else {
        return;
    };
    if public == private {
        return;
    }

    eprintln!("ppgit: warning: the two halves came down on different branches");
    eprintln!("    public ({PUBLIC_GIT_DIR}):   {public}");
    eprintln!("    private ({PRIVATE_GIT_DIR}): {private}");
    eprintln!("  They share one working tree, so ppgit needs them on the same branch.");
    eprintln!("  `pp checkout {private}` puts them in step — the work tree already holds");
    eprintln!("  the private half's files.");
}

pub fn cmd_clone(args: &[OsString]) -> ExitCode {
    match try_clone(&args[1..]) {
        Ok(()) => ExitCode::SUCCESS,
        Err(code) => code,
    }
}

fn try_clone(args: &[OsString]) -> Result<(), ExitCode> {
    let Some((repo, directory)) = parse(args) else {
        return Err(usage());
    };
    let Some(repo) = repo.to_str() else {
        eprintln!("ppgit: repository name is not valid UTF-8");
        return Err(ExitCode::FAILURE);
    };

    ensure_gh_ready()?;

    let identity = io_checked(repo_identity(repo), &format!("look up {repo} on GitHub"))?;
    let (public, private) = io_checked(halves(&identity), "work out the project's two halves")?;

    // One half has just been looked up; whichever it was, the other has to
    // be there too or this isn't a ppgit project.
    let counterpart = if identity.name_with_owner == private {
        &public
    } else {
        &private
    };
    if !repo_exists(counterpart) {
        eprintln!("ppgit: {counterpart} does not exist, or isn't visible to the account");
        eprintln!(
            "  gh is logged in as — so {} has no counterpart.",
            identity.name_with_owner
        );
        eprintln!("  `ppgit clone` sets up a project made by `ppgit init`, which always creates");
        eprintln!("  the pair <name> and {PRIVATE_NAME_PREFIX}<name>. A repository that stands on");
        eprintln!("  its own is `git clone`'s business, not ppgit's.");
        return Err(ExitCode::FAILURE);
    }

    let public_url = io_checked(repo_url(&public), "get the public repository's URL")?;
    let private_url = io_checked(repo_url(&private), "get the private repository's URL")?;

    let destination = destination_dir(directory, &public);

    announce(PUBLIC_GIT_DIR);
    run_loud_checked(
        "git",
        [OsStr::new("clone"), OsStr::new(&public_url), &destination],
    )?;

    // From here on everything addresses `.git` and `.ppgit` by relative
    // path, exactly as every other ppgit command does.
    io_checked(
        env::set_current_dir(&destination),
        "enter the cloned directory",
    )?;

    announce(PRIVATE_GIT_DIR);
    clone_private(&private_url)?;

    io_checked(sync_excludes(), "sync exclude files")?;
    ensure_auto_upstream(&[])?;
    ensure_auto_upstream(&[PRIVATE_GIT_ARG])?;

    warn_if_branches_differ();

    println!(
        "ppgit: cloned {public} and {private} into {}",
        Path::new(&destination).display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(name_with_owner: &str, is_private: bool) -> RepoIdentity {
        RepoIdentity {
            name_with_owner: name_with_owner.to_string(),
            is_private,
        }
    }

    #[test]
    fn derives_the_private_half_from_the_public_one() {
        let (public, private) = halves(&identity("me/proj", false)).unwrap();
        assert_eq!(public, "me/proj");
        assert_eq!(private, "me/pp-proj");
    }

    #[test]
    fn derives_the_public_half_from_the_private_one() {
        let (public, private) = halves(&identity("me/pp-proj", true)).unwrap();
        assert_eq!(public, "me/proj");
        assert_eq!(private, "me/pp-proj");
    }

    /// A public `pp-`-named repository is a project in its own right —
    /// only the private one is a ppgit private half.
    #[test]
    fn treats_a_public_pp_name_as_a_public_half() {
        let (public, private) = halves(&identity("me/pp-proj", false)).unwrap();
        assert_eq!(public, "me/pp-proj");
        assert_eq!(private, "me/pp-pp-proj");
    }

    /// Someone's genuinely private repository that ppgit didn't make is
    /// still a public half as far as `clone` is concerned — it just won't
    /// find the counterpart, which is reported separately.
    #[test]
    fn treats_a_private_repo_without_the_prefix_as_a_public_half() {
        let (public, private) = halves(&identity("me/proj", true)).unwrap();
        assert_eq!(public, "me/proj");
        assert_eq!(private, "me/pp-proj");
    }

    #[test]
    fn defaults_the_directory_to_the_public_name_without_its_owner() {
        assert_eq!(destination_dir(None, "me/proj"), OsString::from("proj"));
    }

    #[test]
    fn prefers_an_explicit_directory() {
        let explicit = OsString::from("elsewhere");
        assert_eq!(destination_dir(Some(&explicit), "me/proj"), explicit);
    }

    #[test]
    fn parses_the_two_positional_forms() {
        let repo = OsString::from("proj");
        let dir = OsString::from("dir");
        assert!(parse(std::slice::from_ref(&repo)).is_some());
        assert!(parse(&[repo.clone(), dir.clone()]).is_some());
    }

    #[test]
    fn refuses_options_and_wrong_arity() {
        let repo = OsString::from("proj");
        let dir = OsString::from("dir");
        assert!(parse(&[]).is_none());
        assert!(parse(&[repo.clone(), dir.clone(), dir.clone()]).is_none());
        assert!(parse(&[OsString::from("--bare"), repo]).is_none());
    }
}
