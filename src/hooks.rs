use std::fs;
use std::io;
use std::path::Path;
use std::process::ExitCode;

use crate::exec::io_checked;
use crate::ppgitignore::PUBLIC_GIT_DIR;

/// How the hook identifies itself as ppgit's — both to `install` (which
/// must never overwrite someone else's hook) and to `doctor`.
pub const HOOK_MARKER: &str = "installed by ppgit";

/// The pre-push safety net. Everything ppgit's own push refusal checks
/// is re-checked here at the git level, so a raw `git push` that never
/// went through ppgit is caught too. Deliberately self-contained: it
/// reads `.ppgitignore` directly (`--exclude-from`, verified to consult
/// only that file) rather than trusting `info/exclude` to have been
/// synced, so it stays correct however long ago ppgit last ran.
const HOOK_TEMPLATE: &str = r#"#!/bin/sh
# pre-push safety net, installed by ppgit. Refuses to push while the
# public repository tracks a file .ppgitignore lists as private - the
# same check ppgit itself makes, re-run at the git level so a raw
# `git push` can't slip past it. Delete this file to opt out; a re-run
# of `ppgit init` puts it back.

[ -f .ppgitignore ] || exit 0

leaks="$(git ls-files -i -c --exclude-from=.ppgitignore)"
listed="$(git ls-files -c -- .ppgitignore)"
[ -z "$leaks" ] && [ -z "$listed" ] && exit 0

{
    echo "ppgit pre-push hook: this push would publish private files:"
    printf '%s\n%s\n' "$leaks" "$listed" | grep -v '^$' | sed 's/^/    /'
    echo "  Untrack them first (the files stay on disk):"
    echo "    pp privatize <path>"
    echo "  then commit the removal and push again."
} >&2
exit 1
"#;

/// What `install_pre_push_hook` found in place.
pub enum HookStatus {
    Installed,
    AlreadyOurs,
    /// Someone else's hook — left untouched, the caller should say so.
    Foreign,
}

fn hook_path() -> std::path::PathBuf {
    Path::new(PUBLIC_GIT_DIR).join("hooks").join("pre-push")
}

/// Puts the safety net in the public repository's hooks, never touching
/// a hook ppgit didn't write. Re-installs over an older ppgit hook, so
/// template improvements reach existing projects on the next `init`.
pub fn install_pre_push_hook() -> io::Result<HookStatus> {
    let path = hook_path();
    let status = match fs::read_to_string(&path) {
        Ok(existing) if !existing.contains(HOOK_MARKER) => return Ok(HookStatus::Foreign),
        Ok(existing) if existing == HOOK_TEMPLATE => return Ok(HookStatus::AlreadyOurs),
        Ok(_) => HookStatus::AlreadyOurs,
        Err(e) if e.kind() == io::ErrorKind::NotFound => HookStatus::Installed,
        Err(e) => return Err(e),
    };

    if let Some(hooks_dir) = path.parent() {
        fs::create_dir_all(hooks_dir)?;
    }
    fs::write(&path, HOOK_TEMPLATE)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755))?;
    }

    Ok(status)
}

/// `install_pre_push_hook` as a command step: bails out on IO errors,
/// and says so when a foreign hook made it do nothing.
pub fn ensure_pre_push_hook() -> Result<(), ExitCode> {
    match io_checked(install_pre_push_hook(), "install the pre-push hook")? {
        HookStatus::Foreign => {
            eprintln!("ppgit: a pre-push hook already exists and isn't ppgit's — leaving it alone");
            eprintln!(
                "  ppgit's own push refusal still applies; only a raw `git push` bypasses it."
            );
            Ok(())
        }
        HookStatus::Installed | HookStatus::AlreadyOurs => Ok(()),
    }
}

/// What `doctor` needs to know about the hook, without installing
/// anything.
pub enum HookState {
    Ours,
    Missing,
    Foreign,
    NotExecutable,
}

pub fn pre_push_hook_state() -> HookState {
    let path = hook_path();
    let Ok(contents) = fs::read_to_string(&path) else {
        return HookState::Missing;
    };
    if !contents.contains(HOOK_MARKER) {
        return HookState::Foreign;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let executable = fs::metadata(&path)
            .map(|meta| meta.permissions().mode() & 0o111 != 0)
            .unwrap_or(false);
        if !executable {
            // git silently skips a non-executable hook, which is the
            // worst failure mode: the net looks installed and isn't.
            return HookState::NotExecutable;
        }
    }

    HookState::Ours
}
