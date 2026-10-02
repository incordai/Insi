//! One attempt at one task, in a throwaway worktree.
//!
//! The worktree comes from Insi's OWN clone of the task's source, under the
//! work folder, so the source repository — often someone's live checkout —
//! is only ever read. The hidden tests are copied in after the worker has
//! stopped, so nothing the worker did could have read or edited them.

use std::path::{Path, PathBuf};
use std::time::Duration;

use insi_core::{normalize, EvalResult, Task, Worker};

use crate::git::git;
use crate::process;
use crate::store::Store;

/// The setup command's deadline: installing a project's dependencies.
pub const SETUP_SECS: u64 = 30 * 60;
/// The hidden check's deadline.
pub const CHECK_SECS: u64 = 15 * 60;

/// What one attempt runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The measured attempt: the worker, then the check.
    Worker,
    /// Control: the check at the base commit, with no worker. It must FAIL,
    /// or the task asks for nothing.
    Base,
    /// Control: the check at the reference commit. It must PASS, or the task
    /// cannot be solved as set.
    Reference,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Worker => "worker",
            Kind::Base => "base",
            Kind::Reference => "reference",
        }
    }
}

/// Insi's work folder: its clones, worktrees and results.
///
/// Attempts may run on several threads at once. Their worktrees are their
/// own, but the clone they come from is shared, and git refuses two writers
/// on one repository (`index.lock`, `worktrees/` bookkeeping): every git
/// command on a clone takes [`Work::git_lock`] first.
pub struct Work {
    pub root: PathBuf,
    git_lock: std::sync::Mutex<()>,
}

impl Work {
    pub fn new(root: PathBuf) -> Work {
        Work {
            root,
            git_lock: std::sync::Mutex::new(()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ()> {
        // A thread that panicked while holding it left no git state the next
        // command cannot meet; the lock only orders commands.
        self.git_lock.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Insi's clone of `source`, made on first use and fetched after.
    pub fn clone_of(&self, source: &str) -> Result<PathBuf, String> {
        let _held = self.lock();
        let name: String = source
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        let dir = self.root.join("clones").join(name.trim_matches('_'));
        if dir.join(".git").exists() || dir.join("HEAD").exists() {
            git(&dir, &["fetch", "--quiet", "origin"])?;
        } else {
            std::fs::create_dir_all(self.root.join("clones")).map_err(|e| e.to_string())?;
            let parent = dir.parent().unwrap_or(&self.root);
            git(
                parent,
                &[
                    "clone",
                    "--quiet",
                    "--no-checkout",
                    source,
                    &dir.display().to_string(),
                ],
            )?;
        }
        Ok(dir)
    }

    fn worktree(&self, clone: &Path, commit: &str, name: &str) -> Result<PathBuf, String> {
        let _held = self.lock();
        let path = self.root.join("runs").join(name);
        if path.exists() {
            let _ = git(
                clone,
                &["worktree", "remove", "--force", &path.display().to_string()],
            );
        }
        let _ = git(clone, &["worktree", "prune"]);
        git(
            clone,
            &[
                "worktree",
                "add",
                "--detach",
                "--force",
                &path.display().to_string(),
                commit,
            ],
        )?;
        Ok(path)
    }

    fn drop_worktree(&self, clone: &Path, path: &Path) {
        let _held = self.lock();
        let _ = git(
            clone,
            &["worktree", "remove", "--force", &path.display().to_string()],
        );
        let _ = git(clone, &["worktree", "prune"]);
    }
}

/// The settings of one attempt.
pub struct Attempt<'a> {
    pub task: &'a Task,
    pub kind: Kind,
    pub seed: u32,
    pub worker: &'a Worker,
    /// Text put in front of the task's prompt (a briefing of lessons), or
    /// empty.
    pub preface: &'a str,
    /// The lessons in that preface, by id, recorded with the result.
    pub briefed: Vec<String>,
    pub tags: Vec<String>,
    /// Leave the worktree on disk afterwards, to look at what happened.
    pub keep: bool,
}

/// Run one attempt and record what came of it. Never panics on a broken
/// task or tool: an attempt that cannot be judged is a result with `error`.
pub fn attempt(work: &Work, store: &Store, a: &Attempt) -> EvalResult {
    let started = std::time::Instant::now();
    let mut result = EvalResult {
        task: a.task.id.clone(),
        seed: a.seed,
        kind: a.kind.name().to_string(),
        passed: false,
        timed_out: false,
        wall_secs: 0.0,
        tokens: None,
        signature: None,
        error: None,
        tags: a.tags.clone(),
        artifacts: None,
        briefed: a.briefed.clone(),
    };
    if let Err(why) = attempt_into(work, store, a, &mut result) {
        result.error = Some(why);
    }
    result.wall_secs = started.elapsed().as_secs_f64();
    result
}

fn attempt_into(
    work: &Work,
    store: &Store,
    a: &Attempt,
    result: &mut EvalResult,
) -> Result<(), String> {
    let task = a.task;
    let clone = work.clone_of(&task.source)?;
    let commit = match a.kind {
        Kind::Reference => &task.reference,
        Kind::Worker | Kind::Base => &task.base,
    };
    let name = attempt_name(a);
    let artifacts = work.root.join("attempts").join(&name);
    std::fs::create_dir_all(&artifacts).map_err(|e| format!("{}: {e}", artifacts.display()))?;
    result.artifacts = Some(artifacts.display().to_string());
    let tree = work.worktree(&clone, commit, &name)?;
    let outcome = run_in(&tree, &artifacts, store, a, result);
    if !a.keep {
        work.drop_worktree(&clone, &tree);
    }
    outcome
}

/// One name per task, kind, seed AND condition: two conditions of the same
/// task and seed may run at once, and must not share a worktree.
fn attempt_name(a: &Attempt) -> String {
    let mut name = format!("{}-{}-s{}", a.task.id, a.kind.name(), a.seed);
    if !a.tags.is_empty() {
        let slug: String = a
            .tags
            .join("_")
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        name.push('-');
        name.push_str(&slug);
    }
    name
}

/// What an attempt leaves for a lesson to be written from. Files in its
/// artifacts folder:
pub const WORKER_DIFF: &str = "worker.diff";
pub const WORKER_OUTPUT: &str = "worker.out";
pub const CHECK_OUTPUT: &str = "check.out";

/// The most of any one artifact that is kept: a worker can write a diff of
/// any size, and what a lesson is written from is read in far less.
pub const MAX_ARTIFACT: usize = 512 * 1024;

fn keep(dir: &Path, file: &str, text: &str) -> Result<(), String> {
    let path = dir.join(file);
    let text = if text.len() > MAX_ARTIFACT {
        let mut end = MAX_ARTIFACT;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        format!(
            "{}\n[insi: cut at {MAX_ARTIFACT} of {} bytes]\n",
            &text[..end],
            text.len()
        )
    } else {
        text.to_string()
    };
    std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))
}

fn run_in(
    tree: &Path,
    artifacts: &Path,
    store: &Store,
    a: &Attempt,
    result: &mut EvalResult,
) -> Result<(), String> {
    let task = a.task;
    if !task.setup.is_empty() {
        let setup = process::run(&task.setup, tree, Duration::from_secs(SETUP_SECS))?;
        if !setup.success() {
            return Err(format!(
                "setup failed ({}): {}",
                if setup.timed_out {
                    "timed out".to_string()
                } else {
                    format!("exit {:?}", setup.code)
                },
                last_lines(&setup.tail(), 6)
            ));
        }
    }
    if a.kind == Kind::Worker {
        let prompt = if a.preface.is_empty() {
            task.prompt.clone()
        } else {
            format!("{}\n\n{}", a.preface, task.prompt)
        };
        let argv = a.worker.render(&prompt)?;
        let ran = process::run(&argv, tree, Duration::from_secs(task.budget.wall_secs))?;
        result.timed_out = ran.timed_out;
        result.tokens = tokens_in(&ran.stdout);
        keep(artifacts, WORKER_OUTPUT, &ran.tail())?;
        // What the worker changed, new files included, before the hidden
        // tests are copied over it. The worktree's index is its own.
        git(tree, &["add", "--all"])?;
        keep(
            artifacts,
            WORKER_DIFF,
            &git(tree, &["diff", "--cached", "--no-color"])?,
        )?;
    }
    // The hidden tests, only now: whatever the worker wrote at these paths
    // is replaced by the real check.
    for path in &task.hidden_tests {
        let bytes = store.hidden(task, path)?;
        let target = tree.join(path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::write(&target, bytes).map_err(|e| format!("{}: {e}", target.display()))?;
    }
    let check = process::run(&task.check_argv(), tree, Duration::from_secs(CHECK_SECS))?;
    keep(artifacts, CHECK_OUTPUT, &check.tail())?;
    result.passed = check.success();
    if !result.passed {
        // The whole kept output: a runner prints the failure's details above
        // its summary, and `normalize` picks the lines that say what failed.
        result.signature = Some(normalize(&check.tail()));
    }
    Ok(())
}

/// The verified fix: what the reference commit changed, read from Insi's own
/// clone, without the hidden test files (they judge the task; the lesson is
/// about the code).
pub fn fix_diff(work: &Work, task: &Task) -> Result<String, String> {
    let clone = work.clone_of(&task.source)?;
    let range = format!("{}..{}", task.base, task.reference);
    let mut args = vec![
        "diff".to_string(),
        "--no-color".to_string(),
        range,
        "--".to_string(),
        ".".to_string(),
    ];
    args.extend(task.hidden_tests.iter().map(|t| format!(":(exclude){t}")));
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    git(&clone, &args)
}

/// The total tokens a worker reported: incode's `stream-json` ends with a
/// `result` object whose `usage` holds the run's totals.
pub fn tokens_in(stdout: &str) -> Option<u64> {
    stdout.lines().rev().find_map(|line| {
        let v: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
        if v["type"] != "result" {
            return None;
        }
        let usage = &v["usage"];
        let sum = [
            "input_tokens",
            "output_tokens",
            "cache_creation_tokens",
            "cache_read_tokens",
        ]
        .iter()
        .filter_map(|k| usage[*k].as_u64())
        .sum::<u64>();
        Some(sum)
    })
}

fn last_lines(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_read_from_incodes_final_result() {
        let out = "{\"type\":\"init\"}\n{\"type\":\"usage\",\"input_tokens\":5}\n\
                   {\"type\":\"result\",\"usage\":{\"input_tokens\":100,\"output_tokens\":20,\
                   \"cache_creation_tokens\":3,\"cache_read_tokens\":400}}\n";
        assert_eq!(tokens_in(out), Some(523));
        assert_eq!(tokens_in("plain text, no json"), None);
    }
}
