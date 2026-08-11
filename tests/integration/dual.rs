use crate::harness::{TestEnv, stderr};

#[test]
fn add_and_commit_land_in_both_halves() {
    let env = TestEnv::new();
    let project = env.project("proj");
    project.init();

    project.write("shared.txt", "hello\n");
    project.commit_all("first");

    assert!(project.tracked_public().contains(&"shared.txt".to_string()));
    assert!(
        project
            .tracked_private()
            .contains(&"shared.txt".to_string())
    );
}

#[test]
fn a_listed_path_stays_out_of_the_public_half() {
    let env = TestEnv::new();
    let project = env.project("proj");
    project.init();

    project.append(".ppgitignore", "secret.txt\n");
    project.write("public.txt", "hello\n");
    project.write("secret.txt", "shh\n");
    project.commit_all("first");

    let public = project.tracked_public();
    assert!(public.contains(&"public.txt".to_string()));
    assert!(!public.contains(&"secret.txt".to_string()));
    // The list itself is private too — the public repository must not
    // even give away which paths are excluded.
    assert!(!public.contains(&".ppgitignore".to_string()));

    let private = project.tracked_private();
    assert!(private.contains(&"public.txt".to_string()));
    assert!(private.contains(&"secret.txt".to_string()));
    assert!(private.contains(&".ppgitignore".to_string()));
}

#[test]
fn an_explicit_scope_narrows_to_one_half() {
    let env = TestEnv::new();
    let project = env.project("proj");
    project.init();

    project.write("f.txt", "x\n");
    project.pp_ok(&["--private", "add", "f.txt"]);

    assert!(project.tracked_private().contains(&"f.txt".to_string()));
    assert!(!project.tracked_public().contains(&"f.txt".to_string()));
}

#[test]
fn a_dual_command_announces_each_half() {
    let env = TestEnv::new();
    let project = env.project("proj");
    project.init();

    let output = project.pp_ok(&["status"]);
    let announcements = stderr(&output);
    assert!(announcements.contains("== private (.ppgit) =="));
    assert!(announcements.contains("== public (.git) =="));
}
