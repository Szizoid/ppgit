//! Dual `mv`. Two plain `git mv`s can't work: `git mv` moves the file on
//! disk as well as in the index, and both halves share one working tree,
//! so once the private half has run it the source is gone and the public
//! `git mv` dies with "bad source" — leaving the rename staged privately
//! and an unstaged delete + untracked file publicly.
//!
//! Same shape as `checkout`: the real `git mv` only ever runs on the
//! private (superset) half, and the public half is brought in line with
//! index-only operations — `update-index --force-remove` for each old
//! path, `update-index --add --cacheinfo` re-adding the public half's own
//! entry (mode and blob, so whatever was staged there stays staged) under
//! the new path. Verified in scratch to stage exactly the rename `git mv`
//! would have, without touching the working tree.
//!
//! Which old path became which new one is predicted from the arguments by
//! git's own rule (a single source onto a path that isn't a directory is
//! renamed to it; otherwise each source lands inside the destination
//! under its own name), then checked against the private index after the
//! move — so `-n`, `-k` skipping a source, or a prediction that's wrong
//! for some exotic case all end up mirroring nothing rather than
//! mirroring something that didn't happen. Only publicly tracked paths
//! are mirrored at all: a private-only file has nothing to move publicly.

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::Path;
use std::process::ExitCode;

use crate::announce;
use crate::cli::Scope;
use crate::exec::{PRIVATE_GIT_PREFIX, PUBLIC_GIT_PREFIX, io_checked, run_quiet_stdout, to_git};
use crate::ppgitignore::{PRIVATE_GIT_DIR, PUBLIC_GIT_DIR};

pub fn cmd_mv(scope: Scope, args: &[OsString]) -> ExitCode {
    match scope {
        Scope::Public => to_git(PUBLIC_GIT_PREFIX, args),
        Scope::Private => to_git(PRIVATE_GIT_PREFIX, args),
        Scope::Both if !Path::new(PRIVATE_GIT_DIR).is_dir() => to_git(PUBLIC_GIT_PREFIX, args),
        Scope::Both => match try_mv(args) {
            Ok(code) | Err(code) => code,
        },
    }
}

fn try_mv(args: &[OsString]) -> Result<ExitCode, ExitCode> {
    let Some(paths) = paths(args) else {
        eprintln!("ppgit: mv can only be mirrored onto both halves for UTF-8 paths");
        eprintln!("  Run it with --private, then bring the public half in step by hand.");
        return Err(ExitCode::FAILURE);
    };
    // Fewer than two paths is a usage error git reports better than
    // ppgit could; the private run below fails with it, nothing moves.
    let renames = match paths.split_last() {
        Some((destination, sources)) if !sources.is_empty() => {
            let public = io_checked(
                index_entries(PUBLIC_GIT_PREFIX, sources),
                "list publicly tracked files",
            )?;
            predict(&public, sources, destination)
        }
        _ => Vec::new(),
    };

    announce(PRIVATE_GIT_DIR);
    let private = to_git(PRIVATE_GIT_PREFIX, args);
    if private != ExitCode::SUCCESS || renames.is_empty() {
        return Ok(private);
    }

    announce(PUBLIC_GIT_DIR);
    let moved = io_checked(confirmed(&renames), "read the private index")?;
    if let Err(e) = mirror(&moved) {
        eprintln!("ppgit: failed to mirror the move onto the public half: {e}");
        eprintln!("  The private half has the rename staged; the public half doesn't.");
        eprintln!("  `pp status` shows what's out of step.");
        return Err(ExitCode::FAILURE);
    }
    if moved.len() < renames.len() {
        warn_unmirrored(&renames, &moved)?;
    }
    Ok(ExitCode::SUCCESS)
}

/// A publicly tracked file's move: its old path, its new path, and its
/// public index entry as `<mode>,<blob>` — the form `--cacheinfo` takes.
struct Rename {
    from: String,
    to: String,
    entry: String,
}

/// Every non-option argument, in order — the sources followed by the
/// destination. None of `git mv`'s options takes a value, and git lets
/// them sit anywhere on the line, so anything dash-led before a `--` is
/// an option and everything else a path. `None` for a non-UTF-8 path.
fn paths(args: &[OsString]) -> Option<Vec<String>> {
    let mut paths = Vec::new();
    let mut options_done = false;
    for arg in &args[1..] {
        let arg = arg.to_str()?;
        if !options_done && arg == "--" {
            options_done = true;
        } else if options_done || !arg.starts_with('-') {
            paths.push(normalise(arg));
        }
    }
    Some(paths)
}

/// The spelling git itself uses for a path: no leading `./`, no trailing
/// `/` — so a prefix comparison against `ls-files` output works.
fn normalise(path: &str) -> String {
    let mut path = path;
    while let Some(rest) = path.strip_prefix("./") {
        path = rest;
    }
    let path = path.trim_end_matches('/');
    if path.is_empty() {
        ".".to_string()
    } else {
        path.to_string()
    }
}

/// Where each publicly tracked file under `sources` will end up, by git's
/// rule: one source onto a destination that isn't a directory is renamed
/// to it; otherwise every source moves *into* the destination, keeping
/// its own name. A file inside a source directory keeps its path below
/// that directory. Checked against the real outcome afterwards (see
/// `confirmed`), so this only has to be right, not exhaustive.
fn predict(public: &HashMap<String, String>, sources: &[String], destination: &str) -> Vec<Rename> {
    // lstat, like git: a symlink to a directory is a path to rename onto,
    // not a directory to move into.
    let into_directory =
        sources.len() > 1 || fs::symlink_metadata(destination).is_ok_and(|meta| meta.is_dir());

    let mut renames = Vec::new();
    for source in sources {
        let target = if into_directory {
            let name = Path::new(source)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(source);
            if destination == "." {
                name.to_string()
            } else {
                format!("{destination}/{name}")
            }
        } else {
            destination.to_string()
        };

        for (path, entry) in public {
            let below = if path == source {
                ""
            } else if let Some(rest) = path.strip_prefix(&format!("{source}/")) {
                rest
            } else {
                continue;
            };
            let to = if below.is_empty() {
                target.clone()
            } else {
                format!("{target}/{below}")
            };
            renames.push(Rename {
                from: path.clone(),
                to,
                entry: entry.clone(),
            });
        }
    }
    renames
}

/// The renames the private `git mv` actually carried out: the old path
/// gone from its index, the new one present. Whatever it didn't do —
/// `-n`, a source `-k` skipped — falls out here.
fn confirmed(renames: &[Rename]) -> io::Result<Vec<&Rename>> {
    let paths: Vec<String> = renames
        .iter()
        .flat_map(|rename| [rename.from.clone(), rename.to.clone()])
        .collect();
    let private = index_entries(PRIVATE_GIT_PREFIX, &paths)?;
    Ok(renames
        .iter()
        .filter(|rename| !private.contains_key(&rename.from) && private.contains_key(&rename.to))
        .collect())
}

/// The public half of the rename, in its index only: the old entries
/// dropped, the same entries added back under their new paths. Neither
/// `update-index` form looks at the working tree, which the private
/// `git mv` has already rearranged.
fn mirror(moved: &[&Rename]) -> io::Result<()> {
    if moved.is_empty() {
        return Ok(());
    }

    let mut remove = vec!["update-index", "--force-remove", "--"];
    remove.extend(moved.iter().map(|rename| rename.from.as_str()));
    run_quiet_stdout("git", &remove)?;

    let cacheinfo: Vec<String> = moved
        .iter()
        .map(|rename| format!("{},{}", rename.entry, rename.to))
        .collect();
    let mut add = vec!["update-index", "--add"];
    for info in &cacheinfo {
        add.extend(["--cacheinfo", info.as_str()]);
    }
    run_quiet_stdout("git", &add)?;
    Ok(())
}

/// Reports publicly tracked files the private half no longer tracks at
/// their old path but that weren't mirrored — a move the prediction got
/// wrong. Files `-n` or `-k` left alone are still where they were, and
/// aren't worth a word.
fn warn_unmirrored(renames: &[Rename], moved: &[&Rename]) -> Result<(), ExitCode> {
    let missed: Vec<String> = renames
        .iter()
        .filter(|rename| !moved.iter().any(|done| done.from == rename.from))
        .map(|rename| rename.from.clone())
        .collect();
    let private = io_checked(
        index_entries(PRIVATE_GIT_PREFIX, &missed),
        "read the private index",
    )?;
    let lost: Vec<&String> = missed
        .iter()
        .filter(|path| !private.contains_key(*path))
        .collect();
    if lost.is_empty() {
        return Ok(());
    }

    eprintln!("ppgit: warning: these moved privately but couldn't be mirrored publicly:");
    for path in &lost {
        eprintln!("    {path}");
    }
    eprintln!("  Stage the public side by hand: `pp --public rm --cached <old>` and");
    eprintln!("  `pp --public add <new>`.");
    Ok(())
}

/// Index entries under `paths` in one half, path → `<mode>,<blob>`.
/// Literal pathspecs, since a file name may contain glob characters; a
/// directory still matches everything tracked below it.
fn index_entries(prefix: &[&str], paths: &[String]) -> io::Result<HashMap<String, String>> {
    if paths.is_empty() {
        return Ok(HashMap::new());
    }
    let mut args = prefix.to_vec();
    args.extend(["--literal-pathspecs", "ls-files", "-s", "-z", "--"]);
    args.extend(paths.iter().map(String::as_str));
    let listing = run_quiet_stdout("git", &args)?;
    Ok(listing
        .split('\0')
        .filter(|record| !record.is_empty())
        .filter_map(|record| {
            // `<mode> <blob> <stage>\t<path>`, the path literal under -z.
            let (meta, path) = record.split_once('\t')?;
            let mut fields = meta.split_whitespace();
            let (mode, blob) = (fields.next()?, fields.next()?);
            Some((path.to_string(), format!("{mode},{blob}")))
        })
        .collect())
}
