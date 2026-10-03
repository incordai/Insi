//! `insi` — the command line.
//!
//! ```text
//! insi init     [--repo <dir>] [--check "<cmd {tests}>"] [--setup "<cmd>"] [--force]
//! insi mine     [--source <repo>] [--tasks <dir>] [--private <dir>] [--check "<cmd {tests}>"]
//!               [--setup "<cmd>"] [--prefix <name>] [--limit N] [--wall-secs N]
//! insi validate --tasks <dir> --private <dir> [--only id,id] [--half older|newer] [--results <file>] [--jobs N]
//! insi eval     --tasks <dir> --private <dir> [--seeds N] [--tag k=v] [--worker "<cmd {prompt}>"]
//!               [--lessons <file>] [--fair-only] [--only id,id] [--half older|newer]
//!               [--results <file>] [--keep] [--jobs N] [--max-dollars D]
//! insi learn    --tasks <dir> --private <dir> [--lessons <file>] [--worker "<cmd {prompt}>"] [--to-memory]
//!               [--only id,id] [--half older|newer] [--results <file>] [--jobs N]
//! insi report   [--results <file>]
//! insi brief    --tasks <dir> --private <dir> [--only id,id] [--half older|newer]
//! insi mcp      [--lessons <file>]
//! insi hook     [--lessons <file>]   (a Claude Code PreToolUse/PostToolUse hook)
//! insi issue    --number N [--repo <dir>] [--worker "<cmd {prompt}>"] [--lessons <file>] [--ready]
//! ```
//!
//! Commands are written as one string and split on spaces; an argument with
//! a space in it does not fit that form, and a command that needs one is
//! better put in `.insi/config.toml`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

mod hook;
mod init;
mod issue;
mod learn;
mod mcp;

use insi_core::{Config, Task, Worker};
use insi_eval::mine::{self, Options};
use insi_eval::pool;
use insi_eval::report;
use insi_eval::run::{self, Attempt, Kind, Work};
use insi_eval::store::{self, Store};
use insi_memory::briefing;
use insi_memory::store::{FileLessons, LessonStore};

const USAGE: &str = "usage:
  insi init     [--repo <dir>] [--check \"<cmd {tests}>\"] [--setup \"<cmd>\"] [--force]
  insi mine     [--source <repo>] [--tasks <dir>] [--private <dir>] [--check \"<cmd {tests}>\"]
                [--setup \"<cmd>\"] [--prefix <name>] [--limit N] [--wall-secs N]
  insi validate --tasks <dir> --private <dir> [--only id,id] [--half older|newer] [--results <file>] [--jobs N]
  insi eval     --tasks <dir> --private <dir> [--seeds N] [--tag k=v] [--worker \"<cmd {prompt}>\"]
                [--lessons <file>] [--fair-only] [--only id,id] [--half older|newer]
                [--results <file>] [--keep] [--jobs N] [--max-dollars D]
                (D: stop starting attempts once they cost D US dollars, by
                [prices] in .insi/config.toml)
  insi learn    --tasks <dir> --private <dir> [--lessons <file>] [--worker \"<cmd {prompt}>\"] [--to-memory]
                [--only id,id] [--half older|newer] [--results <file>] [--jobs N]
  insi report   [--results <file>]
  insi brief    --tasks <dir> --private <dir> [--only id,id] [--half older|newer]
                (the interface each task's worker is told: what its hidden tests import
                that the base lacks)
  insi mcp      [--lessons <file>]      (MCP on stdio: insi_briefing, insi_lessons_for_error)
  insi hook     [--lessons <file>]      (Claude Code hook: lessons before an edit, after a failure)
  insi issue    --number N [--repo <dir>] [--worker \"<cmd {prompt}>\"] [--lessons <file>] [--ready]
                (work a GitHub issue labelled trusted_issue_label; a draft PR if test_cmd passes)

In a repository with .insi/config.toml (write one with `insi init`), the
check, setup, prefix and budget come from it, and --tasks/--private default
to <private_checks>/tasks and <private_checks>/hidden, so `insi mine` needs
no flags at all.

Tasks come from a repository's history and are only read from it; every
worktree lives in Insi's own clone under --work (default .insi-work).
--jobs N runs N attempts at once; --setup-jobs N (default 1) lets only N of
them install dependencies at once, which is bound by the disk.
Lessons default to <work>/lessons.jsonl. A lesson is briefed only to tasks
dated after the one it was learned from; --half older|newer splits the tasks
by date, to learn on one half and measure on the other.";

/// The most of a prompt a briefing may take.
const BRIEFING_CHARS: usize = 4_000;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(why) => {
            eprintln!("insi: {why}");
            ExitCode::FAILURE
        }
    }
}

/// `--key value` pairs and bare `--flag`s.
struct Flags(BTreeMap<String, Option<String>>);

impl Flags {
    fn parse(args: &[String]) -> Result<Flags, String> {
        let mut map = BTreeMap::new();
        let mut i = 0;
        while i < args.len() {
            let key = args[i]
                .strip_prefix("--")
                .ok_or_else(|| format!("unexpected argument \"{}\"\n\n{USAGE}", args[i]))?;
            let value = args.get(i + 1).filter(|v| !v.starts_with("--")).cloned();
            i += if value.is_some() { 2 } else { 1 };
            map.insert(key.to_string(), value);
        }
        Ok(Flags(map))
    }
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|v| v.as_deref())
    }
    fn has(&self, key: &str) -> bool {
        self.0.contains_key(key)
    }
    fn number(&self, key: &str, default: u64) -> Result<u64, String> {
        self.get(key).map_or(Ok(default), |v| {
            v.parse().map_err(|_| format!("--{key} must be a number"))
        })
    }
    fn argv(&self, key: &str) -> Vec<String> {
        self.get(key)
            .map(|v| v.split_whitespace().map(String::from).collect())
            .unwrap_or_default()
    }
    fn list(&self, key: &str) -> Vec<String> {
        self.get(key)
            .map(|v| {
                v.split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let Some((command, rest)) = args.split_first() else {
        return Err(USAGE.to_string());
    };
    let flags = Flags::parse(rest)?;
    let work = Work::with_setup_jobs(
        PathBuf::from(flags.get("work").unwrap_or(".insi-work")),
        flags.number("setup-jobs", 1)? as usize,
    );
    let results = flags
        .get("results")
        .map(PathBuf::from)
        .unwrap_or_else(|| work.root.join("results.jsonl"));
    let outcome = command_by_name(command, &flags, &work, &results);
    if work.root.join("trash").exists() {
        eprintln!("insi: removing finished worktrees …");
    }
    work.wait_for_cleanup();
    outcome
}

fn command_by_name(
    command: &str,
    flags: &Flags,
    work: &Work,
    results: &Path,
) -> Result<(), String> {
    match command {
        "init" => init_cmd(flags),
        "issue" => issue_cmd(flags, work),
        "mine" => mine_cmd(flags),
        "validate" => validate_cmd(flags, work, results),
        "eval" => eval_cmd(flags, work, results),
        "learn" => learn_cmd(flags, work, results),
        "mcp" => {
            // stdout is the protocol from here on: nothing else may print.
            let lessons = lessons_file(flags, work);
            mcp::serve(&|| lessons.all())
        }
        "hook" => {
            hook_cmd(flags, work);
            Ok(())
        }
        "report" => report_cmd(results),
        "brief" => brief_cmd(flags),
        "-h" | "--help" | "help" => {
            println!("{USAGE}");
            Ok(())
        }
        "-V" | "--version" => {
            println!("insi {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        other => Err(format!("no command \"{other}\"\n\n{USAGE}")),
    }
}

/// For each selected task, the interface its worker will be told: what the
/// hidden tests import that the base lacks. Read from the task's source
/// repository with `git show <base>:<path>`, which reads and never writes,
/// so a task can be checked before half an hour is spent on it.
fn brief_cmd(flags: &Flags) -> Result<(), String> {
    let store = store(flags)?;
    for task in selected(flags, &store)? {
        let tests = task
            .hidden_tests
            .iter()
            .map(|p| {
                let bytes = store.hidden(&task, p)?;
                Ok((p.clone(), String::from_utf8_lossy(&bytes).into_owned()))
            })
            .collect::<Result<Vec<_>, String>>()?;
        let source = PathBuf::from(&task.source);
        let at_base = |path: &str| {
            insi_eval::git::git(&source, &["show", &format!("{}:{path}", task.base)]).ok()
        };
        let text = insi_eval::interface::brief(&insi_eval::interface::needs(&tests, &at_base));
        println!(
            "{}\n{}\n",
            task.id,
            if text.is_empty() {
                "  nothing missing: the tests use what the base already has".to_string()
            } else {
                text.lines()
                    .map(|l| format!("  {l}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        );
    }
    Ok(())
}

/// One Claude Code hook event from stdin; the answer, if any, on stdout.
/// Never an error: a hook that fails must not stop the agent, so anything
/// unreadable is no answer, and why goes to stderr (which Claude Code shows
/// only in its verbose view when the hook exits 0).
fn hook_cmd(flags: &Flags, work: &Work) {
    use std::io::Read;
    let mut input = String::new();
    if let Err(e) = std::io::stdin().read_to_string(&mut input) {
        eprintln!("insi hook: could not read the event: {e}");
        return;
    }
    let event = match serde_json::from_str::<serde_json::Value>(&input) {
        Ok(event) => event,
        Err(e) => {
            eprintln!("insi hook: the event is not JSON: {e}");
            return;
        }
    };
    let repo = event["cwd"]
        .as_str()
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok());
    let config = match repo.as_deref().map(Config::load) {
        Some(Ok(config)) => config,
        Some(Err(e)) => {
            eprintln!("insi hook: {e}");
            return;
        }
        None => Config::default(),
    };
    let lessons = lessons_file(flags, work);
    match hook::respond(&event, &|| lessons.all(), config.topics("task"), &work.root) {
        Ok(Some(answer)) => println!("{answer}"),
        Ok(None) => {}
        Err(why) => eprintln!("insi hook: {why} ({})", lessons.path.display()),
    }
}

/// The current folder's `.insi/config.toml`, or the defaults.
fn repo_config() -> Result<(PathBuf, Config), String> {
    let here = std::env::current_dir().map_err(|e| e.to_string())?;
    let config = Config::load(&here)?;
    Ok((here, config))
}

/// Where tasks and hidden tests are: `--tasks` and `--private`, else the
/// config's `private_checks` folder.
fn store(flags: &Flags) -> Result<Store, String> {
    let (here, config) = repo_config()?;
    let defaults = config.private_dirs(&here);
    let pick = |flag: &str, default: Option<&PathBuf>| -> Result<PathBuf, String> {
        flags
            .get(flag)
            .map(PathBuf::from)
            .or_else(|| default.cloned())
            .ok_or_else(|| {
                format!(
                    "--{flag} is required (or run `insi init` to set private_checks)\n\n{USAGE}"
                )
            })
    };
    Ok(Store {
        tasks: pick("tasks", defaults.as_ref().map(|d| &d.0))?,
        private: pick("private", defaults.as_ref().map(|d| &d.1))?,
    })
}

fn init_cmd(flags: &Flags) -> Result<(), String> {
    let repo = PathBuf::from(flags.get("repo").unwrap_or("."));
    let setup = flags.has("setup").then(|| flags.argv("setup"));
    let done = init::init(&repo, flags.argv("check"), setup, flags.has("force"))?;
    match &done.detected {
        Some(d) => println!("detected: {}", d.stack),
        None => println!("detected: nothing; using the commands given"),
    }
    println!("wrote {}", done.config_path.display());
    println!("  check   {}", done.config.check.join(" "));
    println!(
        "  setup   {}",
        if done.config.setup.is_empty() {
            "(none)".to_string()
        } else {
            done.config.setup.join(" ")
        }
    );
    println!(
        "  private {}  (outside the checkout)",
        done.config.private_checks.as_deref().unwrap_or_default()
    );
    if done.gitignore_changed {
        println!("added {} to .gitignore", init::WORK_IGNORE);
    }
    match done.mcp {
        init::Registration::Written => {
            println!(
                "wrote {}: agents here can ask `insi mcp` for lessons",
                init::MCP_FILE
            )
        }
        init::Registration::AlreadyThere => {
            println!("{} already registers insi", init::MCP_FILE)
        }
        init::Registration::LeftAlone => println!(
            "{} exists and was not changed; to register Insi in it:\n  {}",
            init::MCP_FILE,
            init::MCP_ADD
        ),
    }
    match done.hooks {
        init::Registration::Written => println!(
            "wrote {}: `insi hook` shows lessons before an edit and after a failed command",
            init::SETTINGS_FILE
        ),
        init::Registration::AlreadyThere => {
            println!("{} already runs insi hook", init::SETTINGS_FILE)
        }
        init::Registration::LeftAlone => println!(
            "{} exists and was not changed; to have Insi's lessons shown, merge in:\n{}",
            init::SETTINGS_FILE,
            serde_json::to_string_pretty(&init::hooks_file()).unwrap_or_default()
        ),
    }
    match done.ci {
        Some(init::Registration::Written) => println!(
            "wrote {}: pull requests from insi/ branches run `{}` before review",
            init::CI_FILE,
            done.config.test_cmd.join(" ")
        ),
        Some(init::Registration::AlreadyThere) => {
            println!("{} already checks insi/ branches", init::CI_FILE)
        }
        Some(init::Registration::LeftAlone) => println!(
            "{} exists and was not changed; Insi's pull requests are checked only by what it runs",
            init::CI_FILE
        ),
        None => println!("no CI workflow written: no stack detected to set one up for"),
    }
    println!(
        "\nnext, from this folder:\n  insi mine\n  insi validate --jobs 2\n  insi eval --fair-only"
    );
    Ok(())
}

fn issue_cmd(flags: &Flags, work: &Work) -> Result<(), String> {
    let repo = std::path::absolute(flags.get("repo").unwrap_or("."))
        .map_err(|e| format!("--repo: {e}"))?;
    let config = Config::load(&repo)?;
    let number = match flags.number("number", 0)? {
        0 => return Err(format!("--number <issue> is required\n\n{USAGE}")),
        n => n,
    };
    let worker = match flags.get("worker") {
        Some(_) => Worker::command(flags.argv("worker")),
        None => config.worker(),
    };
    worker.render("probe")?;
    // Lessons of this repository, when asked for: everything learned so far.
    let briefing = if flags.has("lessons") {
        let topic = config
            .topic
            .clone()
            .unwrap_or_else(|| repo_folder_name(&repo));
        briefing::build(
            &lessons_file(flags, work).all()?,
            &topic,
            "",
            Some("9999-12-31T23:59:59Z"),
            BRIEFING_CHARS,
        )
        .text
    } else {
        String::new()
    };
    let outcome = issue::work_issue(
        &issue::Run {
            repo: &repo,
            number,
            config: &config,
            worker: &worker,
            work_root: &work.root,
            briefing: &briefing,
            draft: !flags.has("ready"),
        },
        &issue::GhCli,
    )?;
    match outcome {
        issue::Outcome::Opened { url, branch } => {
            println!(
                "opened {url}\n  from {branch} — a draft unless --ready; review it before merging"
            )
        }
        issue::Outcome::Kept { branch, why } => {
            println!("kept on {branch}, nothing pushed: {why}")
        }
    }
    Ok(())
}

fn repo_folder_name(repo: &Path) -> String {
    repo.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "repo".to_string())
}

fn mine_cmd(flags: &Flags) -> Result<(), String> {
    let (_, config) = repo_config()?;
    // Absolute: a task's source is later cloned from inside Insi's own work
    // folder, where "." would mean that folder. (`absolute`, not
    // `canonicalize`, which on Windows gives a `\\?\` path git may not take.)
    let source = std::path::absolute(flags.get("source").unwrap_or("."))
        .map_err(|e| format!("--source: {e}"))?;
    let check = match flags.argv("check") {
        given if !given.is_empty() => given,
        _ => config.check.clone(),
    };
    if check.is_empty() {
        return Err(format!(
            "--check is required (or run `insi init` to set it)\n\n{USAGE}"
        ));
    }
    let opts = Options {
        test_markers: Options::default_markers(),
        check,
        setup: if flags.has("setup") {
            flags.argv("setup")
        } else {
            config.setup.clone()
        },
        setup_cache: config.setup_cache.clone(),
        wall_secs: flags.number("wall-secs", config.wall_secs.unwrap_or(1800))?,
        limit: flags.number("limit", 30)? as usize,
        id_prefix: flags
            .get("prefix")
            .map(String::from)
            .or(config.topic.clone())
            .unwrap_or_else(|| "task".to_string()),
    };
    let store = store(flags)?;
    let mined = mine::mine(&source, &opts)?;
    for m in &mined {
        store.save(m)?;
        println!(
            "{}  {}  ({} hidden test file(s))",
            m.task.id,
            first_line(&m.task.prompt),
            m.hidden.len()
        );
    }
    println!(
        "{} task(s) written to {}",
        mined.len(),
        store.tasks.display()
    );
    Ok(())
}

fn selected(flags: &Flags, store: &Store) -> Result<Vec<Task>, String> {
    let only = flags.list("only");
    let mut tasks: Vec<_> = store
        .load()?
        .into_iter()
        .filter(|t| only.is_empty() || only.contains(&t.id))
        .collect();
    if let Some(which) = flags.get("half") {
        tasks = half(tasks, which)?;
    }
    if tasks.is_empty() {
        return Err("no tasks selected".to_string());
    }
    Ok(tasks)
}

/// The older or newer half of `tasks` by commit date; the older half gets
/// the odd one. Lessons learned on the older half are measured on the newer,
/// which none of them came from.
fn half(mut tasks: Vec<Task>, which: &str) -> Result<Vec<Task>, String> {
    let mut dated = Vec::with_capacity(tasks.len());
    for t in tasks.drain(..) {
        let at = t
            .date
            .as_deref()
            .and_then(insi_core::lesson::epoch_secs)
            .ok_or_else(|| format!("task {} has no date; mine it again", t.id))?;
        dated.push((at, t));
    }
    dated.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.id.cmp(&b.1.id)));
    let older = dated.len().div_ceil(2);
    let newer = dated.split_off(older);
    let pick = match which {
        "older" => dated,
        "newer" => newer,
        other => return Err(format!("--half is older or newer, not \"{other}\"")),
    };
    Ok(pick.into_iter().map(|(_, t)| t).collect())
}

fn worker(flags: &Flags) -> Result<Worker, String> {
    let worker = match flags.get("worker") {
        Some(_) => Worker::command(flags.argv("worker")),
        None => Config::load(&std::env::current_dir().map_err(|e| e.to_string())?)?.worker(),
    };
    worker.render("probe")?;
    Ok(worker)
}

fn lessons_file(flags: &Flags, work: &Work) -> FileLessons {
    FileLessons {
        path: flags
            .get("lessons")
            .map(PathBuf::from)
            .unwrap_or_else(|| work.root.join("lessons.jsonl")),
    }
}

/// A task's topic: its id without the commit part (`watchman-1a2b3c4d` →
/// `watchman`), i.e. the repository it was mined from.
fn topic(task: &Task) -> String {
    task.id
        .rsplit_once('-')
        .map_or(task.id.as_str(), |(prefix, _)| prefix)
        .to_string()
}

fn learn_cmd(flags: &Flags, work: &Work, results: &Path) -> Result<(), String> {
    let store = store(flags)?;
    // `--half older` learns only from the half the measurement does not
    // use: a lesson from a measured task would brief the measured tasks
    // after it, and the comparison would be learning on its own test set.
    let tasks = selected(flags, &store)?;
    let config = repo_config()?.1;
    learn::learn(&learn::Learn {
        work,
        tasks: &tasks,
        results,
        lessons: &lessons_file(flags, work),
        worker: &worker(flags)?,
        topic: &topic,
        topic_rules: &config.topic_rules,
        jobs: jobs(flags)?,
        to_memory: if flags.has("to-memory") {
            Some(insi_memory::incord::address()?)
        } else {
            None
        },
    })
}

fn validate_cmd(flags: &Flags, work: &Work, results: &Path) -> Result<(), String> {
    let store = store(flags)?;
    // The controls never start a worker; one is named only to fill the
    // attempt.
    let worker = Worker::incode();
    let tasks = selected(flags, &store)?;
    let runs: Vec<(&insi_core::Task, Kind)> = tasks
        .iter()
        .flat_map(|t| [(t, Kind::Base), (t, Kind::Reference)])
        .collect();
    pool::each(
        &runs,
        jobs(flags)?,
        |(task, kind)| {
            run::attempt(
                work,
                &store,
                &Attempt {
                    task,
                    kind: *kind,
                    seed: 0,
                    worker: &worker,
                    preface: "",
                    briefed: vec![],
                    tags: vec![],
                    keep: flags.has("keep"),
                },
            )
        },
        |(task, kind), r| {
            store::append_result(results, &r)?;
            let verdict = match (&r.error, r.passed) {
                (Some(e), _) => format!("ERROR {}", first_line(e)),
                (None, p) => (if p { "pass" } else { "fail" }).to_string(),
            };
            println!(
                "{}  {}: {verdict}  {:.0}s",
                task.id,
                kind.name(),
                r.wall_secs
            );
            Ok(())
        },
    )?;
    let all = store::read_results(results)?;
    let fair = report::fair_tasks(&all);
    let good = fair.values().filter(|v| **v == Some(true)).count();
    println!(
        "{good} of {} task(s) are fair tests (fail at base, pass at reference)",
        fair.len()
    );
    Ok(())
}

fn eval_cmd(flags: &Flags, work: &Work, results: &Path) -> Result<(), String> {
    let store = store(flags)?;
    let worker = worker(flags)?;
    let seeds = flags.number("seeds", 1)? as u32;
    // With --lessons, each task is briefed with what earlier tasks taught.
    let lessons = if flags.has("lessons") {
        lessons_file(flags, work).all()?
    } else {
        Vec::new()
    };
    let mut tags = flags.list("tag");
    // Every attempt says whether it was briefed, so the two conditions are
    // never mixed up in a report.
    if !tags.iter().any(|t| t.starts_with("lessons=")) {
        let on = if flags.has("lessons") { "on" } else { "off" };
        tags.insert(0, format!("lessons={on}"));
    }
    let mut tasks = selected(flags, &store)?;
    if flags.has("fair-only") {
        let fair = report::fair_tasks(&store::read_results(results).unwrap_or_default());
        tasks.retain(|t| fair.get(&t.id) == Some(&Some(true)));
        if tasks.is_empty() {
            return Err("no task has passed `insi validate` yet".to_string());
        }
    }
    let runs: Vec<(u32, &insi_core::Task)> = (0..seeds)
        .flat_map(|seed| tasks.iter().map(move |t| (seed, t)))
        .collect();
    let prices = repo_config()?.1.prices;
    let mut spend = Spend {
        cap: match flags.get("max-dollars") {
            Some(v) => Some(
                v.parse::<f64>()
                    .ok()
                    .filter(|d| *d > 0.0)
                    .ok_or("--max-dollars must be a positive number")?,
            ),
            None => None,
        },
        spent: 0.0,
    };
    pool::each(
        &runs,
        jobs(flags)?,
        |(seed, task)| {
            let brief = briefing::build(
                &lessons,
                &topic(task),
                &task.id,
                task.date.as_deref(),
                BRIEFING_CHARS,
            );
            run::attempt(
                work,
                &store,
                &Attempt {
                    task,
                    kind: Kind::Worker,
                    seed: *seed,
                    worker: &worker,
                    preface: &brief.text,
                    briefed: brief.lesson_ids,
                    tags: tags.clone(),
                    keep: flags.has("keep"),
                },
            )
        },
        |(seed, task), mut r| {
            r.cost_usd = cost_of(&prices, &r);
            store::append_result(results, &r)?;
            println!(
                "{} s{seed}  {}  {:.0}s{}{}{}",
                task.id,
                match (&r.error, r.passed) {
                    (Some(_), _) => "ERROR",
                    (None, true) => "pass",
                    (None, false) => "fail",
                },
                r.wall_secs,
                r.tokens
                    .map(|t| format!("  {t} tokens"))
                    .unwrap_or_default(),
                r.cost_usd.map(|c| format!("  ${c:.2}")).unwrap_or_default(),
                r.error
                    .as_deref()
                    .map(|e| format!("  {}", first_line(e)))
                    .unwrap_or_default()
            );
            // After the result is saved: the attempt that crosses the cap is
            // kept, and no new one starts.
            spend.add(&r)
        },
    )?;
    report_cmd(results)
}

/// What an attempt cost, from the config's price for the model it reported.
fn cost_of(
    prices: &std::collections::BTreeMap<String, insi_core::Price>,
    r: &insi_core::EvalResult,
) -> Option<f64> {
    let price = prices.get(r.model.as_deref()?)?;
    Some(price.cost(r.usage.as_ref()?))
}

/// A run's spend against `--max-dollars`.
///
/// Bounds the ACTION, not the measurement: no attempt starts once the cap is
/// reached. An attempt already running is bounded by its wall time, so a run
/// can end above the cap by at most the attempts in flight — and says so.
/// An attempt whose worker ran but whose cost is not known stops the run
/// too: counting it as free is how a cap stops meaning anything.
struct Spend {
    cap: Option<f64>,
    spent: f64,
}

impl Spend {
    fn add(&mut self, r: &insi_core::EvalResult) -> Result<(), String> {
        let Some(cap) = self.cap else {
            return Ok(());
        };
        match r.cost_usd {
            Some(cost) => {
                self.spent += cost;
                if self.spent >= cap {
                    return Err(format!(
                        "spent ${:.2} of the ${cap:.2} cap (--max-dollars); no new attempt was \
                         started, and every result so far is saved",
                        self.spent
                    ));
                }
                Ok(())
            }
            None if r.phases.contains_key("worker") => Err(format!(
                "{}: the cost is not known ({}), so --max-dollars cannot be kept; stopping \
                 with ${:.2} counted",
                r.task,
                match (&r.model, &r.usage) {
                    (_, None) => "the worker reported no usage".to_string(),
                    (None, Some(_)) => "the worker reported no model".to_string(),
                    (Some(m), Some(_)) => format!("no price for \"{m}\" in .insi/config.toml"),
                },
                self.spent
            )),
            None => Ok(()),
        }
    }
}

fn report_cmd(results: &Path) -> Result<(), String> {
    let all = store::read_results(results)?;
    let by = report::by_condition(&all);
    if by.is_empty() {
        println!("no worker attempts in {}", results.display());
    }
    for (condition, t) in &by {
        println!(
            "{condition}: {}/{} passed ({:.0}%), {} repeat failure(s), {} timed out, {} error(s), {} tokens, {:.0}s, ${:.2}{}",
            t.passed,
            t.attempts - t.errors,
            t.pass_rate() * 100.0,
            t.repeat_failures,
            t.timed_out,
            t.errors,
            t.tokens,
            t.wall_secs,
            t.cost_usd,
            if t.uncosted > 0 {
                format!(" (+{} attempt(s) of unknown cost)", t.uncosted)
            } else {
                String::new()
            }
        );
    }
    Ok(())
}

/// How many attempts run at once: `--jobs`, default 1.
fn jobs(flags: &Flags) -> Result<usize, String> {
    match flags.number("jobs", 1)? {
        0 => Err("--jobs must be at least 1".to_string()),
        n => Ok(n as usize),
    }
}

fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use insi_core::{Budget, EvalResult};

    fn task(id: &str, date: Option<&str>) -> Task {
        Task {
            id: id.into(),
            source: "repo".into(),
            base: "a".into(),
            reference: "b".into(),
            date: date.map(String::from),
            prompt: "p".into(),
            hidden_tests: vec!["t.test.ts".into()],
            check: vec!["x".into()],
            setup: vec![],
            setup_cache: None,
            budget: Budget {
                wall_secs: 1,
                tokens: None,
            },
        }
    }

    fn spent(task: &str, model: Option<&str>, usage: Option<u64>, worker_ran: bool) -> EvalResult {
        let mut phases = std::collections::BTreeMap::new();
        if worker_ran {
            phases.insert("worker".to_string(), 1.0);
        }
        EvalResult {
            task: task.into(),
            seed: 0,
            kind: "worker".into(),
            passed: false,
            timed_out: false,
            wall_secs: 1.0,
            tokens: usage,
            model: model.map(String::from),
            usage: usage.map(|n| insi_core::Usage {
                input: n,
                ..Default::default()
            }),
            cost_usd: None,
            signature: None,
            error: None,
            tags: vec![],
            artifacts: None,
            briefed: vec![],
            phases,
        }
    }

    /// **THE CAP STOPS NEW ATTEMPTS, AND AN UNKNOWN COST STOPS THEM TOO.**
    /// Costs come from the model's price; crossing the cap stops the run
    /// after that attempt is saved; an attempt that ran with no usage or no
    /// price stops it rather than counting as free; one that never reached
    /// its worker costs nothing.
    #[test]
    fn spend_is_capped_and_an_unknown_cost_is_not_free() {
        let prices: std::collections::BTreeMap<String, insi_core::Price> = [(
            "m".to_string(),
            insi_core::Price {
                input: 1.0,
                ..Default::default()
            },
        )]
        .into();
        let mut a = spent("a", Some("m"), Some(400_000), true);
        a.cost_usd = cost_of(&prices, &a);
        assert!((a.cost_usd.expect("priced") - 0.4).abs() < 1e-9);

        let mut spend = Spend {
            cap: Some(1.0),
            spent: 0.0,
        };
        assert!(spend.add(&a).is_ok());
        assert!(spend.add(&a).is_ok());
        let why = spend.add(&a).expect_err("over the cap");
        assert!(why.contains("$1.20 of the $1.00"), "{why}");

        let mut spend = Spend {
            cap: Some(10.0),
            spent: 0.0,
        };
        let unpriced = spent("b", Some("other"), Some(5), true);
        assert_eq!(cost_of(&prices, &unpriced), None);
        assert!(spend
            .add(&unpriced)
            .expect_err("unknown")
            .contains("no price for \"other\""));
        let silent = spent("c", None, None, true);
        assert!(spend
            .add(&silent)
            .expect_err("unknown")
            .contains("no usage"));
        assert!(spend.add(&spent("d", None, None, false)).is_ok());

        let mut uncapped = Spend {
            cap: None,
            spent: 0.0,
        };
        assert!(uncapped.add(&silent).is_ok());
    }

    /// **LEARN ON THE PAST, MEASURE ON THE FUTURE.** The halves split by
    /// instant (not by text: offsets differ), cover every task once, and an
    /// undated task is refused rather than guessed.
    #[test]
    fn tasks_split_into_an_older_and_a_newer_half() {
        let tasks = vec![
            task("app-c", Some("2026-03-01T00:00:00Z")),
            task("app-a", Some("2026-01-01T09:00:00+05:30")),
            task("app-b", Some("2026-01-01T05:00:00Z")),
            task("app-d", Some("2026-04-01T00:00:00Z")),
            task("app-e", Some("2026-05-01T00:00:00Z")),
        ];
        let ids = |v: Vec<Task>| v.into_iter().map(|t| t.id).collect::<Vec<_>>();
        assert_eq!(
            ids(half(tasks.clone(), "older").unwrap()),
            ["app-a", "app-b", "app-c"]
        );
        assert_eq!(
            ids(half(tasks.clone(), "newer").unwrap()),
            ["app-d", "app-e"]
        );
        assert!(half(tasks, "middle").is_err());
        assert!(half(vec![task("app-x", None)], "older").is_err());
        assert_eq!(topic(&task("watchman-1a2b3c4d", None)), "watchman");
    }
}
