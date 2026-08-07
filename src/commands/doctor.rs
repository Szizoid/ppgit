use std::collections::HashMap;
use std::ffi::OsString;
use std::io;
use std::path::Path;
use std::process::ExitCode;

use crate::exec::{PRIVATE_GIT_ARG, run_loud, run_quiet_stdout};
use crate::gh::{repo_identity, repo_url};
use crate::ppgitignore::{PPGITIGNORE, PRIVATE_GIT_DIR, PUBLIC_GIT_DIR, tracked_but_ignored};

/// How much a finding matters. Only `Problem` decides the exit code —
/// `Note` is for what is perfectly normal to see (a commit waiting to be
/// pushed, a check that needed the network and didn't get it) but still
/// worth saying out loud.
enum Level {
    Ok,
    Note,
    Problem,
}

/// One line of the report, plus however many indented lines of
/// explanation and remedy it needs. Findings are collected rather than
/// printed as they're made, so a check that fails can't cut the report
/// short — `doctor` is only useful if it says everything it found.
struct Finding {
    level: Level,
    headline: String,
    detail: Vec<String>,
}

impl Finding {
    fn ok(headline: impl Into<String>) -> Self {
        Self {
            level: Level::Ok,
            headline: headline.into(),
            detail: Vec::new(),
        }
    }

    fn note(headline: impl Into<String>, detail: Vec<String>) -> Self {
        Self {
            level: Level::Note,
            headline: headline.into(),
            detail,
        }
    }

    fn problem(headline: impl Into<String>, detail: Vec<String>) -> Self {
        Self {
            level: Level::Problem,
            headline: headline.into(),
            detail,
        }
    }

    fn report(&self) {
        let tag = match self.level {
            Level::Ok => "ok",
            Level::Note => "note",
            Level::Problem => "PROBLEM",
        };
        println!("{tag:>8}  {}", self.headline);
        for line in &self.detail {
            println!("          {line}");
        }
    }
}

/// One of the two repositories, as `doctor` addresses it.
struct Half {
    name: &'static str,
    git_dir: &'static str,
    args: &'static [&'static str],
}

const PUBLIC: Half = Half {
    name: "public",
    git_dir: PUBLIC_GIT_DIR,
    args: &[],
};
const PRIVATE: Half = Half {
    name: "private",
    git_dir: PRIVATE_GIT_DIR,
    args: &[PRIVATE_GIT_ARG],
};

/// Runs a read-only git query against one half and returns its stdout.
fn git_output(half: &Half, args: &[&str]) -> io::Result<String> {
    let mut full = half.args.to_vec();
    full.extend(args);
    run_quiet_stdout("git", &full)
}

/// Trims the decorations that make two spellings of the same remote look
/// different: `gh` reports `https://github.com/o/n` where a configured
/// origin is usually `https://github.com/o/n.git`, and comparing those
/// verbatim would report a mismatch on every healthy repository.
fn normalise_url(url: &str) -> &str {
    let url = url.trim_end_matches('/');
    url.strip_suffix(".git").unwrap_or(url)
}

/// Brings the remote-tracking refs up to date, so everything compared
/// against them below reflects the remotes now rather than whenever they
/// were last contacted. Kept on `run_loud` because a fetch may need to
/// prompt for credentials, which the quiet helpers (stdin closed) would
/// hang or fail on. Being offline isn't fatal — the comparisons are still
/// worth making against what was last seen, as long as the report says
/// that's what they are.
fn refresh_remotes() -> Option<Finding> {
    let stale: Vec<&str> = [&PUBLIC, &PRIVATE]
        .into_iter()
        .filter(|half| {
            let mut args = half.args.to_vec();
            args.extend(["fetch", "--quiet"]);
            !run_loud("git", &args).is_ok_and(|status| status.success())
        })
        .map(|half| half.name)
        .collect();

    if stale.is_empty() {
        return None;
    }
    Some(Finding::note(
        format!("could not fetch the {} remote(s)", stale.join(" and ")),
        vec![
            "Everything below is compared against the last state ppgit saw, which".into(),
            "may be out of date.".into(),
        ],
    ))
}

fn check_branches() -> Finding {
    let public = git_output(&PUBLIC, &["symbolic-ref", "--short", "HEAD"]);
    let private = git_output(&PRIVATE, &["symbolic-ref", "--short", "HEAD"]);

    match (public, private) {
        (Ok(public), Ok(private)) if public == private => {
            Finding::ok(format!("branches: both halves on {public}"))
        }
        (Ok(public), Ok(private)) => Finding::problem(
            format!("branches: public on {public}, private on {private}"),
            vec![
                "The two share one working tree, so the next commit would land where".into(),
                "its counterpart cannot follow.".into(),
                format!("Fix: pp checkout {private}"),
            ],
        ),
        _ => Finding::problem(
            "branches: could not read HEAD in both halves".to_string(),
            vec!["Is this still a ppgit project?".into()],
        ),
    }
}

/// Checks that a half has an `origin` and that it points where ppgit
/// itself would point it. The second half of that matters because
/// `ensure_remote` leaves an existing `origin` alone whatever it holds —
/// which is exactly how an SSH remote survived on a machine that
/// authenticates over HTTPS, failing every push.
fn check_remote(half: &Half) -> Finding {
    let Ok(configured) = git_output(half, &["config", "--get", "remote.origin.url"]) else {
        return Finding::problem(
            format!("{}: no origin configured", half.name),
            vec![
                "Nothing to push to or pull from. `ppgit init` sets this, or add it".into(),
                format!(
                    "by hand: git --git-dir={} remote add origin <url>",
                    half.git_dir
                ),
            ],
        );
    };

    // Ask gh what it would hand out for this same repository today. Two
    // lookups: one to canonicalise whatever spelling the remote is in
    // (gh takes URLs, `owner/name` and bare names alike), one for the URL
    // in the protocol gh is configured for.
    let expected = repo_identity(&configured).and_then(|id| repo_url(&id.name_with_owner));

    match expected {
        Ok(expected) if normalise_url(&expected) == normalise_url(&configured) => {
            Finding::ok(format!("{}: origin is {configured}", half.name))
        }
        Ok(expected) => Finding::problem(
            format!("{}: origin is not what ppgit would set", half.name),
            vec![
                format!("configured: {configured}"),
                format!("gh would use: {expected}"),
                "A remote in the wrong protocol fails every push — over SSH it needs a".into(),
                "key GitHub knows, where HTTPS works with the token gh already holds.".into(),
                format!(
                    "Fix: git --git-dir={} remote set-url origin {expected}",
                    half.git_dir
                ),
            ],
        ),
        Err(_) => Finding::note(
            format!("{}: could not check origin against gh", half.name),
            vec![
                format!("configured: {configured}"),
                "gh is missing, not logged in, or the network is down, so whether this".into(),
                "is the URL ppgit would choose is unverified.".into(),
            ],
        ),
    }
}

/// A fetch refspec is what makes a remote-tracking branch exist at all.
/// Without one, `@{upstream}` never resolves, `status` can't report
/// ahead/behind and `pull` falls back to guessing — while *appearing* to
/// work, which is what makes it worth checking for. A private half made
/// by `git clone --bare` has no refspec until something puts one there.
fn check_fetch_refspec(half: &Half) -> Finding {
    if git_output(half, &["config", "--get", "remote.origin.fetch"]).is_ok() {
        return Finding::ok(format!("{}: fetch refspec configured", half.name));
    }
    Finding::problem(
        format!("{}: no fetch refspec", half.name),
        vec![
            "Nothing will ever land in refs/remotes/origin/*, so no branch can have".into(),
            "an upstream and `pull` is left guessing.".into(),
            format!(
                "Fix: git --git-dir={} config remote.origin.fetch \
                 '+refs/heads/*:refs/remotes/origin/*'",
                half.git_dir
            ),
        ],
    )
}

/// How one half stands against its own remote. Being ahead or behind is
/// ordinary and only reported; having *diverged* is a problem, because
/// neither `push` nor `pull` will resolve it on its own.
fn check_sync(half: &Half) -> Finding {
    let Ok(branch) = git_output(half, &["symbolic-ref", "--short", "HEAD"]) else {
        return Finding::note(
            format!("{}: not on a branch", half.name),
            vec!["Nothing to compare against a remote.".into()],
        );
    };

    let Ok(upstream) = git_output(half, &["rev-parse", "--abbrev-ref", "@{upstream}"]) else {
        // A branch the remote hasn't got yet has no upstream for a
        // perfectly good reason, and with push.autoSetupRemote the first
        // push creates both at once. It's only worth complaining about
        // when the remote *does* have the branch and the two simply
        // aren't connected.
        let remote_branch = format!("origin/{branch}");
        if git_output(half, &["rev-parse", "--verify", "-q", &remote_branch]).is_err() {
            return Finding::note(
                format!("{}: {branch} is not on the remote yet", half.name),
                vec!["`pp push` will create it and set the upstream.".into()],
            );
        }

        return Finding::problem(
            format!(
                "{}: {branch} has no upstream, though {remote_branch} exists",
                half.name
            ),
            vec![
                "`push` and `pull` have nothing to default to.".into(),
                format!(
                    "Fix: git --git-dir={} branch --set-upstream-to={remote_branch} {branch}",
                    half.git_dir
                ),
            ],
        );
    };

    let range = format!("{branch}...{upstream}");
    let Ok(counts) = git_output(half, &["rev-list", "--left-right", "--count", &range]) else {
        return Finding::note(
            format!("{}: could not compare {branch} with {upstream}", half.name),
            Vec::new(),
        );
    };

    let mut fields = counts.split_whitespace();
    let ahead: u32 = fields.next().and_then(|n| n.parse().ok()).unwrap_or(0);
    let behind: u32 = fields.next().and_then(|n| n.parse().ok()).unwrap_or(0);

    match (ahead, behind) {
        (0, 0) => Finding::ok(format!("{}: {branch} in step with {upstream}", half.name)),
        (ahead, 0) => Finding::note(
            format!("{}: {ahead} commit(s) to push", half.name),
            vec!["Run: pp push".into()],
        ),
        (0, behind) => Finding::note(
            format!("{}: {behind} commit(s) to pull", half.name),
            vec!["Run: pp pull".into()],
        ),
        (ahead, behind) => Finding::problem(
            format!(
                "{}: {branch} has diverged from {upstream} ({ahead} local, {behind} remote)",
                half.name
            ),
            vec![
                "Neither push nor pull settles this by itself. If the remote commits are".into(),
                "wanted, rebase onto them: pp pull --rebase".into(),
                "If the local ones supersede them — an amend or rebase of something".into(),
                "already pushed — republish instead: pp push --force-with-lease".into(),
            ],
        ),
    }
}

/// Every path a half's HEAD commit tracks, mapped to its blob hash.
///
/// Comparing hashes works across the two repositories even though they
/// have separate object stores and share no commits: a blob's name is a
/// hash of its contents, so the same file content has the same name in
/// both.
fn tracked_blobs(half: &Half) -> io::Result<HashMap<String, String>> {
    let listing = git_output(half, &["ls-tree", "-r", "-z", "HEAD"])?;
    Ok(listing
        .split('\0')
        .filter(|record| !record.is_empty())
        .filter_map(|record| {
            // `<mode> <type> <hash>\t<path>`, and with -z the path is
            // literal rather than quoted, so a tab or a newline in a
            // filename can't be mistaken for the delimiter.
            let (meta, path) = record.split_once('\t')?;
            let hash = meta.split_whitespace().nth(2)?;
            Some((path.to_string(), hash.to_string()))
        })
        .collect())
}

/// The invariant the whole design rests on: the private repository holds
/// everything the public one does, plus the private files. It breaks
/// quietly — a public-only change arriving by `pull` lands in `.git`
/// alone, and the private half, which never saw that commit, is left
/// behind without anything saying so.
fn check_superset() -> Finding {
    let (Ok(public), Ok(private)) = (tracked_blobs(&PUBLIC), tracked_blobs(&PRIVATE)) else {
        return Finding::note(
            "superset: could not compare the two halves".to_string(),
            vec!["One of them has no commits yet.".into()],
        );
    };

    let mut absent = Vec::new();
    let mut stale = Vec::new();
    for (path, hash) in &public {
        match private.get(path) {
            None => absent.push(path.clone()),
            Some(private_hash) if private_hash != hash => stale.push(path.clone()),
            Some(_) => {}
        }
    }
    absent.sort();
    stale.sort();

    if absent.is_empty() && stale.is_empty() {
        return Finding::ok(format!(
            "superset: private holds all {} publicly tracked file(s)",
            public.len()
        ));
    }

    let mut detail = Vec::new();
    if !absent.is_empty() {
        detail.push(format!(
            "{} file(s) the private half doesn't track:",
            absent.len()
        ));
        detail.extend(absent.iter().map(|path| format!("    {path}")));
    }
    if !stale.is_empty() {
        detail.push(format!(
            "{} file(s) the private half has at an older version:",
            stale.len()
        ));
        detail.extend(stale.iter().map(|path| format!("    {path}")));
    }
    detail.push("The private repository is meant to contain everything the public one".into());
    detail.push("does. This is what a public-only `pull` leaves behind.".into());
    detail.push("Fix: pp add . && pp commit".into());

    Finding::problem("superset: the private half is missing public work", detail)
}

/// Files the public repository still tracks despite `.ppgitignore`
/// listing them. Excluding a path only ever keeps it from being picked
/// up while untracked, so one committed publicly before it was listed
/// goes out with every push regardless.
fn check_tracked_but_ignored() -> Finding {
    let conflicts = match tracked_but_ignored() {
        Ok(conflicts) => conflicts,
        Err(e) => {
            return Finding::note(
                format!("{PPGITIGNORE}: could not check for tracked-but-ignored files"),
                vec![format!("{e}")],
            );
        }
    };

    if conflicts.is_empty() {
        return Finding::ok(format!("{PPGITIGNORE}: nothing listed is tracked publicly"));
    }

    let mut detail = vec![format!(
        "{} file(s) listed as private but tracked publicly:",
        conflicts.len()
    )];
    detail.extend(conflicts.iter().map(|path| format!("    {path}")));
    detail.push("These keep going out with every push. To untrack them, keeping the".into());
    detail.push("files on disk and in the private repository:".into());
    detail.extend(
        conflicts
            .iter()
            .map(|path| format!("    git rm --cached -- {path}")),
    );
    detail.push("then commit that removal to the public repository.".into());

    Finding::problem(
        format!("{PPGITIGNORE}: private files are tracked publicly"),
        detail,
    )
}

fn check_layout() -> Finding {
    if Path::new(PPGITIGNORE).exists() {
        return Finding::ok(format!(
            "layout: {PUBLIC_GIT_DIR}, {PRIVATE_GIT_DIR} and {PPGITIGNORE} present"
        ));
    }
    Finding::note(
        format!("layout: no {PPGITIGNORE}"),
        vec![
            "Without it nothing is private: the public half is excluding only the".into(),
            "private git-dir itself.".into(),
        ],
    )
}

pub fn cmd_doctor(args: &[OsString]) -> ExitCode {
    if args.len() > 1 {
        eprintln!("usage: ppgit doctor");
        eprintln!("  Checks the two halves are in step. Takes no arguments.");
        return ExitCode::FAILURE;
    }

    if !Path::new(PRIVATE_GIT_DIR).is_dir() || !Path::new(PUBLIC_GIT_DIR).is_dir() {
        eprintln!(
            "ppgit: not a ppgit project — expected both {PUBLIC_GIT_DIR} and {PRIVATE_GIT_DIR} here"
        );
        eprintln!("  `ppgit init` sets a project up, `ppgit clone` fetches an existing one.");
        return ExitCode::FAILURE;
    }

    println!("ppgit doctor\n");

    let mut findings = Vec::new();
    findings.push(check_layout());
    findings.extend(refresh_remotes());
    findings.push(check_branches());
    for half in [&PRIVATE, &PUBLIC] {
        findings.push(check_remote(half));
        findings.push(check_fetch_refspec(half));
        findings.push(check_sync(half));
    }
    findings.push(check_superset());
    findings.push(check_tracked_but_ignored());

    for finding in &findings {
        finding.report();
    }

    let problems = findings
        .iter()
        .filter(|finding| matches!(finding.level, Level::Problem))
        .count();
    let notes = findings
        .iter()
        .filter(|finding| matches!(finding.level, Level::Note))
        .count();

    println!();
    match (problems, notes) {
        (0, 0) => {
            println!("Everything checks out.");
            ExitCode::SUCCESS
        }
        (0, notes) => {
            println!("No problems, {notes} note(s).");
            ExitCode::SUCCESS
        }
        (problems, notes) => {
            println!("{problems} problem(s), {notes} note(s).");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalises_the_spellings_gh_and_git_disagree_on() {
        let bare = "https://github.com/o/n";
        assert_eq!(normalise_url("https://github.com/o/n.git"), bare);
        assert_eq!(normalise_url("https://github.com/o/n/"), bare);
        assert_eq!(normalise_url(bare), bare);
    }

    #[test]
    fn leaves_an_ssh_url_comparable_with_itself() {
        assert_eq!(
            normalise_url("git@github.com:o/n.git"),
            "git@github.com:o/n"
        );
    }
}
