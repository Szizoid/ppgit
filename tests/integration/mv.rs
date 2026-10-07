use crate::harness::{Project, TestEnv, stderr, stdout};

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
        "M\t.ppgitignore\nR100\td/secret.txt\te/secret.txt\nR100\td/x.txt\te/x.txt"
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

    project.write("e/e.txt", "e\n");
    project.commit_all("second");

    // Listed by bare name, which matches at any depth: still private
    // after the move, so there's nothing to rewrite or warn about.
    let output = project.pp_ok(&["mv", "secret.txt", "e/secret.txt"]);
    assert!(!stderr(&output).contains("warning"));

    assert_eq!(staged_private(&project), "R100\tsecret.txt\te/secret.txt");
    assert_eq!(staged_public(&project), "");
    assert!(
        !project
            .tracked_public()
            .contains(&"e/secret.txt".to_string())
    );
    assert!(public_excludes(&project, "e/secret.txt"));
}

fn public_excludes(project: &Project, path: &str) -> bool {
    project
        .git(&["check-ignore", "--no-index", "-q", path])
        .status
        .success()
}

#[test]
fn mv_rewrites_an_exact_ppgitignore_line_and_stages_it() {
    let env = TestEnv::new();
    let project = pair(&env);

    let output = project.pp_ok(&["mv", "d/secret.txt", "d/renamed.txt"]);
    assert!(stdout(&output).contains("d/secret.txt → d/renamed.txt"));

    let list = project.read(".ppgitignore");
    assert!(list.lines().any(|line| line == "d/renamed.txt"));
    assert!(!list.lines().any(|line| line == "d/secret.txt"));
    assert!(public_excludes(&project, "d/renamed.txt"));
    assert_eq!(
        staged_private(&project),
        "M\t.ppgitignore\nR100\td/secret.txt\td/renamed.txt"
    );
    assert_eq!(staged_public(&project), "");
}

#[test]
fn mv_keeps_the_form_of_a_rewritten_directory_line() {
    let env = TestEnv::new();
    let project = pair(&env);
    project.append(".ppgitignore", "/vault/\n");
    project.write("vault/key.txt", "k\n");
    project.commit_all("second");

    project.pp_ok(&["mv", "vault", "safe"]);

    assert!(
        project
            .read(".ppgitignore")
            .lines()
            .any(|line| line == "/safe/")
    );
    assert!(public_excludes(&project, "safe/key.txt"));
    assert!(
        !project
            .tracked_public()
            .contains(&"safe/key.txt".to_string())
    );
}

#[test]
fn mv_warns_when_a_private_file_leaves_its_pattern() {
    let env = TestEnv::new();
    let project = pair(&env);
    project.append(".ppgitignore", "*.pdf\n");
    project.write("a.pdf", "pdf\n");
    project.commit_all("second");
    let list = project.read(".ppgitignore");

    // Renamed away from a bare-name line, and out of a glob: neither is
    // rewritten — the move may be meant to publish the file — but both
    // are pointed out, since the next `pp add` would.
    let output = project.pp_ok(&["mv", "secret.txt", "renamed.txt"]);
    assert!(stderr(&output).contains("secret.txt → renamed.txt  (was private through secret.txt)"));
    let output = project.pp_ok(&["mv", "a.pdf", "a.md"]);
    assert!(stderr(&output).contains("a.pdf → a.md  (was private through *.pdf)"));

    assert_eq!(project.read(".ppgitignore"), list);
    assert_eq!(staged_public(&project), "");

    // Still matching the glob: no warning.
    project.write("b.pdf", "pdf\n");
    project.commit_all("third");
    let output = project.pp_ok(&["mv", "b.pdf", "c.pdf"]);
    assert!(!stderr(&output).contains("warning"));
}

#[test]
fn mv_onto_a_private_path_untracks_the_file_publicly() {
    let env = TestEnv::new();
    let project = pair(&env);
    project.append(".ppgitignore", "/vault/\n");
    project.write("vault/key.txt", "k\n");
    project.commit_all("second");

    let output = project.pp_ok(&["mv", "a.txt", "vault"]);
    assert!(stdout(&output).contains("a.txt → vault/a.txt  (/vault/)"));

    assert_eq!(staged_private(&project), "R100\ta.txt\tvault/a.txt");
    assert_eq!(staged_public(&project), "D\ta.txt");
    assert!(
        !project
            .tracked_public()
            .contains(&"vault/a.txt".to_string())
    );
    // Nothing tracked-but-ignored left behind for the push gate to trip on.
    assert_eq!(
        project.git_stdout(&["ls-files", "-i", "-c", "--exclude-standard"]),
        ""
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
