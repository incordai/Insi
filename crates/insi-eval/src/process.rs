//! Running a command with a deadline, its output drained while it runs.
//!
//! Output is read on its own threads WHILE the process runs: a child that
//! writes more than a pipe's buffer would otherwise block, and the wait for
//! it would only end at the deadline. What is kept is bounded (the last
//! [`KEEP`] bytes of each stream), since a test run can print megabytes and
//! only its end says why it failed.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The tail of each output stream that is kept.
pub const KEEP: usize = 64 * 1024;

#[derive(Debug, Clone)]
pub struct Outcome {
    /// The exit code; `None` when the process was killed or ended by signal.
    pub code: Option<i32>,
    pub timed_out: bool,
    pub stdout: String,
    pub stderr: String,
    pub wall: Duration,
}

impl Outcome {
    pub fn success(&self) -> bool {
        self.code == Some(0) && !self.timed_out
    }

    /// The end of both streams, for a failure's signature.
    pub fn tail(&self) -> String {
        format!("{}\n{}", self.stderr, self.stdout)
    }
}

/// Run `argv` in `dir`, stopping it — and everything it started — at
/// `timeout`.
pub fn run(argv: &[String], dir: &Path, timeout: Duration) -> Result<Outcome, String> {
    let (program, args) = argv
        .split_first()
        .ok_or_else(|| "an empty command".to_string())?;
    let program = resolve(program);
    let mut command = Command::new(&program);
    command
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        // Its own process group, so a timeout can stop the whole tree.
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let started = Instant::now();
    let mut child = command
        .spawn()
        .map_err(|e| format!("could not start {}: {e}", program.display()))?;
    let out = drain(child.stdout.take());
    let err = drain(child.stderr.take());
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(100))
            }
            Ok(None) => {
                timed_out = true;
                kill_tree(child.id());
                let _ = child.kill();
                break child.wait().ok();
            }
            Err(e) => return Err(format!("waiting for {}: {e}", program.display())),
        }
    };
    let wall = started.elapsed();
    Ok(Outcome {
        code: if timed_out {
            None
        } else {
            status.and_then(|s| s.code())
        },
        timed_out,
        stdout: out.join().unwrap_or_default(),
        stderr: err.join().unwrap_or_default(),
        wall,
    })
}

fn drain<R: Read + Send + 'static>(pipe: Option<R>) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let Some(mut pipe) = pipe else {
            return String::new();
        };
        let mut kept: Vec<u8> = Vec::new();
        let mut chunk = [0u8; 8192];
        while let Ok(n) = pipe.read(&mut chunk) {
            if n == 0 {
                break;
            }
            kept.extend_from_slice(&chunk[..n]);
            if kept.len() > 2 * KEEP {
                kept.drain(..kept.len() - KEEP);
            }
        }
        if kept.len() > KEEP {
            kept.drain(..kept.len() - KEEP);
        }
        String::from_utf8_lossy(&kept).into_owned()
    })
}

/// Stop a process and everything it started.
fn kill_tree(pid: u32) {
    #[cfg(windows)]
    let _ = Command::new("taskkill")
        .args(["/T", "/F", "/PID", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    #[cfg(unix)]
    let _ = Command::new("kill")
        .args(["-KILL", &format!("-{pid}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// The program to start. On Windows a bare name like `npx` is a `.cmd`
/// script, which `Command` does not look for on its own: PATH is searched
/// for each executable extension, as a shell would.
fn resolve(program: &str) -> PathBuf {
    #[cfg(windows)]
    {
        let path = Path::new(program);
        if path.extension().is_none() && path.components().count() == 1 {
            if let Some(dirs) = std::env::var_os("PATH") {
                for dir in std::env::split_paths(&dirs) {
                    for ext in ["exe", "cmd", "bat"] {
                        let candidate = dir.join(format!("{program}.{ext}"));
                        if candidate.is_file() {
                            return candidate;
                        }
                    }
                }
            }
        }
    }
    PathBuf::from(program)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn here() -> PathBuf {
        std::env::current_dir().expect("cwd")
    }

    #[test]
    fn a_command_runs_and_its_output_is_kept() {
        let out = run(
            &["git".into(), "--version".into()],
            &here(),
            Duration::from_secs(30),
        )
        .expect("git runs");
        assert!(out.success(), "{out:?}");
        assert!(out.stdout.contains("git version"), "{}", out.stdout);
    }

    #[test]
    fn a_failing_command_reports_its_code() {
        let out = run(
            &["git".into(), "no-such-subcommand-x".into()],
            &here(),
            Duration::from_secs(30),
        )
        .expect("git runs");
        assert!(!out.success());
        assert!(out.code.is_some_and(|c| c != 0));
    }

    /// **A DEADLINE STOPS THE RUN.** A command that would wait forever is
    /// killed at its timeout and reported as timed out, not as a failure of
    /// its own.
    #[test]
    fn a_command_past_its_deadline_is_stopped() {
        let argv: Vec<String> = if cfg!(windows) {
            [
                "powershell",
                "-NoProfile",
                "-Command",
                "Start-Sleep -Seconds 30",
            ]
            .map(String::from)
            .to_vec()
        } else {
            ["sleep", "30"].map(String::from).to_vec()
        };
        let out = run(&argv, &here(), Duration::from_secs(2)).expect("starts");
        assert!(out.timed_out);
        assert!(!out.success());
        assert!(out.wall < Duration::from_secs(20), "{:?}", out.wall);
    }

    #[test]
    fn a_missing_program_is_an_error_not_a_failure() {
        assert!(run(
            &["insi-no-such-program".into()],
            &here(),
            Duration::from_secs(5)
        )
        .is_err());
        assert!(run(&[], &here(), Duration::from_secs(5)).is_err());
    }
}
