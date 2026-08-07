use std::fs;
use std::io::{self, ErrorKind};
use std::path::Path;

use crate::exec::run_quiet_stdout;

pub const PPGITIGNORE: &str = ".ppgitignore";
pub const PUBLIC_GIT_DIR: &str = ".git";
pub const PRIVATE_GIT_DIR: &str = ".ppgit";

const BLOCK_BEGIN: &str = "# >>> ppgit: generated from .ppgitignore, do not edit";
const BLOCK_END: &str = "# <<< ppgit";

/// Reads a file, treating "not there" as empty rather than an error — both
/// `.ppgitignore` and an `info/exclude` are legitimately absent sometimes.
fn read_or_empty(path: impl AsRef<Path>) -> io::Result<String> {
    match fs::read_to_string(path) {
        Ok(contents) => Ok(contents),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(e),
    }
}

/// Returns `existing` with ppgit's managed block replaced by one holding
/// `body`, leaving everything outside the markers alone (an `info/exclude`
/// may well have hand-written lines of its own). Appends the block if
/// there isn't one yet; if the begin marker is there but the end marker
/// isn't, everything from the marker on is treated as the (mangled) block
/// and replaced.
fn with_managed_block(existing: &str, body: &str) -> String {
    let mut block = format!("{BLOCK_BEGIN}\n{body}");
    if !block.ends_with('\n') {
        block.push('\n');
    }
    block.push_str(BLOCK_END);
    block.push('\n');

    let Some(start) = existing.find(BLOCK_BEGIN) else {
        let mut updated = existing.to_string();
        if !updated.is_empty() {
            if !updated.ends_with('\n') {
                updated.push('\n');
            }
            updated.push('\n');
        }
        updated.push_str(&block);
        return updated;
    };

    let mut after_block = match existing[start..].find(BLOCK_END) {
        Some(offset) => start + offset + BLOCK_END.len(),
        None => existing.len(),
    };
    if existing[after_block..].starts_with('\n') {
        after_block += 1;
    }

    format!("{}{}{}", &existing[..start], block, &existing[after_block..])
}

/// Rewrites the managed block in `<git_dir>/info/exclude`. Skips the write
/// entirely when the result would be identical, so running this on every
/// ppgit invocation doesn't churn file mtimes.
fn write_exclude(git_dir: &str, body: &str) -> io::Result<()> {
    let info = Path::new(git_dir).join("info");
    fs::create_dir_all(&info)?;

    let path = info.join("exclude");
    let existing = read_or_empty(&path)?;
    let updated = with_managed_block(&existing, body);
    if updated != existing {
        fs::write(&path, updated)?;
    }
    Ok(())
}

/// Regenerates both git-dirs' exclude files from `.ppgitignore`. A no-op
/// outside a ppgit repository, so it's safe to call unconditionally.
///
/// Note this only governs *untracked* files — see `tracked_but_ignored`
/// for the other half of the story.
pub fn sync_excludes() -> io::Result<()> {
    if !Path::new(PRIVATE_GIT_DIR).is_dir() {
        return Ok(());
    }

    // The public half hides the private git-dir, the private list itself
    // (so the public repo doesn't even give away which paths are private),
    // and everything that list names — copied verbatim, since it's already
    // gitignore syntax.
    let mut public = format!("/{PRIVATE_GIT_DIR}/\n/{PPGITIGNORE}\n");
    public.push_str(&read_or_empty(PPGITIGNORE)?);
    write_exclude(PUBLIC_GIT_DIR, &public)?;

    // The private half is the superset and tracks everything — except its
    // own git-dir, which `git add -A` would otherwise pull into itself.
    write_exclude(PRIVATE_GIT_DIR, &format!("/{PRIVATE_GIT_DIR}/\n"))?;

    Ok(())
}

/// Files the public repo still tracks even though `.ppgitignore` now says
/// they're private. Excluding a path only ever stops git picking up
/// *untracked* files, so anything committed publicly before it was listed
/// keeps going out with every push until it's untracked.
///
/// Asks git rather than matching patterns by hand: `ls-files -i -c` is
/// exactly this query — cached (tracked) entries that the active exclude
/// rules match.
pub fn tracked_but_ignored() -> io::Result<Vec<String>> {
    if !Path::new(PRIVATE_GIT_DIR).is_dir() {
        return Ok(Vec::new());
    }

    let listing = run_quiet_stdout("git", &["ls-files", "-i", "-c", "--exclude-standard"])?;
    Ok(listing
        .lines()
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect())
}

/// Explains a `tracked_but_ignored` result and how to fix it.
pub fn warn_tracked_but_ignored(paths: &[String]) {
    eprintln!(
        "ppgit: warning: the public repository still tracks {} file(s) listed in {PPGITIGNORE}:",
        paths.len()
    );
    for path in paths {
        eprintln!("    {path}");
    }
    eprintln!("  Listing a path only hides it while it's untracked — these were committed");
    eprintln!("  publicly first, so they keep going out with every push. To untrack them");
    eprintln!("  (keeping the files on disk, and in the private repository):");
    for path in paths {
        eprintln!("    git rm --cached -- {path}");
    }
    eprintln!("  then commit that removal to the public repository.");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The block as `with_managed_block` writes it, for building
    /// expectations without repeating the marker strings by hand.
    fn block(body: &str) -> String {
        format!("{BLOCK_BEGIN}\n{body}{BLOCK_END}\n")
    }

    #[test]
    fn appends_to_an_empty_file() {
        assert_eq!(with_managed_block("", "a\n"), block("a\n"));
    }

    #[test]
    fn appends_after_existing_content_with_a_blank_line() {
        assert_eq!(
            with_managed_block("mine\n", "a\n"),
            format!("mine\n\n{}", block("a\n"))
        );
    }

    #[test]
    fn adds_the_missing_newline_of_an_unterminated_file() {
        assert_eq!(
            with_managed_block("mine", "a\n"),
            format!("mine\n\n{}", block("a\n"))
        );
    }

    #[test]
    fn replaces_an_existing_block_and_keeps_what_surrounds_it() {
        let existing = format!("before\n\n{}after\n", block("old\n"));
        assert_eq!(
            with_managed_block(&existing, "new\n"),
            format!("before\n\n{}after\n", block("new\n"))
        );
    }

    #[test]
    fn stays_idempotent_when_nothing_changed() {
        let once = with_managed_block("mine\n", "a\nb\n");
        assert_eq!(with_managed_block(&once, "a\nb\n"), once);
    }

    /// A begin marker with no end marker means someone mangled the block;
    /// everything from the marker on is ours to replace.
    #[test]
    fn repairs_a_block_that_lost_its_end_marker() {
        let existing = format!("mine\n{BLOCK_BEGIN}\nleftover\n");
        assert_eq!(
            with_managed_block(&existing, "a\n"),
            format!("mine\n{}", block("a\n"))
        );
    }

    #[test]
    fn terminates_a_body_that_has_no_trailing_newline() {
        assert_eq!(with_managed_block("", "a"), block("a\n"));
    }
}
