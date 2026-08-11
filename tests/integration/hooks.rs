use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use crate::harness::{Project, TestEnv, assert_failure, stderr, stdout};

const HOOK: &str = ".git/hooks/pre-push";

fn pair(env: &TestEnv) -> Project<'_> {
    let project = env.project("proj");
    project.init();
    project
}

#[test]
#[cfg(unix)]
fn init_installs_an_executable_hook() {
    let env = TestEnv::new();
    let project = pair(&env);

    let hook = project.dir.join(HOOK);
    assert!(project.read(HOOK).contains("installed by ppgit"));
    let mode = fs::metadata(&hook).unwrap().permissions().mode();
    assert_ne!(mode & 0o111, 0, "hook must be executable, got {mode:o}");
}

#[test]
fn clone_installs_the_hook_too() {
    let env = TestEnv::new();
    let project = pair(&env);
    project.write("f.txt", "x\n");
    project.commit_all("first");
    project.pp_ok(&["push"]);

    let output = env.pp_in(&env.work_root(), &["clone", "proj", "cloned"]);
    crate::harness::assert_success(&output, "pp clone");
    let clone = env.project("cloned");
    assert!(clone.read(HOOK).contains("installed by ppgit"));
}

#[test]
fn a_raw_git_push_is_refused_when_a_listed_file_is_tracked() {
    let env = TestEnv::new();
    let project = pair(&env);

    project.write("leak.txt", "oops\n");
    project.commit_all("first");
    // Listed behind ppgit's back: no pp command runs after this, so the
    // exclude files are stale — exactly the bypass the hook exists for.
    project.append(".ppgitignore", "leak.txt\n");

    let output = project.git(&["push", "--quiet"]);
    assert_failure(&output, "raw git push with a tracked private file");
    assert!(stderr(&output).contains("would publish private files"));
    assert!(stderr(&output).contains("leak.txt"));

    // Nothing reached the remote.
    let remote = env.run_in(
        &env.remote("tester/proj"),
        "git",
        &["rev-parse", "--verify", "HEAD"],
        &[],
    );
    assert_failure(&remote, "rev-parse in a remote that must be empty");
}

#[test]
fn a_clean_raw_push_passes_the_hook() {
    let env = TestEnv::new();
    let project = pair(&env);

    project.write("f.txt", "x\n");
    project.commit_all("first");

    let output = project.git(&["push", "--quiet"]);
    crate::harness::assert_success(&output, "raw git push of a clean tree");
}

#[test]
fn init_leaves_a_foreign_hook_alone() {
    let env = TestEnv::new();
    let project = pair(&env);

    project.write(HOOK, "#!/bin/sh\n# the user's own hook\nexit 0\n");
    let output = project.pp_ok(&["init"]);

    assert_eq!(
        project.read(HOOK),
        "#!/bin/sh\n# the user's own hook\nexit 0\n"
    );
    assert!(stderr(&output).contains("isn't ppgit's"));
}

#[test]
fn doctor_reports_a_missing_hook() {
    let env = TestEnv::new();
    let project = pair(&env);
    project.write("f.txt", "x\n");
    project.commit_all("first");
    project.pp_ok(&["push"]);

    fs::remove_file(project.dir.join(HOOK)).unwrap();

    let output = project.pp(&["doctor"]);
    assert_failure(&output, "doctor without the hook");
    assert!(stdout(&output).contains("pre-push hook: not installed"));
}

#[test]
fn doctor_reports_a_non_executable_hook() {
    let env = TestEnv::new();
    let project = pair(&env);
    project.write("f.txt", "x\n");
    project.commit_all("first");
    project.pp_ok(&["push"]);

    #[allow(unused)]
    let hook = project.dir.join(HOOK);
    #[cfg(unix)]
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o644)).unwrap();

    let output = project.pp(&["doctor"]);
    assert_failure(&output, "doctor with a non-executable hook");
    assert!(stdout(&output).contains("not executable"));
}

#[test]
fn doctor_only_notes_a_foreign_hook() {
    let env = TestEnv::new();
    let project = pair(&env);
    project.write("f.txt", "x\n");
    project.commit_all("first");
    project.pp_ok(&["push"]);

    project.write(HOOK, "#!/bin/sh\nexit 0\n");

    let output = project.pp_ok(&["doctor"]);
    assert!(stdout(&output).contains("isn't ppgit's"));
    assert!(stdout(&output).contains("No problems"));
}
