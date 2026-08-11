use crate::harness::{TestEnv, assert_failure, stderr, stdout};

#[test]
fn push_lands_in_both_remotes_and_leaks_nothing() {
    let env = TestEnv::new();
    let project = env.project("proj");
    project.init();

    project.append(".ppgitignore", "secret.txt\n");
    project.write("public.txt", "hello\n");
    project.write("secret.txt", "shh\n");
    project.commit_all("first");
    project.pp_ok(&["push"]);

    // Each remote holds its half's HEAD…
    let remote_head = |name: &str| {
        let output = env.run_in(&env.remote(name), "git", &["rev-parse", "HEAD"], &[]);
        crate::harness::assert_success(&output, "rev-parse in a remote");
        stdout(&output)
    };
    assert_eq!(remote_head("tester/proj"), project.head_public());
    assert_eq!(remote_head("tester/pp-proj"), project.head_private());

    // …and the public remote saw neither the secret nor the list naming it.
    let output = env.run_in(
        &env.remote("tester/proj"),
        "git",
        &["ls-tree", "-r", "--name-only", "HEAD"],
        &[],
    );
    crate::harness::assert_success(&output, "ls-tree in the public remote");
    let public_files = stdout(&output);
    assert!(public_files.contains("public.txt"));
    assert!(!public_files.contains("secret.txt"));
    assert!(!public_files.contains(".ppgitignore"));
}

#[test]
fn push_is_refused_while_a_private_file_is_tracked_publicly() {
    let env = TestEnv::new();
    let project = env.project("proj");
    project.init();

    // Committed publicly first, listed as private only afterwards — the
    // exclusion can no longer help, so pushing would leak it.
    project.write("leak.txt", "oops\n");
    project.commit_all("first");
    project.append(".ppgitignore", "leak.txt\n");

    let output = project.pp(&["push"]);
    assert_failure(&output, "pp push with a tracked private file");
    assert!(stderr(&output).contains("refusing to push"));

    // Nothing reached the remote: it has no commits at all. (--verify
    // matters: a plain `rev-parse HEAD` in an empty repository echoes
    // "HEAD" and exits 0.)
    let remote = env.run_in(
        &env.remote("tester/proj"),
        "git",
        &["rev-parse", "--verify", "HEAD"],
        &[],
    );
    assert_failure(&remote, "rev-parse in a remote that must be empty");
}
