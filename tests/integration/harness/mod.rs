//! The shared fixture: a temp directory holding a mock `gh`, an isolated
//! git configuration, and a `work/` area for projects. The mock keeps its
//! "GitHub" repositories as local bare repos, so `init`, `clone`, `push`
//! and `doctor` all run their full paths against real remotes that never
//! leave the disk.

use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use tempfile::TempDir;

/// The account every mock repository lives under, mirroring how gh
/// canonicalises a bare `name` to `owner/name`.
pub const OWNER: &str = "tester";

/// Mock of the `gh` subcommands ppgit uses, answering from local state
/// under $FAKE_GH_DIR: `repo create` makes a bare repository in there,
/// `repo view` reads it back. Anything unrecognised exits 64, so a gh
/// call the mock doesn't know about surfaces as a test failure instead
/// of silently passing — and without $FAKE_GH_DIR set at all, every call
/// fails, which is the "gh isn't available" environment.
const FAKE_GH: &str = r##"#!/bin/sh
set -eu

state="${FAKE_GH_DIR:?}"
owner="${FAKE_GH_OWNER:-tester}"

# Prints the state directory for whatever spelling ppgit handed over. A
# configured remote (a filesystem path, or the mock's fake SSH URL) is
# matched against every repository's recorded URLs — the counterpart of
# the real gh canonicalising a URL; anything else is owner/name already,
# or a bare name missing its owner.
resolve() {
    for recorded in "$state"/repos/*/*/url "$state"/repos/*/*/ssh; do
        if [ -e "$recorded" ] && [ "$(cat "$recorded")" = "$1" ]; then
            dirname "$recorded"
            return 0
        fi
    done
    case "$1" in
    */*) printf '%s/repos/%s\n' "$state" "$1" ;;
    *) printf '%s/repos/%s/%s\n' "$state" "$owner" "$1" ;;
    esac
}

case "${1:-}" in
auth)
    # auth status: reaching this line at all means the mock is "logged in".
    exit 0
    ;;
config)
    # config get git_protocol
    echo https
    ;;
repo)
    subcommand="${2:-}"
    shift 2
    case "$subcommand" in
    create)
        case "$1" in
        */*) dir="$state/repos/$1" ;;
        *) dir="$state/repos/$owner/$1" ;;
        esac
        visibility=public
        if [ "${2:-}" = "--private" ]; then
            visibility=private
        fi
        mkdir -p "$dir"
        git init --quiet --bare "$dir/repo.git"
        printf '%s' "$visibility" > "$dir/visibility"
        printf '%s' "$dir/repo.git" > "$dir/url"
        printf 'ssh://mock-gh/%s/%s' \
            "$(basename "$(dirname "$dir")")" "$(basename "$dir")" > "$dir/ssh"
        echo "Created repository $1"
        ;;
    view)
        dir="$(resolve "$1")"
        shift
        if [ ! -d "$dir" ]; then
            exit 1
        fi
        if [ $# -eq 0 ]; then
            exit 0
        fi
        # --json <fields> -q <query>: ppgit's queries always mirror the
        # fields, so matching the fields alone is enough.
        case "${2:-}" in
        nameWithOwner,isPrivate)
            printf '%s/%s\n' "$(basename "$(dirname "$dir")")" "$(basename "$dir")"
            if [ "$(cat "$dir/visibility")" = private ]; then
                echo true
            else
                echo false
            fi
            ;;
        url)
            cat "$dir/url"
            echo
            ;;
        sshUrl)
            cat "$dir/ssh"
            echo
            ;;
        *)
            exit 64
            ;;
        esac
        ;;
    *)
        exit 64
        ;;
    esac
    ;;
*)
    exit 64
    ;;
esac
"##;

/// What every git in the tests sees instead of the developer's own
/// configuration. Signing is off because no test key exists; `main` is
/// pinned so the tests don't depend on the machine's default.
const GLOBAL_GIT_CONFIG: &str = "\
[user]
\tname = Tester
\temail = tester@example.com
[init]
\tdefaultBranch = main
[commit]
\tgpgsign = false
[tag]
\tgpgSign = false
";

pub struct TestEnv {
    root: TempDir,
    gh_enabled: bool,
}

impl TestEnv {
    /// An environment where the mock gh works — `init` and `clone` run
    /// their full paths, creating "GitHub" repositories on local disk.
    pub fn new() -> Self {
        Self::build(true)
    }

    /// An environment where every gh call fails, as on a machine without
    /// gh installed or logged in.
    pub fn without_gh() -> Self {
        Self::build(false)
    }

    fn build(gh_enabled: bool) -> Self {
        let root = TempDir::new().expect("create the test root");
        let at = |rel: &str| root.path().join(rel);

        fs::create_dir(at("bin")).unwrap();
        fs::write(at("bin/gh"), FAKE_GH).unwrap();
        #[cfg(unix)]
        fs::set_permissions(at("bin/gh"), fs::Permissions::from_mode(0o755)).unwrap();

        fs::create_dir(at("home")).unwrap();
        fs::write(at("home/.gitconfig"), GLOBAL_GIT_CONFIG).unwrap();

        fs::create_dir(at("work")).unwrap();
        fs::create_dir_all(at("gh/repos")).unwrap();

        Self { root, gh_enabled }
    }

    /// Where projects live; also the place to run `pp clone` from.
    pub fn work_root(&self) -> PathBuf {
        self.root.path().join("work")
    }

    /// A scratch path outside any work tree, for files that must not show
    /// up as untracked in a project (editor scripts, counters).
    pub fn scratch(&self, name: &str) -> PathBuf {
        self.root.path().join(name)
    }

    /// The mock's record directory for `owner/name` — holds `repo.git`,
    /// `visibility`, `url` and `ssh`.
    pub fn record(&self, name_with_owner: &str) -> PathBuf {
        self.root.path().join("gh/repos").join(name_with_owner)
    }

    /// The bare repository standing in for the GitHub remote `owner/name`.
    pub fn remote(&self, name_with_owner: &str) -> PathBuf {
        self.record(name_with_owner).join("repo.git")
    }

    /// Runs `program args` in `dir` under the controlled environment:
    /// mock gh first on PATH, isolated HOME, no system git config, and an
    /// editor that fails loudly unless a test overrides it.
    pub fn run_in(
        &self,
        dir: &Path,
        program: &str,
        args: &[&str],
        envs: &[(&str, &str)],
    ) -> Output {
        let mut command = Command::new(program);
        command.args(args).current_dir(dir);

        let mut path = self.root.path().join("bin").into_os_string();
        path.push(":");
        path.push(std::env::var_os("PATH").unwrap_or_default());

        let home = self.root.path().join("home");
        command
            .env("PATH", path)
            .env("HOME", &home)
            .env("XDG_CONFIG_HOME", home.join(".config"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_EDITOR", "false")
            .env("LC_ALL", "C")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE");
        if self.gh_enabled {
            command
                .env("FAKE_GH_DIR", self.root.path().join("gh"))
                .env("FAKE_GH_OWNER", OWNER);
        }
        for (key, value) in envs {
            command.env(key, value);
        }

        // Retried because of the classic multi-threaded fork/exec race:
        // while one test thread is still fs::write-ing its own mock gh,
        // another thread's fork briefly inherits that write-open fd
        // (closed only at the child's exec, via CLOEXEC), and exec-ing
        // the script in that window fails with ETXTBSY.
        let mut attempts = 0;
        loop {
            match command.output() {
                Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy && attempts < 50 => {
                    attempts += 1;
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                other => return other.expect("launch a test command"),
            }
        }
    }

    /// Runs the in-tree `pp` binary in `dir`.
    pub fn pp_in(&self, dir: &Path, args: &[&str]) -> Output {
        self.run_in(dir, env!("CARGO_BIN_EXE_pp"), args, &[])
    }

    /// A project directory under `work/` — created if needed, so this
    /// also wraps a directory another command (e.g. `pp clone`) made.
    pub fn project(&self, name: &str) -> Project<'_> {
        let dir = self.work_root().join(name);
        fs::create_dir_all(&dir).unwrap();
        Project { env: self, dir }
    }

    /// An ordinary `git clone` of a mock remote: a second party to push
    /// from when a test needs the remote to move on its own.
    pub fn side_clone(&self, name_with_owner: &str, dir_name: &str) -> Project<'_> {
        let remote = self.remote(name_with_owner);
        let output = self.run_in(
            &self.work_root(),
            "git",
            &["clone", "--quiet", remote.to_str().unwrap(), dir_name],
            &[],
        );
        assert_success(&output, "git clone (side clone)");
        self.project(dir_name)
    }
}

pub struct Project<'a> {
    env: &'a TestEnv,
    pub dir: PathBuf,
}

impl Project<'_> {
    pub fn pp(&self, args: &[&str]) -> Output {
        self.env.pp_in(&self.dir, args)
    }

    pub fn pp_ok(&self, args: &[&str]) -> Output {
        let output = self.pp(args);
        assert_success(&output, &format!("pp {}", args.join(" ")));
        output
    }

    pub fn pp_env(&self, args: &[&str], envs: &[(&str, &str)]) -> Output {
        self.env
            .run_in(&self.dir, env!("CARGO_BIN_EXE_pp"), args, envs)
    }

    /// Raw git against the public half — for setting up fixtures and for
    /// read-only assertions.
    pub fn git(&self, args: &[&str]) -> Output {
        self.env.run_in(&self.dir, "git", args, &[])
    }

    pub fn git_ok(&self, args: &[&str]) -> Output {
        let output = self.git(args);
        assert_success(&output, &format!("git {}", args.join(" ")));
        output
    }

    pub fn git_stdout(&self, args: &[&str]) -> String {
        stdout(&self.git_ok(args))
    }

    /// Raw git against the private half (bare git-dir plus the shared
    /// work tree), mirroring how ppgit itself addresses it.
    pub fn private_git(&self, args: &[&str]) -> Output {
        let mut full = vec!["--git-dir=.ppgit", "--work-tree=."];
        full.extend_from_slice(args);
        self.git(&full)
    }

    pub fn private_git_ok(&self, args: &[&str]) -> Output {
        let output = self.private_git(args);
        assert_success(&output, &format!("git (private) {}", args.join(" ")));
        output
    }

    pub fn private_git_stdout(&self, args: &[&str]) -> String {
        stdout(&self.private_git_ok(args))
    }

    /// `pp init`, expected to succeed — i.e. with the mock gh available.
    pub fn init(&self) {
        self.pp_ok(&["init"]);
    }

    pub fn write(&self, rel: &str, contents: &str) {
        let path = self.dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }

    pub fn append(&self, rel: &str, contents: &str) {
        let mut updated = self.read(rel);
        updated.push_str(contents);
        self.write(rel, &updated);
    }

    pub fn read(&self, rel: &str) -> String {
        fs::read_to_string(self.dir.join(rel)).unwrap()
    }

    /// Stages and commits everything, in both halves, through pp itself.
    pub fn commit_all(&self, message: &str) {
        self.pp_ok(&["add", "."]);
        self.pp_ok(&["commit", "-m", message]);
    }

    pub fn tracked_public(&self) -> Vec<String> {
        lines(&self.git_stdout(&["ls-files"]))
    }

    pub fn tracked_private(&self) -> Vec<String> {
        lines(&self.private_git_stdout(&["ls-files"]))
    }

    pub fn head_public(&self) -> String {
        self.git_stdout(&["rev-parse", "HEAD"])
    }

    pub fn head_private(&self) -> String {
        self.private_git_stdout(&["rev-parse", "HEAD"])
    }

    pub fn head_message_public(&self) -> String {
        self.git_stdout(&["log", "-1", "--format=%B"])
    }

    pub fn head_message_private(&self) -> String {
        self.private_git_stdout(&["log", "-1", "--format=%B"])
    }
}

pub fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

pub fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).trim().to_string()
}

fn lines(text: &str) -> Vec<String> {
    text.lines().map(str::to_string).collect()
}

pub fn assert_success(output: &Output, what: &str) {
    assert!(
        output.status.success(),
        "{what} failed ({})\n--- stdout ---\n{}\n--- stderr ---\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

pub fn assert_failure(output: &Output, what: &str) {
    assert!(
        !output.status.success(),
        "{what} unexpectedly succeeded\n--- stdout ---\n{}\n--- stderr ---\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}
