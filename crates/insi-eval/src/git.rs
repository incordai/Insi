//! The git command line, as Insi uses it: text out, or why not.

use std::path::Path;
use std::process::Command;

/// Run git in `dir` and return its trimmed stdout.
pub fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    git_bytes(dir, args).map(|out| String::from_utf8_lossy(&out).trim_end().to_string())
}

/// Run git in `dir` and return its stdout untouched (a file's contents).
pub fn git_bytes(dir: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let mut command = Command::new("git");
    crate::process::detach_from_caller_repository(&mut command);
    let output = command
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|e| format!("could not run git: {e}"))?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        // The exit status always: a git that dies before it can print (a
        // process that cannot start, a crash) says nothing on stderr.
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!(
            "git {} ({}): {}",
            args.first().copied().unwrap_or_default(),
            exit(&output.status),
            if stderr.trim().is_empty() {
                "no output"
            } else {
                stderr.trim()
            }
        ))
    }
}

/// How a process ended: its exit code, in hex too where it looks like a
/// Windows NTSTATUS (0xC0000142: the process could not initialise).
fn exit(status: &std::process::ExitStatus) -> String {
    match status.code() {
        Some(code) if !(0..=0xFFFF).contains(&code) => {
            format!("exit {code} = {:#010X}", code as u32)
        }
        Some(code) => format!("exit {code}"),
        None => "ended by a signal".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **A FAILED GIT SAYS HOW IT ENDED, EVEN WHEN IT PRINTS NOTHING.**
    #[test]
    fn a_failure_names_its_exit_status() {
        let here = std::env::current_dir().expect("cwd");
        let why = git(&here, &["no-such-subcommand-x"]).expect_err("fails");
        assert!(why.starts_with("git no-such-subcommand-x (exit 1"), "{why}");
    }
}
