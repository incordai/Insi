//! Tasks from history: every fix commit that changed source and a separate
//! test file.
//!
//! The commit before the fix is where the worker starts; the commit's
//! message is what it is asked; the test files the fix added or changed are
//! the hidden check. A test kept in the same file as the code it tests (Rust's
//! inline `#[cfg(test)]`) cannot be held back without handing over the fix
//! too, so only test files of their own count.

use std::path::Path;

use insi_core::task::{is_safe_relative, Budget, Task};

use crate::git::{git, git_bytes};

#[derive(Debug, Clone)]
pub struct Options {
    /// Path fragments that mark a test file: a suffix such as `.test.ts`, or
    /// a folder such as `/tests/` or `__tests__/`.
    pub test_markers: Vec<String>,
    /// The check, as argv, `{tests}` standing for the hidden test paths.
    pub check: Vec<String>,
    /// Run in a fresh worktree before anything else; empty for none.
    pub setup: Vec<String>,
    pub wall_secs: u64,
    /// At most this many tasks, newest first.
    pub limit: usize,
    /// Task ids are `<prefix>-<first 8 of the fix commit>`.
    pub id_prefix: String,
}

impl Options {
    /// Test-file markers for the common layouts: JS/TS (`*.test.*`,
    /// `*.spec.*`, `__tests__/`), Rust and others (`tests/`), Go (`_test.go`),
    /// Python (`test_*.py`).
    pub fn default_markers() -> Vec<String> {
        [
            ".test.ts",
            ".test.tsx",
            ".test.js",
            ".test.jsx",
            ".spec.ts",
            ".spec.tsx",
            ".spec.js",
            "__tests__/",
            "/tests/",
            "_test.go",
            "_test.rs",
        ]
        .map(String::from)
        .to_vec()
    }
}

/// A task and the hidden test files that judge it.
#[derive(Debug, Clone)]
pub struct Mined {
    pub task: Task,
    pub hidden: Vec<(String, Vec<u8>)>,
}

/// Whether `path` is a test file of its own.
pub fn is_test_file(path: &str, markers: &[String]) -> bool {
    let path = format!("/{}", path.replace('\\', "/"));
    let name = path.rsplit('/').next().unwrap_or_default();
    markers.iter().any(|m| path.contains(m.as_str()))
        || (name.starts_with("test_") && name.ends_with(".py"))
}

/// Whether a commit subject reads as a fix.
pub fn looks_like_fix(subject: &str) -> bool {
    let s = subject.to_ascii_lowercase();
    s.starts_with("fix")
        || [
            "fix(", "fix:", " fix ", "fixes ", "bug", "crash", "regress", "broke", "wrong",
        ]
        .iter()
        .any(|w| s.contains(w))
}

/// Files a docs-only or config-only change touches; a fix must change more.
fn is_source(path: &str, markers: &[String]) -> bool {
    !is_test_file(path, markers)
        && !path.ends_with(".md")
        && !path.ends_with(".txt")
        && !path.ends_with(".lock")
        && !path.ends_with("package-lock.json")
}

/// Mine `source`'s history (read only) for tasks.
pub fn mine(source: &Path, opts: &Options) -> Result<Vec<Mined>, String> {
    let log = git(
        source,
        &[
            "log",
            "--no-merges",
            "--format=%H%x1f%P%x1f%cI%x1f%s%x1f%b%x1e",
        ],
    )?;
    let mut out = Vec::new();
    for record in log.split('\x1e') {
        if out.len() >= opts.limit {
            break;
        }
        let fields: Vec<&str> = record.trim_start_matches('\n').split('\x1f').collect();
        let [sha, parents, date, subject, body] = fields[..] else {
            continue;
        };
        let parents: Vec<&str> = parents.split_whitespace().collect();
        if sha.is_empty() || parents.len() != 1 || !looks_like_fix(subject) {
            continue;
        }
        let changed = git(
            source,
            &[
                "diff-tree",
                "--no-commit-id",
                "-r",
                "--name-status",
                "-M",
                sha,
            ],
        )?;
        let mut tests = Vec::new();
        let mut sources = 0;
        for line in changed.lines() {
            let parts: Vec<&str> = line.split('\t').collect();
            let (status, path) = match parts[..] {
                [status, path] => (status, path),
                // A rename lists old and new; the new path is what exists.
                [status, _, new] => (status, new),
                _ => continue,
            };
            if status.starts_with('D') {
                continue;
            }
            if is_test_file(path, &opts.test_markers) {
                if is_safe_relative(path) {
                    tests.push(path.to_string());
                }
            } else if is_source(path, &opts.test_markers) {
                sources += 1;
            }
        }
        if tests.is_empty() || sources == 0 {
            continue;
        }
        let mut hidden = Vec::new();
        for path in &tests {
            hidden.push((
                path.clone(),
                git_bytes(source, &["show", &format!("{sha}:{path}")])?,
            ));
        }
        let prompt = match body.trim() {
            "" => subject.trim().to_string(),
            body => format!("{}\n\n{body}", subject.trim()),
        };
        out.push(Mined {
            task: Task {
                id: format!("{}-{}", opts.id_prefix, &sha[..8.min(sha.len())]),
                source: source.display().to_string(),
                base: parents[0].to_string(),
                reference: sha.to_string(),
                date: Some(date.trim().to_string()).filter(|d| !d.is_empty()),
                prompt,
                hidden_tests: tests,
                check: opts.check.clone(),
                setup: opts.setup.clone(),
                budget: Budget {
                    wall_secs: opts.wall_secs,
                    tokens: None,
                },
            },
            hidden,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn markers() -> Vec<String> {
        Options::default_markers()
    }

    #[test]
    fn test_files_of_their_own_are_recognised() {
        for path in [
            "src/app/App.test.tsx",
            "src/x.spec.ts",
            "pkg/__tests__/a.js",
            "crates/x/tests/it.rs",
            "server/handler_test.go",
            "lib/test_parse.py",
        ] {
            assert!(is_test_file(path, &markers()), "{path}");
        }
        for path in [
            "src/app/App.tsx",
            "src/latest.ts",
            "src/contest.rs",
            "README.md",
        ] {
            assert!(!is_test_file(path, &markers()), "{path}");
        }
    }

    #[test]
    fn fix_commits_are_told_from_the_rest() {
        for s in [
            "fix: don't crash on a malformed chunk",
            "fix(codex): persist images (#399)",
            "Fix project rail reopening lag",
            "projects: a wrong path no longer hides the name",
        ] {
            assert!(looks_like_fix(s), "{s}");
        }
        for s in ["feat: add a picker", "docs: README", "refactor the store"] {
            assert!(!looks_like_fix(s), "{s}");
        }
    }

    /// **A FIX WITH ITS OWN TEST BECOMES A TASK**: started from the parent,
    /// asked the commit message, judged by the test file — and a fix without
    /// a test file of its own does not.
    #[test]
    fn a_fix_commit_with_a_test_file_becomes_a_task() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let dir = std::env::temp_dir().join(format!("insi-mine-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let sh = |args: &[&str]| git(&dir, args).expect("git");
        sh(&["init", "-q", "-b", "main"]);
        sh(&["config", "user.email", "t@example.invalid"]);
        sh(&["config", "user.name", "t"]);
        std::fs::create_dir_all(dir.join("src")).expect("dir");
        std::fs::write(
            dir.join("src/sum.ts"),
            "export const sum = (a, b) => a - b;\n",
        )
        .expect("w");
        sh(&["add", "."]);
        sh(&["commit", "-q", "-m", "feat: sum"]);
        let base = sh(&["rev-parse", "HEAD"]);
        std::fs::write(
            dir.join("src/sum.ts"),
            "export const sum = (a, b) => a + b;\n",
        )
        .expect("w");
        std::fs::write(dir.join("src/sum.test.ts"), "expect(sum(1, 2)).toBe(3);\n").expect("w");
        sh(&["add", "."]);
        sh(&[
            "commit",
            "-q",
            "-m",
            "fix: sum adds",
            "-m",
            "It subtracted.",
        ]);
        let fix = sh(&["rev-parse", "HEAD"]);
        std::fs::write(dir.join("src/other.ts"), "x\n").expect("w");
        sh(&["add", "."]);
        sh(&["commit", "-q", "-m", "fix: other, with no test"]);

        let opts = Options {
            test_markers: markers(),
            check: vec![
                "npx".into(),
                "vitest".into(),
                "run".into(),
                "{tests}".into(),
            ],
            setup: vec![],
            wall_secs: 600,
            limit: 10,
            id_prefix: "demo".into(),
        };
        let mined = mine(&dir, &opts).expect("mined");
        assert_eq!(
            mined.len(),
            1,
            "{:?}",
            mined.iter().map(|m| &m.task.id).collect::<Vec<_>>()
        );
        let task = &mined[0].task;
        assert_eq!(task.base, base);
        assert_eq!(task.reference, fix);
        assert_eq!(task.prompt, "fix: sum adds\n\nIt subtracted.");
        assert!(
            task.date.as_deref().is_some_and(|d| d.starts_with("20")),
            "{:?}",
            task.date
        );
        assert_eq!(task.hidden_tests, ["src/sum.test.ts"]);
        assert_eq!(mined[0].hidden[0].1, b"expect(sum(1, 2)).toBe(3);\n");
        assert!(task.validate().is_ok());
    }
}
