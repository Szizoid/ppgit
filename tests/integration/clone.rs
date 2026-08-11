use crate::harness::{Project, TestEnv, assert_failure, stderr, stdout};

/// A project with one public and one private file, pushed to the mock
/// remotes — what a second machine would find on "GitHub".
fn seeded_pair(env: &TestEnv) -> Project<'_> {
    let project = env.project("proj");
    project.init();
    project.append(".ppgitignore", "secret.txt\n");
    project.write("public.txt", "hello\n");
    project.write("secret.txt", "shh\n");
    project.commit_all("first");
    project.pp_ok(&["push"]);
    project
}

#[test]
fn clone_reproduces_the_pair_from_the_public_name() {
    let env = TestEnv::new();
    seeded_pair(&env);

    let output = env.pp_in(&env.work_root(), &["clone", "proj", "cloned"]);
    crate::harness::assert_success(&output, "pp clone proj");

    let clone = env.project("cloned");

    // The work tree holds the private files, the public repository
    // doesn't track them, the private one does.
    assert_eq!(clone.read("secret.txt"), "shh\n");
    assert!(!clone.tracked_public().contains(&"secret.txt".to_string()));
    assert!(clone.tracked_private().contains(&"secret.txt".to_string()));

    // Both halves came out with a working upstream, which a bare clone
    // does not provide by itself.
    assert_eq!(
        clone.git_stdout(&["rev-parse", "--abbrev-ref", "@{upstream}"]),
        "origin/main"
    );
    assert_eq!(
        clone.private_git_stdout(&["rev-parse", "--abbrev-ref", "@{upstream}"]),
        "origin/main"
    );

    // The freshly cloned pair passes its own health check.
    let doctor = env.pp_in(&clone.dir, &["doctor"]);
    crate::harness::assert_success(&doctor, "doctor in a fresh clone");
    assert!(stdout(&doctor).contains("Everything checks out."));
}

#[test]
fn clone_from_the_private_name_lands_on_the_same_pair() {
    let env = TestEnv::new();
    seeded_pair(&env);

    let output = env.pp_in(&env.work_root(), &["clone", "tester/pp-proj", "cloned"]);
    crate::harness::assert_success(&output, "pp clone tester/pp-proj");

    let clone = env.project("cloned");
    assert_eq!(clone.read("secret.txt"), "shh\n");
    assert!(!clone.tracked_public().contains(&"secret.txt".to_string()));
    // Whichever half was named, the public one is what `.git` holds.
    assert_eq!(
        clone.git_stdout(&["remote", "get-url", "origin"]),
        env.remote("tester/proj").to_str().unwrap()
    );
}

#[test]
fn a_pull_after_clone_brings_new_work_to_both_halves() {
    let env = TestEnv::new();
    let original = seeded_pair(&env);

    let output = env.pp_in(&env.work_root(), &["clone", "proj", "cloned"]);
    crate::harness::assert_success(&output, "pp clone proj");
    let clone = env.project("cloned");

    // The original machine pushes more work, public and private alike.
    original.write("public.txt", "more\n");
    original.write("secret.txt", "more secrets\n");
    original.commit_all("second");
    original.pp_ok(&["push"]);

    clone.pp_ok(&["pull"]);

    assert_eq!(clone.read("public.txt"), "more\n");
    assert_eq!(clone.read("secret.txt"), "more secrets\n");
    let doctor = env.pp_in(&clone.dir, &["doctor"]);
    crate::harness::assert_success(&doctor, "doctor after pull");
}

#[test]
fn clone_warns_when_the_remotes_were_already_out_of_step() {
    let env = TestEnv::new();
    let original = seeded_pair(&env);

    // A public-only change pushed publicly only: the remotes now violate
    // the superset invariant all by themselves.
    original.write("public.txt", "drifted\n");
    original.pp_ok(&["--public", "add", "public.txt"]);
    // Plain, unscoped: with nothing staged privately this still lands
    // only publicly, which is exactly the drift being simulated.
    original.pp_ok(&["commit", "--quiet", "-m", "public only"]);
    original.pp_ok(&["--public", "push", "--quiet"]);

    let output = env.pp_in(&env.work_root(), &["clone", "proj", "cloned"]);
    crate::harness::assert_success(&output, "pp clone of an out-of-step pair");
    assert!(stderr(&output).contains("out of step"));
}

#[test]
fn clone_refuses_options() {
    let env = TestEnv::new();
    seeded_pair(&env);

    let output = env.pp_in(&env.work_root(), &["clone", "--depth", "1", "proj"]);
    assert_failure(&output, "pp clone --depth");
    assert!(stderr(&output).contains("usage: ppgit clone"));
}

#[test]
fn clone_refuses_a_repository_without_a_counterpart() {
    let env = TestEnv::new();

    // A repository that exists on its own, with no pp- private half —
    // `git clone`'s business, not ppgit's.
    let created = env.run_in(
        &env.work_root(),
        "gh",
        &["repo", "create", "solo", "--public"],
        &[],
    );
    crate::harness::assert_success(&created, "mock gh repo create");

    let output = env.pp_in(&env.work_root(), &["clone", "solo"]);
    assert_failure(&output, "pp clone of a lone repository");
    assert!(stderr(&output).contains("does not exist"));
}
