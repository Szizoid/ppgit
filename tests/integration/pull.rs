use crate::harness::{Project, TestEnv, assert_failure, stderr, stdout};

/// Two machines on one project: the original pair, and a `pp clone` of
/// it — the second machine that will pull what the first pushes.
fn two_machines(env: &TestEnv) -> (Project<'_>, Project<'_>) {
    let original = env.project("proj");
    original.init();
    original.append(".ppgitignore", "secret.txt\n");
    original.write("public.txt", "hello\n");
    original.write("secret.txt", "shh\n");
    original.commit_all("first");
    original.pp_ok(&["push"]);

    let output = env.pp_in(&env.work_root(), &["clone", "proj", "cloned"]);
    crate::harness::assert_success(&output, "pp clone");
    (original, env.project("cloned"))
}

#[test]
fn pull_carries_a_change_to_a_public_file_into_both_halves() {
    let env = TestEnv::new();
    let (original, clone) = two_machines(&env);

    // The killer case: the pulled change touches a *publicly tracked*
    // file, which a plain dual pull chokes on in either order.
    original.write("public.txt", "more\n");
    original.write("secret.txt", "more secrets\n");
    original.commit_all("second");
    original.pp_ok(&["push"]);

    let output = clone.pp_ok(&["pull"]);
    assert!(stdout(&output).contains("fast-forwarded"));

    // Same content, same heads, both halves.
    assert_eq!(clone.read("public.txt"), "more\n");
    assert_eq!(clone.read("secret.txt"), "more secrets\n");
    assert_eq!(clone.head_public(), original.head_public());
    assert_eq!(clone.head_private(), original.head_private());

    // And nothing left dangling: clean status, healthy doctor.
    assert_eq!(clone.git_stdout(&["status", "--porcelain"]), "");
    let doctor = env.pp_in(&clone.dir, &["doctor"]);
    crate::harness::assert_success(&doctor, "doctor after pull");
    assert!(stdout(&doctor).contains("Everything checks out."));
}

#[test]
fn pull_with_nothing_new_is_quietly_fine() {
    let env = TestEnv::new();
    let (_, clone) = two_machines(&env);

    let output = clone.pp_ok(&["pull"]);
    assert!(stdout(&output).contains("already up to date"));
}

#[test]
fn pull_reports_a_public_divergence_instead_of_guessing() {
    let env = TestEnv::new();
    let (original, clone) = two_machines(&env);

    // The clone commits something publicly only, while the original
    // pushes other public work: the public halves genuinely diverge.
    clone.write("local.txt", "mine\n");
    clone.pp_ok(&["--public", "add", "local.txt"]);
    // Plain, unscoped: with nothing staged privately this still lands
    // only publicly, which is exactly the divergence being simulated.
    clone.pp_ok(&["commit", "--quiet", "-m", "local only"]);

    original.write("pub2.txt", "theirs\n");
    original.commit_all("second");
    original.pp_ok(&["push"]);

    let public_head = clone.head_public();
    let output = clone.pp(&["pull"]);
    assert_failure(&output, "pp pull over a public divergence");
    assert!(stderr(&output).contains("diverged"));

    // The private half still pulled; the public one was left exactly
    // where it was — nothing discarded, nothing guessed.
    assert_eq!(clone.read("pub2.txt"), "theirs\n");
    assert_eq!(clone.head_public(), public_head);
}

#[test]
fn an_explicitly_narrow_pull_stays_passthrough() {
    let env = TestEnv::new();
    let (original, clone) = two_machines(&env);

    original.write("secret.txt", "more secrets\n");
    original.commit_all("second");
    original.pp_ok(&["push"]);

    // A private-only change: the plain private pull handles it, and the
    // public half is deliberately not consulted at all.
    let public_head = clone.head_public();
    clone.pp_ok(&["--private", "pull"]);
    assert_eq!(clone.read("secret.txt"), "more secrets\n");
    assert_eq!(clone.head_public(), public_head);
}
