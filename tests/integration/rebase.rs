use crate::harness::{TestEnv, assert_failure, assert_success, stderr};

#[test]
fn rebases_a_shared_range_onto_a_diverged_target_and_pairs_the_result() {
    let env = TestEnv::new();
    let project = env.project("proj");
    project.init();

    project.write("public.txt", "a\n");
    project.commit_all("base");

    project.pp_ok(&["checkout", "-b", "feature"]);
    project.write("public.txt", "b\n");
    project.commit_all("on feature");

    project.pp_ok(&["checkout", "main"]);
    project.write("other.txt", "x\n");
    project.commit_all("unrelated on main");
    let public_main = project.head_public();

    project.pp_ok(&["checkout", "feature"]);
    let output = project.pp_ok(&["rebase", "main"]);
    assert_success(&output, "pp rebase main");

    // The rebased commit's new parent is main's tip, on both halves.
    assert_eq!(
        project.git_stdout(&["log", "-1", "--format=%P"]),
        public_main
    );
    assert_eq!(project.read("public.txt"), "b\n");
    assert_eq!(project.read("other.txt"), "x\n");
    assert_eq!(project.git_stdout(&["status", "--porcelain"]), "");
    assert_eq!(project.private_git_stdout(&["status", "--porcelain"]), "");

    // Message and authorship survived the reconstruction.
    assert_eq!(project.head_message_public(), "on feature");

    // The new pair is paired.
    let (private_head, public_head) = (project.head_private(), project.head_public());
    assert_eq!(
        project.private_git_stdout(&["notes", "show", &private_head]),
        public_head
    );
    assert_eq!(
        project.git_stdout(&["notes", "show", &public_head]),
        private_head
    );
}

#[test]
fn rebases_a_mixed_range_skipping_private_only_commits_publicly() {
    let env = TestEnv::new();
    let project = env.project("proj");
    project.init();
    project.append(".ppgitignore", "secret.txt\n");

    project.write("public.txt", "a\n");
    project.commit_all("base");

    project.pp_ok(&["checkout", "-b", "feature"]);
    project.write("public.txt", "b\n");
    project.commit_all("D1 shared");

    project.write("secret.txt", "s1\n");
    project.pp_ok(&["add", "."]);
    project.pp_ok(&["commit", "-m", "D2 private only"]);

    project.write("public.txt", "c\n");
    project.commit_all("D3 shared");

    project.pp_ok(&["checkout", "main"]);
    project.write("other.txt", "x\n");
    project.commit_all("E unrelated on main");

    project.pp_ok(&["checkout", "feature"]);
    let output = project.pp_ok(&["rebase", "main"]);
    assert_success(&output, "pp rebase main (mixed range)");

    // Public history has exactly D3 -> D1 -> E -> base: D2 never
    // appears, and D3's parent is D1 (not D2, which it doesn't have).
    assert_eq!(
        project.git_stdout(&["log", "--format=%s"]),
        "D3 shared\nD1 shared\nE unrelated on main\nbase"
    );
    assert!(!project.tracked_public().contains(&"secret.txt".to_string()));
    for sha in project.git_stdout(&["log", "--format=%H"]).lines() {
        let tracked = project.git_stdout(&["ls-tree", "-r", "--name-only", sha]);
        assert!(
            !tracked.lines().any(|path| path == "secret.txt"),
            "secret.txt must never appear in a public commit's tree (found in {sha})"
        );
    }

    // Working tree and both indexes are clean and correct.
    assert_eq!(project.read("public.txt"), "c\n");
    assert_eq!(project.read("secret.txt"), "s1\n");
    assert_eq!(project.read("other.txt"), "x\n");
    assert_eq!(project.git_stdout(&["status", "--porcelain"]), "");
    assert_eq!(project.private_git_stdout(&["status", "--porcelain"]), "");

    // Every public commit is paired with its private counterpart, and
    // the private-only one (D2) is paired with nothing.
    for public_sha in project.git_stdout(&["log", "--format=%H"]).lines() {
        let private_sha = project.git_stdout(&["notes", "show", public_sha]);
        assert_eq!(
            project.private_git_stdout(&["notes", "show", &private_sha]),
            public_sha
        );
    }
    let d2 = project.private_git_stdout(&["log", "--format=%H", "--grep=D2 private only"]);
    let output = project.private_git(&["notes", "show", &d2]);
    assert!(!output.status.success(), "D2 must have no pairing note");
}

#[test]
fn rolls_back_when_a_commit_becomes_empty_and_is_dropped() {
    let env = TestEnv::new();
    let project = env.project("proj");
    project.init();

    project.write("public.txt", "a\n");
    project.commit_all("base");

    project.pp_ok(&["checkout", "-b", "feature"]);
    project.write("public.txt", "b\n");
    project.commit_all("same change");

    project.pp_ok(&["checkout", "main"]);
    project.write("public.txt", "b\n");
    project.commit_all("same change made independently on main");

    project.pp_ok(&["checkout", "feature"]);
    let (private_before, public_before) = (project.head_private(), project.head_public());

    let output = project.pp(&["rebase", "main"]);
    assert_failure(&output, "pp rebase that drops an empty commit");
    assert!(stderr(&output).contains("changed the number of commits"));

    // Private rolled all the way back; public was never touched.
    assert_eq!(project.head_private(), private_before);
    assert_eq!(project.head_public(), public_before);
    assert_eq!(project.private_git_stdout(&["status", "--porcelain"]), "");
    assert_eq!(project.git_stdout(&["status", "--porcelain"]), "");
}

#[test]
fn force_skips_the_rollback_and_leaves_the_public_half_untouched() {
    let env = TestEnv::new();
    let project = env.project("proj");
    project.init();

    project.write("public.txt", "a\n");
    project.commit_all("base");

    project.pp_ok(&["checkout", "-b", "feature"]);
    project.write("public.txt", "b\n");
    project.commit_all("same change");

    project.pp_ok(&["checkout", "main"]);
    project.write("public.txt", "b\n");
    project.commit_all("same change made independently on main");
    let private_main = project.head_private();

    project.pp_ok(&["checkout", "feature"]);
    let public_before = project.head_public();

    let output = project.pp(&["rebase", "main", "--force"]);
    assert_failure(&output, "pp rebase --force after an empty-commit drop");
    assert!(stderr(&output).contains("leaving the private half rebased"));

    // Private stayed rebased (landed exactly on main's tip, nothing left
    // to replay); public untouched.
    assert_eq!(project.head_private(), private_main);
    assert_eq!(project.head_public(), public_before);
}

#[test]
fn a_conflicting_private_rebase_leaves_the_public_half_untouched() {
    let env = TestEnv::new();
    let project = env.project("proj");
    project.init();

    project.write("public.txt", "a\n");
    project.commit_all("base");

    project.pp_ok(&["checkout", "-b", "feature"]);
    project.write("public.txt", "b\n");
    project.commit_all("feature change");

    project.pp_ok(&["checkout", "main"]);
    project.write("public.txt", "z\n");
    project.commit_all("conflicting change on main");

    project.pp_ok(&["checkout", "feature"]);
    let public_before = project.head_public();

    let output = project.pp(&["rebase", "main"]);
    assert_failure(&output, "pp rebase that conflicts");
    assert!(stderr(&output).contains("did not complete"));
    assert!(stderr(&output).contains("--continue"));

    assert_eq!(project.head_public(), public_before);

    // Clean up the half-finished private rebase so nothing else in the
    // process (or a future command against this same directory) trips
    // over it.
    project.private_git_ok(&["rebase", "--abort"]);
}

#[test]
fn refuses_an_unpaired_public_upstream() {
    let env = TestEnv::new();
    let project = env.project("proj");
    project.init();
    project.write("public.txt", "a\n");
    project.commit_all("base");

    project.pp_ok(&["checkout", "-b", "feature"]);
    project.git_ok(&["commit", "--allow-empty", "-m", "raw, no pairing"]);
    let orphan = project.head_public();
    // A plain ref, public-only — private never has this object at all
    // (it was never dual-committed), so `pp checkout -b` would fail
    // trying to check it out on both halves. `rebase` only needs the
    // name to resolve, not a real branch switch.
    project.git_ok(&["branch", "other", &orphan]);

    let output = project.pp(&["rebase", "other"]);
    assert_failure(&output, "pp rebase onto an unpaired public commit");
    assert!(stderr(&output).contains("no pairing note"));
}

#[test]
fn refuses_a_private_only_upstream_with_no_shared_ancestor() {
    let env = TestEnv::new();
    let project = env.project("proj");
    project.init();

    project.private_git_ok(&["commit", "--allow-empty", "-m", "solo"]);
    let solo = project.head_private();

    let output = project.pp(&["rebase", &solo]);
    assert_failure(&output, "pp rebase with no shared ancestor at all");
    assert!(stderr(&output).contains("no shared ancestor"));
}

#[test]
fn a_target_that_resolves_nowhere_is_refused() {
    let env = TestEnv::new();
    let project = env.project("proj");
    project.init();
    project.write("f.txt", "x\n");
    project.commit_all("first");

    let output = project.pp(&["rebase", "not-a-real-commit-ish"]);
    assert_failure(&output, "pp rebase onto a nonexistent target");
    assert!(stderr(&output).contains("does not resolve"));
}

#[test]
fn no_target_stays_ordinary_passthrough() {
    let env = TestEnv::new();
    let project = env.project("proj");
    project.init();
    project.write("f.txt", "x\n");
    project.commit_all("first");

    let output = project.pp(&["rebase", "--continue"]);
    assert_failure(&output, "pp rebase --continue with nothing in progress");
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

    project.pp_ok(&["checkout", "main"]);
    project.write("other.txt", "x\n");
    project.commit_all("unrelated on main");
    let public_main = project.head_public();

    project.pp_ok(&["checkout", "feature"]);
    let private_before = project.head_private();

    let output = project.pp_ok(&["--public", "rebase", "main"]);
    assert_success(&output, "pp --public rebase main");
    assert_eq!(
        project.head_private(),
        private_before,
        "private half untouched"
    );
    assert_eq!(
        project.git_stdout(&["log", "-1", "--format=%P"]),
        public_main
    );
}
