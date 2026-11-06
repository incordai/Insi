//! Record build provenance; runtime diagnostics never need repository history.
use std::process::Command;

fn main() {
    println!(
        "cargo:rustc-env=INSI_BUILD_TARGET={}",
        std::env::var("TARGET").expect("Cargo TARGET")
    );
    println!("cargo:rerun-if-changed=build.rs");
    let repo = std::env::var("CARGO_MANIFEST_DIR").expect("Cargo manifest directory");
    let git = |args: &[&str]| {
        let mut command = Command::new("git");
        command.current_dir(&repo).args(args);
        for name in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_COMMON_DIR",
            "GIT_OBJECT_DIRECTORY",
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
            "GIT_INDEX_FILE",
        ] {
            command.env_remove(name);
        }
        command
            .output()
            .ok()
            .filter(|out| out.status.success())
            .and_then(|out| String::from_utf8(out.stdout).ok())
            .map(|s| s.trim().to_string())
    };
    let commit = git(&["rev-parse", "--verify", "HEAD"])
        .filter(|s| [40, 64].contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit()))
        .unwrap_or_else(|| "unknown (source archive)".into());
    println!("cargo:rustc-env=INSI_BUILD_COMMIT={commit}");
    // Track both symbolic refs and detached HEAD, including linked worktrees.
    for name in [
        Some("HEAD".to_string()),
        Some("packed-refs".to_string()),
        git(&["symbolic-ref", "-q", "HEAD"]),
    ]
    .into_iter()
    .flatten()
    {
        if let Some(path) = git(&["rev-parse", "--git-path", &name]) {
            let path = std::path::Path::new(&repo).join(path);
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
}
