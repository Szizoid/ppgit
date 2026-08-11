use std::collections::HashSet;
use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::process::ExitCode;

use crate::exec::{io_checked, run_quiet_stdout};
use crate::ppgitignore::{
    PPGITIGNORE, PRIVATE_GIT_DIR, PUBLIC_GIT_DIR, read_or_empty, sync_excludes,
};

/// Paths that must never be listed: the git-dirs aren't subject to the
/// split, the list itself is already private by construction, and `.`
/// would privatize the entire project in one keystroke.
const REFUSED: &[&str] = &[".", "..", "/", PUBLIC_GIT_DIR, PRIVATE_GIT_DIR, PPGITIGNORE];

fn usage(command: &str, blurb: &str) -> ExitCode {
    eprintln!("usage: ppgit {command} <path>...\n\n{blurb}");
    ExitCode::FAILURE
}

/// The paths from everything after the subcommand: no options, at least
/// one path, UTF-8 only (they're going into a text file), and none of
/// the refused special names. A leading `./` is dropped so the listed
/// line matches what git and the user both call the path.
fn parse(args: &[OsString]) -> Option<Vec<String>> {
    let mut paths = Vec::new();
    for arg in &args[1..] {
        let arg = arg.to_str()?;
        if arg.starts_with('-') || arg.is_empty() {
            return None;
        }
        let path = arg.strip_prefix("./").unwrap_or(arg);
        if REFUSED.contains(&path.trim_end_matches('/')) {
            eprintln!("ppgit: refusing to work on {path}");
            return None;
        }
        paths.push(path.to_string());
    }
    if paths.is_empty() { None } else { Some(paths) }
}

fn ensure_project() -> Result<(), ExitCode> {
    if Path::new(PRIVATE_GIT_DIR).is_dir() {
        return Ok(());
    }
    eprintln!("ppgit: not a ppgit project — no {PRIVATE_GIT_DIR} here");
    eprintln!("  `ppgit init` sets a project up, `ppgit clone` fetches an existing one.");
    Err(ExitCode::FAILURE)
}

/// The publicly tracked files a set of pathspecs matches — what
/// `privatize` is about to untrack, asked of git rather than matched by
/// hand.
fn tracked_publicly(paths: &[String]) -> Result<Vec<String>, ExitCode> {
    let mut args = vec!["ls-files", "--"];
    args.extend(paths.iter().map(String::as_str));
    let listing = io_checked(
        run_quiet_stdout("git", &args),
        "list publicly tracked files",
    )?;
    Ok(listing.lines().map(str::to_string).collect())
}

pub fn cmd_privatize(args: &[OsString]) -> ExitCode {
    match try_privatize(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(code) => code,
    }
}

fn try_privatize(args: &[OsString]) -> Result<(), ExitCode> {
    ensure_project()?;
    let Some(paths) = parse(args) else {
        return Err(usage(
            "privatize",
            "Lists each path in .ppgitignore and untracks it from the public\n\
             repository, keeping the file on disk and in the private one.",
        ));
    };

    let existing = io_checked(read_or_empty(PPGITIGNORE), &format!("read {PPGITIGNORE}"))?;
    let listed: HashSet<&str> = existing.lines().map(str::trim).collect();

    let mut updated = existing.clone();
    for path in &paths {
        if listed.contains(path.as_str()) {
            println!("ppgit: {path} is already listed in {PPGITIGNORE}");
            continue;
        }
        if !updated.is_empty() && !updated.ends_with('\n') {
            updated.push('\n');
        }
        updated.push_str(path);
        updated.push('\n');
        println!("ppgit: listed {path} in {PPGITIGNORE}");
    }
    if updated != existing {
        io_checked(
            fs::write(PPGITIGNORE, &updated),
            &format!("update {PPGITIGNORE}"),
        )?;
    }
    io_checked(sync_excludes(), "sync exclude files")?;

    // Listing only hides a path while it's untracked; anything already
    // committed publicly has to be untracked too, or it keeps going out
    // with every push. `--cached` leaves the work tree (and the private
    // repository) alone.
    let tracked = tracked_publicly(&paths)?;
    if tracked.is_empty() {
        return Ok(());
    }

    let mut rm = vec!["rm", "-r", "--cached", "--ignore-unmatch", "--quiet", "--"];
    rm.extend(paths.iter().map(String::as_str));
    io_checked(
        run_quiet_stdout("git", &rm),
        "untrack from the public repository",
    )?;

    println!(
        "ppgit: untracked {} file(s) from the public repository:",
        tracked.len()
    );
    for path in &tracked {
        println!("    {path}");
    }
    println!("  The files stay on disk and in the private repository. The removal is");
    println!("  staged; publish it so the next push stops sending them out:");
    println!("    pp commit");
    Ok(())
}

pub fn cmd_publicize(args: &[OsString]) -> ExitCode {
    match try_publicize(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(code) => code,
    }
}

fn try_publicize(args: &[OsString]) -> Result<(), ExitCode> {
    ensure_project()?;
    let Some(paths) = parse(args) else {
        return Err(usage(
            "publicize",
            "Removes each path from .ppgitignore, so the public repository can\n\
             track it again.",
        ));
    };

    let existing = io_checked(read_or_empty(PPGITIGNORE), &format!("read {PPGITIGNORE}"))?;

    // All-or-nothing on purpose: a typo among several paths shouldn't
    // leave the list half-edited.
    for path in &paths {
        if !existing.lines().any(|line| line.trim() == path) {
            eprintln!("ppgit: {path} is not listed in {PPGITIGNORE}");
            return Err(ExitCode::FAILURE);
        }
    }

    let updated: String = existing
        .lines()
        .filter(|line| !paths.iter().any(|path| line.trim() == path))
        .map(|line| format!("{line}\n"))
        .collect();
    io_checked(
        fs::write(PPGITIGNORE, &updated),
        &format!("update {PPGITIGNORE}"),
    )?;
    io_checked(sync_excludes(), "sync exclude files")?;

    for path in &paths {
        println!("ppgit: unlisted {path} from {PPGITIGNORE}");
    }
    println!("  The public repository can see the path(s) now, but doesn't track them");
    println!("  yet. To publish:");
    println!("    pp add -- <path> && pp commit");
    Ok(())
}
