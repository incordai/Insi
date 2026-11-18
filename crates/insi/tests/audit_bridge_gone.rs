// Copyright (c) 2026-2027 Incord Ai.
// https://incord.ai
//
// SPDX-License-Identifier: BUSL-1.1

//! `insi suggest --audit-finding` and `--retry-audit` are refused before anything runs: the Incord engine removed the
//! candidate routes they called (2026-11-17); the engine runs Insi through `insi heartbeat` instead.

use std::process::Command;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

#[test]
fn the_audit_bridge_options_are_refused_with_the_reason_and_nothing_runs() -> TestResult {
    let dir = std::env::temp_dir().join(format!("insi-bridge-gone-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    for args in [
        vec!["suggest", "--audit-finding", "finding-1"],
        vec!["suggest", "--retry-audit", "some-dir"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_insi"))
            .args(&args)
            .args(["--repo", dir.to_str().ok_or("non UTF8 temp dir")?])
            .current_dir(&dir)
            .output()?;
        assert!(!output.status.success(), "{args:?} must fail");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("no longer supported") && stderr.contains("insi heartbeat"),
            "{args:?}: {stderr}"
        );
    }
    assert!(!dir.join(".insi-work").exists(), "nothing ran");
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}
