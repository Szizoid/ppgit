use std::fs;

use crate::harness::{Project, TestEnv, assert_failure, stdout};

/// Same healthy starting point as the doctor tests.
fn healthy_pair(env: &TestEnv) -> Project<'_> {
    let project = env.project("proj");
    project.init();
    project.append(".ppgitignore", "secret.txt\n");
    project.write("public.txt", "hello\n");
    project.write("secret.txt", "shh\n");
    project.commit_all("first");
    project.pp_ok(&["push"]);
    project
}

/// `doctor --fix` expected to repair everything: asserts success and the
/// re-check coming up clean.
fn fix_and_recheck(project: &Project<'_>) -> String {
    let output = project.pp_ok(&["doctor", "--fix"]);
    let report = stdout(&output);
    assert!(report.contains("fixed"), "nothing was fixed:\n{report}");

    let recheck = project.pp_ok(&["doctor"]);
    let recheck = stdout(&recheck);
    assert!(
        recheck.contains("Everything checks out.") || recheck.contains("No problems"),
        "still unhealthy after --fix:\n{recheck}"
    );
    report
}

#[test]
fn restores_a_missing_fetch_refspec() {
    let env = TestEnv::new();
    let project = healthy_pair(&env);

    // `--unset-all`, not `--unset`: the key now holds two values (the
    // branch refspec and the pairing-notes one), and a plain `--unset`
    // refuses to pick one on a multi-valued key.
    project.private_git_ok(&["config", "--unset-all", "remote.origin.fetch"]);

    fix_and_recheck(&project);
    assert_eq!(
        project.private_git_stdout(&["config", "remote.origin.fetch"]),
        "+refs/heads/*:refs/remotes/origin/*"
    );
}

#[test]
fn restores_a_missing_origin_over_two_passes() {
    let env = TestEnv::new();
    let project = healthy_pair(&env);

    // `remote remove` also drops branch.main.merge and the
    // remote-tracking refs, so this breaks *two* fixable things — and
    // the second (the severed upstream) only becomes fixable once the
    // first fix has landed and a fetch has repopulated origin/main.
    // Hence the advertised "run doctor again": one pass per layer.
    project.git_ok(&["remote", "remove", "origin"]);

    let first = project.pp(&["doctor", "--fix"]);
    assert!(stdout(&first).contains("fixed"));
    assert_eq!(
        project.git_stdout(&["remote", "get-url", "origin"]),
        env.remote("tester/proj").to_str().unwrap()
    );

    fix_and_recheck(&project);
    assert_eq!(
        project.git_stdout(&["rev-parse", "--abbrev-ref", "@{upstream}"]),
        "origin/main"
    );
}

#[test]
fn repoints_a_wrong_origin() {
    let env = TestEnv::new();
    let project = healthy_pair(&env);

    let ssh = fs::read_to_string(env.record("tester/proj").join("ssh")).unwrap();
    project.git_ok(&["remote", "set-url", "origin", &ssh]);

    fix_and_recheck(&project);
    assert_eq!(
        project.git_stdout(&["remote", "get-url", "origin"]),
        env.remote("tester/proj").to_str().unwrap()
    );
}

#[test]
fn restores_a_severed_upstream() {
    let env = TestEnv::new();
    let project = healthy_pair(&env);

    project.git_ok(&["config", "--unset", "branch.main.merge"]);

    fix_and_recheck(&project);
    assert_eq!(
        project.git_stdout(&["rev-parse", "--abbrev-ref", "@{upstream}"]),
        "origin/main"
    );
}

#[test]
fn untracks_listed_files_but_leaves_the_commit_to_the_user() {
    let env = TestEnv::new();
    let project = healthy_pair(&env);

    // public.txt was committed publicly before being listed.
    project.append(".ppgitignore", "public.txt\n");

    let output = project.pp_ok(&["doctor", "--fix"]);
    let report = stdout(&output);
    assert!(report.contains("fixed"));
    assert!(report.contains("pp commit"));

    // The removal is staged, the file untouched on disk — committing is
    // deliberately the user's move.
    assert_eq!(
        project.git_stdout(&["diff", "--cached", "--name-status"]),
        "D\tpublic.txt"
    );
    assert_eq!(project.read("public.txt"), "hello\n");
    // Plain, unscoped: nothing is staged privately, so it lands only
    // publicly — `commit` itself now refuses `--public`/`--private`.
    project.pp_ok(&["commit", "--quiet", "-m", "untrack"]);

    // Healthy again — the only remainder is the note about that commit
    // waiting to be pushed.
    let recheck = project.pp_ok(&["doctor"]);
    assert!(stdout(&recheck).contains("No problems"));
}

#[test]
fn reinstalls_the_hook() {
    let env = TestEnv::new();
    let project = healthy_pair(&env);

    fs::remove_file(project.dir.join(".git/hooks/pre-push")).unwrap();

    fix_and_recheck(&project);
    assert!(
        project
            .read(".git/hooks/pre-push")
            .contains("installed by ppgit")
    );
}

#[test]
fn a_divergence_stays_report_only() {
    let env = TestEnv::new();
    let project = healthy_pair(&env);

    let side = env.side_clone("tester/proj", "side");
    side.write("side.txt", "b\n");
    side.git_ok(&["add", "side.txt"]);
    side.git_ok(&["commit", "--quiet", "-m", "from the side"]);
    side.git_ok(&["push", "--quiet"]);
    project.write("local.txt", "c\n");
    project.commit_all("local");

    let output = project.pp(&["doctor", "--fix"]);
    assert_failure(&output, "doctor --fix on a divergence");
    assert!(stdout(&output).contains("diverged"));
}

#[test]
fn a_clean_pair_has_nothing_to_fix() {
    let env = TestEnv::new();
    let project = healthy_pair(&env);

    let output = project.pp_ok(&["doctor", "--fix"]);
    assert!(stdout(&output).contains("Everything checks out."));
}

#[test]
fn rejects_unknown_flags() {
    let env = TestEnv::new();
    let project = healthy_pair(&env);

    let output = project.pp(&["doctor", "--fix-everything"]);
    assert_failure(&output, "doctor with an unknown flag");
}
