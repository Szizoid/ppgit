use crate::harness::{Project, TestEnv, assert_failure, stderr};

/// Switches both halves to `branch` via `pp checkout` — a thin wrapper
/// mainly so these tests read the same as before the `checkout` fix
/// (0.21.0) landed. Doubles as coverage that fix actually holds up under
/// realistic use: every call site here switches between branches whose
/// tracked files genuinely differ, exactly the shape that used to strand
/// the public half (see `branch.rs`'s dedicated tests for the direct
/// coverage).
fn checkout_both(project: &Project<'_>, branch: &str) {
    project.pp_ok(&["checkout", branch]);
}

#[test]
fn cherry_picks_a_shared_target_into_both_halves_and_pairs_the_result() {
    let env = TestEnv::new();
    let project = env.project("proj");
    project.init();

    project.write("public.txt", "a\n");
    project.commit_all("base");

    project.pp_ok(&["checkout", "-b", "feature"]);
    project.write("public.txt", "b\n");
    project.commit_all("on feature");
    let (private_x, public_x) = (project.head_private(), project.head_public());

    // `main` moves forward on its own before the cherry-pick lands — the
    // more realistic shape (a branch that hasn't stood still), and one
    // that guarantees the new commit is genuinely distinct: cherry-picking
    // straight onto a target's own original parent can reproduce a
    // byte-identical commit object if every other field happens to
    // coincide too, which isn't a bug, just not what this test means to
    // check.
    checkout_both(&project, "main");
    project.write("other.txt", "unrelated\n");
    project.commit_all("unrelated, on main");
    let (private_before, public_before) = (project.head_private(), project.head_public());

    project.pp_ok(&["cherry-pick", &public_x]);

    let private_after = project.head_private();
    let public_after = project.head_public();
    assert_ne!(private_after, private_before, "private half moved");
    assert_ne!(public_after, public_before, "public half moved");
    assert_eq!(project.read("public.txt"), "b\n");

    // A genuinely new pair, not the original commits.
    assert_ne!(private_after, private_x);
    assert_ne!(public_after, public_x);
    assert_eq!(
        project.private_git_stdout(&["notes", "show", &private_after]),
        public_after
    );
    assert_eq!(
        project.git_stdout(&["notes", "show", &public_after]),
        private_after
    );
}

#[test]
fn cherry_picking_by_the_private_sha_works_the_same_way() {
    let env = TestEnv::new();
    let project = env.project("proj");
    project.init();

    project.write("public.txt", "a\n");
    project.commit_all("base");

    project.pp_ok(&["checkout", "-b", "feature"]);
    project.write("public.txt", "b\n");
    project.commit_all("on feature");
    let private_x = project.head_private();

    checkout_both(&project, "main");
    project.pp_ok(&["cherry-pick", &private_x]);

    assert_eq!(project.read("public.txt"), "b\n");
}

#[test]
fn a_shared_targets_private_only_content_only_reaches_the_private_half() {
    let env = TestEnv::new();
    let project = env.project("proj");
    project.init();
    project.append(".ppgitignore", "secret.txt\n");

    project.write("public.txt", "a\n");
    project.write("secret.txt", "s1\n");
    project.commit_all("base");

    project.pp_ok(&["checkout", "-b", "feature"]);
    project.write("public.txt", "b\n");
    project.write("secret.txt", "s2\n");
    project.pp_ok(&["add", "."]);
    project.pp_ok(&["commit", "-m", "combined change"]);
    let public_x = project.head_public();

    checkout_both(&project, "main");
    project.pp_ok(&["cherry-pick", &public_x]);

    // Both files changed on disk (the private half applied its own,
    // fuller diff) ...
    assert_eq!(project.read("public.txt"), "b\n");
    assert_eq!(project.read("secret.txt"), "s2\n");

    // ... but only public.txt is part of what the public repository
    // actually committed.
    assert_eq!(
        project.git_stdout(&["show", "--format=", "--name-only", "HEAD"]),
        "public.txt"
    );
}

#[test]
fn a_private_only_target_is_cherry_picked_onto_the_private_half_alone() {
    let env = TestEnv::new();
    let project = env.project("proj");
    project.init();
    project.append(".ppgitignore", "secret.txt\n");

    project.write("public.txt", "a\n");
    project.commit_all("base");

    project.pp_ok(&["checkout", "-b", "feature"]);
    project.write("secret.txt", "shh\n");
    project.pp_ok(&["add", "."]);
    project.pp_ok(&["commit", "-m", "private only"]);
    let private_x = project.head_private();

    checkout_both(&project, "main");
    let public_before = project.head_public();

    project.pp_ok(&["cherry-pick", &private_x]);

    assert_eq!(
        project.head_public(),
        public_before,
        "public half untouched"
    );
    assert_eq!(project.read("secret.txt"), "shh\n");
    assert!(
        project
            .tracked_private()
            .contains(&"secret.txt".to_string())
    );
}

#[test]
fn refuses_a_shared_target_while_the_public_index_already_has_staged_changes() {
    let env = TestEnv::new();
    let project = env.project("proj");
    project.init();

    project.write("public.txt", "a\n");
    project.commit_all("base");

    project.pp_ok(&["checkout", "-b", "feature"]);
    project.write("public.txt", "b\n");
    project.commit_all("on feature");
    let public_x = project.head_public();

    checkout_both(&project, "main");
    project.write("other.txt", "staged, unrelated\n");
    project.pp_ok(&["--public", "add", "other.txt"]);
    let public_before = project.head_public();

    let private_before = project.head_private();
    let output = project.pp(&["cherry-pick", &public_x]);
    assert_failure(
        &output,
        "pp cherry-pick with the public index already dirty",
    );
    assert!(stderr(&output).contains("already has staged changes"));

    // Refused before either half was touched, not just the public one.
    assert_eq!(project.head_public(), public_before);
    assert_eq!(project.head_private(), private_before);
}

#[test]
fn refuses_an_unpaired_public_commit() {
    let env = TestEnv::new();
    let project = env.project("proj");
    project.init();
    project.write("public.txt", "a\n");
    project.commit_all("base");

    project.pp_ok(&["checkout", "-b", "feature"]);
    project.git_ok(&["commit", "--allow-empty", "-m", "raw, no pairing"]);
    let orphan = project.head_public();

    checkout_both(&project, "main");
    let output = project.pp(&["cherry-pick", &orphan]);
    assert_failure(&output, "pp cherry-pick on an unpaired public commit");
    assert!(stderr(&output).contains("no pairing note"));
}

#[test]
fn a_target_that_resolves_nowhere_is_refused() {
    let env = TestEnv::new();
    let project = env.project("proj");
    project.init();
    project.write("f.txt", "x\n");
    project.commit_all("first");

    let output = project.pp(&["cherry-pick", "not-a-real-commit-ish"]);
    assert_failure(&output, "pp cherry-pick on a nonexistent target");
    assert!(stderr(&output).contains("does not resolve"));
}

#[test]
fn no_target_stays_ordinary_passthrough() {
    let env = TestEnv::new();
    let project = env.project("proj");
    project.init();
    project.write("f.txt", "x\n");
    project.commit_all("first");

    // No commit-ish argument: `--continue` takes none, and is none of
    // this module's business — it should reach plain git untouched
    // (and fail there, since no cherry-pick is in progress).
    let output = project.pp(&["cherry-pick", "--continue"]);
    assert_failure(
        &output,
        "pp cherry-pick --continue with nothing in progress",
    );
    assert!(!stderr(&output).contains("does not resolve"));
}

#[test]
fn an_explicit_scope_bypasses_the_routing_entirely() {
    let env = TestEnv::new();
    let project = env.project("proj");
    project.init();

    project.write("public.txt", "a\n");
    project.commit_all("base");

    project.pp_ok(&["checkout", "-b", "feature"]);
    project.write("public.txt", "b\n");
    project.commit_all("on feature");
    let public_x = project.head_public();

    checkout_both(&project, "main");
    let private_before = project.head_private();

    project.pp_ok(&["--public", "cherry-pick", &public_x]);
    assert_eq!(
        project.head_private(),
        private_before,
        "private half untouched"
    );
    assert_eq!(project.read("public.txt"), "b\n");
}
