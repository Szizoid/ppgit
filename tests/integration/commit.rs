use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use crate::harness::{TestEnv, assert_failure, stderr};

#[test]
fn the_editor_opens_once_and_the_message_lands_in_both() {
    let env = TestEnv::new();
    let project = env.project("proj");
    project.init();

    // An editor that counts its invocations and writes a fixed message —
    // the only way to prove the "one editor session" guarantee, since an
    // editor opening twice would produce the same end state.
    let count = env.scratch("editor-runs");
    let editor = env.scratch("editor");
    fs::write(
        &editor,
        format!(
            "#!/bin/sh\necho run >> {}\nprintf 'both halves, one message\\n' > \"$1\"\n",
            count.display()
        ),
    )
    .unwrap();
    #[cfg(unix)]
    fs::set_permissions(&editor, fs::Permissions::from_mode(0o755)).unwrap();

    project.write("f.txt", "x\n");
    project.pp_ok(&["add", "."]);
    let output = project.pp_env(&["commit"], &[("GIT_EDITOR", editor.to_str().unwrap())]);
    crate::harness::assert_success(&output, "pp commit (editor)");

    assert_eq!(fs::read_to_string(&count).unwrap().lines().count(), 1);
    assert_eq!(project.head_message_private(), "both halves, one message");
    assert_eq!(project.head_message_public(), "both halves, one message");
}

#[test]
fn an_aborted_editor_commits_nothing() {
    let env = TestEnv::new();
    let project = env.project("proj");
    project.init();

    project.write("f.txt", "x\n");
    project.commit_all("first");
    let (public, private) = (project.head_public(), project.head_private());

    project.write("f.txt", "y\n");
    project.pp_ok(&["add", "."]);
    // The harness default GIT_EDITOR is `false`, i.e. an editor session
    // that fails — git aborts the commit.
    let output = project.pp(&["commit"]);
    assert_failure(&output, "pp commit with an aborting editor");

    // Neither half moved: with no private commit there is no message to
    // reuse, and a public-only commit would put the halves out of step.
    assert_eq!(project.head_public(), public);
    assert_eq!(project.head_private(), private);
}

#[test]
fn a_message_flag_needs_no_editor() {
    let env = TestEnv::new();
    let project = env.project("proj");
    project.init();

    project.write("f.txt", "x\n");
    project.pp_ok(&["add", "."]);
    // GIT_EDITOR stays `false`: if this opened an editor it would fail.
    project.pp_ok(&["commit", "-m", "explicit message"]);

    assert_eq!(project.head_message_private(), "explicit message");
    assert_eq!(project.head_message_public(), "explicit message");
}

#[test]
fn narrowing_commit_to_one_half_is_refused() {
    let env = TestEnv::new();
    let project = env.project("proj");
    project.init();
    project.write("f.txt", "x\n");
    project.commit_all("first");
    let (public, private) = (project.head_public(), project.head_private());

    project.write("f.txt", "y\n");
    project.pp_ok(&["add", "."]);

    let output = project.pp(&["--public", "commit", "-m", "drift"]);
    assert_failure(&output, "pp --public commit -m");
    assert!(stderr(&output).contains("can only run on both"));

    // Refused before anything ran: neither half moved.
    assert_eq!(project.head_public(), public);
    assert_eq!(project.head_private(), private);
}
