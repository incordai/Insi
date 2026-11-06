//! The CLI must dispatch diagnostics before Work can start trash cleanup.
use std::path::PathBuf;

#[test]
fn doctor_is_offline_redacted_and_preserves_pending_cleanup() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../.claude/temp")
        .join(format!("doctor-cli-{}", std::process::id()));
    std::fs::create_dir_all(root.join(".insi")).expect("config folder");
    std::fs::create_dir_all(root.join("work/trash/keep")).expect("trash");
    std::fs::write(root.join("work/trash/keep/sentinel"), "preserve").expect("sentinel");
    std::fs::write(
        root.join(".insi/config.toml"),
        "[worker]\ncommand = ['secret-worker', 'secret-token', '{prompt}']\n",
    )
    .expect("config");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_insi"))
        .args(["doctor", "--repo"])
        .arg(&root)
        .args(["--work", "work", "--markdown"])
        .env("INSI_DOCTOR_SECRET", "private-environment")
        .output()
        .expect("doctor");
    assert!(out.status.success());
    let report = String::from_utf8(out.stdout).expect("utf8");
    assert!(report.contains("### Insi diagnostics"));
    assert!(report.contains("Build commit:"));
    for secret in ["secret-worker", "secret-token", "private-environment"] {
        assert!(!report.contains(secret));
    }
    assert!(out.stderr.is_empty());
    assert!(root.join("work/trash/keep/sentinel").exists());
    assert!(!root.join("work/git.lock").exists());
    std::fs::remove_dir_all(root).expect("cleanup");
}
