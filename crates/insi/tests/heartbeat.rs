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
