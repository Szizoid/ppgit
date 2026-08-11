use std::fs;

use crate::harness::{Project, TestEnv, assert_failure, stderr, stdout};

/// A freshly initialised pair with one public and one private file,
/// committed and pushed — the state every check should call healthy.
fn healthy_pair(env: &TestEnv) -> Project<'_> {
    let project = env.project("proj");
    project.init();
    project.append(".ppgitignore", "secret.txt\n");
    project.write("public.txt", "hello\n");
    project.write("secret.txt", "shh\n");
    project.commit_all("first");
    project.pp_ok(&["push"]);
    project
}

fn doctor(project: &Project<'_>) -> std::process::Output {
    project.pp(&["doctor"])
}

#[test]
fn a_healthy_pair_comes_up_clean() {
    let env = TestEnv::new();
    let project = healthy_pair(&env);

    let output = project.pp_ok(&["doctor"]);
    assert!(stdout(&output).contains("Everything checks out."));
}

#[test]
fn detects_a_public_only_commit_breaking_the_superset() {
    let env = TestEnv::new();
    let project = healthy_pair(&env);

    // What a public-only `pull` leaves behind, made locally: new public
    // work the private half never saw.
    project.write("public.txt", "changed\n");
    project.pp_ok(&["--public", "add", "public.txt"]);
    // Plain, unscoped: `commit` refuses `--public`/`--private` outright
    // now, but with nothing staged on the private side this still lands
    // only publicly — exactly the drift being simulated here.
    project.pp_ok(&["commit", "--quiet", "-m", "public only"]);

    let output = doctor(&project);
    assert_failure(&output, "doctor on a broken superset");
    assert!(stdout(&output).contains("superset: the private half is missing public work"));
}

#[test]
fn detects_the_halves_on_different_branches() {
    let env = TestEnv::new();
    let project = healthy_pair(&env);

    // Deliberately public-only: exactly the drift the branch rule exists
    // to prevent.
    project.git_ok(&["checkout", "--quiet", "-b", "drift"]);

    let output = doctor(&project);
    assert_failure(&output, "doctor on split branches");
    assert!(stdout(&output).contains("public on drift, private on main"));
}

#[test]
fn detects_a_missing_origin() {
    let env = TestEnv::new();
    let project = healthy_pair(&env);

    project.git_ok(&["remote", "remove", "origin"]);

    let output = doctor(&project);
    assert_failure(&output, "doctor without an origin");
    assert!(stdout(&output).contains("public: no origin configured"));
}

#[test]
fn detects_a_missing_fetch_refspec() {
    let env = TestEnv::new();
    let project = healthy_pair(&env);

    // `--unset-all`, not `--unset`: the key now holds two values (the
    // branch refspec and the pairing-notes one), and a plain `--unset`
    // refuses to pick one on a multi-valued key.
    project.private_git_ok(&["config", "--unset-all", "remote.origin.fetch"]);

    let output = doctor(&project);
    assert_failure(&output, "doctor without a fetch refspec");
    assert!(stdout(&output).contains("private: no fetch refspec"));
}

#[test]
fn detects_an_origin_ppgit_would_not_have_set() {
    let env = TestEnv::new();
    let project = healthy_pair(&env);

    // The same repository in the wrong protocol — how a stale SSH remote
    // once survived on an HTTPS machine, failing every push.
    let ssh = fs::read_to_string(env.record("tester/proj").join("ssh")).unwrap();
    project.git_ok(&["remote", "set-url", "origin", &ssh]);

    let output = doctor(&project);
    assert_failure(&output, "doctor with a wrong-protocol origin");
    assert!(stdout(&output).contains("public: origin is not what ppgit would set"));
}

#[test]
fn detects_a_private_file_still_tracked_publicly() {
    let env = TestEnv::new();
    let project = healthy_pair(&env);

    // public.txt was committed publicly before being listed — listing it
    // now can no longer untrack it.
    project.append(".ppgitignore", "public.txt\n");

    let output = doctor(&project);
    assert_failure(&output, "doctor with a tracked private file");
    assert!(stdout(&output).contains("private files are tracked publicly"));
}

#[test]
fn notes_unpushed_work_without_calling_it_a_problem() {
    let env = TestEnv::new();
    let project = healthy_pair(&env);

    project.write("public.txt", "more\n");
    project.commit_all("second");

    let output = project.pp_ok(&["doctor"]);
    assert!(stdout(&output).contains("commit(s) to push"));
    assert!(stdout(&output).contains("No problems"));
}

#[test]
fn detects_divergence_from_the_remote() {
    let env = TestEnv::new();
    let project = healthy_pair(&env);

    // The remote moves on its own (another machine pushed)…
    let side = env.side_clone("tester/proj", "side");
    side.write("side.txt", "b\n");
    side.git_ok(&["add", "side.txt"]);
    side.git_ok(&["commit", "--quiet", "-m", "from the side"]);
    side.git_ok(&["push", "--quiet"]);

    // …while local work continues from the old tip.
    project.write("local.txt", "c\n");
    project.commit_all("local");

    let output = doctor(&project);
    assert_failure(&output, "doctor on a diverged branch");
    assert!(stdout(&output).contains("diverged"));
}

#[test]
fn detects_an_upstream_missing_while_the_remote_branch_exists() {
    let env = TestEnv::new();
    let project = healthy_pair(&env);

    project.git_ok(&["config", "--unset", "branch.main.merge"]);

    let output = doctor(&project);
    assert_failure(&output, "doctor with a severed upstream");
    assert!(stdout(&output).contains("main has no upstream, though origin/main exists"));
}

#[test]
fn refuses_to_run_outside_a_ppgit_project() {
    let env = TestEnv::new();
    let project = env.project("empty");

    let output = doctor(&project);
    assert_failure(&output, "doctor outside a project");
    assert!(stderr(&output).contains("not a ppgit project"));
}

#[test]
fn takes_no_arguments() {
    let env = TestEnv::new();
    let project = healthy_pair(&env);

    let output = project.pp(&["doctor", "extra"]);
    assert_failure(&output, "doctor with an argument");
    assert!(stderr(&output).contains("usage: ppgit doctor"));
}
