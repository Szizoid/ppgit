use crate::harness::{Project, TestEnv, assert_failure, stderr, stdout};

/// A pair with one public file committed in both halves — the starting
/// point for moving files across the line.
fn pair(env: &TestEnv) -> Project<'_> {
    let project = env.project("proj");
    project.init();
    project.write("public.txt", "hello\n");
    project.write("docs/notes.md", "notes\n");
    project.commit_all("first");
    project
}

#[test]
fn privatize_lists_and_untracks_a_committed_file() {
    let env = TestEnv::new();
    let project = pair(&env);

    let output = project.pp_ok(&["privatize", "public.txt"]);
    assert!(stdout(&output).contains("listed public.txt"));
    assert!(stdout(&output).contains("untracked 1 file(s)"));

    // Listed, excluded, and the public index holds a staged deletion —
    // while the file itself stays on disk and in the private half.
    assert!(project.read(".ppgitignore").contains("public.txt"));
    assert!(project.read(".git/info/exclude").contains("public.txt"));
    assert_eq!(
        project.git_stdout(&["diff", "--cached", "--name-status"]),
        "D\tpublic.txt"
    );
    assert_eq!(project.read("public.txt"), "hello\n");
    assert!(
        project
            .tracked_private()
            .contains(&"public.txt".to_string())
    );

    // After committing the removal the pair is healthy again. Plain,
    // unscoped commit: nothing is staged privately (the file's content
    // there is untouched), so it lands only publicly — just what
    // `privatize` itself suggests doing.
    project.pp_ok(&["commit", "--quiet", "-m", "untrack"]);
    assert!(!project.tracked_public().contains(&"public.txt".to_string()));
}

#[test]
fn privatize_untracks_a_whole_directory() {
    let env = TestEnv::new();
    let project = pair(&env);

    project.pp_ok(&["privatize", "docs"]);

    assert!(project.read(".ppgitignore").contains("docs"));
    assert_eq!(
        project.git_stdout(&["diff", "--cached", "--name-status"]),
        "D\tdocs/notes.md"
    );
    assert!(
        project
            .tracked_private()
            .contains(&"docs/notes.md".to_string())
    );
}

#[test]
fn privatize_a_new_file_only_lists_it() {
    let env = TestEnv::new();
    let project = pair(&env);

    project.write("secret.txt", "shh\n");
    let output = project.pp_ok(&["privatize", "secret.txt"]);
    assert!(stdout(&output).contains("listed secret.txt"));
    assert!(!stdout(&output).contains("untracked"));

    // Nothing to stage: the file was never tracked publicly, listing it
    // is the whole job.
    assert_eq!(
        project.git_stdout(&["diff", "--cached", "--name-status"]),
        ""
    );

    project.commit_all("add a secret");
    assert!(!project.tracked_public().contains(&"secret.txt".to_string()));
    assert!(
        project
            .tracked_private()
            .contains(&"secret.txt".to_string())
    );
}

#[test]
fn privatize_is_idempotent() {
    let env = TestEnv::new();
    let project = pair(&env);

    project.pp_ok(&["privatize", "public.txt"]);
    let output = project.pp_ok(&["privatize", "public.txt"]);
    assert!(stdout(&output).contains("already listed"));

    let listing = project.read(".ppgitignore");
    assert_eq!(listing.matches("public.txt").count(), 1);
}

#[test]
fn privatize_refuses_the_machinery_paths() {
    let env = TestEnv::new();
    let project = pair(&env);

    for path in [".", ".git", ".ppgit", ".ppgitignore"] {
        let output = project.pp(&["privatize", path]);
        assert_failure(&output, &format!("privatize {path}"));
    }
}

#[test]
fn publicize_unlists_so_the_path_can_be_published() {
    let env = TestEnv::new();
    let project = pair(&env);

    project.pp_ok(&["privatize", "public.txt"]);
    project.pp_ok(&["commit", "--quiet", "-m", "untrack"]);

    let output = project.pp_ok(&["publicize", "public.txt"]);
    assert!(stdout(&output).contains("unlisted public.txt"));
    assert!(!project.read(".ppgitignore").contains("public.txt"));
    assert!(!project.read(".git/info/exclude").contains("public.txt"));

    // The suggested follow-up publishes it again — exactly what
    // `publicize` itself prints: a plain (dual) add, then a plain commit.
    project.pp_ok(&["add", "--", "public.txt"]);
    project.pp_ok(&["commit", "--quiet", "-m", "publish"]);
    assert!(project.tracked_public().contains(&"public.txt".to_string()));
}

#[test]
fn publicize_refuses_an_unlisted_path() {
    let env = TestEnv::new();
    let project = pair(&env);

    let output = project.pp(&["publicize", "public.txt"]);
    assert_failure(&output, "publicize of an unlisted path");
    assert!(stderr(&output).contains("not listed"));
}

#[test]
fn both_require_a_path_and_a_project() {
    let env = TestEnv::new();

    let project = pair(&env);
    for command in ["privatize", "publicize"] {
        let output = project.pp(&[command]);
        assert_failure(&output, &format!("{command} with no paths"));
        assert!(stderr(&output).contains("usage:"));
    }

    let outside = env.project("empty");
    let output = outside.pp(&["privatize", "x"]);
    assert_failure(&output, "privatize outside a project");
    assert!(stderr(&output).contains("not a ppgit project"));
}
