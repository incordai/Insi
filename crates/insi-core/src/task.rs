//! A benchmark task and the record of one attempt at it.

use serde::{Deserialize, Serialize};

/// One task: a repository at a commit, a request, and a hidden check that
/// decides whether the request was met.
///
/// The public part — this struct — is what `evals/tasks/<id>.toml` holds and
/// what a worker may in principle see. The hidden test files themselves are
/// stored apart, under the private-checks directory, and are copied into the
/// worktree only after the worker has stopped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Task {
    /// Stable id, e.g. `watchman-1a2b3c4d`.
    pub id: String,
    /// The repository the task comes from: a path or a git URL.
    pub source: String,
    /// The commit the worker starts from.
    pub base: String,
    /// The commit that solved it in history — the reference answer, used to
    /// prove the task is solvable and never shown to a worker.
    pub reference: String,
    /// When the reference was committed (RFC 3339). Orders tasks in time, so
    /// that a lesson is only ever briefed to work that came after it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    /// What the worker is asked to do.
    pub prompt: String,
    /// Repo-relative paths of the hidden test files.
    pub hidden_tests: Vec<String>,
    /// The hidden check, as argv; `{tests}` expands to the hidden test paths.
    pub check: Vec<String>,
    /// Run once in a fresh worktree before anything else (e.g. installing
    /// dependencies), as argv; empty for none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub setup: Vec<String>,
    /// What setup produces that a later attempt may reuse instead of running
    /// setup again; `None` runs setup every time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup_cache: Option<SetupCache>,
    pub budget: Budget,
}

/// Setup output reused between attempts: `keep` folders (`node_modules`),
/// valid for as long as the `key` files (the lockfile) are unchanged.
///
/// Measured on Watchman: `npm ci` was 4 to 5.5 minutes of every 13 to 20
/// minute attempt, installing the same lockfile each time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetupCache {
    /// Repo-relative files whose contents decide whether the kept output is
    /// still right, e.g. `package-lock.json`.
    pub key: Vec<String>,
    /// Repo-relative folders setup produces, e.g. `node_modules`.
    pub keep: Vec<String>,
}

impl SetupCache {
    /// Refused when a path could reach outside the worktree: these folders
    /// are moved, and a path out of the tree would move something else.
    pub fn validate(&self) -> Result<(), String> {
        if self.key.is_empty() || self.keep.is_empty() {
            return Err("setup_cache needs at least one key file and one kept folder".into());
        }
        for path in self.key.iter().chain(&self.keep) {
            if !is_safe_relative(path) {
                return Err(format!(
                    "setup_cache path \"{path}\" is not a plain repo-relative path"
                ));
            }
        }
        Ok(())
    }
}

/// What one attempt may spend. The same for every attempt at a task, so a
/// gain comes from method, not from spending more.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Budget {
    /// Wall-clock seconds the worker may run before it is stopped.
    pub wall_secs: u64,
    /// Tokens the worker may spend, when the worker reports them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<u64>,
}

/// The placeholder in [`Task::check`] that stands for the hidden test paths.
pub const TESTS: &str = "{tests}";

impl Task {
    /// The check's argv with `{tests}` expanded: an argument that is exactly
    /// `{tests}` becomes one argument per test file.
    pub fn check_argv(&self) -> Vec<String> {
        let mut out = Vec::new();
        for arg in &self.check {
            if arg == TESTS {
                out.extend(self.hidden_tests.iter().cloned());
            } else {
                out.push(arg.replace(TESTS, &self.hidden_tests.join(" ")));
            }
        }
        out
    }

    /// Refused when a hidden test path could land outside the worktree.
    pub fn validate(&self) -> Result<(), String> {
        if self.id.trim().is_empty() || self.base.trim().is_empty() {
            return Err("a task needs an id and a base commit".to_string());
        }
        if self.hidden_tests.is_empty() {
            return Err(format!("task {} has no hidden test", self.id));
        }
        for path in &self.hidden_tests {
            if !is_safe_relative(path) {
                return Err(format!(
                    "task {}: hidden test path \"{path}\" is not a plain repo-relative path",
                    self.id
                ));
            }
        }
        if self.check.is_empty() {
            return Err(format!("task {} has no check command", self.id));
        }
        if let Some(cache) = &self.setup_cache {
            cache
                .validate()
                .map_err(|e| format!("task {}: {e}", self.id))?;
        }
        Ok(())
    }
}

/// A path that stays inside the folder it is joined to: relative, no `..`,
/// no drive or root, no empty component.
pub fn is_safe_relative(path: &str) -> bool {
    let path = path.replace('\\', "/");
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains(':')
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

/// The record of one attempt, one JSON line in a results file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvalResult {
    pub task: String,
    pub seed: u32,
    /// What ran: `worker` (the measured attempt), `base` or `reference`
    /// (the controls that prove a task is a fair test).
    pub kind: String,
    /// The hidden check exited zero.
    pub passed: bool,
    /// The worker was stopped at its wall-clock budget.
    #[serde(default)]
    pub timed_out: bool,
    pub wall_secs: f64,
    /// Tokens the worker reported, when it reports them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<u64>,
    /// The model the worker said it ran on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The tokens by kind, which is what a price applies to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    /// What the attempt cost in US dollars, from the config's price for
    /// [`Self::model`]; `None` when there is no usage or no price.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    /// The normalized failure signature, when the check failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    /// Why the attempt could not be judged at all (setup failed, git failed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Labels for the run's conditions, e.g. `lessons=off`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// The folder holding what the attempt left behind: the worker's diff,
    /// its output, the check's output. What a lesson is written from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifacts: Option<String>,
    /// The lessons the worker was briefed with, by id.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub briefed: Vec<String>,
    /// Seconds per phase — `prepare` (clone and worktree), `setup`, `worker`,
    /// `check` — so a slow attempt says where its time went. Cleanup is in
    /// none of them, and not in `wall_secs`.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub phases: std::collections::BTreeMap<String, f64>,
    /// What the harness noticed about the attempt that did not change its
    /// verdict: why setup's output was not reused or not kept, for one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

/// Tokens a worker reported, by kind.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
    pub cache_creation: u64,
    pub cache_read: u64,
}

impl Usage {
    pub fn total(&self) -> u64 {
        self.input + self.output + self.cache_creation + self.cache_read
    }
}

impl EvalResult {
    /// The condition an attempt ran under: its tags, joined.
    pub fn condition(&self) -> String {
        if self.tags.is_empty() {
            "untagged".to_string()
        } else {
            self.tags.join(",")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task() -> Task {
        Task {
            id: "t1".into(),
            source: "repo".into(),
            base: "abc".into(),
            reference: "def".into(),
            date: Some("2026-10-02T09:15:00+00:00".into()),
            prompt: "fix it".into(),
            hidden_tests: vec!["src/a.test.ts".into(), "src/b.test.ts".into()],
            check: vec!["npx".into(), "vitest".into(), "run".into(), TESTS.into()],
            setup: vec![],
            setup_cache: None,
            budget: Budget {
                wall_secs: 600,
                tokens: None,
            },
        }
    }

    #[test]
    fn the_check_names_each_hidden_test() {
        assert_eq!(
            task().check_argv(),
            ["npx", "vitest", "run", "src/a.test.ts", "src/b.test.ts"]
        );
    }

    #[test]
    fn a_hidden_test_cannot_escape_the_worktree() {
        assert!(task().validate().is_ok());
        for bad in [
            "../x.test.ts",
            "/etc/passwd",
            "C:/x.ts",
            "a//b.ts",
            "a/./b.ts",
            "",
        ] {
            let mut t = task();
            t.hidden_tests = vec![bad.into()];
            assert!(t.validate().is_err(), "{bad}");
        }
    }

    #[test]
    fn a_task_round_trips_through_toml() {
        let text = toml::to_string(&task()).expect("serialize");
        let back: Task = toml::from_str(&text).expect("parse");
        assert_eq!(back, task());
    }
}
