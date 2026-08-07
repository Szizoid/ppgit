use std::io;
use std::process::ExitCode;

use crate::exec::{run_loud_checked, run_quiet_ok, run_quiet_stdout};

/// Confirms `gh` is installed and logged in. Call this once, before relying
/// on `repo_exists` returning `false` to actually mean "doesn't exist" —
/// otherwise "gh missing"/"not authenticated" silently look the same as
/// "repo not found" to callers.
pub fn ensure_gh_ready() -> Result<(), ExitCode> {
    if run_quiet_ok("gh", &["auth", "status"]) {
        Ok(())
    } else {
        eprintln!("ppgit: `gh` is not installed or not logged in — run `gh auth login`");
        Err(ExitCode::FAILURE)
    }
}

/// Whether a GitHub repository named `name` already exists under the
/// currently authenticated account. Only meaningful once `ensure_gh_ready`
/// has succeeded.
pub fn repo_exists(name: &str) -> bool {
    run_quiet_ok("gh", &["repo", "view", name])
}

/// Creates a new, empty GitHub repository named `name` (no `--source`/
/// `--push` — ppgit wires the remote up itself afterwards, since gh's
/// "current repo" auto-detection isn't meant for a directory with two
/// independent git-dirs in it). Left on `run_loud_checked` (not a
/// `run_quiet_*`) so the user actually sees gh's own "created repository"
/// output.
pub fn repo_create(name: &str, private: bool) -> Result<(), ExitCode> {
    let visibility = if private { "--private" } else { "--public" };
    run_loud_checked("gh", &["repo", "create", name, visibility])
}

/// The repository's clone URL, in whichever protocol `gh` is configured to
/// use for git operations — the same choice `gh repo clone` would make.
/// Picking one unconditionally is how ppgit handed an SSH remote to a
/// machine that authenticates over HTTPS, where every push then failed
/// with `Permission denied (publickey)`.
pub fn repo_url(name: &str) -> io::Result<String> {
    let field = match run_quiet_stdout("gh", &["config", "get", "git_protocol"]) {
        Ok(protocol) if protocol == "ssh" => "sshUrl",
        // `url` is the HTTPS one. Also the fallback when gh can't say:
        // HTTPS works with the token gh already holds, whereas SSH needs a
        // key the user may never have set up.
        _ => "url",
    };

    run_quiet_stdout(
        "gh",
        &[
            "repo",
            "view",
            name,
            "--json",
            field,
            "-q",
            &format!(".{field}"),
        ],
    )
}
