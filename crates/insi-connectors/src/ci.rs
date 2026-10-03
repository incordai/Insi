//! Failed CI runs: GitHub Actions, through the `gh` CLI, which holds the
//! login.
//!
//! One item per recent failed run: the workflow, the branch, and the end of
//! its failed steps' log. Internal: the repository's own machinery wrote it,
//! though a log can quote anything the code under test printed. Its topic is
//! that of the repository files the log names.

use std::path::{Path, PathBuf};
use std::time::Duration;

use insi_core::topic::Topics;
use insi_core::Trust;
use serde::Deserialize;

use crate::{Item, Source};

/// How much of a failed run's log is kept: its end, where the failure is.
pub const LOG_TAIL: usize = 4_000;
/// How long one `gh` call may take.
const GH_SECS: u64 = 60;

/// A failed run, as `gh run list --json` gives it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    pub database_id: u64,
    #[serde(default)]
    pub display_title: String,
    #[serde(default)]
    pub head_branch: String,
    #[serde(default)]
    pub workflow_name: String,
    #[serde(default)]
    pub url: String,
}

/// Where runs come from: GitHub through `gh`, or a stand-in.
pub trait Runs {
    fn failed(&self, limit: usize) -> Result<Vec<Run>, String>;
    fn failed_log(&self, id: u64) -> Result<String, String>;
}

/// GitHub Actions through `gh`, run in the repository's folder.
pub struct GhRuns {
    pub repo: PathBuf,
}

impl GhRuns {
    fn gh(&self, args: &[&str]) -> Result<String, String> {
        let argv: Vec<String> = std::iter::once("gh")
            .chain(args.iter().copied())
            .map(String::from)
            .collect();
        let out = insi_eval::process::run(&argv, &self.repo, Duration::from_secs(GH_SECS))?;
        if out.success() {
            Ok(out.stdout)
        } else {
            Err(format!(
                "gh {}: {}",
                args.first().copied().unwrap_or_default(),
                out.stderr.lines().last().unwrap_or("failed").trim()
            ))
        }
    }
}

impl Runs for GhRuns {
    fn failed(&self, limit: usize) -> Result<Vec<Run>, String> {
        let raw = self.gh(&[
            "run",
            "list",
            "--status",
            "failure",
            "--limit",
            &limit.to_string(),
            "--json",
            "databaseId,displayTitle,headBranch,workflowName,url",
        ])?;
        serde_json::from_str(&raw).map_err(|e| format!("gh run list: {e}"))
    }

    fn failed_log(&self, id: u64) -> Result<String, String> {
        self.gh(&["run", "view", &id.to_string(), "--log-failed"])
    }
}

pub struct Ci<'a> {
    pub runs: &'a dyn Runs,
    /// The repository, to tell which paths in a log are its files.
    pub repo: PathBuf,
    pub limit: usize,
}

impl Source for Ci<'_> {
    fn name(&self) -> &str {
        "ci"
    }

    fn read(&self, topics: &Topics) -> Result<Vec<Item>, String> {
        let mut items = Vec::new();
        for run in self.runs.failed(self.limit)? {
            // One run's log that cannot be read leaves that run out.
            let Ok(log) = self.runs.failed_log(run.database_id) else {
                continue;
            };
            let log = insi_core::signature::strip_ansi(&tail(&log));
            let paths = files_named(&log, &self.repo);
            items.push(Item {
                source: "ci".into(),
                trust: Trust::Internal,
                topic: topics.for_paths(&paths),
                title: format!(
                    "{} failed on {}: {}",
                    run.workflow_name, run.head_branch, run.display_title
                ),
                body: log,
                url: (!run.url.is_empty()).then_some(run.url),
            });
        }
        Ok(items)
    }
}

fn tail(text: &str) -> String {
    if text.len() <= LOG_TAIL {
        return text.to_string();
    }
    let mut start = text.len() - LOG_TAIL;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    text[start..].to_string()
}

/// The repository files a log names: words that, stripped of quotes and a
/// `:line:col` suffix, are files under `repo`.
pub fn files_named(log: &str, repo: &Path) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for word in log.split(|c: char| c.is_whitespace() || "()[]'\"`,".contains(c)) {
        let word = word.trim_start_matches("./");
        let path = word.split(':').next().unwrap_or_default();
        if !(path.contains('/') || path.contains('.'))
            || path.starts_with('/')
            || path.contains("..")
            || path.len() > 200
        {
            continue;
        }
        let path = path.replace('\\', "/");
        if !found.contains(&path) && repo.join(&path).is_file() {
            found.push(path);
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use insi_core::topic::TopicRule;

    struct FakeRuns;

    impl Runs for FakeRuns {
        fn failed(&self, _: usize) -> Result<Vec<Run>, String> {
            serde_json::from_str(
                r#"[{"databaseId": 7, "displayTitle": "fix refunds", "headBranch": "insi/issue-3",
                     "workflowName": "insi", "url": "https://github.com/o/r/actions/runs/7"},
                    {"databaseId": 8, "displayTitle": "lost log"}]"#,
            )
            .map_err(|e| e.to_string())
        }
        fn failed_log(&self, id: u64) -> Result<String, String> {
            match id {
                7 => Ok("check\tRun npm test\n\u{1b}[31mFAIL\u{1b}[0m src/api/refund.test.ts:12:5\n  expected 4.99".into()),
                _ => Err("log expired".into()),
            }
        }
    }

    /// **A FAILED RUN BECOMES AN ITEM ON THE TOPIC OF THE FILES ITS LOG
    /// NAMES.** Colour codes go; a run whose log cannot be read is left out.
    #[test]
    fn failed_runs_become_items_on_the_topic_of_their_files() {
        let repo = std::env::temp_dir().join(format!("insi-ci-{}", std::process::id()));
        std::fs::create_dir_all(repo.join("src/api")).expect("dirs");
        std::fs::write(repo.join("src/api/refund.test.ts"), "test").expect("w");
        let rules = vec![TopicRule {
            topic: "shop/api".into(),
            paths: vec!["src/api".into()],
            commands: vec![],
        }];
        let ci = Ci {
            runs: &FakeRuns,
            repo: repo.clone(),
            limit: 5,
        };
        let items = ci
            .read(&Topics {
                base: "shop",
                rules: &rules,
            })
            .expect("read");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].topic, "shop/api");
        assert_eq!(items[0].trust, Trust::Internal);
        assert_eq!(items[0].title, "insi failed on insi/issue-3: fix refunds");
        assert!(
            items[0].body.contains("FAIL src/api/refund.test.ts:12:5"),
            "{}",
            items[0].body
        );
        assert_eq!(
            files_named("see ../etc/passwd and /abs/x.ts", &repo),
            Vec::<String>::new()
        );
    }
}
