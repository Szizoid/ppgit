use crate::harness::{TestEnv, assert_failure, stderr};

#[test]
fn init_sets_up_both_halves_and_their_remotes() {
    let env = TestEnv::new();
    let project = env.project("proj");
    project.init();

    assert!(project.dir.join(".git").is_dir());
    assert!(project.dir.join(".ppgit").is_dir());
    assert!(project.dir.join(".ppgitignore").is_file());

    // The two "GitHub" repositories exist, with the right visibility.
    assert!(env.remote("tester/proj").is_dir());
    assert!(env.remote("tester/pp-proj").is_dir());
    let visibility =
        |name: &str| std::fs::read_to_string(env.record(name).join("visibility")).unwrap();
    assert_eq!(visibility("tester/proj"), "public");
    assert_eq!(visibility("tester/pp-proj"), "private");

    // Each half's origin points at its own remote.
    assert_eq!(
        project.git_stdout(&["remote", "get-url", "origin"]),
        env.remote("tester/proj").to_str().unwrap()
    );
    assert_eq!(
        project.private_git_stdout(&["remote", "get-url", "origin"]),
        env.remote("tester/pp-proj").to_str().unwrap()
    );

    // The first push of a branch needs no --set-upstream in either half.
    assert_eq!(
        project.git_stdout(&["config", "push.autoSetupRemote"]),
        "true"
    );
    assert_eq!(
        project.private_git_stdout(&["config", "push.autoSetupRemote"]),
        "true"
    );
}

#[test]
fn init_without_gh_still_sets_up_the_local_half() {
    let env = TestEnv::without_gh();
    let project = env.project("proj");

    let output = project.pp(&["init"]);
    assert_failure(&output, "pp init without gh");
    assert!(stderr(&output).contains("gh"));

    // Everything local is in place regardless; only the remotes are
    // missing.
    assert!(project.dir.join(".git").is_dir());
    assert!(project.dir.join(".ppgit").is_dir());
    assert!(project.dir.join(".ppgitignore").is_file());

    let public_exclude = project.read(".git/info/exclude");
    assert!(public_exclude.contains("/.ppgit/"));
    assert!(public_exclude.contains("/.ppgitignore"));
    let private_exclude = project.read(".ppgit/info/exclude");
    assert!(private_exclude.contains("/.ppgit/"));
}

#[test]
fn init_is_idempotent() {
    let env = TestEnv::new();
    let project = env.project("proj");
    project.init();

    // A second run must not clobber what the user put in .ppgitignore.
    project.append(".ppgitignore", "kept.txt\n");
    project.init();
    assert!(project.read(".ppgitignore").contains("kept.txt"));
}

#[test]
fn init_adopts_existing_public_history() {
    let env = TestEnv::new();
    let project = env.project("proj");
    project.git_ok(&["init", "--quiet"]);
    project.write("a.txt", "a\n");
    project.git_ok(&["add", "a.txt"]);
    project.git_ok(&["commit", "--quiet", "-m", "first"]);

    project.init();

    // The private half inherited the history instead of starting empty…
    assert_eq!(project.head_public(), project.head_private());
    // …its origin is its own remote, not the public git-dir it was
    // cloned from…
    assert_eq!(
        project.private_git_stdout(&["remote", "get-url", "origin"]),
        env.remote("tester/pp-proj").to_str().unwrap()
    );
    // …and its index was settled, so nothing reads as modified.
    assert_eq!(
        project.private_git_stdout(&["diff", "--name-only", "HEAD"]),
        ""
    );
}
