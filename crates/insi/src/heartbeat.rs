// Copyright (c) 2026-2027 Incord Ai.
// https://incord.ai
//
// SPDX-License-Identifier: BUSL-1.1

//! Persistent scheduling around the existing gated suggestion cycle.
use crate::Flags;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Durable scheduling and failure counts survive restarts.
#[derive(Default, Serialize, Deserialize)]
struct State {
    next_due: u64,
    attempts: u64,
    failures: u64,
    status: String,
    error: Option<String>,
    /// Why the last run did not happen when it is waiting on something that is not a failure (uncommitted work):
    /// the words the owner reads. Cleared by the next run.
    #[serde(default)]
    waiting: Option<String>,
}

/// Wall clock seconds allow another process to resume the schedule.
fn now() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|e| e.to_string())
}

/// Replace state atomically so interrupted writes cannot corrupt the previous record.
fn save(dir: &Path, state: &State) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(state).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("state.pending"), bytes).map_err(|e| e.to_string())?;
    std::fs::rename(dir.join("state.pending"), dir.join("state.json")).map_err(|e| e.to_string())
}

/// Run due suggestions under a repository-wide OS lock, using crate::run's existing gates.
pub fn run(flags: &Flags) -> Result<(), String> {
    // Reject unsupported options rather than silently scheduling a different operation.
    for key in flags.0.keys() {
        if ![
            "repo",
            "work",
            "interval-secs",
            "max-cycles",
            "once",
            "status",
            "reset-failures",
            "publish",
            "local",
            "worker",
            "proposer",
            "samples",
            "margin",
            "to-memory",
            "benchmarks",
            "suggestion",
            // Direct audit execution requires suggest's engine workflow; never silently ignore that boundary.
            // Autonomous policy, offline observation and backlog inspection.
            "plan",
            "offline",
            "queue",
            // Research and new-work proposals on top of testing (autonomous.rs `needs_model`); off by default, the
            // engine passes it from its `insi_research` setting (Inmem `insi_runner::heartbeat_args`).
            "research",
        ]
        .contains(&key.as_str())
        {
            return Err(format!("unsupported heartbeat option --{key}"));
        }
    }
    let interval = flags.number("interval-secs", 3600)?;
    let limit = flags.number("max-cycles", 0)?;
    if !(1..=604800).contains(&interval) {
        return Err("--interval-secs must be 1..604800".into());
    }
    let repo =
        std::fs::canonicalize(flags.get("repo").unwrap_or(".")).map_err(|e| e.to_string())?;
    let common = insi_eval::git::git(&repo, &["rev-parse", "--git-common-dir"])?;
    let dir = repo.join(common.trim()).join("insi-heartbeat");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    // Queue inspection does not need a worker or project configuration.
    if flags.has("queue") {
        println!(
            "{}",
            serde_json::to_string_pretty(&crate::autonomous::cycle(flags, &dir)?)
                .map_err(|e| e.to_string())?
        );
        return Ok(());
    }
    if flags.has("status") {
        match std::fs::read_to_string(dir.join("state.json")) {
            Ok(text) => println!("{text}"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                println!("{{\"status\":\"never-run\"}}")
            }
            Err(e) => return Err(e.to_string()),
        }
        return Ok(());
    }
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join("lock"))
        .map_err(|e| e.to_string())?;
    lock.try_lock()
        .map_err(|e| format!("heartbeat already running or lock unavailable: {e}"))?;
    let mut state: State = match std::fs::read(dir.join("state.json")) {
        Ok(bytes) => {
            serde_json::from_slice(&bytes).map_err(|e| format!("invalid heartbeat state: {e}"))?
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => State::default(),
        Err(e) => return Err(e.to_string()),
    };
    if flags.has("reset-failures") {
        state.failures = 0;
        save(&dir, &state)?;
    }
    let mut child = flags.0.clone();
    for key in [
        "interval-secs",
        "max-cycles",
        "once",
        "status",
        "reset-failures",
        "publish",
    ] {
        child.remove(key);
    }
    child.insert("repo".into(), Some(repo.to_string_lossy().into_owned()));
    if !flags.has("publish") {
        child.insert("local".into(), None);
    }
    let work = flags
        .get("work")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| repo.join(".insi-work"));
    child.insert(
        "work".into(),
        Some(
            std::path::absolute(work)
                .map_err(|e| e.to_string())?
                .to_string_lossy()
                .into_owned(),
        ),
    );
    // Each tick uses persistent observation and task selection rather than an untracked suggestion.
    let cycle_flags = Flags(child);
    let mut cycles = 0;
    loop {
        if state.failures >= 3 {
            return Err("heartbeat paused after 3 consecutive errors; inspect state, then use --reset-failures".into());
        }
        let current = now()?;
        if current < state.next_due {
            if flags.has("once") {
                println!("heartbeat not due until {}", state.next_due);
                return Ok(());
            }
            std::thread::sleep(Duration::from_secs((state.next_due - current).min(1)));
            continue;
        }
        // Record before invoking workers; crashes also count toward the failure cap.
        let failures_before = state.failures;
        state.attempts += 1;
        state.failures += 1;
        state.next_due = current.saturating_add(interval);
        state.status = "running".into();
        state.error = None;
        state.waiting = None;
        save(&dir, &state)?;
        // Return delivery evidence while retaining the scheduler's existing error accounting.
        let outcome = crate::autonomous::cycle(&cycle_flags, &dir).map(|report| {
            println!("{report}");
            report
        });
        state.next_due = now()?.saturating_add(interval);
        match &outcome {
            // WAITING IS NOT A FAILURE (the owner, 2026-11-20): the run was held back (uncommitted work), so the
            // failure count is what it was before this attempt, and no number of waits reaches the pause.
            Ok(report) if report["status"] == "waiting" => {
                state.failures = failures_before;
                state.status = "waiting".into();
                state.waiting = report["reason"].as_str().map(str::to_string);
            }
            Ok(_) => {
                state.failures = 0;
                state.status = "completed".into();
            }
            Err(error) => {
                state.status = "failed".into();
                state.error = Some(error.clone());
            }
        }
        let outcome = outcome.map(|_| ());
        save(&dir, &state)?;
        println!(
            "heartbeat {}: {}; next due {}",
            state.attempts, state.status, state.next_due
        );
        cycles += 1;
        if flags.has("once") || (limit > 0 && cycles >= limit) {
            return outcome;
        }
    }
}
