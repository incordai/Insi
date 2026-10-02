//! `.insi/config.toml` — how one repository is checked and worked on.
//!
//! Every key is optional except where a command needs it; an unknown key is
//! an error, so a misspelt setting is never silently left at its default.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// The folder Insi keeps in a repository.
pub const DIR: &str = ".insi";
/// The config file inside it.
pub const FILE: &str = "config.toml";
/// What a worker command must contain: where the task's prompt goes.
pub const PROMPT: &str = "{prompt}";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// The topic lessons for this repo are filed under (`rust/incord/...`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,
    /// The public check: the repo's own test command, as argv.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub test_cmd: Vec<String>,
    /// The hidden check `insi mine` gives each task, as argv; `{tests}`
    /// stands for the task's hidden test files.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub check: Vec<String>,
    /// Run in a fresh worktree before anything else (installing
    /// dependencies), as argv.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub setup: Vec<String>,
    /// The wall-clock seconds a worker gets per task.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wall_secs: Option<u64>,
    /// Paths a worker must never write (glob patterns, repo-relative).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub protected: Vec<String>,
    /// Where the public task definitions live, repo-relative.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eval_tasks: Option<String>,
    /// Where the hidden checks live: a path outside every checkout, or a git
    /// URL. Never inside the repository a worker can read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub private_checks: Option<String>,
    /// The label a maintainer puts on an issue to let an agent work it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trusted_issue_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker: Option<Worker>,
}

/// The worker adapter: a command run inside the task's worktree, with the
/// task's prompt in place of `{prompt}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Worker {
    pub command: Vec<String>,
    /// Files the harness writes for itself in the worktree — its state, its
    /// generated docs, its backups — as git glob pathspecs. They are left out
    /// of the worker's recorded diff, which is what a lesson is written from:
    /// a lesson about the harness's bookkeeping teaches nothing about the
    /// code. The check still runs on the whole worktree.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub own_files: Vec<String>,
}

impl Worker {
    /// The default adapter: incode, printing its events as JSON lines so the
    /// run's token use can be read back.
    pub fn incode() -> Self {
        Worker {
            command: ["incode", "-t", PROMPT, "--output-format", "stream-json"]
                .map(String::from)
                .to_vec(),
            // incode's state, the spec set its spec step writes
            // (incode src/spec.rs SPEC_SET), and the backups its edit hook
            // keeps beside each edited file.
            own_files: [
                ".incode/**",
                ".incord/**",
                "**/.backups/**",
                "docs/ARCHITECTURE.md",
                "docs/CONVENTIONS.md",
                "docs/VERIFICATION.md",
                "docs/ASSUMPTIONS.md",
            ]
            .map(String::from)
            .to_vec(),
        }
    }

    /// A worker given as a bare command, with no files of its own declared.
    pub fn command(command: Vec<String>) -> Self {
        Worker {
            command,
            own_files: Vec::new(),
        }
    }

    /// The command for one prompt. Refused when the program is missing or no
    /// argument takes the prompt: a worker that never sees the task would
    /// still be scored, as a failure the task did not earn.
    pub fn render(&self, prompt: &str) -> Result<Vec<String>, String> {
        if self.command.first().is_none_or(|p| p.trim().is_empty()) {
            return Err("worker.command is empty; name the program to run".to_string());
        }
        if !self.command.iter().any(|a| a.contains(PROMPT)) {
            return Err(format!(
                "worker.command has no {PROMPT} argument, so the worker would never see the task"
            ));
        }
        Ok(self
            .command
            .iter()
            .map(|a| a.replace(PROMPT, prompt))
            .collect())
    }
}

impl Config {
    /// Read `<repo>/.insi/config.toml`; a repo without one has the defaults.
    pub fn load(repo: &Path) -> Result<Config, String> {
        let path = repo.join(DIR).join(FILE);
        match std::fs::read_to_string(&path) {
            Ok(text) => Self::parse(&text).map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(format!("could not read {}: {e}", path.display())),
        }
    }

    pub fn parse(text: &str) -> Result<Config, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }

    /// The worker this repo names, else incode.
    pub fn worker(&self) -> Worker {
        self.worker.clone().unwrap_or_else(Worker::incode)
    }

    /// The config as the file `insi init` writes.
    pub fn to_toml(&self) -> Result<String, String> {
        toml::to_string(self).map_err(|e| e.to_string())
    }

    /// Where tasks and hidden tests live by default, given the folder the
    /// config was read from: `private_checks` resolved against it, with
    /// `tasks/` and `hidden/` inside. `None` when it names none.
    pub fn private_dirs(&self, repo: &Path) -> Option<(std::path::PathBuf, std::path::PathBuf)> {
        let base = repo.join(self.private_checks.as_deref()?);
        Some((base.join("tasks"), base.join("hidden")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_config_reads_and_unknown_keys_are_refused() {
        let config = Config::parse(
            r#"
            topic = "incord/memory-service"
            test_cmd = ["cargo", "test"]
            protected = [".github/workflows/**", "evals/**"]
            eval_tasks = "evals/tasks"
            private_checks = "../insi-private"
            trusted_issue_label = "agent-ok"
            [worker]
            command = ["my-agent", "--task", "{prompt}"]
            own_files = [".my-agent/**"]
            "#,
        )
        .expect("valid");
        assert_eq!(config.test_cmd, ["cargo", "test"]);
        assert_eq!(config.worker().command[0], "my-agent");
        assert_eq!(config.worker().own_files, [".my-agent/**"]);

        let why = Config::parse("tset_cmd = [\"x\"]").expect_err("misspelt");
        assert!(why.contains("tset_cmd"), "{why}");
    }

    #[test]
    fn an_absent_config_is_the_defaults_with_incode_as_the_worker() {
        let dir = std::env::temp_dir().join(format!("insi-config-{}", std::process::id()));
        let config = Config::load(&dir).expect("absent is fine");
        assert_eq!(config, Config::default());
        assert_eq!(config.worker().command[0], "incode");
    }

    #[test]
    fn a_worker_must_be_handed_the_prompt() {
        let argv = Worker::incode().render("fix the bug").expect("renders");
        assert!(argv.contains(&"fix the bug".to_string()), "{argv:?}");

        let blind = Worker::command(vec!["agent".into(), "--go".into()]);
        assert!(blind
            .render("x")
            .expect_err("no prompt")
            .contains("{prompt}"));
        let empty = Worker::command(vec![]);
        assert!(empty.render("x").is_err());
    }
}
