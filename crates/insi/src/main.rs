//! `insi` — the command line.
//!
//! ```text
//! insi mine     --source <repo> --tasks <dir> --private <dir> --check "<cmd {tests}>"
//!               [--setup "<cmd>"] [--prefix <name>] [--limit N] [--wall-secs N]
//! insi validate --tasks <dir> --private <dir> [--only id,id] [--results <file>] [--jobs N]
//! insi eval     --tasks <dir> --private <dir> [--seeds N] [--tag k=v] [--worker "<cmd {prompt}>"]
//!               [--fair-only] [--only id,id] [--results <file>] [--keep] [--jobs N]
//! insi report   [--results <file>]
//! ```
//!
//! Commands are written as one string and split on spaces; an argument with
//! a space in it does not fit that form, and a command that needs one is
//! better put in `.insi/config.toml`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use insi_core::{Config, Worker};
use insi_eval::mine::{self, Options};
use insi_eval::pool;
use insi_eval::report;
use insi_eval::run::{self, Attempt, Kind, Work};
use insi_eval::store::{self, Store};

const USAGE: &str = "usage:
  insi mine     --source <repo> --tasks <dir> --private <dir> --check \"<cmd {tests}>\"
                [--setup \"<cmd>\"] [--prefix <name>] [--limit N] [--wall-secs N]
  insi validate --tasks <dir> --private <dir> [--only id,id] [--results <file>] [--jobs N]
  insi eval     --tasks <dir> --private <dir> [--seeds N] [--tag k=v] [--worker \"<cmd {prompt}>\"]
                [--fair-only] [--only id,id] [--results <file>] [--keep] [--jobs N]
  insi report   [--results <file>]

Tasks come from a repository's history and are only read from it; every
worktree lives in Insi's own clone under --work (default .insi-work).";

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
    fn need(&self, key: &str) -> Result<&str, String> {
        self.get(key)
            .ok_or_else(|| format!("--{key} is required\n\n{USAGE}"))
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
    let work = Work::new(PathBuf::from(flags.get("work").unwrap_or(".insi-work")));
    let results = flags
        .get("results")
        .map(PathBuf::from)
        .unwrap_or_else(|| work.root.join("results.jsonl"));
    match command.as_str() {
        "mine" => mine_cmd(&flags),
        "validate" => validate_cmd(&flags, &work, &results),
        "eval" => eval_cmd(&flags, &work, &results),
        "report" => report_cmd(&results),
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

fn store(flags: &Flags) -> Result<Store, String> {
    Ok(Store {
        tasks: PathBuf::from(flags.need("tasks")?),
        private: PathBuf::from(flags.need("private")?),
    })
}

fn mine_cmd(flags: &Flags) -> Result<(), String> {
    let source = PathBuf::from(flags.need("source")?);
    let check = flags.argv("check");
    if check.is_empty() {
        return Err(format!("--check is required\n\n{USAGE}"));
    }
    let opts = Options {
        test_markers: Options::default_markers(),
        check,
        setup: flags.argv("setup"),
        wall_secs: flags.number("wall-secs", 900)?,
        limit: flags.number("limit", 30)? as usize,
        id_prefix: flags.get("prefix").unwrap_or("task").to_string(),
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

fn selected(flags: &Flags, store: &Store) -> Result<Vec<insi_core::Task>, String> {
    let only = flags.list("only");
    let tasks: Vec<_> = store
        .load()?
        .into_iter()
        .filter(|t| only.is_empty() || only.contains(&t.id))
        .collect();
    if tasks.is_empty() {
        return Err("no tasks selected".to_string());
    }
    Ok(tasks)
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
    let worker = match flags.get("worker") {
        Some(_) => Worker {
            command: flags.argv("worker"),
        },
        None => Config::load(&std::env::current_dir().map_err(|e| e.to_string())?)?.worker(),
    };
    worker.render("probe")?;
    let seeds = flags.number("seeds", 1)? as u32;
    let tags = flags.list("tag");
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
    pool::each(
        &runs,
        jobs(flags)?,
        |(seed, task)| {
            run::attempt(
                work,
                &store,
                &Attempt {
                    task,
                    kind: Kind::Worker,
                    seed: *seed,
                    worker: &worker,
                    preface: "",
                    tags: tags.clone(),
                    keep: flags.has("keep"),
                },
            )
        },
        |(seed, task), r| {
            store::append_result(results, &r)?;
            println!(
                "{} s{seed}  {}  {:.0}s{}{}",
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
                r.error
                    .as_deref()
                    .map(|e| format!("  {}", first_line(e)))
                    .unwrap_or_default()
            );
            Ok(())
        },
    )?;
    report_cmd(results)
}

fn report_cmd(results: &Path) -> Result<(), String> {
    let all = store::read_results(results)?;
    let by = report::by_condition(&all);
    if by.is_empty() {
        println!("no worker attempts in {}", results.display());
    }
    for (condition, t) in &by {
        println!(
            "{condition}: {}/{} passed ({:.0}%), {} repeat failure(s), {} timed out, {} error(s), {} tokens, {:.0}s",
            t.passed,
            t.attempts - t.errors,
            t.pass_rate() * 100.0,
            t.repeat_failures,
            t.timed_out,
            t.errors,
            t.tokens,
            t.wall_secs
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
