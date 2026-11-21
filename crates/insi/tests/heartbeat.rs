// Copyright (c) 2026-2027 Incord Ai.
// https://incord.ai
//
// SPDX-License-Identifier: BUSL-1.1

//! Exercise the compiled scheduler without invoking a model or network service.
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// Return fixture failures to the test harness without suppressing error-handling lints.
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// Keep fixtures in the project's ignored scratch directory, including in CI.
fn repo() -> TestResult<PathBuf> {
    // Wall-clock precision alone can collide between parallel tests on macOS.
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let serial = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.incord/temp")
        .join(format!("heartbeat-{}-{stamp}-{serial}", std::process::id()));
    std::fs::create_dir_all(&path)?;
    assert!(Command::new("git")
        .args(["init", "--quiet"])
        .arg(&path)
        .status()?
        .success());
    Ok(path)
}

/// Run the real CLI with a missing test configuration so no worker is reached.
fn run(repo: &Path, args: &[&str]) -> TestResult<Output> {
    Ok(Command::new(env!("CARGO_BIN_EXE_insi"))
        .args(["heartbeat", "--repo"])
        .arg(repo)
        .args(args)
        .output()?)
}

/// Read durable scheduler evidence independently of its console output.
fn state(repo: &Path) -> TestResult<Value> {
    Ok(serde_json::from_slice(&std::fs::read(
        repo.join(".git/insi-heartbeat/state.json"),
    )?)?)
}

/// An error is recorded, and restarting before the deadline must not retry it.
#[test]
fn restart_respects_deadline_and_records_error() -> TestResult<()> {
    let repo = repo()?;
    assert!(!run(&repo, &["--once"])?.status.success());
    let first = state(&repo)?;
    assert_eq!(first["attempts"], 1);
    assert_eq!(first["status"], "failed");
    assert!(run(&repo, &["--once"])?.status.success());
    assert_eq!(state(&repo)?, first);
    let status = run(&repo, &["--status"])?;
    assert_eq!(serde_json::from_slice::<Value>(&status.stdout)?, first);
    Ok(())
}

/// Failures pause the loop, and explicit reset restores scheduling without bypassing deadlines.
#[test]
fn repeated_errors_pause_until_reset() -> TestResult<()> {
    let repo = repo()?;
    let output = run(&repo, &["--interval-secs", "1", "--max-cycles", "3"])?;
    assert!(!output.status.success());
    let first = state(&repo)?;
    assert_eq!(first["attempts"], 3);
    assert_eq!(first["failures"], 3);
    assert!(!run(&repo, &["--once"])?.status.success());
    assert_eq!(state(&repo)?, first);
    // Pin a future deadline so reset behavior remains deterministic on slow CI hosts.
    let mut future = first.clone();
    future["next_due"] = json!(u64::MAX);
    std::fs::write(
        repo.join(".git/insi-heartbeat/state.json"),
        future.to_string(),
    )?;
    assert!(run(&repo, &["--once", "--reset-failures"])?
        .status
        .success());
    assert_eq!(state(&repo)?["failures"], 0);
    Ok(())
}

/// The OS lock excludes other scheduler processes even when their work paths differ.
#[test]
fn competing_process_is_refused_and_corrupt_state_fails_closed() -> TestResult<()> {
    let repo = repo()?;
    let dir = repo.join(".git/insi-heartbeat");
    std::fs::create_dir_all(&dir)?;
    let lock = std::fs::File::create(dir.join("lock"))?;
    lock.try_lock()?;
    let output = run(&repo, &["--once", "--work", "other-work"])?;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("already running"));
    drop(lock);
    std::fs::write(dir.join("state.json"), "broken")?;
    assert!(!run(&repo, &["--once"])?.status.success());
    assert_eq!(std::fs::read_to_string(dir.join("state.json"))?, "broken");
    Ok(())
}

/// Reject invalid timing before starting a cycle, and count interrupted attempts toward the cap.
#[test]
fn invalid_interval_and_interrupted_attempt() -> TestResult<()> {
    let repo = repo()?;
    assert!(!run(&repo, &["--once", "--interval-secs", "0"])?
        .status
        .success());
    let dir = repo.join(".git/insi-heartbeat");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(
        dir.join("state.json"),
        json!({"next_due":0,"attempts":2,"failures":2,"status":"running","error":null}).to_string(),
    )?;
    assert!(!run(&repo, &["--once"])?.status.success());
    assert_eq!(state(&repo)?["failures"], 3);
    assert_eq!(state(&repo)?["attempts"], 3);
    Ok(())
}

/// Unsupported execution controls must fail before a task is observed or claimed.
#[test]
fn unsupported_execution_options_are_refused_before_observation() -> TestResult<()> {
    let repo = repo()?;
    for args in [["--audit-finding", "finding-1"], ["--setup-jobs", "2"]] {
        let output = run(&repo, &args)?;
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("unsupported heartbeat option"));
    }
    assert!(!repo.join(".git/insi-heartbeat/state.json").exists());
    Ok(())
}

/// A committed project Insi can run in: its config, its plan, nothing else. `git` is the one program every check
/// calls, so a clean run needs no model and no scanner.
fn committed() -> TestResult<PathBuf> {
    let repo = repo()?;
    let git = |args: &[&str]| -> TestResult<()> {
        let out = Command::new("git")
            .current_dir(&repo)
            .args(["-c", "user.name=Test", "-c", "user.email=test@localhost"])
            .args(args)
            .output()?;
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        Ok(())
    };
    std::fs::create_dir_all(repo.join(".insi"))?;
    std::fs::write(
        repo.join(".insi/config.toml"),
        "test_cmd = [\"git\", \"--version\"]\nwall_secs = 60\n",
    )?;
    std::fs::write(
        repo.join(".insi/heartbeat.json"),
        json!({"security_checks":[["git","--version"]],"max_tasks":2}).to_string(),
    )?;
    std::fs::write(repo.join(".gitignore"), ".insi-work/\n")?;
    git(&["add", "."])?;
    git(&["commit", "--quiet", "-m", "Baseline"])?;
    Ok(repo)
}

/// An address nothing listens on, so a ping to the engine fails at once.
fn closed() -> TestResult<String> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?.to_string())
}

/// Run one offline heartbeat tick, the engine at `engine`.
fn tick(repo: &Path, engine: &str, args: &[&str]) -> TestResult<Output> {
    Ok(Command::new(env!("CARGO_BIN_EXE_insi"))
        .args(["heartbeat", "--once", "--offline", "--repo"])
        .arg(repo)
        .args(args)
        .env("INCORD_MEMORY_ADDR", engine)
        .output()?)
}

/// Uncommitted work is waiting, never a failure (the owner, 2026-11-20): the state says why, no failure is counted,
/// and the run goes ahead once the work is committed. The engine is pinged with the project.
#[test]
fn uncommitted_work_waits_and_asks_the_engine_to_have_it_committed() -> TestResult<()> {
    use std::io::{Read, Write};
    let repo = committed()?;
    std::fs::write(repo.join("one.txt"), "agent work")?;
    std::fs::write(repo.join("two.txt"), "agent work")?;
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?.to_string();
    let server = std::thread::spawn(move || -> Result<String, String> {
        let (mut stream, _) = listener.accept().map_err(|e| e.to_string())?;
        let mut raw = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            let n = stream.read(&mut buf).map_err(|e| e.to_string())?;
            raw.extend_from_slice(&buf[..n]);
            let text = String::from_utf8_lossy(&raw).into_owned();
            if let Some((head, body)) = text.split_once("\r\n\r\n") {
                let want = head
                    .lines()
                    .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                    .unwrap_or(0);
                if body.len() >= want {
                    let reply = r#"{"asked":["session-1"]}"#;
                    write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len())
                        .map_err(|e| e.to_string())?;
                    return Ok(text);
                }
            }
            if n == 0 {
                return Err("the ping ended early".into());
            }
        }
    });
    let output = tick(&repo, &address, &["--interval-secs", "1"])?;
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let request = server.join().map_err(|_| "ping server panicked")??;
    assert!(request.starts_with("POST /v1/memory/insi/waiting "), "{request}");
    let body: Value = serde_json::from_str(request.split_once("\r\n\r\n").ok_or("body")?.1)?;
    assert_eq!(body["uncommitted"], 2);
    let named = std::fs::canonicalize(&repo)?;
    assert_eq!(
        std::fs::canonicalize(body["cwd"].as_str().ok_or("cwd")?.trim_start_matches(r"\\?\"))?,
        named
    );
    let waiting = state(&repo)?;
    assert_eq!(waiting["status"], "waiting");
    assert_eq!(waiting["failures"], 0);
    assert_eq!(waiting["error"], Value::Null);
    assert_eq!(
        waiting["waiting"],
        "2 uncommitted files; Insi runs when they are committed"
    );
    let printed = String::from_utf8_lossy(&output.stdout);
    assert!(printed.contains(r#""status":"waiting""#), "{printed}");
    assert!(printed.contains("session-1"), "the proxy's answer is in the report: {printed}");

    // The agent commits; the next scheduled run goes ahead.
    let committed = Command::new("git")
        .current_dir(&repo)
        .args(["-c", "user.name=Test", "-c", "user.email=test@localhost", "add", "."])
        .status()?;
    assert!(committed.success());
    assert!(Command::new("git")
        .current_dir(&repo)
        .args(["-c", "user.name=Test", "-c", "user.email=test@localhost", "commit", "--quiet", "-m", "Agent work"])
        .status()?
        .success());
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let ran = tick(&repo, &closed()?, &["--interval-secs", "1"])?;
    let done = state(&repo)?;
    assert_eq!(done["attempts"], 2, "{done}");
    assert_eq!(done["waiting"], Value::Null);
    assert_ne!(done["status"], "waiting", "a committed checkout is not held back: {done}");
    if cfg!(unix) {
        // The whole cycle runs. (On Windows the observation clone of a `\\?\` path is refused by git, which is
        // Insi's own limit and not what this test is about: it is past the uncommitted check either way.)
        assert!(ran.status.success(), "{}", String::from_utf8_lossy(&ran.stderr));
        assert_eq!(done["status"], "completed", "{done}");
        assert_eq!(done["failures"], 0);
    }
    Ok(())
}

/// Waits never reach the pause: after three of them the next run is not refused, and earlier failures stay counted.
#[test]
fn waiting_three_times_does_not_pause_and_keeps_earlier_failures() -> TestResult<()> {
    let repo = committed()?;
    std::fs::write(repo.join("agent.txt"), "still working")?;
    let engine = closed()?;
    // Three scheduled runs, each its own process, as the engine starts them.
    for _ in 0..3 {
        assert!(tick(&repo, &engine, &["--interval-secs", "1"])?.status.success());
        std::thread::sleep(std::time::Duration::from_millis(1100));
    }
    let after = state(&repo)?;
    assert_eq!(after["attempts"], 3, "{after}");
    assert_eq!(after["failures"], 0, "waiting is not a failure: {after}");
    assert_eq!(after["status"], "waiting");
    // A fourth run is not refused as paused.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let fourth = tick(&repo, &engine, &["--interval-secs", "1"])?;
    assert!(fourth.status.success(), "{}", String::from_utf8_lossy(&fourth.stderr));
    assert_eq!(state(&repo)?["attempts"], 4);

    // Failures from before stay counted through a wait: the wait neither adds one nor wipes them.
    let mut earlier = state(&repo)?;
    earlier["failures"] = json!(2);
    earlier["next_due"] = json!(0);
    std::fs::write(repo.join(".git/insi-heartbeat/state.json"), earlier.to_string())?;
    assert!(tick(&repo, &engine, &["--interval-secs", "1"])?.status.success());
    let kept = state(&repo)?;
    assert_eq!(kept["failures"], 2, "{kept}");
    assert_eq!(kept["status"], "waiting");
    Ok(())
}
