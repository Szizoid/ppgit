use crate::harness::{Project, TestEnv, assert_failure, stderr};

fn pair(env: &TestEnv) -> Project<'_> {
    let project = env.project("proj");
    project.init();
    project.append(".ppgitignore", "secret.txt\n");
    project.write("public.txt", "hello\n");
    project.write("secret.txt", "shh\n");
    project.commit_all("first");
    project
}

#[test]
fn stash_runs_on_the_private_half_and_sees_every_change() {
    let env = TestEnv::new();
    let project = pair(&env);

    project.write("public.txt", "changed\n");
    project.write("secret.txt", "changed too\n");
    project.pp_ok(&["stash"]);

    // Both files went into one stash — a public stash would have left
    // the private file's change sitting in the tree.
    assert_eq!(project.read("public.txt"), "hello\n");
    assert_eq!(project.read("secret.txt"), "shh\n");
    assert!(!project.private_git_stdout(&["stash", "list"]).is_empty());
    assert_eq!(project.git_stdout(&["stash", "list"]), "");

    project.pp_ok(&["stash", "pop"]);
    assert_eq!(project.read("public.txt"), "changed\n");
    assert_eq!(project.read("secret.txt"), "changed too\n");
}

#[test]
fn stash_refuses_to_take_ignored_files() {
    let env = TestEnv::new();
    let project = pair(&env);
    project.write("public.txt", "changed\n");

    for args in [
        &["stash", "--all"][..],
        &["stash", "-a"][..],
        &["stash", "push", "-au"][..],
    ] {
        let output = project.pp(args);
        assert_failure(&output, &format!("pp {}", args.join(" ")));
        assert!(stderr(&output).contains("--all"));
    }

    // The private git-dir survived every attempt.
    assert!(project.dir.join(".ppgit").is_dir());
}

#[test]
fn stash_refuses_both_halves() {
    let env = TestEnv::new();
    let project = pair(&env);
    project.write("public.txt", "changed\n");

    let output = project.pp(&["--both", "stash"]);
    assert_failure(&output, "pp --both stash");
    assert!(stderr(&output).contains("cannot run on both"));
}

#[test]
fn clean_spares_private_files_and_the_git_dir() {
    let env = TestEnv::new();
    let project = pair(&env);

    project.write("junk.txt", "rubbish\n");
    project.pp_ok(&["clean", "-xdf"]);

    // The genuinely untracked file went; everything private — tracked
    // by the private half or its very git-dir — stayed.
    assert!(!project.dir.join("junk.txt").exists());
    assert_eq!(project.read("secret.txt"), "shh\n");
    assert!(project.dir.join(".ppgit").is_dir());
    assert!(project.dir.join(".ppgitignore").is_file());
}

#[test]
fn clean_refuses_the_public_half() {
    let env = TestEnv::new();
    let project = pair(&env);
    project.write("junk.txt", "rubbish\n");

    let output = project.pp(&["--public", "clean", "-xdf"]);
    assert_failure(&output, "pp --public clean");
    assert!(stderr(&output).contains("private repository only"));
    assert!(project.dir.join("junk.txt").exists());
}

#[test]
fn outside_a_project_both_pass_through_to_plain_git() {
    let env = TestEnv::new();
    let project = env.project("plain");
    project.git_ok(&["init", "--quiet"]);
    project.write("f.txt", "x\n");
    project.git_ok(&["add", "."]);
    project.git_ok(&["commit", "--quiet", "-m", "first"]);

    project.write("f.txt", "y\n");
    project.pp_ok(&["stash"]);
    assert_eq!(project.read("f.txt"), "x\n");
    assert!(!project.git_stdout(&["stash", "list"]).is_empty());

    project.write("junk.txt", "z\n");
    project.pp_ok(&["clean", "-f"]);
    assert!(!project.dir.join("junk.txt").exists());
}
