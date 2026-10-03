//! The whole Phase 1 pipeline on a small repository, in seconds: mine,
//! validate, a learning run on the older half, `insi learn`, the newer half
//! with lessons off and on, and the report.
//!
//! The workers are `git` commands, so it runs wherever git does and needs
//! no model:
//!
//! - the CARELESS worker writes the task's prompt outside `src/` — it fails
//!   every task, which is what gives `insi learn` something to learn from;
//! - the DILIGENT worker writes the prompt into `src/` — it passes a task
//!   only when the prompt carries the marker the hidden test looks for,
//!   which happens only when a lesson briefed it;
//! - the lesson WRITER prints a fixed lesson naming that marker.
//!
//! So "lessons on" beating "lessons off" here proves that a lesson learned
//! on an older task reaches a newer task's worker and changes the result —
//! the mechanism the real measurement depends on — and that it is never
//! briefed to the task it came from or to an older one.

use std::path::{Path, PathBuf};
use std::process::Command;

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "insi-pipeline-{}-{}-{name}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn git(dir: &Path, args: &[&str], date: Option<&str>) {
    let mut cmd = Command::new("git");
    cmd.args(args).current_dir(dir);
    if let Some(date) = date {
        cmd.env("GIT_AUTHOR_DATE", date)
            .env("GIT_COMMITTER_DATE", date);
    }
    let out = cmd.output().expect("git");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A repository with four dated fixes, each adding a source marker and a
/// hidden test (`tests/<name>.txt`) holding that marker as a pattern.
fn shop() -> PathBuf {
    let repo = temp("shop");
    git(&repo, &["init", "-q", "-b", "main"], None);
    for (k, v) in [
        ("user.email", "t@example.invalid"),
        ("user.name", "t"),
        ("core.autocrlf", "false"),
    ] {
        git(&repo, &["config", k, v], None);
    }
    std::fs::create_dir_all(repo.join("src")).expect("src");
    std::fs::create_dir_all(repo.join("tests")).expect("tests");
    // `.ts`, not `.txt`: mining counts a `.txt` change as docs, not code.
    std::fs::write(repo.join("src/app.ts"), "// shop v1\n").expect("write");
    // A lockfile and an ignored folder setup fills, like `package-lock.json`
    // and `node_modules`.
    std::fs::write(repo.join("deps.lock"), "v1\n").expect("write");
    std::fs::write(repo.join(".gitignore"), "vendor/\n").expect("write");
    git(&repo, &["add", "."], None);
    git(
        &repo,
        &["commit", "-q", "-m", "feat: the shop"],
        Some("2026-01-01T10:00:00+00:00"),
    );
    let fixes = [
        (
            "cart",
            "CART-SUM",
            "fix: cart total",
            "2026-01-02T10:00:00+00:00",
        ),
        (
            "tax",
            "TAX-ROUND",
            "fix: tax rounding",
            "2026-01-03T10:00:00+00:00",
        ),
        (
            "session",
            "SESSION-KEEP",
            "fix: checkout keeps the session",
            "2026-01-04T10:00:00+00:00",
        ),
        (
            "refund",
            "REFUND-OK",
            "fix: refunds",
            "2026-01-05T10:00:00+00:00",
        ),
    ];
    for (name, marker, subject, date) in fixes {
        std::fs::write(
            repo.join(format!("src/{name}.ts")),
            format!("// {marker}\n"),
        )
        .expect("write");
        std::fs::write(
            repo.join(format!("tests/{name}.txt")),
            format!("{marker}\n"),
        )
        .expect("write");
        git(&repo, &["add", "."], None);
        git(&repo, &["commit", "-q", "-m", subject], Some(date));
    }
    repo
}

struct Insi {
    bin: PathBuf,
    cwd: PathBuf,
    common: Vec<String>,
}

impl Insi {
    fn run(&self, args: &[&str]) -> String {
        let out = Command::new(&self.bin)
            .args(args)
            .args(&self.common)
            .current_dir(&self.cwd)
            .output()
            .expect("insi");
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(out.status.success(), "insi {args:?} failed:\n{text}");
        text
    }
}

fn results(file: &Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(file)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

/// Worker attempts carrying every one of `tags`: (task id, passed, briefed
/// lesson ids), sorted by task.
fn attempts(all: &[serde_json::Value], tags: &[&str]) -> Vec<(String, bool, Vec<String>)> {
    let mut out: Vec<(String, bool, Vec<String>)> = all
        .iter()
        .filter(|r| r["kind"] == "worker")
        .filter(|r| {
            r["tags"]
                .as_array()
                .is_some_and(|t| tags.iter().all(|tag| t.iter().any(|x| x == tag)))
        })
        .map(|r| {
            (
                r["task"].as_str().unwrap_or_default().to_string(),
                r["passed"].as_bool().unwrap_or(false),
                r["briefed"]
                    .as_array()
                    .map(|b| {
                        b.iter()
                            .filter_map(|x| x.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default(),
            )
        })
        .collect();
    out.sort();
    out
}

#[test]
fn the_whole_pipeline_runs_and_a_lesson_changes_a_later_result() {
    let repo = shop();
    let tasks = temp("tasks");
    let private = temp("private");
    let work = temp("work");
    let cwd = temp("cwd");
    // The repository's config, read by `mine` from the folder it runs in:
    // setup fills `vendor/`, reusable while `deps.lock` is unchanged.
    std::fs::create_dir_all(cwd.join(".insi")).expect("config dir");
    std::fs::write(
        cwd.join(".insi/config.toml"),
        "setup = [\"git\", \"init\", \"-q\", \"vendor\"]\n\
         [setup_cache]\nkey = [\"deps.lock\"]\nkeep = [\"vendor\"]\n",
    )
    .expect("config");
    let results_file = work.join("results.jsonl");
    let lesson_ini = temp("lesson").join("lesson.ini");
    std::fs::write(
        &lesson_ini,
        "[lesson]\n\tjson = \"{\\\"issue\\\": \\\"The change was written outside src.\\\", \
         \\\"root_cause\\\": \\\"The checks read only the src folder.\\\", \
         \\\"use\\\": \\\"Write fixes under src and keep the SESSION-KEEP marker.\\\", \
         \\\"avoid\\\": \\\"Writing notes outside src\\\"}\"\n",
    )
    .expect("lesson ini");

    let insi = Insi {
        bin: PathBuf::from(env!("CARGO_BIN_EXE_insi")),
        cwd: cwd.clone(),
        common: [
            "--tasks",
            tasks.to_str().expect("utf8"),
            "--private",
            private.to_str().expect("utf8"),
            "--work",
            work.to_str().expect("utf8"),
            "--results",
            results_file.to_str().expect("utf8"),
        ]
        .map(String::from)
        .to_vec(),
    };

    // MINE: four fixes, each a task, its test held back.
    let mined = Command::new(&insi.bin)
        .args([
            "mine",
            "--source",
            repo.to_str().expect("utf8"),
            "--tasks",
            tasks.to_str().expect("utf8"),
            "--private",
            private.to_str().expect("utf8"),
            "--prefix",
            "shop",
            "--wall-secs",
            "60",
            "--check",
            "git grep --no-index -q -f {tests} -- src",
        ])
        .current_dir(&cwd)
        .output()
        .expect("mine");
    let mined_text = String::from_utf8_lossy(&mined.stdout).into_owned();
    assert!(
        mined.status.success(),
        "{mined_text}{}",
        String::from_utf8_lossy(&mined.stderr)
    );
    assert_eq!(
        mined_text
            .lines()
            .filter(|l| l.starts_with("shop-"))
            .count(),
        4,
        "{mined_text}"
    );

    // VALIDATE: every task fails at its base and passes at its fix.
    let validated = insi.run(&["validate", "--jobs", "1"]);
    assert!(validated.contains("4 of 4"), "{validated}");

    let careless = "git config --file notes/fix.ini fix.line {prompt}";
    let diligent = "git config --file src/fix.ini fix.line {prompt}";
    let writer = format!(
        // `--default` before `--get`: after it, git reads it as a second
        // argument and refuses (exit 129).
        "git config --file {} --default {{prompt}} --get lesson.json",
        lesson_ini.display().to_string().replace('\\', "/")
    );

    // LEARNING RUN on the older half: the careless worker fails both.
    insi.run(&[
        "eval",
        "--half",
        "older",
        "--fair-only",
        "--tag",
        "phase=learn",
        "--worker",
        careless,
    ]);
    let all = results(&results_file);
    let learned_on = attempts(&all, &["phase=learn"]);
    assert_eq!(learned_on.len(), 2, "{learned_on:?}");
    assert!(learned_on.iter().all(|(_, passed, _)| !passed));

    // LEARN from the older half only.
    let learned = insi.run(&["learn", "--half", "older", "--worker", &writer]);
    assert!(learned.contains("2 lesson(s) written"), "{learned}");

    // MEASURE the newer half: off, then on.
    insi.run(&[
        "eval",
        "--half",
        "newer",
        "--fair-only",
        "--tag",
        "phase=measure",
        "--worker",
        diligent,
    ]);
    insi.run(&[
        "eval",
        "--half",
        "newer",
        "--fair-only",
        "--tag",
        "phase=measure",
        "--lessons",
        "--worker",
        diligent,
    ]);

    let all = results(&results_file);
    let off = attempts(&all, &["lessons=off", "phase=measure"]);
    let on = attempts(&all, &["lessons=on", "phase=measure"]);
    assert_eq!(off.len(), 2, "{off:?}");
    assert_eq!(on.len(), 2, "{on:?}");
    assert!(
        off.iter()
            .all(|(_, passed, briefed)| !passed && briefed.is_empty()),
        "{off:?}"
    );
    // With lessons, the session task passes: its worker was told the marker.
    // The refund task still fails: no lesson is about it.
    let passed_on: Vec<&str> = on
        .iter()
        .filter(|(_, passed, _)| *passed)
        .map(|(task, ..)| task.as_str())
        .collect();
    assert_eq!(passed_on.len(), 1, "{on:?}");
    // Both older lessons give the same advice, so the briefing carries it
    // once; whichever it is, it comes from a learning-half task, never from
    // a measured one.
    let older: Vec<&str> = learned_on.iter().map(|(t, ..)| t.as_str()).collect();
    for (task, _, briefed) in &on {
        assert_eq!(briefed.len(), 1, "{task}: {briefed:?}");
        assert!(
            older
                .iter()
                .any(|t| briefed[0].starts_with(&format!("{t}-"))),
            "{task} was briefed with {briefed:?}, not a lesson from {older:?}"
        );
    }

    // The report compares the conditions.
    let report = insi.run(&["report"]);
    assert!(
        report.contains("lessons=on,phase=measure: 1/2 passed"),
        "{report}"
    );
    assert!(
        report.contains("lessons=off,phase=measure: 0/2 passed"),
        "{report}"
    );

    // SETUP RAN ONCE. Every attempt — controls and workers, fourteen in all —
    // shares one lockfile, so the first installs and every later one reuses
    // what it left instead of installing again.
    let all = results(&results_file);
    let ran_setup = all
        .iter()
        .filter(|r| r["phases"].get("setup").is_some())
        .count();
    let reused = all
        .iter()
        .filter(|r| r["phases"].get("reuse").is_some() && r["phases"].get("setup").is_none())
        .count();
    assert_eq!(all.len(), 14, "8 controls + 2 learning + 2 off + 2 on");
    assert_eq!(ran_setup, 1, "setup ran more than once");
    assert_eq!(reused, all.len() - 1);
}
