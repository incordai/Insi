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
    run_with(argv, dir, timeout, &[])
}

/// [`run`], with `env` added to the environment the command inherits.
pub fn run_with(
    argv: &[String],
    dir: &Path,
    timeout: Duration,
    env: &[(&str, &std::ffi::OsStr)],
) -> Result<Outcome, String> {
    let (program, args) = argv
        .split_first()
        .ok_or_else(|| "an empty command".to_string())?;
    let program = resolve(program);
    let mut command = Command::new(&program);
    // Before `env`, so a variable the caller sets on purpose still wins.
    detach_from_parent_session(&mut command, std::env::vars_os().map(|(name, _)| name));
    command
        .args(args)
        .envs(env.iter().copied())
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

/// Names that tie a process to the agent session that launched Insi. Removed
/// whether or not this process has them: removing an absent variable costs
/// nothing.
const SESSION_NAMES: [&str; 6] = [
    "CLAUDECODE",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_CODE_CHILD_SESSION",
    "CLAUDE_CODE_MESSAGING_SOCKET",
    "CLAUDE_CODE_MESSAGING_TOKEN",
    "CLAUDE_SESSION_INGRESS_TOKEN_FILE",
];

/// Families of session variables that are not a fixed list (the session id,
/// its attendance flag, a remote session's handles), so they are matched
/// against what is actually inherited.
const SESSION_PREFIXES: [&str; 2] = ["CLAUDE_CODE_SESSION", "CLAUDE_CODE_REMOTE"];

/// A remote session's ingress endpoint. Generic enough that it is removed only
/// when it is really inherited.
const INGRESS_URL: &str = "SESSION_INGRESS_URL";

/// Start `command` outside the agent session Insi runs in, given the names of
/// the variables it would `inherited`.
///
/// # Why every command
///
/// Insi is often started from inside an agent CLI's session, and an agent CLI
/// a worker starts reads that session's variables as its own: in a run of
/// three concurrent attempts, every `worker.out` carried the operator's
/// session id as its `agent_session_id` — attempts not isolated from each
/// other, or from the person running the benchmark. A check or `gh` has no
/// use for them either, so no command keeps them.
///
/// Only the session is removed. Credentials and configuration
/// (`ANTHROPIC_API_KEY`, `CLAUDE_CODE_OAUTH_TOKEN`, `CLAUDE_CONFIG_DIR`, …)
/// say who may run a session, not which one this is, and a worker cannot
/// authenticate without them.
fn detach_from_parent_session(
    command: &mut Command,
    inherited: impl IntoIterator<Item = std::ffi::OsString>,
) {
    for name in SESSION_NAMES {
        command.env_remove(name);
    }
    for name in inherited {
        // Upper-cased: Windows reads environment names case-insensitively.
        let upper = name.to_string_lossy().to_ascii_uppercase();
        if upper == INGRESS_URL || SESSION_PREFIXES.iter().any(|p| upper.starts_with(p)) {
            command.env_remove(name);
        }
    }
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
    fn the_environment_given_reaches_the_command() {
        let out = run_with(
            &[
                "git".into(),
                "config".into(),
                "--get".into(),
                "insi.probe".into(),
            ],
            &here(),
            Duration::from_secs(30),
            &[
                ("GIT_CONFIG_COUNT", "1".as_ref()),
                ("GIT_CONFIG_KEY_0", "insi.probe".as_ref()),
                ("GIT_CONFIG_VALUE_0", "reached".as_ref()),
            ],
        )
        .expect("git runs");
        assert_eq!(out.stdout.trim(), "reached", "{out:?}");
    }

    /// What the built command does with `name`: `Some(None)` is removed,
    /// `Some(Some(_))` is set, `None` is left to inheritance.
    fn fate(command: &Command, name: &str) -> Option<Option<std::ffi::OsString>> {
        command
            .get_envs()
            .find(|(k, _)| *k == std::ffi::OsStr::new(name))
            .map(|(_, v)| v.map(std::ffi::OsStr::to_os_string))
    }

    /// **A COMMAND NEVER INHERITS THE AGENT SESSION INSI RUNS IN** — and keeps
    /// the credentials it needs to start one of its own.
    #[test]
    fn a_command_is_detached_from_the_parent_session_and_keeps_its_credentials() {
        let parent = [
            "CLAUDECODE",
            "CLAUDE_CODE_SESSION_ID",
            "Claude_Code_Session_Attended",
            "CLAUDE_CODE_REMOTE_SESSION_ID",
            "SESSION_INGRESS_URL",
            "ANTHROPIC_API_KEY",
            "CLAUDE_CODE_OAUTH_TOKEN",
            "CLAUDE_CONFIG_DIR",
            "PATH",
        ];
        let mut command = Command::new("worker");
        detach_from_parent_session(&mut command, parent.map(std::ffi::OsString::from));
        for name in [
            "CLAUDECODE",
            "CLAUDE_CODE_SESSION_ID",
            "Claude_Code_Session_Attended",
            "CLAUDE_CODE_REMOTE_SESSION_ID",
            "SESSION_INGRESS_URL",
        ] {
            assert_eq!(
                fate(&command, name),
                Some(None),
                "{name} reached the worker, which then reports the parent's session as its own"
            );
        }
        for name in [
            "ANTHROPIC_API_KEY",
            "CLAUDE_CODE_OAUTH_TOKEN",
            "CLAUDE_CONFIG_DIR",
            "PATH",
        ] {
            assert_eq!(
                fate(&command, name),
                None,
                "{name} was removed, and the worker cannot authenticate or run without it"
            );
        }
    }

    /// Set only in the child process the end-to-end test starts.
    const SESSION_CHILD: &str = "INSI_PARENT_SESSION_CHILD";

    /// The child half: a process started from inside an agent session runs a
    /// command through [`run_with`] and prints the environment it got.
    #[test]
    fn parent_session_child() {
        if std::env::var_os(SESSION_CHILD).is_none() {
            return;
        }
        let argv: Vec<String> = if cfg!(windows) {
            ["cmd", "/C", "set"].map(String::from).to_vec()
        } else {
            vec!["env".to_string()]
        };
        let out = run_with(&argv, &here(), Duration::from_secs(30), &[]).expect("runs");
        println!("CHILD-ENV-BEGIN\n{}\nCHILD-ENV-END", out.stdout);
    }

    /// **RUN FROM INSIDE AN AGENT SESSION, A COMMAND STILL STARTS OUTSIDE
    /// IT.** Through the real spawn, in a process whose own environment
    /// carries the session, as `insi eval` does when an operator starts it
    /// from an agent CLI.
    #[test]
    fn a_command_started_from_an_agent_session_does_not_inherit_it() {
        let out = std::process::Command::new(std::env::current_exe().expect("test binary"))
            .args([
                "--exact",
                "process::tests::parent_session_child",
                "--nocapture",
                "--test-threads=1",
                "-q",
            ])
            .env(SESSION_CHILD, "1")
            .env("CLAUDE_CODE_SESSION_ID", "parent")
            .env("CLAUDECODE", "1")
            .env("INSI_PROBE_CREDENTIAL", "kept")
            .stdin(Stdio::null())
            .output()
            .expect("run the child");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let env = stdout
            .split("CHILD-ENV-BEGIN")
            .nth(1)
            .and_then(|rest| rest.split("CHILD-ENV-END").next())
            .unwrap_or_else(|| panic!("the child did not run: {stdout}"));
        let names: Vec<String> = env
            .lines()
            .filter_map(|line| line.split_once('='))
            .map(|(name, _)| name.to_ascii_uppercase())
            .collect();
        assert!(
            names.iter().any(|n| n == "INSI_PROBE_CREDENTIAL"),
            "control: the rest of the environment arrives: {env}"
        );
        for name in ["CLAUDE_CODE_SESSION_ID", "CLAUDECODE"] {
            assert!(
                !names.iter().any(|n| n == name),
                "{name} reached the command, so an attempt joins the operator's session: {env}"
            );
        }
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
