//! The git command line, as Insi uses it: text out, or why not.

use std::path::Path;
use std::process::Command;

/// Run git in `dir` and return its trimmed stdout.
pub fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    git_bytes(dir, args).map(|out| String::from_utf8_lossy(&out).trim_end().to_string())
}

/// Run git in `dir` and return its stdout untouched (a file's contents).
pub fn git_bytes(dir: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|e| format!("could not run git: {e}"))?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(format!(
            "git {}: {}",
            args.first().copied().unwrap_or_default(),
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}
