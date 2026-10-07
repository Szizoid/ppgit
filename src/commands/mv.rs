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
//! for some exotic case all end up acting on nothing rather than on
//! something that didn't happen.
//!
//! A move can also carry a file across the private/public line, and
//! `.ppgitignore` is what decides which side it lands on:
//!
//! - A line naming exactly a moved path (anchored, no glob — see
//!   `rewritten`) follows the move: it's rewritten to the new path, and
//!   the list is staged privately alongside the rename.
//! - A private file that ends up outside every pattern anyway (a bare
//!   name that's been renamed, a listed directory it left, a glob it no
//!   longer matches) is left alone but warned about — that may well be
//!   deliberate, but the next `pp add` would publish it.
//! - A public file moved onto a private path is untracked from the public
//!   half rather than mirrored there, the same as `privatize` would do.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::Path;
use std::process::ExitCode;

use crate::announce;
use crate::cli::Scope;
use crate::exec::{
    PRIVATE_GIT_PREFIX, PUBLIC_GIT_PREFIX, io_checked, run_quiet, run_quiet_stdout, to_git,
};
use crate::ppgitignore::{
    PPGITIGNORE, PRIVATE_GIT_DIR, PUBLIC_GIT_DIR, read_or_empty, sync_excludes,
};

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
    let plan = match paths.split_last() {
        Some((destination, sources)) if !sources.is_empty() => {
            io_checked(plan_move(sources, destination), "plan the move")?
        }
        _ => Plan::default(),
    };

    announce(PRIVATE_GIT_DIR);
    let private = to_git(PRIVATE_GIT_PREFIX, args);
    if private != ExitCode::SUCCESS {
        return Ok(private);
    }
    let moved = io_checked(confirmed(&plan.renames), "read the private index")?;
    if moved.is_empty() {
        return Ok(private);
    }

    // Before the public half looks at anything: the rewritten list is
    // what decides which side each moved file now belongs to.
    let roots: Vec<&Move> = plan
        .moves
        .iter()
        .filter(|root| {
            moved
                .iter()
                .any(|rename| root.relocate(&rename.from).is_some())
        })
        .collect();
    follow_in_ppgitignore(&roots)?;

    announce(PUBLIC_GIT_DIR);
    let new_paths: Vec<String> = moved.iter().map(|rename| rename.to.clone()).collect();
    let now_excluded = io_checked(excluded(&new_paths), "check the public exclude rules")?;

    let mut remove = Vec::new();
    let mut add = Vec::new();
    let mut privatized = Vec::new();
    let mut exposed = Vec::new();
    for rename in &moved {
        match (plan.public.get(&rename.from), now_excluded.get(&rename.to)) {
            (Some(entry), None) => {
                remove.push(rename.from.as_str());
                add.push(format!("{entry},{}", rename.to));
            }
            (Some(_), Some(pattern)) => {
                remove.push(rename.from.as_str());
                privatized.push((rename, pattern));
            }
            (None, None) => {
                if let Some(pattern) = plan.was_excluded.get(&rename.from) {
                    exposed.push((rename, pattern));
                }
            }
            (None, Some(_)) => {}
        }
    }

    if let Err(e) = mirror(&remove, &add) {
        eprintln!("ppgit: failed to mirror the move onto the public half: {e}");
        eprintln!("  The private half has the rename staged; the public half doesn't.");
        eprintln!("  `pp status` shows what's out of step.");
        return Err(ExitCode::FAILURE);
    }

    if !privatized.is_empty() {
        println!("ppgit: moved onto private paths, so untracked from the public half");
        println!("  (the removal is staged there):");
        for (rename, pattern) in &privatized {
            println!("    {} → {}  ({pattern})", rename.from, rename.to);
        }
    }
    if !exposed.is_empty() {
        eprintln!("ppgit: warning: moved off private paths — the public half can see these now:");
        for (rename, pattern) in &exposed {
            eprintln!(
                "    {} → {}  (was private through {pattern})",
                rename.from, rename.to
            );
        }
        eprintln!("  If they should stay private: pp privatize <new path>");
        eprintln!("  Otherwise the next `pp add` publishes them.");
    }
    warn_unmirrored(&plan, &moved);
    Ok(ExitCode::SUCCESS)
}

/// One `git mv` source and the path it lands at.
struct Move {
    from: String,
    to: String,
}

impl Move {
    /// Where `path` ends up if it is this source or lies below it.
    fn relocate(&self, path: &str) -> Option<String> {
        if path == self.from {
            return Some(self.to.clone());
        }
        let below = path.strip_prefix(&self.from)?.strip_prefix('/')?;
        Some(format!("{}/{below}", self.to))
    }
}

/// A tracked file's predicted (or, after `confirmed`, actual) move.
struct Rename {
    from: String,
    to: String,
}

/// Everything worked out before the private `git mv` runs, while the old
/// paths still exist: the sources' destinations, every privately tracked
/// file under them, the public index entries among those (path →
/// `<mode>,<blob>`, the form `--cacheinfo` takes), and which of the old
/// paths the public half excludes (path → the pattern responsible).
#[derive(Default)]
struct Plan {
    moves: Vec<Move>,
    renames: Vec<Rename>,
    public: BTreeMap<String, String>,
    was_excluded: BTreeMap<String, String>,
}

fn plan_move(sources: &[String], destination: &str) -> io::Result<Plan> {
    let moves = targets(sources, destination);
    let private = index_entries(PRIVATE_GIT_PREFIX, sources)?;
    let public = index_entries(PUBLIC_GIT_PREFIX, sources)?;
    let renames: Vec<Rename> = private
        .keys()
        .filter_map(|path| {
            let to = moves.iter().find_map(|root| root.relocate(path))?;
            Some(Rename {
                from: path.clone(),
                to,
            })
        })
        .collect();
    let old_paths: Vec<String> = renames.iter().map(|rename| rename.from.clone()).collect();
    let was_excluded = excluded(&old_paths)?;
    Ok(Plan {
        moves,
        renames,
        public,
        was_excluded,
    })
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

/// Where each source lands, by git's rule: one source onto a destination
/// that isn't a directory is renamed to it; otherwise every source moves
/// *into* the destination, keeping its own name. Checked against the
/// real outcome afterwards (see `confirmed`), so this only has to be
/// right, not exhaustive.
fn targets(sources: &[String], destination: &str) -> Vec<Move> {
    // lstat, like git: a symlink to a directory is a path to rename onto,
    // not a directory to move into.
    let into_directory =
        sources.len() > 1 || fs::symlink_metadata(destination).is_ok_and(|meta| meta.is_dir());

    sources
        .iter()
        .map(|source| {
            let to = if !into_directory {
                destination.to_string()
            } else {
                let name = Path::new(source)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(source);
                if destination == "." {
                    name.to_string()
                } else {
                    format!("{destination}/{name}")
                }
            };
            Move {
                from: source.clone(),
                to,
            }
        })
        .collect()
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

/// Rewrites every `.ppgitignore` line that names a moved path exactly,
/// then re-syncs the exclude files and stages the list privately, so it
/// goes into the same commit as the rename.
fn follow_in_ppgitignore(roots: &[&Move]) -> Result<(), ExitCode> {
    let existing = io_checked(read_or_empty(PPGITIGNORE), &format!("read {PPGITIGNORE}"))?;
    let mut changed = Vec::new();
    let mut updated = String::new();
    for line in existing.lines() {
        match rewritten(line, roots) {
            Some(new) => {
                updated.push_str(&new);
                changed.push((line, new));
            }
            None => updated.push_str(line),
        }
        updated.push('\n');
    }
    if changed.is_empty() {
        return Ok(());
    }

    io_checked(
        fs::write(PPGITIGNORE, &updated),
        &format!("update {PPGITIGNORE}"),
    )?;
    io_checked(sync_excludes(), "sync exclude files")?;
    let mut stage = PRIVATE_GIT_PREFIX.to_vec();
    stage.extend(["add", "--", PPGITIGNORE]);
    io_checked(
        run_quiet_stdout("git", &stage),
        &format!("stage {PPGITIGNORE}"),
    )?;

    for (old, new) in &changed {
        println!("ppgit: {PPGITIGNORE}: {old} → {new}");
    }
    Ok(())
}

/// `line` with the moved path swapped in, if it names exactly one path
/// that moved. That means anchored — a slash at the start or in the
/// middle, since a bare name matches at any depth and may be meant for
/// files elsewhere too — with no glob characters, and not a comment or a
/// negation. The leading and trailing slash, if any, are kept. Trailing
/// whitespace is dropped, which gitignore ignores anyway (its escape, a
/// backslash, already rules a line out).
fn rewritten(line: &str, roots: &[&Move]) -> Option<String> {
    let line = line.trim_end();
    if line.is_empty()
        || line.starts_with('#')
        || line.starts_with('!')
        || line.contains(['*', '?', '[', '\\'])
    {
        return None;
    }
    let leading = line.starts_with('/');
    let trailing = line.ends_with('/');
    let path = line.trim_start_matches('/').trim_end_matches('/');
    if !leading && !path.contains('/') {
        return None;
    }
    let moved = roots.iter().find_map(|root| root.relocate(path))?;
    Some(format!(
        "{}{moved}{}",
        if leading { "/" } else { "" },
        if trailing { "/" } else { "" },
    ))
}

/// Which of `paths` the public half's exclude rules match, path → the
/// pattern responsible. `--no-index` because the question is about the
/// rules, not about what's tracked (a tracked path is otherwise never
/// reported as excluded). `--non-matching` prints a line for every path,
/// in order — verified in scratch, including for paths git has to quote —
/// so the lines are matched to `paths` by position rather than by
/// parsing the path back out. A path matched only by a `!` pattern is
/// listed too, and is not excluded.
fn excluded(paths: &[String]) -> io::Result<BTreeMap<String, String>> {
    if paths.is_empty() {
        return Ok(BTreeMap::new());
    }
    let mut args = vec![
        "check-ignore",
        "--no-index",
        "--verbose",
        "--non-matching",
        "--",
    ];
    args.extend(paths.iter().map(String::as_str));
    let output = run_quiet("git", &args)?;
    // 0 if any path matched a pattern, 1 if none did; anything else failed.
    if !matches!(output.status.code(), Some(0 | 1)) {
        return Err(io::Error::other(format!(
            "git check-ignore exited with {}",
            output.status
        )));
    }
    let listing = String::from_utf8_lossy(&output.stdout);
    Ok(paths
        .iter()
        .zip(listing.lines())
        .filter_map(|(path, line)| {
            // `<source>:<line>:<pattern>\t<path>`, all three empty for a
            // path no pattern matched.
            let (meta, _) = line.split_once('\t')?;
            let pattern = meta.splitn(3, ':').nth(2)?;
            if pattern.is_empty() || pattern.starts_with('!') {
                return None;
            }
            Some((path.clone(), pattern.to_string()))
        })
        .collect())
}

/// The public half's side of the move, in its index only: `remove` drops
/// old entries, `add` (`<mode>,<blob>,<path>` each) puts entries back
/// under their new paths. Neither `update-index` form looks at the
/// working tree, which the private `git mv` has already rearranged.
fn mirror(remove: &[&str], add: &[String]) -> io::Result<()> {
    if !remove.is_empty() {
        let mut args = vec!["update-index", "--force-remove", "--"];
        args.extend(remove);
        run_quiet_stdout("git", &args)?;
    }
    if !add.is_empty() {
        let mut args = vec!["update-index", "--add"];
        for info in add {
            args.extend(["--cacheinfo", info.as_str()]);
        }
        run_quiet_stdout("git", &args)?;
    }
    Ok(())
}

/// Reports publicly tracked files that are gone from their old path but
/// weren't dealt with publicly — a move the prediction got wrong, or a
/// file the public half tracked and the private one didn't. Files `-n`
/// or `-k` left alone are still where they were, and aren't worth a word.
fn warn_unmirrored(plan: &Plan, moved: &[&Rename]) {
    let lost: Vec<&String> = plan
        .public
        .keys()
        .filter(|path| !moved.iter().any(|rename| &&rename.from == path))
        .filter(|path| fs::symlink_metadata(path).is_err())
        .collect();
    if lost.is_empty() {
        return;
    }

    eprintln!("ppgit: warning: these moved but couldn't be mirrored publicly:");
    for path in &lost {
        eprintln!("    {path}");
    }
    eprintln!("  Stage the public side by hand: `pp --public rm --cached <old>` and");
    eprintln!("  `pp --public add <new>`.");
}

/// Index entries under `paths` in one half, path → `<mode>,<blob>`.
/// Literal pathspecs, since a file name may contain glob characters; a
/// directory still matches everything tracked below it.
fn index_entries(prefix: &[&str], paths: &[String]) -> io::Result<BTreeMap<String, String>> {
    if paths.is_empty() {
        return Ok(BTreeMap::new());
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
