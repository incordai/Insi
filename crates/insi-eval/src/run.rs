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

/// Held while git's bookkeeping on a clone runs. Without the file lock (it
/// could not be opened) git still refuses a second writer by itself, and the
/// command reports it; the thread lock is always held.
struct GitLock<'a> {
    _process: Result<insi_core::lock::FileLock, String>,
    _thread: std::sync::MutexGuard<'a, ()>,
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
    /// Background deletions of finished worktrees.
    reapers: std::sync::Mutex<Vec<std::thread::JoinHandle<()>>>,
    /// Setups that may run at once. Installing dependencies is bound by the
    /// disk, not the CPU: measured on Watchman, one `npm ci` alone took
    /// under two minutes and three at once took over seventeen each.
    setup_slots: Slots,
}

/// A counting semaphore.
struct Slots {
    free: std::sync::Mutex<usize>,
    freed: std::sync::Condvar,
}

impl Slots {
    fn new(n: usize) -> Slots {
        Slots {
            free: std::sync::Mutex::new(n.max(1)),
            freed: std::sync::Condvar::new(),
        }
    }

    fn take(&self) -> SlotGuard<'_> {
        let mut free = self.free.lock().unwrap_or_else(|e| e.into_inner());
        while *free == 0 {
            free = self.freed.wait(free).unwrap_or_else(|e| e.into_inner());
        }
        *free -= 1;
        SlotGuard(self)
    }
}

struct SlotGuard<'a>(&'a Slots);

impl Drop for SlotGuard<'_> {
    fn drop(&mut self) {
        *self.0.free.lock().unwrap_or_else(|e| e.into_inner()) += 1;
        self.0.freed.notify_one();
    }
}

impl Work {
    pub fn new(root: PathBuf) -> Work {
        Work::with_setup_jobs(root, 1)
    }

    /// A work folder where up to `setup_jobs` setups run at once.
    pub fn with_setup_jobs(root: PathBuf, setup_jobs: usize) -> Work {
        let work = Work {
            root,
            git_lock: std::sync::Mutex::new(()),
            reapers: std::sync::Mutex::new(Vec::new()),
            setup_slots: Slots::new(setup_jobs),
        };
        // What an earlier run left in the trash (it exited mid-delete).
        if let Ok(entries) = std::fs::read_dir(work.trash()) {
            for entry in entries.flatten() {
                work.reap(entry.path());
            }
        }
        work
    }

    fn trash(&self) -> PathBuf {
        self.root.join("trash")
    }

    /// Delete `dir` on a thread of its own. A worktree with installed
    /// dependencies takes minutes to delete on Windows, and that time is
    /// neither part of the attempt nor a reason to hold its slot.
    fn reap(&self, dir: PathBuf) {
        let handle = std::thread::spawn(move || {
            let _ = std::fs::remove_dir_all(&dir);
        });
        self.reapers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(handle);
    }

    /// Wait for every background deletion, so a finished command leaves no
    /// half-deleted tree behind.
    pub fn wait_for_cleanup(&self) {
        let handles: Vec<_> = self
            .reapers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain(..)
            .collect();
        for handle in handles {
            let _ = handle.join();
        }
    }

    /// This process's threads, then other `insi` processes sharing the work
    /// folder (a validation and an eval at once): both must queue.
    fn lock(&self) -> GitLock<'_> {
        // A thread that panicked while holding it left no git state the next
        // command cannot meet; the lock only orders commands.
        let thread = self.git_lock.lock().unwrap_or_else(|e| e.into_inner());
        GitLock {
            _process: insi_core::lock::lock_beside(&self.root.join("git")),
            _thread: thread,
        }
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

    /// A fresh worktree at `commit`. Only git's bookkeeping holds the lock:
    /// the slow parts — deleting a previous tree (its installed
    /// dependencies run to tens of thousands of files) and checking out the
    /// files — touch this worktree alone and run outside it, so attempts do
    /// not queue behind each other's disk work.
    fn worktree(&self, clone: &Path, commit: &str, name: &str) -> Result<PathBuf, String> {
        let path = self.root.join("runs").join(name);
        self.drop_worktree(clone, &path);
        {
            let _held = self.lock();
            git(
                clone,
                &[
                    "worktree",
                    "add",
                    "--detach",
                    "--force",
                    "--no-checkout",
                    &path.display().to_string(),
                    commit,
                ],
            )?;
        }
        // The worktree's index and files are its own.
        git(&path, &["reset", "--hard", "--quiet"])?;
        Ok(path)
    }

    /// Take a worktree away: moved into the trash at once (a rename on the
    /// same volume), unregistered from the clone, and deleted in the
    /// background. A tree that cannot be moved is deleted here and now.
    fn drop_worktree(&self, clone: &Path, path: &Path) {
        if !path.exists() {
            let _held = self.lock();
            let _ = git(clone, &["worktree", "prune"]);
            return;
        }
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let binned = self.trash().join(format!("{name}-{nanos}"));
        let moved =
            std::fs::create_dir_all(self.trash()).is_ok() && std::fs::rename(path, &binned).is_ok();
        if moved {
            {
                let _held = self.lock();
                let _ = git(clone, &["worktree", "prune"]);
            }
            self.reap(binned);
            return;
        }
        if std::fs::remove_dir_all(path).is_err() {
            // What the filesystem would not delete, git may (it knows the
            // tree); that one waits for the lock.
            let _held = self.lock();
            let _ = git(
                clone,
                &["worktree", "remove", "--force", &path.display().to_string()],
            );
        }
        let _held = self.lock();
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
        phases: Default::default(),
    };
    if let Err(why) = attempt_into(work, store, a, started, &mut result) {
        result.error = Some(why);
    }
    // An attempt that failed before its check never set its time.
    if result.wall_secs == 0.0 {
        result.wall_secs = started.elapsed().as_secs_f64();
    }
    result
}

/// Time one phase of an attempt into its result.
fn timed<T>(result: &mut EvalResult, phase: &str, f: impl FnOnce() -> T) -> T {
    let started = std::time::Instant::now();
    let out = f();
    result
        .phases
        .insert(phase.to_string(), started.elapsed().as_secs_f64());
    out
}

fn attempt_into(
    work: &Work,
    store: &Store,
    a: &Attempt,
    started: std::time::Instant,
    result: &mut EvalResult,
) -> Result<(), String> {
    let task = a.task;
    let commit = match a.kind {
        Kind::Reference => &task.reference,
        Kind::Worker | Kind::Base => &task.base,
    };
    let name = attempt_name(a);
    let artifacts = work.root.join("attempts").join(&name);
    std::fs::create_dir_all(&artifacts).map_err(|e| format!("{}: {e}", artifacts.display()))?;
    result.artifacts = Some(artifacts.display().to_string());
    let (clone, tree) = timed(result, "prepare", || {
        let clone = work.clone_of(&task.source)?;
        let tree = work.worktree(&clone, commit, &name)?;
        Ok::<_, String>((clone, tree))
    })?;
    let outcome = run_in(work, &tree, &artifacts, store, a, result);
    // The attempt is over; taking its worktree away is not part of it.
    result.wall_secs = started.elapsed().as_secs_f64();
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
    work: &Work,
    tree: &Path,
    artifacts: &Path,
    store: &Store,
    a: &Attempt,
    result: &mut EvalResult,
) -> Result<(), String> {
    let task = a.task;
    if !task.setup.is_empty() {
        // Waiting for a slot is its own phase, so it is never read as setup.
        let slot = timed(result, "queue", || work.setup_slots.take());
        let setup = timed(result, "setup", || {
            process::run(&task.setup, tree, Duration::from_secs(SETUP_SECS))
        })?;
        drop(slot);
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
        let ran = timed(result, "worker", || {
            process::run(&argv, tree, Duration::from_secs(task.budget.wall_secs))
        })?;
        result.timed_out = ran.timed_out;
        result.tokens = tokens_in(&ran.stdout);
        keep(artifacts, WORKER_OUTPUT, &ran.tail())?;
        // What the worker changed, new files included, before the hidden
        // tests are copied over it. The worktree's index is its own.
        git(tree, &["add", "--all"])?;
        keep(artifacts, WORKER_DIFF, &worker_diff(tree, a.worker)?)?;
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
    let check = timed(result, "check", || {
        process::run(&task.check_argv(), tree, Duration::from_secs(CHECK_SECS))
    })?;
    keep(artifacts, CHECK_OUTPUT, &check.tail())?;
    result.passed = check.success();
    if !result.passed {
        // The whole kept output: a runner prints the failure's details above
        // its summary, and `normalize` picks the lines that say what failed.
        result.signature = Some(normalize(&check.tail()));
    }
    Ok(())
}

/// What the worker changed, without the files its harness writes for itself
/// ([`Worker::own_files`]).
fn worker_diff(tree: &Path, worker: &Worker) -> Result<String, String> {
    let mut args = vec![
        "diff".to_string(),
        "--cached".to_string(),
        "--no-color".to_string(),
        "--".to_string(),
        ".".to_string(),
    ];
    args.extend(
        worker
            .own_files
            .iter()
            .map(|p| format!(":(glob,exclude){p}")),
    );
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    git(tree, &args)
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

    /// **NO MORE SETUPS AT ONCE THAN ALLOWED.** Six threads through two
    /// slots never have three inside.
    #[test]
    fn slots_bound_how_many_run_at_once() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let slots = Slots::new(2);
        let (inside, most) = (AtomicUsize::new(0), AtomicUsize::new(0));
        std::thread::scope(|s| {
            for _ in 0..6 {
                s.spawn(|| {
                    let _slot = slots.take();
                    let now = inside.fetch_add(1, Ordering::SeqCst) + 1;
                    most.fetch_max(now, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(20));
                    inside.fetch_sub(1, Ordering::SeqCst);
                });
            }
        });
        assert_eq!(most.load(Ordering::SeqCst), 2);
        // Zero is read as one: a gate that never opens would hang the run.
        drop(Slots::new(0).take());
    }

    fn temp_dir(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let dir = std::env::temp_dir().join(format!("insi-{label}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    /// **THE CONTROLS RUN FOR REAL.** In a two-commit repository whose fix
    /// changes `a - b` to `a + b`, the check fails at the base and passes at
    /// the reference; the hidden test file arrives in the worktree; each
    /// attempt keeps its check output; no worktree is left behind; and the
    /// fix's diff leaves out the hidden test.
    #[test]
    fn base_fails_reference_passes_and_nothing_is_left_behind() {
        let source = temp_dir("src");
        let sh = |args: &[&str]| git(&source, args).expect("git");
        sh(&["init", "-q", "-b", "main"]);
        sh(&["config", "user.email", "t@example.invalid"]);
        sh(&["config", "user.name", "t"]);
        std::fs::write(
            source.join("sum.ts"),
            "export const sum = (a, b) => a - b;\n",
        )
        .expect("w");
        sh(&["add", "."]);
        sh(&["commit", "-q", "-m", "feat: sum"]);
        let base = sh(&["rev-parse", "HEAD"]);
        std::fs::write(
            source.join("sum.ts"),
            "export const sum = (a, b) => a + b;\n",
        )
        .expect("w");
        std::fs::write(source.join("sum.test.ts"), "expect(sum(1, 2)).toBe(3);\n").expect("w");
        sh(&["add", "."]);
        sh(&["commit", "-q", "-m", "fix: sum adds"]);
        let reference = sh(&["rev-parse", "HEAD"]);

        let private = temp_dir("private");
        let store = Store {
            tasks: temp_dir("tasks"),
            private: private.clone(),
        };
        let task = Task {
            id: "demo-1".into(),
            source: source.display().to_string(),
            base,
            reference,
            date: None,
            prompt: "fix: sum adds".into(),
            hidden_tests: vec!["sum.test.ts".into()],
            // Passes only where the fix is in, and only when the hidden test
            // was copied in (`--no-index` reads files git does not track).
            check: vec![
                "git".into(),
                "grep".into(),
                "--no-index".into(),
                "-q".into(),
                "-e".into(),
                "a + b".into(),
                "--".into(),
                "sum.ts".into(),
                "sum.test.ts".into(),
            ],
            setup: vec![],
            budget: insi_core::Budget {
                wall_secs: 60,
                tokens: None,
            },
        };
        store
            .save(&crate::mine::Mined {
                task: task.clone(),
                hidden: vec![(
                    "sum.test.ts".into(),
                    b"expect(sum(1, 2)).toBe(3);\n".to_vec(),
                )],
            })
            .expect("saved");
        let work = Work::new(temp_dir("work"));
        let worker = Worker::incode();
        let run_kind = |kind| {
            attempt(
                &work,
                &store,
                &Attempt {
                    task: &task,
                    kind,
                    seed: 0,
                    worker: &worker,
                    preface: "",
                    briefed: vec![],
                    tags: vec![],
                    keep: false,
                },
            )
        };
        let at_base = run_kind(Kind::Base);
        assert_eq!(at_base.error, None);
        assert!(!at_base.passed, "{at_base:?}");
        let at_reference = run_kind(Kind::Reference);
        assert_eq!(at_reference.error, None);
        assert!(at_reference.passed, "{at_reference:?}");

        for r in [&at_base, &at_reference] {
            let dir = PathBuf::from(r.artifacts.as_deref().expect("artifacts"));
            assert!(dir.join(CHECK_OUTPUT).is_file(), "{}", dir.display());
            // Each phase that ran says how long it took; there was no setup.
            assert!(
                r.phases.contains_key("prepare") && r.phases.contains_key("check"),
                "{:?}",
                r.phases
            );
            assert!(!r.phases.contains_key("setup"));
        }
        let entries = |dir: &str| -> Vec<std::fs::DirEntry> {
            std::fs::read_dir(work.root.join(dir))
                .map(|d| d.flatten().collect())
                .unwrap_or_default()
        };
        assert!(entries("runs").is_empty(), "worktrees left behind");
        work.wait_for_cleanup();
        assert!(
            entries("trash").is_empty(),
            "the background delete did not finish"
        );
        let clone = work.clone_of(&task.source).expect("clone");
        let listed = git(&clone, &["worktree", "list"]).expect("list");
        assert_eq!(listed.lines().count(), 1, "{listed}");

        let fix = fix_diff(&work, &task).expect("diff");
        assert!(
            fix.contains("+export const sum = (a, b) => a + b;"),
            "{fix}"
        );
        assert!(
            !fix.contains("sum.test.ts"),
            "the hidden test leaked into the fix: {fix}"
        );
    }
}
