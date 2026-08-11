use crate::harness::{Project, TestEnv};

#[test]
fn a_dual_commit_pairs_both_halves_with_notes() {
    let env = TestEnv::new();
    let project = env.project("proj");
    project.init();

    project.write("f.txt", "x\n");
    project.pp_ok(&["add", "."]);
    project.pp_ok(&["commit", "-m", "first"]);

    let private_head = project.head_private();
    let public_head = project.head_public();
    assert_ne!(private_head, public_head, "sanity: the two trees differ");

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
fn a_second_dual_commit_pairs_too_not_just_the_first() {
    let env = TestEnv::new();
    let project = env.project("proj");
    project.init();
    project.write("f.txt", "x\n");
    project.commit_all("first");

    project.write("f.txt", "y\n");
    project.pp_ok(&["add", "."]);
    project.pp_ok(&["commit", "-m", "second"]);

    let private_head = project.head_private();
    let public_head = project.head_public();
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
fn a_private_only_commit_leaves_no_pairing_note() {
    let env = TestEnv::new();
    let project = env.project("proj");
    project.init();
    project.append(".ppgitignore", "secret.txt\n");

    project.write("secret.txt", "shh\n");
    project.pp_ok(&["add", "."]);
    project.pp_ok(&["commit", "-m", "private only"]);

    // The public half never moved, so there's nothing to pair — and
    // nothing to look up a note by, since `notes show` on a commit with
    // no note fails.
    let private_head = project.head_private();
    let output = project.private_git(&["notes", "show", &private_head]);
    assert!(
        !output.status.success(),
        "expected no note on a private-only commit"
    );
}

#[test]
fn a_public_only_commit_after_privatize_leaves_no_pairing_note() {
    let env = TestEnv::new();
    let project = env.project("proj");
    project.init();
    project.write("public.txt", "hello\n");
    project.commit_all("first");

    project.pp_ok(&["privatize", "public.txt"]);
    project.pp_ok(&["commit", "-m", "untrack"]);

    let public_head = project.head_public();
    let output = project.git(&["notes", "show", &public_head]);
    assert!(
        !output.status.success(),
        "expected no note on a public-only commit"
    );
}

/// Two machines on one project: the original pair, and a `pp clone` of
/// it — mirrors `pull.rs`'s fixture of the same name.
fn two_machines(env: &TestEnv) -> (Project<'_>, Project<'_>) {
    let original = env.project("proj");
    original.init();
    original.write("f.txt", "x\n");
    original.commit_all("first");
    original.pp_ok(&["push"]);

    let output = env.pp_in(&env.work_root(), &["clone", "proj", "cloned"]);
    crate::harness::assert_success(&output, "pp clone");
    (original, env.project("cloned"))
}

#[test]
fn clone_brings_the_pairing_notes_over_too() {
    let env = TestEnv::new();
    let (original, cloned) = two_machines(&env);

    let private_head = original.head_private();
    let public_head = original.head_public();
    assert_eq!(cloned.head_private(), private_head);
    assert_eq!(cloned.head_public(), public_head);

    // Not just the same trees — the same *notes*, fetched by clone
    // rather than left behind the way a plain `git clone`/`git fetch`
    // would leave them.
    assert_eq!(
        cloned.private_git_stdout(&["notes", "show", &private_head]),
        public_head
    );
    assert_eq!(
        cloned.git_stdout(&["notes", "show", &public_head]),
        private_head
    );
}

#[test]
fn push_and_pull_carry_a_new_pairing_note_between_machines() {
    let env = TestEnv::new();
    let (original, cloned) = two_machines(&env);

    original.write("f.txt", "y\n");
    original.pp_ok(&["add", "."]);
    original.pp_ok(&["commit", "-m", "second"]);
    original.pp_ok(&["push"]);

    cloned.pp_ok(&["pull"]);

    let private_head = original.head_private();
    let public_head = original.head_public();
    assert_eq!(cloned.head_private(), private_head);
    assert_eq!(cloned.head_public(), public_head);
    assert_eq!(
        cloned.private_git_stdout(&["notes", "show", &private_head]),
        public_head
    );
    assert_eq!(
        cloned.git_stdout(&["notes", "show", &public_head]),
        private_head
    );
}
