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
    fn gh(repo: &Path, args: &[&str], stdin: Option<&str>) -> Result<String, String> {
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

    // Everything but the files the harness writes for itself, and the prompt
    // a long one was handed over in.
    let mut add = vec![
        "add".to_string(),
        "--all".into(),
        "--".into(),
        ".".into(),
        format!(":(exclude){}", insi_core::config::TASK_FILE),
    ];
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

    git(
        tree,
        &["push", "--quiet", "--set-upstream", "origin", branch],
    )?;
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

    /// **A LONG PROMPT'S FILE IS NOT PUBLISHED.** A prompt too long for a
    /// command line reaches the worker as a file in its tree; the branch
    /// pushed for the draft carries the worker's change, not that file.
    #[test]
    fn a_long_prompts_file_stays_out_of_the_pushed_branch() {
        let (work, origin) = repo();
        let mut long = issue(&["insi"], true);
        long.body = "Say hello. ".repeat(3000);
        let hub = FakeHub {
            issue: long,
            created: RefCell::new(Vec::new()),
        };
        let cfg = config(&["git", "--version"]);
        let worker = rewriting_worker();
        work_issue(
            &Run {
                repo: &work,
                number: 42,
                config: &cfg,
                worker: &worker,
                work_root: &temp("long-root"),
                briefing: "",
                draft: true,
            },
            &hub,
        )
        .expect("worked");
        let files = git(&origin, &["ls-tree", "--name-only", "insi/issue-42"]).expect("tree");
        assert!(files.contains("greet.txt"), "{files}");
        assert!(
            !files.contains(insi_core::config::TASK_FILE),
            "the prompt file was pushed: {files}"
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
}
