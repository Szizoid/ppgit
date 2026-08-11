use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use crate::harness::{TestEnv, assert_failure, stderr};

#[test]
fn m_mode_appends_the_private_text_to_the_private_message_only() {
    let env = TestEnv::new();
    let project = env.project("proj");
    project.init();

    project.write("f.txt", "x\n");
    project.pp_ok(&["add", "."]);
    project.pp_ok(&["commit", "-m", "base message", "--private", "secret detail"]);

    let private_message = project.head_message_private();
    let public_message = project.head_message_public();
    assert!(private_message.contains("base message"));
    assert!(private_message.contains("secret detail"));
    assert_eq!(public_message, "base message");
    assert!(!public_message.contains("secret detail"));
}

#[test]
fn f_mode_combines_the_file_with_the_private_text_for_the_private_commit_only() {
    let env = TestEnv::new();
    let project = env.project("proj");
    project.init();

    project.write("f.txt", "x\n");
    project.pp_ok(&["add", "."]);
    project.write("msg.txt", "from a file\n");
    project.pp_ok(&["commit", "-F", "msg.txt", "--private", "secret detail"]);

    let private_message = project.head_message_private();
    let public_message = project.head_message_public();
    assert!(private_message.contains("from a file"));
    assert!(private_message.contains("secret detail"));
    assert_eq!(public_message, "from a file");
    assert!(!public_message.contains("secret detail"));

    // The original file is untouched — the public commit read it as-is.
    assert_eq!(project.read("msg.txt"), "from a file\n");
}

#[test]
fn editor_mode_amends_the_private_commit_after_the_fact() {
    let env = TestEnv::new();
    let project = env.project("proj");
    project.init();

    let editor = env.scratch("editor");
    fs::write(&editor, "#!/bin/sh\nprintf 'from the editor\\n' > \"$1\"\n").unwrap();
    #[cfg(unix)]
    fs::set_permissions(&editor, fs::Permissions::from_mode(0o755)).unwrap();

    project.write("f.txt", "x\n");
    project.pp_ok(&["add", "."]);
    let output = project.pp_env(
        &["commit", "--private", "secret detail"],
        &[("GIT_EDITOR", editor.to_str().unwrap())],
    );
    crate::harness::assert_success(&output, "pp commit --private (editor)");

    let private_message = project.head_message_private();
    let public_message = project.head_message_public();
    assert!(private_message.contains("from the editor"));
    assert!(private_message.contains("secret detail"));
    // The public commit reused the pre-amend message, not the amended one.
    assert_eq!(public_message, "from the editor");
    assert!(!public_message.contains("secret detail"));

    // Still exactly one private commit — amended, not appended.
    assert_eq!(
        project.private_git_stdout(&["rev-list", "--count", "HEAD"]),
        "1"
    );
}

#[test]
fn private_flag_refuses_when_nothing_is_staged_privately() {
    let env = TestEnv::new();
    let project = env.project("proj");
    project.init();
    project.write("public.txt", "hello\n");
    project.commit_all("first");

    project.pp_ok(&["privatize", "public.txt"]);
    let output = project.pp(&["commit", "-m", "untrack", "--private", "secret detail"]);
    assert_failure(&output, "pp commit --private with nothing staged privately");
    assert!(stderr(&output).contains("nothing to attach to"));

    // Refused before anything ran.
    assert_eq!(
        project.git_stdout(&["diff", "--cached", "--name-status"]),
        "D\tpublic.txt"
    );
}

#[test]
fn private_flag_refuses_an_unsupported_message_source() {
    let env = TestEnv::new();
    let project = env.project("proj");
    project.init();
    project.write("f.txt", "x\n");
    project.commit_all("first");

    project.write("f.txt", "y\n");
    project.pp_ok(&["add", "."]);
    let output = project.pp(&[
        "commit",
        "--no-edit",
        "--amend",
        "--private",
        "secret detail",
    ]);
    assert_failure(&output, "pp commit --no-edit --private");
    assert!(stderr(&output).contains("only knows how to combine"));
}

#[test]
fn editor_mode_pairing_note_points_at_the_amended_private_sha() {
    let env = TestEnv::new();
    let project = env.project("proj");
    project.init();

    let editor = env.scratch("editor");
    fs::write(&editor, "#!/bin/sh\nprintf 'from the editor\\n' > \"$1\"\n").unwrap();
    #[cfg(unix)]
    fs::set_permissions(&editor, fs::Permissions::from_mode(0o755)).unwrap();

    project.write("f.txt", "x\n");
    project.pp_ok(&["add", "."]);
    project.pp_env(
        &["commit", "--private", "secret detail"],
        &[("GIT_EDITOR", editor.to_str().unwrap())],
    );

    // The note has to name the *final* (amended) private SHA, not the
    // one from before `--private` folded its text in.
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
fn private_flag_needs_a_value() {
    let env = TestEnv::new();
    let project = env.project("proj");
    project.init();

    let output = project.pp(&["commit", "-m", "x", "--private"]);
    assert_failure(&output, "pp commit --private (no value)");
    assert!(stderr(&output).contains("needs a message"));
}

#[test]
fn private_flag_refuses_being_given_twice() {
    let env = TestEnv::new();
    let project = env.project("proj");
    project.init();

    let output = project.pp(&["commit", "-m", "x", "--private", "one", "--private", "two"]);
    assert_failure(&output, "pp commit --private twice");
    assert!(stderr(&output).contains("more than once"));
}
