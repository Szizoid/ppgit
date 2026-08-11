use crate::harness::{TestEnv, assert_failure, stderr};

#[test]
fn branch_commands_run_on_both_halves() {
    let env = TestEnv::new();
    let project = env.project("proj");
    project.init();
    project.write("f.txt", "x\n");
    project.commit_all("first");

    project.pp_ok(&["checkout", "-b", "feature"]);

    assert_eq!(
        project.git_stdout(&["symbolic-ref", "--short", "HEAD"]),
        "feature"
    );
    assert_eq!(
        project.private_git_stdout(&["symbolic-ref", "--short", "HEAD"]),
        "feature"
    );
}

#[test]
fn narrowing_a_branch_command_is_refused() {
    let env = TestEnv::new();
    let project = env.project("proj");
    project.init();
    project.write("f.txt", "x\n");
    project.commit_all("first");

    let output = project.pp(&["--public", "checkout", "-b", "drift"]);
    assert_failure(&output, "pp --public checkout -b");
    assert!(stderr(&output).contains("can only run on both"));

    // Refused before anything ran: the branch exists in neither half.
    assert_eq!(project.git_stdout(&["branch", "--list", "drift"]), "");
    assert_eq!(
        project.private_git_stdout(&["branch", "--list", "drift"]),
        ""
    );
}

#[test]
fn checkout_between_branches_with_different_content_keeps_both_halves_in_step() {
    let env = TestEnv::new();
    let project = env.project("proj");
    project.init();

    project.write("f.txt", "a\n");
    project.commit_all("base");
    let (private_main, public_main) = (project.head_private(), project.head_public());

    project.pp_ok(&["checkout", "-b", "feature"]);
    project.write("f.txt", "b\n");
    project.commit_all("on feature");

    // The bug this guards against: a naive double checkout leaves the
    // public half's index disagreeing with the tree the private
    // checkout already rewrote, and — since the old routing only
    // reported the private result — `pp checkout main` would claim
    // success while quietly stranding the public half on `feature`.
    let output = project.pp_ok(&["checkout", "main"]);
    crate::harness::assert_success(&output, "pp checkout main");

    assert_eq!(
        project.git_stdout(&["symbolic-ref", "--short", "HEAD"]),
        "main"
    );
    assert_eq!(
        project.private_git_stdout(&["symbolic-ref", "--short", "HEAD"]),
        "main"
    );
    // Not just "some commit on main" — exactly the tip main actually had.
    assert_eq!(project.head_public(), public_main);
    assert_eq!(project.head_private(), private_main);
    assert_eq!(project.read("f.txt"), "a\n");

    // Both indexes are actually clean, not just the refs — the whole
    // point of `reset --mixed` in the fix.
    assert_eq!(project.git_stdout(&["status", "--porcelain"]), "");
    assert_eq!(project.private_git_stdout(&["status", "--porcelain"]), "");
}

#[test]
fn checkout_round_trips_between_branches_repeatedly() {
    let env = TestEnv::new();
    let project = env.project("proj");
    project.init();

    project.write("f.txt", "a\n");
    project.commit_all("base");
    project.pp_ok(&["checkout", "-b", "feature"]);
    project.write("f.txt", "b\n");
    project.commit_all("on feature");

    for _ in 0..2 {
        project.pp_ok(&["checkout", "main"]);
        assert_eq!(project.read("f.txt"), "a\n");
        project.pp_ok(&["checkout", "feature"]);
        assert_eq!(project.read("f.txt"), "b\n");
    }

    // An ordinary commit cycle still works cleanly after the round trip.
    project.write("f.txt", "c\n");
    project.pp_ok(&["add", "."]);
    project.pp_ok(&["commit", "-m", "after round trip"]);
    assert_eq!(project.head_message_public(), "after round trip");
    assert_eq!(project.head_message_private(), "after round trip");
}

#[test]
fn a_path_checkout_is_not_forced_dual() {
    let env = TestEnv::new();
    let project = env.project("proj");
    project.init();
    project.write("f.txt", "one\n");
    project.commit_all("first");

    project.write("f.txt", "two\n");
    // With `--` this is a file restore, not a branch switch, so a single
    // scope is legitimate.
    project.pp_ok(&["--public", "checkout", "--", "f.txt"]);

    assert_eq!(project.read("f.txt"), "one\n");
}
