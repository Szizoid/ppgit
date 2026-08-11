use crate::harness::{TestEnv, assert_failure, stderr};

#[test]
fn resets_both_halves_to_a_shared_target_given_by_its_public_sha() {
    let env = TestEnv::new();
    let project = env.project("proj");
    project.init();

    project.write("public.txt", "one\n");
    project.commit_all("first");
    let (private_a, public_a) = (project.head_private(), project.head_public());

    project.write("public.txt", "two\n");
    project.commit_all("second");

    project.pp_ok(&["reset", &public_a]);
    assert_eq!(project.head_private(), private_a);
    assert_eq!(project.head_public(), public_a);
}

#[test]
fn preserves_flags_like_hard_across_both_halves() {
    let env = TestEnv::new();
    let project = env.project("proj");
    project.init();

    project.write("public.txt", "one\n");
    project.commit_all("first");
    let (private_a, public_a) = (project.head_private(), project.head_public());

    project.write("public.txt", "two\n");
    project.commit_all("second");

    project.pp_ok(&["reset", "--hard", &public_a]);
    assert_eq!(project.head_private(), private_a);
    assert_eq!(project.head_public(), public_a);
    // `--hard` actually touched the working tree, not just the refs.
    assert_eq!(project.read("public.txt"), "one\n");
    assert_eq!(
        project.git_stdout(&["status", "--porcelain"]),
        "",
        "a --hard reset should leave a clean working tree"
    );
}

#[test]
fn resets_both_halves_to_a_shared_target_given_by_its_private_sha() {
    let env = TestEnv::new();
    let project = env.project("proj");
    project.init();

    project.write("public.txt", "one\n");
    project.commit_all("first");
    let (private_a, public_a) = (project.head_private(), project.head_public());

    project.write("public.txt", "two\n");
    project.commit_all("second");

    // Same target, named by the private-side SHA this time — it never
    // resolves publicly (the two repositories hold different objects
    // even for a paired commit), so this exercises the other half of
    // `commits::resolve`.
    project.pp_ok(&["reset", &private_a]);
    assert_eq!(project.head_private(), private_a);
    assert_eq!(project.head_public(), public_a);
}

#[test]
fn resets_the_public_half_to_the_nearest_shared_ancestor_of_a_private_only_target() {
    let env = TestEnv::new();
    let project = env.project("proj");
    project.init();
    project.append(".ppgitignore", "secret.txt\n");

    project.write("public.txt", "one\n");
    project.commit_all("first"); // shared, call it A

    project.write("public.txt", "two\n");
    project.commit_all("second"); // shared, call it B
    let public_b = project.head_public();

    project.write("secret.txt", "shh\n");
    project.pp_ok(&["add", "."]);
    project.pp_ok(&["commit", "-m", "private only"]); // private-only, call it C
    let private_c = project.head_private();
    assert_eq!(
        project.head_public(),
        public_b,
        "sanity: the private-only commit left public where it was"
    );

    project.write("public.txt", "three\n");
    project.commit_all("third"); // shared, call it D, on top of C privately

    project.pp_ok(&["reset", &private_c]);
    assert_eq!(project.head_private(), private_c);
    assert_eq!(
        project.head_public(),
        public_b,
        "public has nothing at C, so it lands on B instead"
    );
}

#[test]
fn refuses_an_unpaired_public_commit() {
    let env = TestEnv::new();
    let project = env.project("proj");
    project.init();
    project.write("public.txt", "one\n");
    project.commit_all("first");

    // Bypasses `pp commit` entirely, so this commit gets no pairing note
    // — the orphan case `doctor` is meant to catch.
    project.git_ok(&["commit", "--allow-empty", "-m", "raw, no pairing"]);
    let orphan = project.head_public();

    let output = project.pp(&["reset", &orphan]);
    assert_failure(&output, "pp reset on an unpaired public commit");
    assert!(stderr(&output).contains("no pairing note"));
}

#[test]
fn refuses_a_private_only_target_with_no_shared_ancestor() {
    let env = TestEnv::new();
    let project = env.project("proj");
    project.init();

    // A private-only root commit, made with no `pp commit` ever having
    // run — nothing in its history can possibly be paired.
    project.private_git_ok(&["commit", "--allow-empty", "-m", "solo"]);
    let solo = project.head_private();

    let output = project.pp(&["reset", &solo]);
    assert_failure(&output, "pp reset with no shared ancestor at all");
    assert!(stderr(&output).contains("no shared ancestor"));
}

#[test]
fn a_target_that_resolves_nowhere_is_refused() {
    let env = TestEnv::new();
    let project = env.project("proj");
    project.init();
    project.write("f.txt", "x\n");
    project.commit_all("first");

    let output = project.pp(&["reset", "not-a-real-commit-ish"]);
    assert_failure(&output, "pp reset on a nonexistent target");
    assert!(stderr(&output).contains("does not resolve"));
}

#[test]
fn no_target_stays_ordinary_public_only_passthrough() {
    let env = TestEnv::new();
    let project = env.project("proj");
    project.init();
    project.write("f.txt", "x\n");
    project.commit_all("first");

    project.write("f.txt", "staged change\n");
    project.pp_ok(&["add", "."]);

    // No commit-ish argument at all: unstages, touches the public index
    // only, exactly like before this command had any routing.
    project.pp_ok(&["reset"]);
    assert_eq!(
        project.git_stdout(&["diff", "--cached", "--name-status"]),
        ""
    );
    assert_eq!(
        project.private_git_stdout(&["diff", "--cached", "--name-status"]),
        "M\tf.txt"
    );
}

#[test]
fn a_path_form_reset_stays_passthrough() {
    let env = TestEnv::new();
    let project = env.project("proj");
    project.init();
    project.write("f.txt", "x\n");
    project.commit_all("first");

    project.write("f.txt", "staged change\n");
    project.pp_ok(&["add", "."]);

    project.pp_ok(&["reset", "--", "f.txt"]);
    assert_eq!(
        project.git_stdout(&["diff", "--cached", "--name-status"]),
        ""
    );
    assert_eq!(
        project.private_git_stdout(&["diff", "--cached", "--name-status"]),
        "M\tf.txt"
    );
}

#[test]
fn an_explicit_scope_bypasses_the_routing_entirely() {
    let env = TestEnv::new();
    let project = env.project("proj");
    project.init();

    project.write("public.txt", "one\n");
    project.commit_all("first");
    let public_a = project.head_public();

    project.write("public.txt", "two\n");
    project.commit_all("second");
    let private_b = project.head_private();

    // Literal single-half reset: only the public half moves, exactly as
    // a plain `git --git-dir=.git reset` would, with no classification.
    project.pp_ok(&["--public", "reset", &public_a]);
    assert_eq!(project.head_public(), public_a);
    assert_eq!(project.head_private(), private_b);
}
