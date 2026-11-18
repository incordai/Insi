//! `insi issue <n>`: work a GitHub issue a maintainer has labelled, and open
//! a draft pull request with the evidence.
//!
//! # Who decides what is worked
//!
//! Only an OPEN issue carrying the repository's `trusted_issue_label` is
//! worked. Issue text is written by anyone; a label is applied only by
//! someone with write access, so the label is the maintainer saying "an
//! agent may take this". An unlabelled issue is refused before its text is
//! read into anything.
//!
//! # What has to be true before anything is pushed
//!
//! The repository's own check (`test_cmd`) passes in the branch's worktree,
//! and nothing the branch adds is shaped like a credential or personal data
//! (`insi_core::secrets`). A branch that
//! fails either is kept on this machine — committed, so the work is not lost
//! — and nothing leaves. A pull request is opened as a DRAFT: a person reads
//! it and decides; Insi never merges.

use std::path::Path;
use std::time::Duration;

use insi_core::{Config, Worker};
use insi_eval::git::git;
use insi_eval::process;

/// What the hosting service is asked; GitHub through `gh`, or a stand-in.
pub trait Hub {
    fn issue(&self, repo: &Path, number: u64) -> Result<Issue, String>;
    fn default_branch(&self, repo: &Path) -> Result<String, String>;
    /// Open a pull request; its URL.
    fn create_pr(&self, repo: &Path, pr: &NewPr) -> Result<String, String>;
    /// Publish the committed local `branch` of the checkout `tree`, so a pull request can name it: the person's
    /// own `git push` to `origin` unless a hub says otherwise ([`AppHub`] pushes as the GitHub App, through the
    /// engine). Called by [`work_in`] and `harness_pr::propose_in`, after the credential scan.
    fn push(&self, tree: &Path, branch: &str) -> Result<(), String> {
        git(tree, &["push", "--quiet", "--set-upstream", "origin", branch]).map(|_| ())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    pub number: u64,
    pub title: String,
    pub body: String,
    pub labels: Vec<String>,
    pub open: bool,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewPr {
    pub head: String,
    pub base: String,
    pub title: String,
    pub body: String,
    pub draft: bool,
}

/// GitHub through the `gh` CLI, which holds the login: Insi reads, holds and
/// sends no token.
pub struct GhCli;

impl GhCli {
    pub(crate) fn gh(repo: &Path, args: &[&str], stdin: Option<&str>) -> Result<String, String> {
        use std::io::Write;
        let mut child = std::process::Command::new("gh")
            .args(args)
            .current_dir(repo)
            .env("GH_PROMPT_DISABLED", "1")
            .stdin(if stdin.is_some() {
                std::process::Stdio::piped()
            } else {
                std::process::Stdio::null()
            })
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("could not run gh (is the GitHub CLI installed?): {e}"))?;
        if let (Some(text), Some(mut input)) = (stdin, child.stdin.take()) {
            input
                .write_all(text.as_bytes())
                .map_err(|e| format!("gh: {e}"))?;
        }
        let out = child.wait_with_output().map_err(|e| format!("gh: {e}"))?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
        } else {
            Err(format!(
                "gh {}: {}",
                args.first().copied().unwrap_or_default(),
                String::from_utf8_lossy(&out.stderr).trim()
            ))
        }
    }
}

impl Hub for GhCli {
    fn issue(&self, repo: &Path, number: u64) -> Result<Issue, String> {
        let raw = Self::gh(
            repo,
            &[
                "issue",
                "view",
                &number.to_string(),
                "--json",
                "number,title,body,labels,state,url",
            ],
            None,
        )?;
        parse_issue(&raw)
    }

    fn default_branch(&self, repo: &Path) -> Result<String, String> {
        Self::gh(
            repo,
            &[
                "repo",
                "view",
                "--json",
                "defaultBranchRef",
                "--jq",
                ".defaultBranchRef.name",
            ],
            None,
        )
    }

    fn create_pr(&self, repo: &Path, pr: &NewPr) -> Result<String, String> {
        let mut args = vec![
            "pr",
            "create",
            "--head",
            &pr.head,
            "--base",
            &pr.base,
            "--title",
            &pr.title,
            "--body-file",
            "-",
        ];
        if pr.draft {
            args.push("--draft");
        }
        let out = Self::gh(repo, &args, Some(&pr.body))?;
        out.lines()
            .rev()
            .find(|l| l.trim().starts_with("https://"))
            .map(|l| l.trim().to_string())
            .ok_or_else(|| format!("gh pr create printed no URL: {out}"))
    }
}

/// GitHub as the owner's GitHub App (`hub = "app"` in the config), through the Incord memory engine's tools
/// `github_issue`, `github_default_branch`, `github_push` and `github_pr_open`: the pull request shows the app
/// (`incordai[bot]`) as its author, and Insi holds no login and no token, not even the app's. The engine holds the
/// app's key and uses its installation access for the one push and the one request that needs it.
///
/// Every call can be refused with the engine's own reason (it is not running, no app is set up, the app is not
/// installed on the repository or lacks a permission); that reason is the error, and nothing falls back to `gh`.
/// Pull requests are drafts only. Chosen by [`hub_for`].
pub struct AppHub {
    addr: std::net::SocketAddr,
}

impl AppHub {
    /// The hub for the engine at `INCORD_MEMORY_ADDR` (this machine's, by default). Nothing is called yet: the
    /// first call says whether the engine runs and has the app set up. Called by [`hub_for`].
    pub fn connect() -> Result<AppHub, String> {
        Ok(AppHub {
            addr: insi_memory::incord::address()?,
        })
    }

    /// The hub for the engine at `addr`.
    pub fn at(addr: std::net::SocketAddr) -> AppHub {
        AppHub { addr }
    }

    /// One engine tool, its refusal worded as the app hub's.
    fn call(&self, tool: &str, arguments: serde_json::Value) -> Result<serde_json::Value, String> {
        insi_memory::incord::tool_result(self.addr, tool, arguments)
            .map_err(|why| format!("hub = \"app\" (GitHub through the memory engine): {why}"))
    }
}

impl Hub for AppHub {
    fn issue(&self, repo: &Path, number: u64) -> Result<Issue, String> {
        let v = self.call(
            "github_issue",
            serde_json::json!({"cwd": repo.display().to_string(), "number": number}),
        )?;
        Ok(Issue {
            number: v["number"].as_u64().ok_or("the engine's issue has no number")?,
            title: v["title"].as_str().unwrap_or_default().to_string(),
            body: v["body"].as_str().unwrap_or_default().to_string(),
            labels: v["labels"]
                .as_array()
                .map(|ls| ls.iter().filter_map(|l| l.as_str().map(String::from)).collect())
                .unwrap_or_default(),
            open: v["open"].as_bool() == Some(true),
            url: v["url"].as_str().unwrap_or_default().to_string(),
        })
    }

    fn default_branch(&self, repo: &Path) -> Result<String, String> {
        let v = self.call(
            "github_default_branch",
            serde_json::json!({"cwd": repo.display().to_string()}),
        )?;
        v["branch"]
            .as_str()
            .filter(|b| !b.is_empty())
            .map(String::from)
            .ok_or_else(|| "the engine named no default branch".to_string())
    }

    fn create_pr(&self, repo: &Path, pr: &NewPr) -> Result<String, String> {
        if !pr.draft {
            return Err(
                "hub = \"app\" opens draft pull requests only; leave out --ready".to_string(),
            );
        }
        let v = self.call(
            "github_pr_open",
            serde_json::json!({
                "cwd": repo.display().to_string(),
                "head": pr.head,
                "base": pr.base,
                "title": pr.title,
                "body": pr.body,
            }),
        )?;
        v["url"]
            .as_str()
            .filter(|u| u.starts_with("https://"))
            .map(String::from)
            .ok_or_else(|| "the engine returned no pull request URL".to_string())
    }

    fn push(&self, tree: &Path, branch: &str) -> Result<(), String> {
        self.call(
            "github_push",
            serde_json::json!({"cwd": tree.display().to_string(), "branch": branch}),
        )
        .map(|_| ())
    }
}

/// The hub the config names: `gh` (the default) or the GitHub App through the engine. An unreachable engine, or
/// one with no app, surfaces at the first call, before any worker runs (every flow asks the hub for the issue or
/// the default branch first). Called by `issue_cmd` and `improve_cmd` (crates/insi/src/main.rs).
pub fn hub_for(config: &Config) -> Result<Box<dyn Hub>, String> {
    match config.hub.unwrap_or_default() {
        insi_core::config::HubKind::Gh => Ok(Box::new(GhCli)),
        insi_core::config::HubKind::App => Ok(Box::new(AppHub::connect()?)),
    }
}

/// Refuse `what` when the config names the app hub: it publishes from a clone the engine cannot push (its
/// `origin` is the local repository, not a github.com address), so it keeps `gh`'s login. The refusal is the
/// reason and the way out; nothing falls back silently. Called by `suggest_cmd` and the autonomous cycle.
pub fn gh_hub_only(config: &Config, what: &str) -> Result<(), String> {
    match config.hub.unwrap_or_default() {
        insi_core::config::HubKind::Gh => Ok(()),
        insi_core::config::HubKind::App => Err(format!(
            "{what} publishes from a clone, which the app hub cannot push: set hub = \"gh\" in .insi/config.toml for it, or use --local"
        )),
    }
}

/// An issue from `gh issue view --json number,title,body,labels,state,url`.
pub fn parse_issue(raw: &str) -> Result<Issue, String> {
    let v: serde_json::Value =
        serde_json::from_str(raw).map_err(|e| format!("gh's issue is not JSON: {e}"))?;
    Ok(Issue {
        number: v["number"].as_u64().ok_or("the issue has no number")?,
        title: v["title"].as_str().unwrap_or_default().to_string(),
        body: v["body"].as_str().unwrap_or_default().to_string(),
        labels: v["labels"]
            .as_array()
            .map(|ls| {
                ls.iter()
                    .filter_map(|l| l["name"].as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        open: v["state"].as_str() == Some("OPEN"),
        url: v["url"].as_str().unwrap_or_default().to_string(),
    })
}

/// What working the issue came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// A draft pull request was opened.
    Opened { url: String, branch: String },
    /// The work is committed on a local branch and nothing was pushed.
    Kept { branch: String, why: String },
}

pub struct Run<'a> {
    pub repo: &'a Path,
    pub number: u64,
    pub config: &'a Config,
    pub worker: &'a Worker,
    /// Where the issue's worktree is made.
    pub work_root: &'a Path,
    /// Lessons put in front of the prompt, or empty.
    pub briefing: &'a str,
    pub draft: bool,
}

/// The branch an issue is worked on.
pub fn branch_for(number: u64) -> String {
    format!("insi/issue-{number}")
}

/// Work the issue. Refused, with nothing done, when the issue is not open,
/// does not carry the trusted label, the repository names no label or no
/// check, or the branch already exists (the issue was worked before; that
/// branch is a person's to look at first).
pub fn work_issue(run: &Run, hub: &dyn Hub) -> Result<Outcome, String> {
    let label = run
        .config
        .trusted_issue_label
        .as_deref()
        .filter(|l| !l.trim().is_empty())
        .ok_or("set trusted_issue_label in .insi/config.toml: the label a maintainer puts on an issue an agent may work")?;
    if run.config.test_cmd.is_empty() {
        return Err(
            "set test_cmd in .insi/config.toml: nothing is pushed without the repository's own check passing"
                .to_string(),
        );
    }
    let issue = hub.issue(run.repo, run.number)?;
    if !issue.open {
        return Err(format!("issue #{} is not open", issue.number));
    }
    if !issue.labels.iter().any(|l| l.eq_ignore_ascii_case(label)) {
        return Err(format!(
            "issue #{} does not carry the label \"{label}\"; only a labelled issue is worked",
            issue.number
        ));
    }
    let branch = branch_for(issue.number);
    if git(
        run.repo,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ],
    )
    .is_ok()
    {
        return Err(format!(
            "branch {branch} already exists: issue #{} was worked before; look at that branch first",
            issue.number
        ));
    }

    let base = hub.default_branch(run.repo)?;
    git(run.repo, &["fetch", "--quiet", "origin", &base])?;
    let tree = run
        .work_root
        .join("issues")
        .join(format!("issue-{}", issue.number));
    std::fs::create_dir_all(run.work_root.join("issues")).map_err(|e| e.to_string())?;
    git(
        run.repo,
        &[
            "worktree",
            "add",
            "-b",
            &branch,
            &tree.display().to_string(),
            &format!("origin/{base}"),
        ],
    )?;
    let outcome = work_in(run, hub, &issue, &branch, &base, &tree);
    // The branch keeps the work either way; the checkout goes.
    let _ = git(
        run.repo,
        &["worktree", "remove", "--force", &tree.display().to_string()],
    );
    outcome
}

/// Run `argv` for the work in `tree`: inside the repository's sandbox when
/// it names one, otherwise on this machine.
fn step(
    cfg: &Config,
    argv: &[String],
    tree: &Path,
    wall: Duration,
) -> Result<process::Outcome, String> {
    match &cfg.sandbox {
        Some(sandbox) => process::run(&sandbox.wrap(argv, tree), tree, wall),
        None => process::run(argv, tree, wall),
    }
}

fn work_in(
    run: &Run,
    hub: &dyn Hub,
    issue: &Issue,
    branch: &str,
    base: &str,
    tree: &Path,
) -> Result<Outcome, String> {
    let cfg = run.config;
    if !cfg.setup.is_empty() {
        let setup = step(cfg, &cfg.setup, tree, Duration::from_secs(30 * 60))?;
        if !setup.success() {
            return Err(format!("setup failed: {}", last_line(&setup.tail())));
        }
    }
    let prompt = format!(
        "{}Work GitHub issue #{}: {}\n\n{}\n\nMake the change in this repository. \
         The repository's own check (`{}`) must pass when you are done.",
        if run.briefing.is_empty() {
            String::new()
        } else {
            format!("{}\n\n", run.briefing)
        },
        issue.number,
        issue.title,
        issue.body,
        cfg.test_cmd.join(" ")
    );
    let wall = Duration::from_secs(cfg.wall_secs.unwrap_or(1800));
    let started = std::time::Instant::now();
    let ran = step(cfg, &run.worker.render_in(&prompt, tree)?, tree, wall)?;
    let worker_secs = started.elapsed().as_secs();
    let tokens = insi_eval::run::reported(&ran.stdout).map(|(_, usage)| usage.total());

    // Everything but the files the harness writes for itself.
    let mut add = vec!["add".to_string(), "--all".into(), "--".into(), ".".into()];
    add.extend(
        run.worker
            .own_files
            .iter()
            .map(|p| format!(":(glob,exclude){p}")),
    );
    let add: Vec<&str> = add.iter().map(String::as_str).collect();
    git(tree, &add)?;
    if git(tree, &["diff", "--cached", "--quiet"]).is_ok() {
        return Err(format!(
            "the worker changed nothing for issue #{}{}",
            issue.number,
            if ran.timed_out {
                " (it ran out of time)"
            } else {
                ""
            }
        ));
    }
    let title = format!("Fix #{}: {}", issue.number, issue.title.trim());
    git(tree, &["commit", "--quiet", "-m", &title])?;

    if !ran.success() {
        return Ok(Outcome::Kept {
            branch: branch.to_string(),
            why: if ran.timed_out {
                "the worker timed out; candidate kept locally".into()
            } else {
                "the worker failed; candidate kept locally".into()
            },
        });
    }

    // The check runs the code the worker wrote: inside the sandbox too.
    let check = step(cfg, &cfg.test_cmd, tree, Duration::from_secs(30 * 60))?;
    if !check.success() {
        return Ok(Outcome::Kept {
            branch: branch.to_string(),
            why: format!(
                "the check `{}` failed: {}",
                cfg.test_cmd.join(" "),
                last_line(&check.tail())
            ),
        });
    }
    let diff = git(
        tree,
        &["diff", "--no-color", &format!("origin/{base}..HEAD")],
    )?;
    let found = insi_core::secrets::scan_diff(&diff);
    if !found.is_empty() {
        let listed: Vec<String> = found.iter().take(10).map(|f| f.to_string()).collect();
        return Ok(Outcome::Kept {
            branch: branch.to_string(),
            why: format!(
                "the branch adds what looks like a credential or personal data, so nothing \
                 was pushed: {}",
                listed.join("; ")
            ),
        });
    }

    hub.push(tree, branch)?;
    let body = format!(
        "Closes #{n}.\n\n\
         Worked by Insi because the issue carries the label `{label}`. \
         **Review it before merging — nothing here is merged automatically.**\n\n\
         - **Check:** `{check}` passed in this branch's worktree.\n\
         - **Worker:** `{worker}`, {secs}s{tokens}.\n\
         - **Lessons briefed:** {lessons}.\n\
         - **Where it ran:** {place}.\n\
         - **Credential and personal-data scan:** nothing credential- or PII-shaped in the \
         lines this branch adds.\n",
        n = issue.number,
        label = cfg.trusted_issue_label.as_deref().unwrap_or_default(),
        check = cfg.test_cmd.join(" "),
        worker = run
            .worker
            .command
            .first()
            .map(String::as_str)
            .unwrap_or("?"),
        secs = worker_secs,
        tokens = tokens.map(|t| format!(", {t} tokens")).unwrap_or_default(),
        lessons = if run.briefing.is_empty() {
            "none"
        } else {
            "yes, see the commit's prompt"
        },
        place = match &cfg.sandbox {
            Some(s) => format!(
                "setup, worker and check inside the sandbox `{}`",
                s.command.first().map(String::as_str).unwrap_or("?")
            ),
            None => "on the operator's machine (no sandbox configured)".to_string(),
        },
    );
    let url = hub.create_pr(
        run.repo,
        &NewPr {
            head: branch.to_string(),
            base: base.to_string(),
            title,
            body,
            draft: run.draft,
        },
    )?;
    Ok(Outcome::Opened {
        url,
        branch: branch.to_string(),
    })
}

fn last_line(text: &str) -> String {
    text.lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or_default()
        .chars()
        .take(300)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::path::PathBuf;

    struct FakeHub {
        issue: Issue,
        created: RefCell<Vec<NewPr>>,
    }

    impl Hub for FakeHub {
        fn issue(&self, _: &Path, _: u64) -> Result<Issue, String> {
            Ok(self.issue.clone())
        }
        fn default_branch(&self, _: &Path) -> Result<String, String> {
            Ok("main".to_string())
        }
        fn create_pr(&self, _: &Path, pr: &NewPr) -> Result<String, String> {
            self.created.borrow_mut().push(pr.clone());
            Ok("https://github.com/o/r/pull/9".to_string())
        }
    }

    fn issue(labels: &[&str], open: bool) -> Issue {
        Issue {
            number: 42,
            title: "Greeting is rude".into(),
            body: "Say hello.".into(),
            labels: labels.iter().map(|l| l.to_string()).collect(),
            open,
            url: "https://github.com/o/r/issues/42".into(),
        }
    }

    fn temp(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let dir =
            std::env::temp_dir().join(format!("insi-issue-{label}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        dir
    }

    /// A repository with an `origin` (a local bare one) on `main`.
    fn repo() -> (PathBuf, PathBuf) {
        let base = temp("repo");
        let origin = base.join("origin.git");
        let work = base.join("work");
        git(
            &base,
            &["init", "--quiet", "--bare", "-b", "main", "origin.git"],
        )
        .expect("bare");
        git(&base, &["init", "--quiet", "-b", "main", "work"]).expect("init");
        for (k, v) in [("user.email", "t@example.invalid"), ("user.name", "t")] {
            git(&work, &["config", k, v]).expect("config");
        }
        // A git-config file, so the test worker (`git config --file`) can
        // write into it.
        std::fs::write(work.join("greet.txt"), "[greeting]\n\ttext = go away\n").expect("w");
        git(&work, &["add", "."]).expect("add");
        git(&work, &["commit", "--quiet", "-m", "first"]).expect("commit");
        git(
            &work,
            &["remote", "add", "origin", &origin.display().to_string()],
        )
        .expect("remote");
        git(&work, &["push", "--quiet", "origin", "main"]).expect("push");
        (work, origin)
    }

    /// A worker that changes `greet.txt`: a real program, run in the issue's
    /// worktree, given the prompt as one argument and no shell to read it —
    /// git writes it into the file as a config value, escaped.
    fn rewriting_worker() -> Worker {
        Worker::command(
            [
                "git",
                "config",
                "--file",
                "greet.txt",
                "issue.prompt",
                "{prompt}",
            ]
            .map(String::from)
            .to_vec(),
        )
    }

    #[cfg(unix)]
    #[test]
    fn failed_worker_changes_are_kept_local_even_when_checks_pass() {
        let (work, origin) = repo();
        let hub = FakeHub {
            issue: issue(&["insi"], true),
            created: RefCell::new(Vec::new()),
        };
        let cfg = config(&["git", "--version"]);
        let worker = Worker::command(vec![
            "sh".into(),
            "-c".into(),
            "git config --file greet.txt issue.prompt partial; exit 1".into(),
            "{prompt}".into(),
        ]);
        let outcome = work_issue(
            &Run {
                repo: &work,
                number: 42,
                config: &cfg,
                worker: &worker,
                work_root: &temp("failed-worker"),
                briefing: "",
                draft: true,
            },
            &hub,
        )
        .expect("worker outcome");
        assert!(matches!(outcome, Outcome::Kept { .. }), "{outcome:?}");
        assert!(hub.created.borrow().is_empty());
        assert!(git(
            &origin,
            &["rev-parse", "--verify", "refs/heads/insi/issue-42"]
        )
        .is_err());
        assert!(git(&work, &["show", "insi/issue-42:greet.txt"])
            .unwrap()
            .contains("partial"));
    }

    fn config(check: &[&str]) -> Config {
        Config {
            trusted_issue_label: Some("insi".into()),
            test_cmd: check.iter().map(|s| s.to_string()).collect(),
            ..Config::default()
        }
    }

    /// **A LABELLED ISSUE BECOMES A DRAFT PR, WITH ITS EVIDENCE.** The
    /// worker's change is committed on `insi/issue-42`, the check passes, the
    /// branch reaches origin, and the draft names the issue and the check.
    #[test]
    fn a_labelled_issue_is_worked_into_a_draft_pull_request() {
        let (work, origin) = repo();
        let hub = FakeHub {
            issue: issue(&["insi", "bug"], true),
            created: RefCell::new(Vec::new()),
        };
        let cfg = config(&["git", "--version"]);
        let worker = rewriting_worker();
        let outcome = work_issue(
            &Run {
                repo: &work,
                number: 42,
                config: &cfg,
                worker: &worker,
                work_root: &temp("work-root"),
                briefing: "",
                draft: true,
            },
            &hub,
        )
        .expect("worked");
        assert_eq!(
            outcome,
            Outcome::Opened {
                url: "https://github.com/o/r/pull/9".into(),
                branch: "insi/issue-42".into()
            }
        );
        let created = hub.created.borrow();
        assert!(created[0].draft);
        assert_eq!(created[0].title, "Fix #42: Greeting is rude");
        assert!(
            created[0].body.starts_with("Closes #42."),
            "{}",
            created[0].body
        );
        assert!(created[0].body.contains("`git --version` passed"));
        let pushed = git(&origin, &["show", "insi/issue-42:greet.txt"]).expect("pushed");
        assert!(
            pushed.contains("Greeting is rude"),
            "the worker never saw the issue: {pushed}"
        );
    }

    /// **NOTHING UNLABELLED, CLOSED OR UNCHECKED IS WORKED, AND A FAILING
    /// CHECK PUSHES NOTHING.** Each refusal happens before the worker runs;
    /// a failed check keeps the commit locally and opens no PR.
    #[test]
    fn what_may_not_be_worked_is_refused_and_a_failing_check_stays_local() {
        let (work, origin) = repo();
        let worker = rewriting_worker();
        let root = temp("root");
        let attempt = |hub: &FakeHub, cfg: &Config| {
            work_issue(
                &Run {
                    repo: &work,
                    number: 42,
                    config: cfg,
                    worker: &worker,
                    work_root: &root,
                    briefing: "",
                    draft: true,
                },
                hub,
            )
        };
        let hub = |labels: &[&str], open| FakeHub {
            issue: issue(labels, open),
            created: RefCell::new(Vec::new()),
        };
        let checked = config(&["git", "--version"]);

        let why = attempt(&hub(&["bug"], true), &checked).expect_err("unlabelled");
        assert!(why.contains("does not carry the label"), "{why}");
        let why = attempt(&hub(&["insi"], false), &checked).expect_err("closed");
        assert!(why.contains("not open"), "{why}");
        let why = attempt(&hub(&["insi"], true), &config(&[])).expect_err("no check");
        assert!(why.contains("test_cmd"), "{why}");
        let mut unlabelled_repo = checked.clone();
        unlabelled_repo.trusted_issue_label = None;
        let why = attempt(&hub(&["insi"], true), &unlabelled_repo).expect_err("no label set");
        assert!(why.contains("trusted_issue_label"), "{why}");
        assert!(
            git(
                &work,
                &[
                    "rev-parse",
                    "--verify",
                    "--quiet",
                    "refs/heads/insi/issue-42"
                ]
            )
            .is_err(),
            "a refused issue left a branch"
        );

        // A check that fails: committed locally, never pushed, no PR.
        let failing = hub(&["insi"], true);
        let outcome = attempt(&failing, &config(&["git", "no-such-subcommand-x"])).expect("ran");
        assert!(
            matches!(&outcome, Outcome::Kept { why, .. } if why.contains("failed")),
            "{outcome:?}"
        );
        assert!(failing.created.borrow().is_empty());
        git(&work, &["show", "insi/issue-42:greet.txt"]).expect("the work was kept locally");
        assert!(
            git(
                &origin,
                &[
                    "rev-parse",
                    "--verify",
                    "--quiet",
                    "refs/heads/insi/issue-42"
                ]
            )
            .is_err(),
            "a failing branch was pushed"
        );

        // Worked once: the branch exists, so it is not worked again.
        let why = attempt(&hub(&["insi"], true), &checked).expect_err("again");
        assert!(why.contains("already exists"), "{why}");
    }

    /// **WITH A SANDBOX, SETUP, THE WORKER AND THE CHECK ALL RUN INSIDE IT.**
    /// The stand-in sandbox is `git -C {tree}`: the worker's `config …` and
    /// the check's `log …` are no programs on their own, so the issue is
    /// worked only if each went through the sandbox with the worktree's path
    /// in place of `{tree}`; the PR says where the work ran.
    #[test]
    fn a_sandbox_runs_every_step_of_the_issue() {
        let (work, origin) = repo();
        let hub = FakeHub {
            issue: issue(&["insi"], true),
            created: RefCell::new(Vec::new()),
        };
        let argv = |words: &[&str]| words.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let cfg = Config {
            setup: argv(&["status", "--short"]),
            sandbox: Some(insi_core::config::Sandbox {
                command: argv(&["git", "-C", "{tree}"]),
            }),
            ..config(&["log", "-1", "--format=%s"])
        };
        let worker = Worker::command(argv(&[
            "config",
            "--file",
            "greet.txt",
            "issue.prompt",
            "{prompt}",
        ]));
        let outcome = work_issue(
            &Run {
                repo: &work,
                number: 42,
                config: &cfg,
                worker: &worker,
                work_root: &temp("sandbox-root"),
                briefing: "",
                draft: true,
            },
            &hub,
        )
        .expect("worked");
        assert!(matches!(outcome, Outcome::Opened { .. }), "{outcome:?}");
        let pushed = git(&origin, &["show", "insi/issue-42:greet.txt"]).expect("pushed");
        assert!(pushed.contains("Greeting is rude"), "{pushed}");
        assert!(
            hub.created.borrow()[0]
                .body
                .contains("inside the sandbox `git`"),
            "{}",
            hub.created.borrow()[0].body
        );
    }

    #[test]
    fn gh_issue_json_is_read() {
        let raw = r#"{"number":7,"title":"t","body":"b","labels":[{"name":"insi"},{"name":"bug"}],"state":"OPEN","url":"u"}"#;
        let i = parse_issue(raw).expect("parsed");
        assert_eq!((i.number, i.open), (7, true));
        assert_eq!(i.labels, ["insi", "bug"]);
        assert!(
            !parse_issue(r#"{"number":7,"state":"CLOSED"}"#)
                .expect("parsed")
                .open
        );
    }

    use crate::engine_stub;
    use serde_json::json;

    /// What the engine's tools answer for issue 42, and what its `github_push` does: it pushes the branch to the
    /// checkout's `origin` itself, as the engine does with the app's access. `refuse` makes a tool answer with
    /// that reason instead.
    fn engine(refuse: Option<(&'static str, &'static str)>) -> engine_stub::StandIn {
        engine_stub::start(move |tool, args| {
            if let Some((name, why)) = refuse {
                if name == tool {
                    return Err(why.to_string());
                }
            }
            match tool {
                "github_issue" => Ok(json!({
                    "repo": "o/r", "number": 42, "title": "Greeting is rude", "body": "Say hello.",
                    "labels": ["insi", "bug"], "state": "open", "open": true,
                    "url": "https://github.com/o/r/issues/42", "is_pull_request": false
                })),
                "github_default_branch" => Ok(json!({"repo": "o/r", "branch": "main"})),
                "github_push" => {
                    let tree = args["cwd"].as_str().ok_or("no cwd")?;
                    let branch = args["branch"].as_str().ok_or("no branch")?;
                    git(Path::new(tree), &["push", "--quiet", "origin", branch])
                        .map(|_| json!({"repo": "o/r", "branch": branch, "pushed": true}))
                }
                "github_pr_open" => {
                    Ok(json!({"repo": "o/r", "number": 9, "url": "https://github.com/o/r/pull/9", "draft": true}))
                }
                other => Err(format!("unknown tool: '{other}'")),
            }
        })
    }

    fn app_config() -> Config {
        Config {
            hub: Some(insi_core::config::HubKind::App),
            ..config(&["git", "--version"])
        }
    }

    /// **THE APP HUB ASKS THE ENGINE FOR EVERYTHING GITHUB**: the issue, the default branch, the push and the
    /// draft pull request, each by the engine's tool with the repository's folder, and returns what it answers.
    #[test]
    fn the_app_hub_calls_the_engines_github_tools() {
        let stand_in = engine(None);
        let hub = AppHub::at(stand_in.addr);
        let repo = Path::new("/some/repo");
        let got = hub.issue(repo, 42).expect("issue");
        assert_eq!((got.number, got.open), (42, true));
        assert_eq!(got.labels, ["insi", "bug"]);
        assert_eq!(got.title, "Greeting is rude");
        assert_eq!(stand_in.last("github_issue"), json!({"cwd": "/some/repo", "number": 42}));
        assert_eq!(hub.default_branch(repo).expect("branch"), "main");

        let url = hub
            .create_pr(
                repo,
                &NewPr {
                    head: "insi/issue-42".into(),
                    base: "main".into(),
                    title: "Fix #42".into(),
                    body: "Closes #42.".into(),
                    draft: true,
                },
            )
            .expect("opened");
        assert_eq!(url, "https://github.com/o/r/pull/9");
        assert_eq!(
            stand_in.last("github_pr_open"),
            json!({"cwd": "/some/repo", "head": "insi/issue-42", "base": "main", "title": "Fix #42", "body": "Closes #42."})
        );

        // Drafts only: a ready pull request is refused before the engine is asked.
        let before = stand_in.tools().len();
        let why = hub
            .create_pr(
                repo,
                &NewPr {
                    head: "h".into(),
                    base: "main".into(),
                    title: "t".into(),
                    body: "b".into(),
                    draft: false,
                },
            )
            .expect_err("ready");
        assert!(why.contains("draft"), "{why}");
        assert_eq!(stand_in.tools().len(), before);
    }

    /// **WITH THE APP HUB THE ENGINE PUSHES, NOT INSI**: the branch reaches origin only through `github_push`
    /// (the stand-in does the push), the draft names it, and Insi runs no `git push` of its own.
    #[test]
    fn with_the_app_hub_the_branch_is_pushed_by_the_engine() {
        let (work, origin) = repo();
        let stand_in = engine(None);
        let hub = AppHub::at(stand_in.addr);
        let outcome = work_issue(
            &Run {
                repo: &work,
                number: 42,
                config: &app_config(),
                worker: &rewriting_worker(),
                work_root: &temp("app-root"),
                briefing: "",
                draft: true,
            },
            &hub,
        )
        .expect("worked");
        assert_eq!(
            outcome,
            Outcome::Opened {
                url: "https://github.com/o/r/pull/9".into(),
                branch: "insi/issue-42".into()
            }
        );
        assert_eq!(
            stand_in.tools(),
            ["github_issue", "github_default_branch", "github_push", "github_pr_open"],
            "the hub is asked in order, and the push comes before the pull request"
        );
        let push = stand_in.last("github_push");
        assert_eq!(push["branch"], "insi/issue-42");
        assert!(
            push["cwd"].as_str().is_some_and(|c| c.contains("issue-42")),
            "the engine is told the branch's worktree: {push}"
        );
        assert!(git(&origin, &["show", "insi/issue-42:greet.txt"])
            .expect("pushed")
            .contains("Greeting is rude"));
        assert_eq!(stand_in.last("github_pr_open")["head"], "insi/issue-42");
    }

    /// **A REFUSAL BY THE ENGINE STOPS INSI WITH THE ENGINE'S REASON, AND NOTHING FALLS BACK TO `gh`**: with no
    /// app set up the issue read is refused before any worker runs; a refused push leaves nothing on origin and
    /// opens no pull request.
    #[test]
    fn an_engine_refusal_stops_the_app_hub_and_never_falls_back() {
        let (work, origin) = repo();
        let run = |stand_in: &engine_stub::StandIn, root: &str| {
            work_issue(
                &Run {
                    repo: &work,
                    number: 42,
                    config: &app_config(),
                    worker: &rewriting_worker(),
                    work_root: &temp(root),
                    briefing: "",
                    draft: true,
                },
                &AppHub::at(stand_in.addr),
            )
        };
        let off = "GitHub is not set up on this engine: set github_app_id in the audit settings";
        let stand_in = engine(Some(("github_issue", off)));
        let why = run(&stand_in, "off-root").expect_err("refused");
        assert!(why.contains("hub = \"app\"") && why.contains(off), "{why}");
        assert_eq!(stand_in.tools(), ["github_issue"], "stopped at the first call, before any worker ran");

        let denied = "the push of insi/issue-42 to o/r failed: denied; the GitHub App must be installed on o/r with Contents and Pull requests write permission";
        let stand_in = engine(Some(("github_push", denied)));
        let why = run(&stand_in, "denied-root").expect_err("refused push");
        assert!(why.contains("Contents and Pull requests write"), "{why}");
        assert!(!stand_in.tools().contains(&"github_pr_open".to_string()), "no pull request without the push");
        assert!(
            git(&origin, &["rev-parse", "--verify", "--quiet", "refs/heads/insi/issue-42"]).is_err(),
            "Insi pushed nothing itself"
        );
    }

    /// An engine that is not running is the reason, worded as the app hub's.
    #[test]
    fn an_engine_that_is_not_running_is_the_reason() {
        let closed = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
            listener.local_addr().expect("addr")
        };
        let why = AppHub::at(closed).default_branch(Path::new("/r")).expect_err("unreachable");
        assert!(why.contains("hub = \"app\"") && why.contains("unreachable"), "{why}");
    }

    /// WHO PUBLISHES IS THE CONFIG'S CHOICE: `gh` unless it says `app`, and the flows that publish from a
    /// clone, which the engine cannot push, refuse `app` instead of using the person's login.
    #[test]
    fn the_hub_follows_the_config_and_clone_flows_refuse_the_app() {
        assert!(hub_for(&Config::default()).is_ok());
        assert!(hub_for(&Config { hub: Some(insi_core::config::HubKind::Gh), ..Config::default() }).is_ok());
        assert!(hub_for(&app_config()).is_ok(), "connecting calls nothing; the first call says");
        assert!(gh_hub_only(&Config::default(), "insi suggest").is_ok());
        let why = gh_hub_only(&app_config(), "insi suggest").expect_err("refused");
        assert!(why.contains("insi suggest") && why.contains("hub = \"gh\""), "{why}");
    }
}
