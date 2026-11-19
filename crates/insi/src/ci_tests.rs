// Copyright (c) 2026-2027 Incord Ai.
// https://incord.ai
//
// SPDX-License-Identifier: BUSL-1.1

//! `insi ci` (`ci.rs`): which workflows are checks, how expressions and matrices resolve, what is never run, and a
//! real run of a small workflow's steps with its pass, FAIL, skip and unavailable words.

use super::*;

fn doc(text: &str) -> Yaml {
    ci_yaml::parse(text).unwrap()
}

fn ctx<'a>(
    matrix: &'a BTreeMap<String, String>,
    env: &'a BTreeMap<String, String>,
    repo: &'a Path,
) -> Context<'a> {
    Context {
        matrix,
        env,
        os: "linux",
        repo,
    }
}

fn fresh(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("insi-ci-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_workflow(repo: &Path, file: &str, text: &str) {
    let dir = repo.join(".github").join("workflows");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(file), text).unwrap();
}

fn options(repo: &Path) -> Options {
    Options {
        repo: repo.to_path_buf(),
        workflow: None,
        job: None,
        list: false,
        all_jobs: false,
        timeout: Duration::from_secs(60),
    }
}

fn quiet_run(options: &Options) -> Report {
    run(options, &mut |_| {})
}

fn step<'a>(report: &'a Report, job: &str, name: &str) -> &'a StepResult {
    report
        .jobs
        .iter()
        .find(|j| j.id == job)
        .and_then(|j| j.steps.iter().find(|s| s.name == name))
        .unwrap_or_else(|| panic!("no step `{name}` in job `{job}`: {}", render(report, false)))
}

/// A WORKFLOW IS A CHECK when it runs on a push to a branch or a pull request; one run by a tag, a release, a
/// schedule or by hand is not, and neither is a push of tags only.
#[test]
fn a_workflow_is_a_check_when_a_push_or_pull_request_runs_it() {
    for (text, expected) in [
        ("on: push\njobs: {}\n", true),
        ("on: [push, workflow_dispatch]\njobs: {}\n", true),
        (
            "on:\n  pull_request:\n  schedule:\n    - cron: '1 2 * * 3'\njobs: {}\n",
            true,
        ),
        ("on:\n  push:\n    branches: [main]\njobs: {}\n", true),
        ("on:\n  push:\n    tags: ['v*']\njobs: {}\n", false),
        ("on:\n  release:\n    types: [published]\njobs: {}\n", false),
        (
            "on:\n  schedule:\n    - cron: '1 2 * * 3'\n  workflow_dispatch:\njobs: {}\n",
            false,
        ),
        ("jobs: {}\n", false),
    ] {
        assert_eq!(is_check(&doc(text)), expected, "{text}");
    }
}

/// EXPRESSIONS resolve from the matrix, the environment and the runner, with `||` defaults; one that needs a secret
/// or another step's output is named, never guessed.
#[test]
fn expressions_resolve_from_the_matrix_and_the_environment_or_are_named() {
    let matrix: BTreeMap<String, String> = [
        ("os".to_string(), "ubuntu-24.04".to_string()),
        ("python".to_string(), "3.12".to_string()),
    ]
    .into();
    let env: BTreeMap<String, String> = [("MODE".to_string(), "fast".to_string())].into();
    let repo = PathBuf::from("/work/repo");
    let c = ctx(&matrix, &env, &repo);
    assert_eq!(
        expand(
            "py ${{ matrix.python }} on ${{ runner.os }} ${{env.MODE}}",
            &c
        )
        .unwrap(),
        "py 3.12 on Linux fast"
    );
    assert_eq!(
        expand("${{ matrix.missing || 'default' }}", &c).unwrap(),
        "default"
    );
    assert_eq!(expand("${{ github.workspace }}", &c).unwrap(), "/work/repo");
    assert_eq!(expand("no expression", &c).unwrap(), "no expression");
    for needs_more in [
        "${{ secrets.TOKEN }}",
        "${{ steps.build.outputs.path }}",
        "${{ github.ref }}",
    ] {
        let why = expand(&format!("echo {needs_more}"), &c).unwrap_err();
        assert!(why.contains("cannot be resolved"), "{why}");
    }
    assert!(expand("${{ matrix.os", &c)
        .unwrap_err()
        .contains("not closed"));
}

/// CONDITIONS: `always()`, `success()`, `failure()`, comparisons on the matrix and the runner, `!`, `&&` and `||`;
/// one it cannot decide is `None`, and the step is skipped with that said.
#[test]
fn conditions_are_decided_from_what_is_known_here() {
    let matrix: BTreeMap<String, String> = [("os".to_string(), "ubuntu-latest".to_string())].into();
    let env = BTreeMap::new();
    let repo = PathBuf::from(".");
    let c = ctx(&matrix, &env, &repo);
    for (text, failed, expected) in [
        ("always()", true, Some(true)),
        ("${{ always() }}", false, Some(true)),
        ("success()", false, Some(true)),
        ("success()", true, Some(false)),
        ("failure()", true, Some(true)),
        ("runner.os == 'Linux'", false, Some(true)),
        ("runner.os != 'Linux'", false, Some(false)),
        (
            "matrix.os == 'ubuntu-latest' && runner.os == 'Linux'",
            false,
            Some(true),
        ),
        (
            "matrix.os == 'windows-latest' || runner.os == 'Linux'",
            false,
            Some(true),
        ),
        ("!(always())", false, None),
        ("!cancelled()", false, Some(true)),
        ("github.event_name == 'push'", false, Some(true)),
        ("false", false, Some(false)),
        ("github.ref == 'refs/heads/main'", false, None),
        (
            "contains(github.event.head_commit.message, 'skip')",
            false,
            None,
        ),
    ] {
        assert_eq!(
            condition(text, &c, failed),
            expected,
            "{text} (failed before: {failed})"
        );
    }
}

/// MATRICES: the product of the lists, `exclude` removing, `include` adding to the entries it matches or as a new one;
/// an expression for the whole matrix cannot be expanded.
#[test]
fn a_matrix_expands_with_include_and_exclude() {
    let job = doc("strategy:\n  matrix:\n    os: [a, b]\n    v: ['1', '2']\n    exclude:\n      - os: b\n        v: '2'\n    include:\n      - os: a\n        extra: yes\n      - os: c\n        v: '9'\n");
    let combos = matrix_combos(&job).unwrap();
    let words: Vec<String> = combos
        .iter()
        .map(|c| {
            c.iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join(",")
        })
        .collect();
    assert_eq!(
        words,
        [
            "extra=yes,os=a,v=1",
            "extra=yes,os=a,v=2",
            "os=b,v=1",
            "os=c,v=9"
        ]
    );
    assert_eq!(
        matrix_combos(&doc("steps: []\n")).unwrap(),
        vec![BTreeMap::new()]
    );
    assert!(matrix_combos(&doc(
        "strategy:\n  matrix: ${{ fromJson(needs.x.outputs.m) }}\n"
    ))
    .unwrap_err()
    .contains("expression"));
}

/// WHAT IS NEVER RUN from a developer's machine: a step that would publish or push.
#[test]
fn a_step_that_would_publish_or_push_is_never_run() {
    for script in [
        "git push origin main",
        "cargo build\ncargo publish --token x",
        "npm publish",
        "docker push img",
        "gh release create v1",
        "echo x && twine upload dist/*",
    ] {
        assert!(would_publish(script).is_some(), "{script}");
    }
    for script in [
        "cargo test",
        "# git push is only a comment\necho ok",
        "git status",
        "echo \"push the button\"",
    ] {
        assert_eq!(would_publish(script), None, "{script}");
    }
}

/// WHAT A DEVELOPER'S MACHINE KEEPS IS NEVER CHANGED: a CI runner is thrown away, a developer's machine is not (Insi's
/// own CI sets `git config --global user.email`, which would sign every later commit with the CI's address).
#[test]
fn a_step_that_would_change_the_machine_is_never_run() {
    for script in [
        "git config --global user.email \"ci@example.invalid\"",
        "sudo apt-get install -y jq",
        "brew install ripgrep",
        "npm install -g pnpm",
        "echo x\nrustup default nightly",
    ] {
        assert!(would_change_machine(script).is_some(), "{script}");
    }
    for script in [
        "git config user.email ci@example.invalid",
        "cargo test",
        "npm install",
        "# sudo is only a comment\necho ok",
    ] {
        assert_eq!(would_change_machine(script), None, "{script}");
    }
}

/// A working directory stays inside the repository.
#[test]
fn a_working_directory_cannot_leave_the_repository() {
    let repo = Path::new("/work/repo");
    assert_eq!(confined(repo, "crates/x").unwrap(), repo.join("crates/x"));
    for outside in ["../other", "/etc", "a/../../b"] {
        assert!(confined(repo, outside).is_err(), "{outside}");
    }
}

/// THIS REPOSITORY'S OWN WORKFLOWS, planned (nothing runs): the push and pull request ones are read, the test job
/// of `ci.yml` is on this machine with its `cargo fmt --check` step, and the release workflow is left out with why.
#[test]
fn this_repository_workflows_are_planned() {
    let repo = std::fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")).unwrap();
    let mut opts = options(&repo);
    opts.list = true;
    let report = quiet_run(&opts);
    let test = report
        .jobs
        .iter()
        .find(|j| j.workflow == "ci.yml" && j.id == "test")
        .unwrap_or_else(|| panic!("{}", render(&report, false)));
    assert_ne!(
        test.status,
        Status::Skipped,
        "the test job runs on this OS ({}): {}",
        this_os(),
        test.reason
    );
    assert!(
        test.steps
            .iter()
            .any(|s| s.name == "cargo fmt --check" && s.status == Status::Planned),
        "{}",
        render(&report, false)
    );
    assert!(
        test.steps
            .iter()
            .any(|s| s.name.starts_with("actions/checkout") && s.status == Status::Skipped),
        "uses: is skipped"
    );
    assert!(
        report
            .notes
            .iter()
            .any(|n| n.starts_with("release.yml: left out")),
        "{:?}",
        report.notes
    );
    // Nothing ran: no step has a time.
    assert!(report
        .jobs
        .iter()
        .flat_map(|j| &j.steps)
        .all(|s| s.seconds == 0.0));
}

/// THE FILES GITHUB GIVES A STEP: a step that writes its output, a step summary, an env line or a path line passes
/// here as on GitHub (Insi's coverage summary and semver baseline steps failed with the variables unset).
#[test]
fn a_step_writes_to_the_files_github_gives_it() {
    let repo = fresh("github-files");
    let runner = match this_os() {
        "windows" => "windows-latest",
        "macos" => "macos-latest",
        _ => "ubuntu-latest",
    };
    write_workflow(
        &repo,
        "ci.yml",
        &format!("on: [push]\njobs:\n  report:\n    runs-on: {runner}\n    steps:\n      - name: writes\n        shell: bash\n        run: |\n          echo \"x=1\" >> \"$GITHUB_OUTPUT\"\n          echo \"## done\" >> \"$GITHUB_STEP_SUMMARY\"\n          echo \"A=1\" >> \"$GITHUB_ENV\"\n          echo \"/tmp\" >> \"$GITHUB_PATH\"\n"),
    );
    let report = quiet_run(&options(&repo));
    assert_eq!(
        report.jobs[0].steps[0].status,
        Status::Passed,
        "{}",
        render(&report, false)
    );
}

/// A CHECK ACTION RUNS AS ITS TOOL (the owner, 2026-11-19: "all tests local"; Insi's spelling job failed on GitHub
/// while its `crate-ci/typos` action was only ever skipped here): typos, zizmor, cargo-deny and cargo-semver-checks
/// with their `with:` inputs; any other action is not a check and stays GitHub's.
#[test]
fn a_check_action_runs_as_its_tool_and_others_stay_on_github() {
    let step = |yaml: &str| crate::ci_yaml::parse(yaml).unwrap();
    let typos = step("uses: crate-ci/typos@v1\n");
    assert_eq!(
        check_action("crate-ci/typos@abc", &typos).as_deref(),
        Some("typos")
    );
    let deny = step("uses: EmbarkStudios/cargo-deny-action@v2\nwith:\n  command: check bans licenses\n  arguments: --all-features\n");
    assert_eq!(
        check_action("EmbarkStudios/cargo-deny-action@v2", &deny).as_deref(),
        Some("cargo deny --all-features check bans licenses")
    );
    let zizmor = step("uses: zizmorcore/zizmor-action@v0\n");
    assert_eq!(
        check_action("zizmorcore/zizmor-action@v0", &zizmor).as_deref(),
        Some("zizmor --offline .")
    );
    let semver =
        step("uses: obi1kenobi/cargo-semver-checks-action@v2\nwith:\n  package: insi-core\n");
    assert_eq!(
        check_action("obi1kenobi/cargo-semver-checks-action@v2", &semver).as_deref(),
        Some("cargo semver-checks --package insi-core")
    );
    for other in [
        "actions/checkout@v4",
        "actions/upload-artifact@v4",
        "some/deploy-action@v1",
    ] {
        assert_eq!(check_action(other, &typos), None, "{other}");
    }
}

/// EVERY JOB WITH --all-jobs (the owner, 2026-11-19: "add all tests local"): a job GitHub runs on another operating
/// system is skipped, with why, unless asked; asked, it runs here, says it was asked, and its step passes.
#[test]
fn a_job_for_another_os_runs_here_only_when_asked() {
    let repo = fresh("all-jobs");
    let elsewhere = if this_os() == "windows" {
        "ubuntu-latest"
    } else {
        "windows-latest"
    };
    write_workflow(
        &repo,
        "ci.yml",
        &format!("on: [push]\njobs:\n  spelling:\n    runs-on: {elsewhere}\n    steps:\n      - name: says hello\n        run: echo hello\n"),
    );
    let mut opts = options(&repo);
    let report = quiet_run(&opts);
    let job = &report.jobs[0];
    assert_eq!(job.status, Status::Skipped, "{}", render(&report, false));
    assert!(job.reason.contains("not on this machine"), "{}", job.reason);

    opts.all_jobs = true;
    let report = quiet_run(&opts);
    let job = &report.jobs[0];
    assert!(job.reason.contains("--all-jobs"), "{}", job.reason);
    assert_eq!(
        job.steps[0].status,
        Status::Passed,
        "{}",
        render(&report, false)
    );
}

/// A REAL RUN: a passing step, the environment of the workflow, job and step, `uses:` and a secret and a push
/// skipped with why, a failing step, the steps after it skipped unless `always()`, a tool that is not installed
/// (unavailable, not a failure), a job that needs the failed one, and a timeout.
#[test]
fn a_workflow_runs_step_by_step_and_says_what_each_did() {
    let repo = fresh("run");
    write_workflow(
        &repo,
        "ci.yml",
        "name: ci\non: push\nenv:\n  GREETING: hi\njobs:\n  build:\n    runs-on: ${{ matrix.os }}\n    strategy:\n      matrix:\n        os: [ubuntu-latest, macos-latest, windows-latest]\n    env:\n      WHO: ${{ matrix.os }}\n    steps:\n      - uses: actions/checkout@v7\n      - name: environment\n        shell: bash\n        env:\n          STEP: one\n        run: |\n          test \"$GREETING\" = hi\n          test \"$STEP\" = one\n          test -n \"$WHO\"\n          test \"$CI\" = true\n      - name: secret\n        shell: bash\n        run: echo ${{ secrets.TOKEN }}\n      - name: push\n        shell: bash\n        run: git push origin main\n      - name: not installed\n        shell: bash\n        run: definitely-not-an-installed-tool-xyz\n      - name: breaks\n        shell: bash\n        run: |\n          echo about to fail\n          exit 3\n      - name: after the failure\n        shell: bash\n        run: echo never\n      - name: cleanup\n        if: always()\n        shell: bash\n        run: echo cleaned\n  later:\n    needs: [build]\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo hi\n      \n",
    );
    let report = quiet_run(&options(&repo));
    let text = render(&report, false);
    // Only on the OS of the machine the test runs on: the first matrix entry for this OS ran.
    assert_eq!(
        step(&report, "build", "actions/checkout@v7").status,
        Status::Skipped,
        "{text}"
    );
    assert!(
        step(&report, "build", "actions/checkout@v7")
            .reason
            .contains("already here"),
        "{text}"
    );
    assert_eq!(
        step(&report, "build", "environment").status,
        Status::Passed,
        "{text}"
    );
    let secret = step(&report, "build", "secret");
    assert!(
        secret.status == Status::Skipped && secret.reason.contains("secrets.TOKEN"),
        "{text}"
    );
    let pushed = step(&report, "build", "push");
    assert!(
        pushed.status == Status::Skipped && pushed.reason.contains("never run from here"),
        "{text}"
    );
    let missing = step(&report, "build", "not installed");
    assert_eq!(missing.status, Status::Unavailable, "{text}");
    let broke = step(&report, "build", "breaks");
    assert!(
        broke.status == Status::Failed
            && broke.reason == "exit code 3"
            && broke.tail.contains("about to fail"),
        "{text}"
    );
    let after = step(&report, "build", "after the failure");
    assert!(
        after.status == Status::Skipped && after.reason.contains("earlier step failed"),
        "{text}"
    );
    assert_eq!(
        step(&report, "build", "cleanup").status,
        Status::Passed,
        "{text}"
    );
    let build = report.jobs.iter().find(|j| j.id == "build").unwrap();
    assert_eq!(build.status, Status::Failed, "{text}");
    assert!(
        build.reason.contains("also in the matrix on this OS") || build.reason.is_empty(),
        "{text}"
    );
    let later = report.jobs.iter().find(|j| j.id == "later").unwrap();
    assert!(
        later.status == Status::Skipped && later.reason.contains("`build` it needs failed"),
        "{text}"
    );
    assert_eq!(report.failed(), 1, "{text}");
    assert!(
        report.unavailable() == 1 && report.count(Status::Passed) == 2,
        "{text}"
    );
    // The text and the JSON say the same.
    assert!(
        text.contains("[FAIL] breaks")
            && text.contains("exit code 3")
            && text.contains("about to fail"),
        "{text}"
    );
    assert!(
        text.ends_with("2 passed, 1 failed, 5 skipped, 1 unavailable\n"),
        "{text}"
    );
    let json = to_json(&report);
    assert_eq!(json["ok"], false);
    assert_eq!(json["failed"], 1);
    assert_eq!(json["jobs"][0]["steps"][5]["status"], "FAIL");
    // Quiet keeps what failed and the summary only.
    let quiet = render(&report, true);
    assert!(
        quiet.contains("[FAIL] breaks")
            && !quiet.contains("cleanup")
            && quiet.contains("insi ci on"),
        "{quiet}"
    );
    let _ = std::fs::remove_dir_all(&repo);
}

/// A STEP THAT RUNS TOO LONG is stopped at the deadline and failed, saying so.
#[test]
fn a_step_that_runs_too_long_is_stopped() {
    let repo = fresh("timeout");
    write_workflow(&repo, "ci.yml", "on: push\njobs:\n  slow:\n    runs-on: ${{ matrix.os }}\n    strategy:\n      matrix:\n        os: [ubuntu-latest, macos-latest, windows-latest]\n    steps:\n      - name: waits\n        shell: bash\n        run: sleep 30\n");
    let mut opts = options(&repo);
    opts.timeout = Duration::from_secs(1);
    let report = quiet_run(&opts);
    let waits = step(&report, "slow", "waits");
    assert!(
        waits.status == Status::Failed && waits.reason.contains("timed out after 1 s"),
        "{}",
        render(&report, false)
    );
    let _ = std::fs::remove_dir_all(&repo);
}

/// WHAT CANNOT RUN HERE is said, not run: a job for another OS, a job in a container, a workflow that cannot be read,
/// a workflow named with --workflow though it is not a check; --job picks one job.
#[test]
fn jobs_for_other_systems_and_containers_are_reported_not_run() {
    let repo = fresh("skips");
    let other = if this_os() == "windows" {
        "ubuntu-latest"
    } else {
        "windows-latest"
    };
    write_workflow(
        &repo,
        "ci.yml",
        &format!("on: [push]\njobs:\n  elsewhere:\n    runs-on: {other}\n    steps:\n      - run: echo hi\n  boxed:\n    runs-on: ${{{{ matrix.os }}}}\n    strategy:\n      matrix:\n        os: [ubuntu-latest, macos-latest, windows-latest]\n    container: rust:1\n    steps:\n      - run: echo hi\n"),
    );
    write_workflow(&repo, "broken.yml", "on: push\njobs: [unclosed\n");
    write_workflow(&repo, "release.yml", "on:\n  push:\n    tags: ['v*']\njobs:\n  ship:\n    runs-on: ubuntu-latest\n    steps:\n      - run: echo shipping\n");
    let report = quiet_run(&options(&repo));
    let elsewhere = report.jobs.iter().find(|j| j.id == "elsewhere").unwrap();
    assert!(
        elsewhere.status == Status::Skipped && elsewhere.reason.contains("not on this machine"),
        "{}",
        render(&report, false)
    );
    let boxed = report.jobs.iter().find(|j| j.id == "boxed").unwrap();
    assert!(
        boxed.reason.contains("container"),
        "{}",
        render(&report, false)
    );
    assert!(
        report
            .notes
            .iter()
            .any(|n| n.starts_with("broken.yml: could not be read")),
        "{:?}",
        report.notes
    );
    assert!(
        report
            .notes
            .iter()
            .any(|n| n.starts_with("release.yml: left out")),
        "{:?}",
        report.notes
    );
    assert!(report.jobs.iter().all(|j| j.id != "ship"));
    // Named, a workflow that is not a check is run.
    let mut named = options(&repo);
    named.workflow = Some("release".into());
    named.list = true;
    assert!(quiet_run(&named).jobs.iter().any(|j| j.id == "ship"));
    // One job.
    let mut one = options(&repo);
    one.job = Some("elsewhere".into());
    assert_eq!(quiet_run(&one).jobs.len(), 1);
    let _ = std::fs::remove_dir_all(&repo);
}

/// THE PRE-PUSH HOOK is written where git looks for hooks, is ours (marked), is rewritten when ours, is never put
/// over somebody else's hook without --force, and runs `insi ci --quiet`.
#[test]
fn the_pre_push_hook_is_installed_and_never_replaces_another() {
    let repo = fresh("hook");
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(args)
            .output()
            .unwrap()
    };
    assert!(git(&["init", "-q"]).status.success());
    let path = install_hook(&repo, false).unwrap();
    assert!(
        path.ends_with("hooks/pre-push") || path.ends_with("hooks\\pre-push"),
        "{}",
        path.display()
    );
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(
        text.starts_with("#!/bin/sh\n")
            && text.contains(HOOK_MARK)
            && text.contains("insi ci --repo")
            && text.contains("--quiet")
            && text.contains("INSI_CI_SKIP"),
        "{text}"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o111,
            0o111,
            "executable"
        );
    }
    // Ours again: rewritten.
    assert_eq!(install_hook(&repo, false).unwrap(), path);
    // Somebody else's: refused, then replaced only when forced.
    std::fs::write(&path, "#!/bin/sh\necho theirs\n").unwrap();
    assert!(install_hook(&repo, false)
        .unwrap_err()
        .contains("not written by insi"));
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "#!/bin/sh\necho theirs\n"
    );
    install_hook(&repo, true).unwrap();
    assert!(std::fs::read_to_string(&path).unwrap().contains(HOOK_MARK));
    // Not a repository.
    let plain = fresh("hook-plain");
    assert!(install_hook(&plain, false)
        .unwrap_err()
        .contains("not a git repository"));
    let _ = std::fs::remove_dir_all(&repo);
    let _ = std::fs::remove_dir_all(&plain);
}
