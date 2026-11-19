// Copyright (c) 2026-2027 Incord Ai.
// https://incord.ai
//
// SPDX-License-Identifier: BUSL-1.1

//! `insi ci`: the project's own GitHub workflow steps, run on this machine (the owner, 2026-11-18: "test every app
//! with Insi first, even Insi, as it has CI tests"; an agent runs it before any push, so a push never finds out
//! from GitHub what its own machine could have said).
//!
//! It reads `.github/workflows/*.yml` (`ci_yaml.rs`), keeps the workflows that run on a push or a pull request, and
//! for every job that runs on THIS operating system (its `runs-on`, with `${{ matrix.os }}` expanded) runs each
//! `run:` step in order, in the repository, with the step's shell and environment. It reports each step as
//! passed, FAILED, skipped (with why) or unavailable (a tool the step needs is not installed here), and exits with
//! an error when a step failed.
//!
//! What it does not do, and says so for each step it passes over: it runs no `uses:` action (they are GitHub's:
//! `actions/checkout` is the repository already in front of it; `setup-*` and `rust-toolchain` are the tools
//! installed here; a scanner action such as `cargo-deny-action` is not run, so run its tool yourself); it runs no job
//! with `container:` or `services:`; it never runs a step that would publish or push (`git push`, `cargo publish`,
//! `npm publish`, `docker push`, `gh release`, ...); it does not run a step whose `${{ ... }}` it cannot resolve
//! (a secret, another step's output); and a matrix over versions runs once, with the versions installed here. A
//! workflow only run by a tag, a release, a schedule or by hand is not a check and is left out unless named with
//! `--workflow`.
//!
//! `insi ci --install-hook` writes `.git/hooks/pre-push` (the repository's hooks folder), which runs
//! `insi ci --quiet` before every push and stops the push when a step failed.
//!
//! Called by `main.rs` (`insi ci`). Calls `ci_yaml.rs` to read the workflows and `insi_eval::process` to run a step
//! with a deadline and its whole process tree stopped. The siblings that also run a repository's own commands are
//! `benchmark.rs` (the commands `insi benchmark --detect` suggests) and `insi_eval::run` (a task's check command);
//! neither reads a workflow.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use serde_json::{json, Value};

use crate::ci_yaml::{self, Yaml};

/// One step's, job's or run's outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Passed,
    Failed,
    Skipped,
    /// A tool the step needs is not installed here: neither a pass nor a failure of the project.
    Unavailable,
    /// Only for `--list`: the step would run.
    Planned,
}

impl Status {
    fn word(self) -> &'static str {
        match self {
            Status::Passed => "pass",
            Status::Failed => "FAIL",
            Status::Skipped => "skip",
            Status::Unavailable => "unavailable",
            Status::Planned => "would run",
        }
    }
}

#[derive(Debug, Clone)]
pub struct StepResult {
    pub name: String,
    pub status: Status,
    pub reason: String,
    pub seconds: f64,
    /// The end of the output of a failed or unavailable step.
    pub tail: String,
}

#[derive(Debug, Clone)]
pub struct JobResult {
    pub workflow: String,
    pub id: String,
    pub label: String,
    pub status: Status,
    pub reason: String,
    pub steps: Vec<StepResult>,
}

#[derive(Debug, Clone, Default)]
pub struct Report {
    pub jobs: Vec<JobResult>,
    /// Workflows that were left out, or could not be read, with why.
    pub notes: Vec<String>,
}

impl Report {
    fn count(&self, status: Status) -> usize {
        self.jobs
            .iter()
            .flat_map(|j| &j.steps)
            .filter(|s| s.status == status)
            .count()
            + self
                .jobs
                .iter()
                .filter(|j| j.steps.is_empty() && j.status == status)
                .count()
    }

    pub fn failed(&self) -> usize {
        self.count(Status::Failed)
    }

    pub fn unavailable(&self) -> usize {
        self.count(Status::Unavailable)
    }
}

/// What `insi ci` was asked.
#[derive(Debug, Clone)]
pub struct Options {
    pub repo: PathBuf,
    /// A workflow file, by name (`ci.yml` or `ci`): it is run even when it is not a check.
    pub workflow: Option<String>,
    pub job: Option<String>,
    /// Plan only: say what would run.
    pub list: bool,
    pub timeout: Duration,
}

// ── The command ──────────────────────────────────────────────────────────────────────────────────────────────────

/// `insi ci [--repo <dir>] [--workflow <file>] [--job <id>] [--list] [--json] [--quiet] [--strict]
/// [--timeout-secs N]` and `insi ci --install-hook [--repo <dir>] [--force]`. Called by `main.rs`.
pub fn command(flags: &crate::Flags) -> Result<(), String> {
    let repo = std::fs::canonicalize(flags.get("repo").unwrap_or("."))
        .map_err(|e| format!("--repo: {e}"))?;
    if flags.has("install-hook") {
        let path = install_hook(&repo, flags.has("force"))?;
        println!("insi ci: pre-push hook installed at {}", path.display());
        return Ok(());
    }
    let options = Options {
        repo,
        workflow: flags.get("workflow").map(str::to_string),
        job: flags.get("job").map(str::to_string),
        list: flags.has("list"),
        timeout: Duration::from_secs(flags.number("timeout-secs", 1800)?),
    };
    let quiet = flags.has("quiet");
    let report = run(&options, &mut |line| {
        if !quiet && !options.list {
            eprintln!("{line}");
        }
    });
    if flags.has("json") {
        println!(
            "{}",
            serde_json::to_string_pretty(&to_json(&report)).map_err(|e| e.to_string())?
        );
    } else {
        print!("{}", render(&report, quiet && !options.list));
    }
    if report.failed() > 0 {
        return Err(format!("ci: {} step(s) failed", report.failed()));
    }
    if flags.has("strict") && report.unavailable() > 0 {
        return Err(format!(
            "ci: {} step(s) could not run here (a tool is not installed)",
            report.unavailable()
        ));
    }
    Ok(())
}

// ── The workflows ────────────────────────────────────────────────────────────────────────────────────────────────

/// This machine's operating system as `runs-on` names it: `linux`, `macos` or `windows`.
fn this_os() -> &'static str {
    match std::env::consts::OS {
        "linux" => "linux",
        "macos" => "macos",
        "windows" => "windows",
        other => other,
    }
}

/// The operating system a `runs-on` label stands for, or `None` for one this cannot tell (a self-hosted runner).
fn label_os(label: &str) -> Option<&'static str> {
    let l = label.trim().to_ascii_lowercase();
    if l.starts_with("ubuntu") || l == "linux" || l.starts_with("linux-") {
        Some("linux")
    } else if l.starts_with("macos") {
        Some("macos")
    } else if l.starts_with("windows") {
        Some("windows")
    } else {
        None
    }
}

fn runner_os_name(os: &str) -> &'static str {
    match os {
        "linux" => "Linux",
        "macos" => "macOS",
        "windows" => "Windows",
        _ => "Unknown",
    }
}

/// Whether a workflow is a check: it runs on a push (to a branch, not only to a tag) or on a pull request.
fn is_check(doc: &Yaml) -> bool {
    let Some(on) = doc.get("on") else {
        return false;
    };
    let push_checks = |v: &Yaml| match v {
        Yaml::Map(pairs) => {
            pairs.is_empty() || pairs.iter().any(|(k, _)| k != "tags" && k != "tags-ignore")
        }
        _ => true,
    };
    match on {
        Yaml::Str(event) => event == "push" || event == "pull_request",
        Yaml::Seq(events) => events
            .iter()
            .filter_map(Yaml::as_str)
            .any(|e| e == "push" || e == "pull_request"),
        Yaml::Map(pairs) => pairs
            .iter()
            .any(|(event, v)| (event == "push" && push_checks(v)) || event == "pull_request"),
    }
}

struct Workflow {
    file: String,
    doc: Yaml,
}

/// The workflows of `repo`, sorted by file name; one that cannot be read becomes a note.
fn load(repo: &Path, notes: &mut Vec<String>) -> Vec<Workflow> {
    let dir = repo.join(".github").join("workflows");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "yml" || e == "yaml"))
        .collect();
    files.sort();
    let mut out = Vec::new();
    for path in files {
        let file = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        match std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|t| ci_yaml::parse(&t))
        {
            Ok(doc) => out.push(Workflow { file, doc }),
            Err(why) => notes.push(format!("{file}: could not be read: {why}")),
        }
    }
    out
}

// ── Expressions ──────────────────────────────────────────────────────────────────────────────────────────────────

/// What `${{ ... }}` can be resolved from here.
struct Context<'a> {
    matrix: &'a BTreeMap<String, String>,
    env: &'a BTreeMap<String, String>,
    os: &'static str,
    repo: &'a Path,
}

/// `text` split at `sep` where `sep` is outside single quotes.
fn split_outside_quotes<'a>(text: &'a str, sep: &str) -> Vec<&'a str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut skip_until = 0;
    let mut quoted = false;
    for (i, c) in text.char_indices() {
        if i < skip_until {
            continue;
        }
        if c == '\'' {
            quoted = !quoted;
        }
        if !quoted && text[i..].starts_with(sep) {
            parts.push(&text[start..i]);
            skip_until = i + sep.len();
            start = skip_until;
        }
    }
    parts.push(&text[start..]);
    parts
}

/// One operand: a quoted string, a number or `true`/`false`, or a context value; `None` when it cannot be resolved.
fn operand(text: &str, ctx: &Context<'_>) -> Option<String> {
    let t = text.trim();
    if let Some(inner) = t.strip_prefix('\'').and_then(|r| r.strip_suffix('\'')) {
        return Some(inner.replace("''", "'"));
    }
    if t == "true" || t == "false" || t.parse::<f64>().is_ok() {
        return Some(t.to_string());
    }
    // A matrix or env key the workflow does not set is empty, as GitHub evaluates it (null), so
    // `matrix.missing || 'default'` is 'default'. What this machine cannot know (a secret, a step's output) stays None.
    if let Some(key) = t.strip_prefix("matrix.") {
        return Some(ctx.matrix.get(key).cloned().unwrap_or_default());
    }
    if let Some(key) = t.strip_prefix("env.") {
        return Some(ctx.env.get(key).cloned().unwrap_or_default());
    }
    match t {
        "runner.os" => Some(runner_os_name(ctx.os).to_string()),
        "runner.arch" => Some(
            if std::env::consts::ARCH == "aarch64" {
                "ARM64"
            } else {
                "X64"
            }
            .to_string(),
        ),
        "github.workspace" => Some(ctx.repo.to_string_lossy().into_owned()),
        "github.event_name" => Some("push".to_string()),
        _ => None,
    }
}

/// The value of one `${{ expr }}`: an operand, or `a || b` (the first that is set).
fn value_of(expr: &str, ctx: &Context<'_>) -> Option<String> {
    let alternatives = split_outside_quotes(expr, "||");
    if alternatives.len() == 1 {
        return operand(expr, ctx);
    }
    for alt in &alternatives[..alternatives.len() - 1] {
        let v = operand(alt, ctx)?;
        if !v.is_empty() && v != "false" {
            return Some(v);
        }
    }
    operand(alternatives[alternatives.len() - 1], ctx)
}

/// `text` with every `${{ expr }}` replaced by its value, or the expression that cannot be resolved here.
fn expand(text: &str, ctx: &Context<'_>) -> Result<String, String> {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("${{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 3..];
        let Some(end) = after.find("}}") else {
            return Err(format!(
                "an expression that is not closed: {}",
                &rest[start..]
            ));
        };
        let expr = after[..end].trim();
        match value_of(expr, ctx) {
            Some(v) => out.push_str(&v),
            None => {
                return Err(format!(
                    "${{{{ {expr} }}}} cannot be resolved on this machine"
                ))
            }
        }
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    Ok(out)
}

/// The value of a condition (`if:`) given whether an earlier step failed: `None` when it cannot be decided here.
fn condition(text: &str, ctx: &Context<'_>, failed_before: bool) -> Option<bool> {
    let t = text.trim();
    let t = t
        .strip_prefix("${{")
        .and_then(|r| r.strip_suffix("}}"))
        .map_or(t, str::trim);
    let mut any = false;
    for conjunction in split_outside_quotes(t, "||") {
        let mut all = true;
        for atom in split_outside_quotes(conjunction, "&&") {
            all &= atom_value(atom.trim(), ctx, failed_before)?;
        }
        any |= all;
    }
    Some(any)
}

fn atom_value(atom: &str, ctx: &Context<'_>, failed_before: bool) -> Option<bool> {
    if let Some(inner) = atom.strip_prefix('!') {
        return atom_value(inner.trim(), ctx, failed_before).map(|v| !v);
    }
    match atom {
        "always()" => return Some(true),
        "success()" => return Some(!failed_before),
        "failure()" => return Some(failed_before),
        "cancelled()" => return Some(false),
        _ => {}
    }
    for (op, equal) in [("==", true), ("!=", false)] {
        let sides = split_outside_quotes(atom, op);
        if sides.len() == 2 {
            let left = operand(sides[0], ctx)?;
            let right = operand(sides[1], ctx)?;
            return Some((left.eq_ignore_ascii_case(&right)) == equal);
        }
    }
    let v = operand(atom, ctx)?;
    Some(!v.is_empty() && v != "false" && v != "0")
}

// ── Matrices and jobs ────────────────────────────────────────────────────────────────────────────────────────────

/// The combinations of a job's `strategy.matrix` (one empty combination when it has none), or why it cannot be read.
fn matrix_combos(job: &Yaml) -> Result<Vec<BTreeMap<String, String>>, String> {
    let Some(matrix) = job.get("strategy").and_then(|s| s.get("matrix")) else {
        return Ok(vec![BTreeMap::new()]);
    };
    let Yaml::Map(pairs) = matrix else {
        return Err("a matrix given by an expression cannot be expanded here".into());
    };
    let mut combos: Vec<BTreeMap<String, String>> = vec![BTreeMap::new()];
    let mut base_keys = Vec::new();
    for (key, value) in pairs
        .iter()
        .filter(|(k, _)| k != "include" && k != "exclude")
    {
        let Yaml::Seq(values) = value else {
            return Err(format!("matrix.{key} is not a list"));
        };
        base_keys.push(key.clone());
        let mut next = Vec::new();
        for combo in &combos {
            for v in values {
                let Some(text) = v.as_str() else {
                    return Err(format!(
                        "matrix.{key} holds something that is not a plain value"
                    ));
                };
                let mut c = combo.clone();
                c.insert(key.clone(), text.to_string());
                next.push(c);
            }
        }
        combos = next;
    }
    if base_keys.is_empty() {
        combos.clear();
    }
    let entries = |name: &str| -> Vec<BTreeMap<String, String>> {
        matrix
            .get(name)
            .and_then(Yaml::as_seq)
            .into_iter()
            .flatten()
            .filter_map(Yaml::as_map)
            .map(|p| {
                p.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .collect()
    };
    let matches = |combo: &BTreeMap<String, String>, entry: &BTreeMap<String, String>| {
        entry
            .iter()
            .all(|(k, v)| combo.get(k).is_some_and(|c| c == v))
    };
    for excluded in entries("exclude") {
        combos.retain(|c| !matches(c, &excluded));
    }
    for included in entries("include") {
        let mut joined = false;
        for combo in combos.iter_mut() {
            let original_match = base_keys
                .iter()
                .all(|k| included.get(k).is_none_or(|v| combo.get(k) == Some(v)));
            if original_match && !base_keys.is_empty() {
                for (k, v) in &included {
                    combo.entry(k.clone()).or_insert_with(|| v.clone());
                }
                joined = true;
            }
        }
        if !joined {
            combos.push(included);
        }
    }
    if combos.is_empty() {
        combos.push(BTreeMap::new());
    }
    Ok(combos)
}

/// `needs` of a job: its dependencies' ids.
fn needs_of(job: &Yaml) -> Vec<String> {
    match job.get("needs") {
        Some(Yaml::Str(one)) => vec![one.clone()],
        Some(Yaml::Seq(items)) => items
            .iter()
            .filter_map(Yaml::as_str)
            .map(str::to_string)
            .collect(),
        _ => Vec::new(),
    }
}

/// A script's reason not to be run on a developer's machine: it would publish or push.
fn would_publish(script: &str) -> Option<&'static str> {
    const PUBLISHING: [&str; 9] = [
        "git push",
        "cargo publish",
        "npm publish",
        "yarn publish",
        "pnpm publish",
        "twine upload",
        "docker push",
        "gh release",
        "vsce publish",
    ];
    for line in script.lines() {
        let l = line.trim().to_ascii_lowercase();
        if l.starts_with('#') {
            continue;
        }
        if let Some(found) = PUBLISHING.iter().find(|p| l.contains(*p)) {
            return Some(found);
        }
    }
    None
}

/// A script's reason not to be run on a developer's machine: it would change the machine's own setup, which a CI
/// runner throws away and a developer keeps (a `git config --global user.email` step would sign every later commit
/// of theirs with the CI's address).
fn would_change_machine(script: &str) -> Option<&'static str> {
    const MACHINE: [&str; 13] = [
        "git config --global",
        "git config --system",
        "sudo ",
        "apt-get install",
        "apt install",
        "brew install",
        "choco install",
        "winget install",
        "npm install -g",
        "npm install --global",
        "npm i -g",
        "rustup default",
        "corepack enable",
    ];
    for line in script.lines() {
        let l = line.trim().to_ascii_lowercase();
        if l.starts_with('#') {
            continue;
        }
        if let Some(found) = MACHINE.iter().find(|p| l.contains(*p)) {
            return Some(found.trim_end());
        }
    }
    None
}

/// Why an action is not run, in words that tell what to do instead.
fn action_reason(uses: &str) -> String {
    let name = uses.split('@').next().unwrap_or(uses).to_ascii_lowercase();
    if name == "actions/checkout" {
        "the repository is already here".into()
    } else if name.starts_with("actions/setup-")
        || name.starts_with("dtolnay/rust-toolchain")
        || name.contains("install-action")
    {
        "uses the tools installed on this machine".into()
    } else if name.starts_with("actions/upload-artifact")
        || name.starts_with("actions/download-artifact")
        || name.starts_with("actions/cache")
        || name.contains("rust-cache")
    {
        "GitHub-only (artifacts and caches)".into()
    } else {
        format!("an action runs only on GitHub ({uses}): run its tool yourself if it is a check")
    }
}

/// The shell command line (before the script file) and the script file's extension.
fn shell_for(
    shell: Option<&str>,
    runner_os: &str,
) -> Result<(Vec<&'static str>, &'static str), String> {
    // GitHub's default on a Windows runner is PowerShell (7, else the one Windows has); elsewhere bash.
    let name = match shell {
        Some(s) => s.trim().to_string(),
        None if runner_os == "windows" => (if on_path("pwsh") {
            "pwsh"
        } else {
            "powershell"
        })
        .to_string(),
        None => "bash".to_string(),
    };
    match name.as_str() {
        "bash" => Ok((
            vec!["bash", "--noprofile", "--norc", "-eo", "pipefail"],
            "sh",
        )),
        "sh" => Ok((vec!["sh", "-e"], "sh")),
        "pwsh" => Ok((
            vec!["pwsh", "-NoLogo", "-NoProfile", "-NonInteractive", "-File"],
            "ps1",
        )),
        "powershell" => Ok((
            vec![
                "powershell",
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ],
            "ps1",
        )),
        "python" => Ok((vec![if cfg!(windows) { "python" } else { "python3" }], "py")),
        other => Err(format!("the shell `{other}` is not supported by insi ci")),
    }
}

/// Whether a program of this name is in a folder of `PATH` (with `.exe` on Windows).
fn on_path(program: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path)
        .any(|dir| dir.join(program).is_file() || dir.join(format!("{program}.exe")).is_file())
}

/// `dir` (a step's `working-directory`) inside `repo`, or why not.
fn confined(repo: &Path, dir: &str) -> Result<PathBuf, String> {
    let relative = Path::new(dir);
    if relative.is_absolute()
        || relative.components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::Prefix(_) | Component::RootDir
            )
        })
    {
        return Err(format!("working-directory `{dir}` leaves the repository"));
    }
    Ok(repo.join(relative))
}

fn tail_of(text: &str, lines: usize) -> String {
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

fn missing_tool(output: &str) -> Option<String> {
    output
        .lines()
        .find(|l| {
            let l = l.to_ascii_lowercase();
            l.contains("command not found")
                || l.contains("is not recognized as")
                || (l.contains("no such file or directory") && l.contains("exec"))
        })
        .map(|l| l.trim().to_string())
}

/// One step's script run for real. Returns the status, why, the seconds and the output's tail.
fn execute(
    script: &str,
    shell: &(Vec<&'static str>, &'static str),
    dir: &Path,
    env: &BTreeMap<String, String>,
    timeout: Duration,
    scratch: &Path,
    n: usize,
) -> (Status, String, f64, String) {
    let file = scratch.join(format!("step-{n}.{}", shell.1));
    let body = if shell.1 == "ps1" {
        format!("$ErrorActionPreference = 'stop'\n{script}\n")
    } else {
        format!("{script}\n")
    };
    if let Err(e) = std::fs::create_dir_all(scratch).and_then(|()| std::fs::write(&file, body)) {
        return (
            Status::Failed,
            format!("could not write the script: {e}"),
            0.0,
            String::new(),
        );
    }
    let mut argv: Vec<String> = shell.0.iter().map(|s| (*s).to_string()).collect();
    argv.push(file.to_string_lossy().into_owned());
    let vars: Vec<(&str, &OsStr)> = env
        .iter()
        .map(|(k, v)| (k.as_str(), OsStr::new(v.as_str())))
        .collect();
    let outcome = insi_eval::process::run_with(&argv, dir, timeout, &vars);
    let _ = std::fs::remove_file(&file);
    match outcome {
        Err(why) if why.contains("could not start") => (
            Status::Unavailable,
            format!("the shell is not installed here: {}", shell.0[0]),
            0.0,
            String::new(),
        ),
        Err(why) => (Status::Failed, why, 0.0, String::new()),
        Ok(out) => {
            let seconds = out.wall.as_secs_f64();
            let tail = tail_of(&out.tail(), 40);
            if out.success() {
                (Status::Passed, String::new(), seconds, String::new())
            } else if out.timed_out {
                (
                    Status::Failed,
                    format!("timed out after {} s", timeout.as_secs()),
                    seconds,
                    tail,
                )
            } else if let Some(line) = missing_tool(&out.tail())
                .filter(|_| out.code == Some(127) || out.code == Some(1) || out.code.is_none())
            {
                (
                    Status::Unavailable,
                    format!("a command it runs is not installed here: {line}"),
                    seconds,
                    tail,
                )
            } else {
                (
                    Status::Failed,
                    format!(
                        "exit code {}",
                        out.code.map_or("none".to_string(), |c| c.to_string())
                    ),
                    seconds,
                    tail,
                )
            }
        }
    }
}

struct Run<'a> {
    options: &'a Options,
    scratch: PathBuf,
    step_counter: usize,
    say: &'a mut dyn FnMut(&str),
}

/// One job on this machine, for its first matching matrix entry.
fn run_job(
    run: &mut Run<'_>,
    workflow: &Workflow,
    id: &str,
    job: &Yaml,
    combo: &BTreeMap<String, String>,
    label: &str,
    os: &'static str,
) -> JobResult {
    let mut result = JobResult {
        workflow: workflow.file.clone(),
        id: id.to_string(),
        label: label.to_string(),
        status: Status::Skipped,
        reason: String::new(),
        steps: Vec::new(),
    };
    if job.get("container").is_some() || job.get("services").is_some() {
        result.reason =
            "the job runs in a container or with services: it needs a container runtime".into();
        return result;
    }
    if job.get("uses").is_some() {
        result.reason = "a reusable workflow runs only on GitHub".into();
        return result;
    }
    // The workflow's `env:`, then the job's, each value resolved from what is set before it; one that needs a secret
    // or another step's output is left out.
    let mut env: BTreeMap<String, String> = BTreeMap::new();
    for layer in [workflow.doc.get("env"), job.get("env")]
        .into_iter()
        .flatten()
    {
        for (k, v) in layer.as_map().unwrap_or_default() {
            let Some(text) = v.as_str() else { continue };
            let value = expand(
                text,
                &Context {
                    matrix: combo,
                    env: &env,
                    os,
                    repo: &run.options.repo,
                },
            );
            if let Ok(value) = value {
                env.insert(k.clone(), value);
            }
        }
    }
    if let Some(cond) = job.get("if").and_then(Yaml::as_str) {
        let ctx = Context {
            matrix: combo,
            env: &env,
            os,
            repo: &run.options.repo,
        };
        match condition(cond, &ctx, false) {
            Some(true) => {}
            Some(false) => {
                result.reason = format!("its condition is false here: {cond}");
                return result;
            }
            None => {
                result.reason = format!("its condition cannot be decided here: {cond}");
                return result;
            }
        }
    }
    let default_shell = job
        .get("defaults")
        .and_then(|d| d.get("run"))
        .and_then(|r| r.get("shell"))
        .and_then(Yaml::as_str)
        .map(str::to_string);
    let default_dir = job
        .get("defaults")
        .and_then(|d| d.get("run"))
        .and_then(|r| r.get("working-directory"))
        .and_then(Yaml::as_str)
        .map(str::to_string);
    let steps = job.get("steps").and_then(Yaml::as_seq).unwrap_or_default();
    let mut failed_before = false;
    for (index, step) in steps.iter().enumerate() {
        let ctx = Context {
            matrix: combo,
            env: &env,
            os,
            repo: &run.options.repo,
        };
        let name = step_name(step, index);
        let mut record = StepResult {
            name: name.clone(),
            status: Status::Skipped,
            reason: String::new(),
            seconds: 0.0,
            tail: String::new(),
        };
        if let Some(uses) = step.get("uses").and_then(Yaml::as_str) {
            record.reason = action_reason(uses);
            result.steps.push(record);
            continue;
        }
        let Some(script) = step.get("run").and_then(Yaml::as_str) else {
            record.reason = "it has no run: script".into();
            result.steps.push(record);
            continue;
        };
        let wanted = match step.get("if").and_then(Yaml::as_str) {
            Some(cond) => condition(cond, &ctx, failed_before),
            None => Some(!failed_before),
        };
        match wanted {
            Some(true) => {}
            Some(false) => {
                record.reason = if failed_before && step.get("if").is_none() {
                    "an earlier step failed".into()
                } else {
                    "its condition is false here".into()
                };
                result.steps.push(record);
                continue;
            }
            None => {
                record.reason = format!(
                    "its condition cannot be decided here: {}",
                    step.get("if").and_then(Yaml::as_str).unwrap_or("")
                );
                result.steps.push(record);
                continue;
            }
        }
        if let Some(what) = would_publish(script) {
            record.reason = format!("it would publish or push (`{what}`): never run from here");
            result.steps.push(record);
            continue;
        }
        if let Some(what) = would_change_machine(script) {
            record.reason = format!("it would change this machine's own setup (`{what}`): only a CI runner, which is thrown away, runs it");
            result.steps.push(record);
            continue;
        }
        let script = match expand(script, &ctx) {
            Ok(s) => s,
            Err(why) => {
                record.reason = why;
                result.steps.push(record);
                continue;
            }
        };
        let mut step_env = env.clone();
        let mut env_error = None;
        for (k, v) in step.get("env").and_then(Yaml::as_map).unwrap_or_default() {
            if let Some(text) = v.as_str() {
                match expand(text, &ctx) {
                    Ok(value) => {
                        step_env.insert(k.clone(), value);
                    }
                    Err(why) => env_error = Some(why),
                }
            }
        }
        if let Some(why) = env_error {
            record.reason = format!("its env: {why}");
            result.steps.push(record);
            continue;
        }
        step_env.insert("CI".into(), "true".into());
        step_env.insert("INSI_CI".into(), "1".into());
        step_env.insert("RUNNER_OS".into(), runner_os_name(os).into());
        step_env.insert(
            "GITHUB_WORKSPACE".into(),
            run.options.repo.to_string_lossy().into_owned(),
        );
        let shell_name = step
            .get("shell")
            .and_then(Yaml::as_str)
            .map(str::to_string)
            .or_else(|| default_shell.clone());
        let shell = match shell_for(shell_name.as_deref(), os) {
            Ok(s) => s,
            Err(why) => {
                record.reason = why;
                result.steps.push(record);
                continue;
            }
        };
        let dir_name = step
            .get("working-directory")
            .and_then(Yaml::as_str)
            .map(str::to_string)
            .or_else(|| default_dir.clone());
        let dir = match dir_name
            .as_deref()
            .map_or(Ok(run.options.repo.clone()), |d| {
                confined(&run.options.repo, d)
            }) {
            Ok(d) => d,
            Err(why) => {
                record.reason = why;
                result.steps.push(record);
                continue;
            }
        };
        if run.options.list {
            record.status = Status::Planned;
            result.steps.push(record);
            continue;
        }
        (run.say)(&format!("  running {}: {name}", workflow.file));
        run.step_counter += 1;
        let (status, reason, seconds, tail) = execute(
            &script,
            &shell,
            &dir,
            &step_env,
            run.options.timeout,
            &run.scratch,
            run.step_counter,
        );
        record.status = status;
        record.reason = reason;
        record.seconds = seconds;
        record.tail = tail;
        if status == Status::Failed
            && step.get("continue-on-error").and_then(Yaml::as_str) == Some("true")
        {
            record.reason = format!("{} (continue-on-error: the job goes on)", record.reason);
        } else if status == Status::Failed {
            failed_before = true;
        }
        result.steps.push(record);
    }
    result.status = job_status(&result.steps);
    result
}

fn step_name(step: &Yaml, index: usize) -> String {
    let given = step.get("name").and_then(Yaml::as_str).map(str::to_string);
    given
        .or_else(|| {
            step.get("run")
                .and_then(Yaml::as_str)
                .and_then(|r| r.lines().find(|l| !l.trim().is_empty()))
                .map(|l| l.trim().chars().take(100).collect())
        })
        .or_else(|| step.get("uses").and_then(Yaml::as_str).map(str::to_string))
        .unwrap_or_else(|| format!("step {}", index + 1))
}

fn job_status(steps: &[StepResult]) -> Status {
    let ignored = |s: &StepResult| s.reason.contains("continue-on-error");
    if steps
        .iter()
        .any(|s| s.status == Status::Failed && !ignored(s))
    {
        Status::Failed
    } else if steps.iter().any(|s| s.status == Status::Passed) {
        Status::Passed
    } else if steps.iter().any(|s| s.status == Status::Unavailable) {
        Status::Unavailable
    } else if steps.iter().any(|s| s.status == Status::Planned) {
        Status::Planned
    } else {
        Status::Skipped
    }
}

/// Runs (or, with `list`, plans) the project's workflows on this machine. `say` gets a line as each step starts.
/// Called by [`command`].
pub fn run(options: &Options, say: &mut dyn FnMut(&str)) -> Report {
    let mut report = Report::default();
    let workflows = load(&options.repo, &mut report.notes);
    if workflows.is_empty() && report.notes.is_empty() {
        report
            .notes
            .push("no workflows found in .github/workflows".into());
    }
    // A folder of this run's own: two runs in one process (tests run side by side, or a host that runs two repos)
    // never share it, so one's cleanup never removes the other's step scripts.
    static RUNS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nth = RUNS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let scratch = std::env::temp_dir().join(format!("insi-ci-{}-{nth}", std::process::id()));
    let mut run = Run {
        options,
        scratch: scratch.clone(),
        step_counter: 0,
        say,
    };
    let os = this_os();
    for workflow in &workflows {
        let named = options.workflow.as_deref().is_some_and(|w| {
            w == workflow.file
                || Path::new(&workflow.file)
                    .file_stem()
                    .is_some_and(|s| s.to_string_lossy() == w)
        });
        if options.workflow.is_some() && !named {
            continue;
        }
        if options.workflow.is_none() && !is_check(&workflow.doc) {
            report.notes.push(format!("{}: left out: it does not run on a push or a pull request (name it with --workflow to run it)", workflow.file));
            continue;
        }
        run_workflow(&mut run, workflow, os, &mut report);
    }
    let _ = std::fs::remove_dir_all(&scratch);
    report
}

fn run_workflow(run: &mut Run<'_>, workflow: &Workflow, os: &'static str, report: &mut Report) {
    let Some(jobs) = workflow.doc.get("jobs").and_then(Yaml::as_map) else {
        report.notes.push(format!("{}: has no jobs", workflow.file));
        return;
    };
    let mut done: BTreeMap<String, Status> = BTreeMap::new();
    let mut pending: Vec<&(String, Yaml)> = jobs
        .iter()
        .filter(|(id, _)| run.options.job.as_deref().is_none_or(|j| j == id))
        .collect();
    while !pending.is_empty() {
        let ready = pending.iter().position(|(_, job)| {
            needs_of(job).iter().all(|n| {
                done.contains_key(n)
                    || !jobs.iter().any(|(id, _)| id == n)
                    || run.options.job.is_some()
            })
        });
        let Some(position) = ready else {
            for (id, _) in pending.drain(..) {
                report.jobs.push(JobResult {
                    workflow: workflow.file.clone(),
                    id: id.clone(),
                    label: String::new(),
                    status: Status::Skipped,
                    reason: "its needs never finish".into(),
                    steps: Vec::new(),
                });
            }
            break;
        };
        let (id, job) = pending.remove(position);
        let finished = run_one(run, workflow, id, job, os, &done);
        done.insert(id.clone(), finished.status);
        report.jobs.push(finished);
    }
}

/// One job: its matrix entries on this OS (the first runs), or one skipped record when none is.
fn run_one(
    run: &mut Run<'_>,
    workflow: &Workflow,
    id: &str,
    job: &Yaml,
    os: &'static str,
    done: &BTreeMap<String, Status>,
) -> JobResult {
    let skipped = |label: &str, reason: String| JobResult {
        workflow: workflow.file.clone(),
        id: id.to_string(),
        label: label.to_string(),
        status: Status::Skipped,
        reason,
        steps: Vec::new(),
    };
    if let Some(failed) = needs_of(job)
        .into_iter()
        .find(|n| done.get(n) == Some(&Status::Failed))
    {
        return skipped("", format!("the job `{failed}` it needs failed"));
    }
    let combos = match matrix_combos(job) {
        Ok(c) => c,
        Err(why) => return skipped("", why),
    };
    let runs_on = match job.get("runs-on") {
        Some(Yaml::Str(s)) => s.clone(),
        Some(Yaml::Seq(items)) => items
            .iter()
            .filter_map(Yaml::as_str)
            .collect::<Vec<_>>()
            .join(" "),
        Some(Yaml::Map(_)) | None => String::new(),
    };
    let empty_env = BTreeMap::new();
    let mut here: Vec<(BTreeMap<String, String>, String)> = Vec::new();
    let mut elsewhere: Vec<String> = Vec::new();
    for combo in combos {
        let ctx = Context {
            matrix: &combo,
            env: &empty_env,
            os,
            repo: &run.options.repo,
        };
        let label = expand(&runs_on, &ctx).unwrap_or_else(|_| runs_on.clone());
        let labels: Vec<&str> = label.split_whitespace().collect();
        match labels.iter().find_map(|l| label_os(l)) {
            Some(o) if o == os => here.push((combo, label)),
            _ => elsewhere.push(label),
        }
    }
    let Some((combo, label)) = here.first().cloned() else {
        elsewhere.sort();
        elsewhere.dedup();
        return skipped(
            &elsewhere.join(", "),
            format!(
                "it runs on {}, not on this machine ({})",
                if elsewhere.is_empty() {
                    "a runner this cannot identify".to_string()
                } else {
                    elsewhere.join(", ")
                },
                runner_os_name(os)
            ),
        );
    };
    let mut result = run_job(run, workflow, id, job, &combo, &label, os);
    if here.len() > 1 {
        let others: Vec<String> = here[1..].iter().map(|(_, l)| l.clone()).collect();
        let note = format!(
            "also in the matrix on this OS: {} (the versions installed here run once)",
            others.join(", ")
        );
        result.reason = if result.reason.is_empty() {
            note
        } else {
            format!("{}; {note}", result.reason)
        };
    }
    result
}

// ── Output ───────────────────────────────────────────────────────────────────────────────────────────────────────

/// The report as text. `quiet` keeps only what failed and the summary.
pub fn render(report: &Report, quiet: bool) -> String {
    let mut out = String::new();
    for note in &report.notes {
        if !quiet {
            out.push_str(&format!("note: {note}\n"));
        }
    }
    for job in &report.jobs {
        let bad = job.status == Status::Failed;
        if quiet && !bad {
            continue;
        }
        let label = if job.label.is_empty() {
            String::new()
        } else {
            format!(" ({})", job.label)
        };
        out.push_str(&format!(
            "{} / {}{label}: {}",
            job.workflow,
            job.id,
            job.status.word()
        ));
        if !job.reason.is_empty() {
            out.push_str(&format!(" — {}", job.reason));
        }
        out.push('\n');
        for step in &job.steps {
            if quiet && step.status != Status::Failed {
                continue;
            }
            out.push_str(&format!("  [{}] {}", step.status.word(), step.name));
            if step.seconds > 0.0 {
                out.push_str(&format!(" ({:.1}s)", step.seconds));
            }
            if !step.reason.is_empty() {
                out.push_str(&format!(" — {}", step.reason));
            }
            out.push('\n');
            if matches!(step.status, Status::Failed | Status::Unavailable) && !step.tail.is_empty()
            {
                for line in step.tail.lines() {
                    out.push_str(&format!("      {line}\n"));
                }
            }
        }
    }
    out.push_str(&format!(
        "insi ci on {}: {} passed, {} failed, {} skipped, {} unavailable\n",
        runner_os_name(this_os()),
        report.count(Status::Passed),
        report.failed(),
        report.count(Status::Skipped),
        report.unavailable()
    ));
    out
}

/// The report as JSON, for an agent or a script.
pub fn to_json(report: &Report) -> Value {
    json!({
        "os": runner_os_name(this_os()),
        "ok": report.failed() == 0,
        "passed": report.count(Status::Passed),
        "failed": report.failed(),
        "skipped": report.count(Status::Skipped),
        "unavailable": report.unavailable(),
        "notes": report.notes,
        "jobs": report.jobs.iter().map(|j| json!({
            "workflow": j.workflow,
            "job": j.id,
            "runs_on": j.label,
            "status": j.status.word(),
            "reason": j.reason,
            "steps": j.steps.iter().map(|s| json!({
                "name": s.name,
                "status": s.status.word(),
                "reason": s.reason,
                "seconds": s.seconds,
                "tail": s.tail,
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    })
}

// ── The pre-push hook ────────────────────────────────────────────────────────────────────────────────────────────

/// The line that marks a hook as written by this command.
const HOOK_MARK: &str = "# Installed by `insi ci --install-hook`";

/// The hook's text: runs the project's CI steps for this OS, quietly, and stops the push when one failed.
fn hook_text() -> String {
    format!(
        "#!/bin/sh\n{HOOK_MARK}: runs the project's own CI steps on this machine before a push.\n# Skip it once with `INSI_CI_SKIP=1 git push`; remove it by deleting this file.\n[ -n \"$INSI_CI_SKIP\" ] && exit 0\nexec insi ci --repo \"$(git rev-parse --show-toplevel)\" --quiet\n"
    )
}

/// Writes the `pre-push` hook into `repo`'s hooks folder (where git looks, which `core.hooksPath` or a worktree may
/// move). An existing hook that this command did not write is never replaced, unless `force`.
fn install_hook(repo: &Path, force: bool) -> Result<PathBuf, String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "--git-path", "hooks"])
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if !out.status.success() {
        return Err(format!("{} is not a git repository", repo.display()));
    }
    let folder = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let folder = if Path::new(&folder).is_absolute() {
        PathBuf::from(folder)
    } else {
        repo.join(folder)
    };
    std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    let path = folder.join("pre-push");
    if let Ok(existing) = std::fs::read_to_string(&path) {
        if !existing.contains(HOOK_MARK) && !force {
            return Err(format!(
                "{} already exists and was not written by insi; pass --force to replace it",
                path.display()
            ));
        }
    }
    std::fs::write(&path, hook_text()).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| e.to_string())?;
    }
    Ok(path)
}

#[cfg(test)]
#[path = "ci_tests.rs"]
mod tests;
