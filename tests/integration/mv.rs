use crate::harness::{Project, TestEnv, stderr};

/// A pair with a public file, a public directory and a private-only file
/// inside it, all committed.
fn pair(env: &TestEnv) -> Project<'_> {
    let project = env.project("proj");
    project.init();
    project.append(".ppgitignore", "d/secret.txt\nsecret.txt\n");
    project.write("a.txt", "a\n");
    project.write("d/x.txt", "x\n");
    project.write("d/secret.txt", "shh\n");
    project.write("secret.txt", "shh too\n");
    project.commit_all("first");
    project
}

fn staged_public(project: &Project) -> String {
    project.git_stdout(&["diff", "--cached", "-M", "--name-status"])
}

fn staged_private(project: &Project) -> String {
    project.private_git_stdout(&["diff", "--cached", "-M", "--name-status"])
}

fn unstaged_public(project: &Project) -> String {
    project.git_stdout(&["diff", "--name-status"])
}

#[test]
fn mv_stages_the_rename_in_both_halves() {
    let env = TestEnv::new();
    let project = pair(&env);

    // The bug this guards against: the private `git mv` moved the file
    // on disk, so the public one died with "bad source" and left an
    // unstaged delete plus an untracked file behind.
    let output = project.pp_ok(&["mv", "a.txt", "b.txt"]);
    assert!(!stderr(&output).contains("bad source"));

    assert_eq!(staged_private(&project), "R100\ta.txt\tb.txt");
    assert_eq!(staged_public(&project), "R100\ta.txt\tb.txt");
    assert_eq!(unstaged_public(&project), "");
    assert_eq!(project.read("b.txt"), "a\n");
}

#[test]
fn mv_into_a_directory_moves_every_source() {
    let env = TestEnv::new();
    let project = pair(&env);
    project.write("c.txt", "c\n");
    project.commit_all("second");

    project.pp_ok(&["mv", "a.txt", "c.txt", "d"]);

    let expected = "R100\ta.txt\td/a.txt\nR100\tc.txt\td/c.txt";
    assert_eq!(staged_private(&project), expected);
    assert_eq!(staged_public(&project), expected);
    assert_eq!(unstaged_public(&project), "");
}

#[test]
fn mv_of_a_directory_leaves_its_private_files_out_of_the_public_half() {
    let env = TestEnv::new();
    let project = pair(&env);

    project.pp_ok(&["mv", "d", "e"]);

    assert_eq!(
        staged_private(&project),
        "R100\td/secret.txt\te/secret.txt\nR100\td/x.txt\te/x.txt"
    );
    assert_eq!(staged_public(&project), "R100\td/x.txt\te/x.txt");
    assert!(
        !project
            .tracked_public()
            .contains(&"e/secret.txt".to_string())
    );
    assert_eq!(project.read("e/secret.txt"), "shh\n");
}

#[test]
fn mv_of_a_private_only_file_leaves_the_public_half_alone() {
    let env = TestEnv::new();
    let project = pair(&env);

    project.pp_ok(&["mv", "secret.txt", "d/secret2.txt"]);

    assert_eq!(staged_private(&project), "R100\tsecret.txt\td/secret2.txt");
    assert_eq!(staged_public(&project), "");
    assert!(
        !project
            .tracked_public()
            .contains(&"d/secret2.txt".to_string())
    );
}

#[test]
fn mv_keeps_what_was_staged_publicly() {
    let env = TestEnv::new();
    let project = pair(&env);
    project.write("a.txt", "a, edited\n");
    project.pp_ok(&["add", "a.txt"]);

    project.pp_ok(&["mv", "a.txt", "b.txt"]);

    // The public entry moved as it was — the staged edit with it — and
    // nothing in the tree differs from it.
    assert_eq!(unstaged_public(&project), "");
    assert_eq!(
        project
            .git_stdout(&["ls-files", "-s", "b.txt"])
            .split_whitespace()
            .nth(1),
        project
            .git_stdout(&["hash-object", "b.txt"])
            .split_whitespace()
            .next()
    );
    assert_eq!(project.git_stdout(&["ls-files", "a.txt"]), "");
}

#[test]
fn mv_dry_run_moves_nothing_in_either_half() {
    let env = TestEnv::new();
    let project = pair(&env);

    project.pp_ok(&["mv", "-n", "a.txt", "b.txt"]);

    assert_eq!(staged_private(&project), "");
    assert_eq!(staged_public(&project), "");
    assert_eq!(project.read("a.txt"), "a\n");
}

#[test]
fn a_failed_private_mv_stops_before_the_public_half() {
    let env = TestEnv::new();
    let project = pair(&env);

    let output = project.pp(&["mv", "missing.txt", "b.txt"]);
    crate::harness::assert_failure(&output, "pp mv missing.txt b.txt");
    assert!(!stderr(&output).contains("== public"));
    assert_eq!(staged_public(&project), "");
}
