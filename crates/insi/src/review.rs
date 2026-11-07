//! Evidence-first local/PR reviews. Tools come only from the operator's plan.
use crate::issue::GhCli;
use insi_core::Config;
use insi_eval::{git::git, process};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path},
    time::Duration,
};

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub version: u32,
    #[serde(default)]
    pub tools: Vec<Tool>,
    #[serde(default)]
    pub full_checks: Vec<Vec<String>>,
    #[serde(default)]
    pub tests: Vec<TestRule>,
    #[serde(default)]
    pub graph: Vec<GraphNode>,
    #[serde(default)]
    pub graph_commit: Option<String>,
    #[serde(default)]
    pub block_on: Option<String>,
    #[serde(default)]
    pub reproductions: Vec<crate::review_evidence::Reproduction>,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Tool {
    pub name: String,
    pub command: Vec<String>,
    pub format: String,
    #[serde(default = "exits")]
    pub accepted_exits: Vec<i32>,
    #[serde(default = "timeout")]
    pub timeout_secs: u64,
}
fn exits() -> Vec<i32> {
    vec![0]
}
fn timeout() -> u64 {
    300
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TestRule {
    pub paths: Vec<String>,
    pub command: Vec<String>,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GraphNode {
    pub file: String,
    #[serde(default)]
    pub callers: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub id: String,
    pub file: String,
    pub line: u64,
    pub rule: String,
    pub message: String,
    pub severity: String,
    pub verification: String,
    pub sources: Vec<String>,
}

pub fn read(path: &Path) -> Result<Plan, String> {
    if std::fs::metadata(path).map_err(|e| e.to_string())?.len() > 1024 * 1024 {
        return Err("review plan exceeds 1MiB".into());
    }
    let p: Plan = serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    if p.version != 1 || p.tools.len() > 32 || p.full_checks.is_empty() {
        return Err("review plan requires version 1, <=32 tools and full_checks fallback".into());
    }
    if p.block_on
        .as_deref()
        .is_some_and(|s| !matches!(s, "note" | "warning" | "error"))
    {
        return Err("block_on must be note, warning or error".into());
    }
    let mut names = BTreeSet::new();
    for t in &p.tools {
        if !names.insert(&t.name)
            || t.name.is_empty()
            || !t
                .name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            || t.command.first().is_none_or(|s| s.trim().is_empty())
            || !matches!(t.format.as_str(), "json" | "sarif" | "eslint")
            || t.accepted_exits.is_empty()
            || !(1..=3600).contains(&t.timeout_secs)
        {
            return Err("invalid review tool".into());
        }
    }
    for command in p
        .full_checks
        .iter()
        .chain(p.tests.iter().map(|t| &t.command))
    {
        if command.first().is_none_or(|s| s.trim().is_empty()) {
            return Err("empty review check command".into());
        }
    }
    for rule in &p.tests {
        for path in &rule.paths {
            safe_path(path)?;
        }
    }
    for node in &p.graph {
        safe_path(&node.file)?;
        for caller in &node.callers {
            safe_path(caller)?;
        }
    }
    let mut ids = BTreeSet::new();
    let mut fixture_files = BTreeSet::new();
    for r in &p.reproductions {
        crate::review_evidence::validate(r)?;
        if !ids.insert(&r.id) || r.files.keys().any(|p| !fixture_files.insert(p)) {
            return Err("duplicate reproduction ID or fixture path".into());
        }
    }
    Ok(p)
}

pub fn safe_path(path: &str) -> Result<(), String> {
    if path.is_empty()
        || path.contains('\\')
        || Path::new(path).is_absolute()
        || Path::new(path)
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err("review paths must be relative and contain no traversal".into());
    }
    Ok(())
}

fn fingerprint(file: &str, rule: &str, message: &str) -> String {
    let text = format!(
        "{file}\0{rule}\0{}",
        message.split_whitespace().collect::<Vec<_>>().join(" ")
    );
    let mut hash = 14695981039346656037u64;
    for byte in text.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(1099511628211);
    }
    format!("{hash:016x}")
}

/// All imported claims are tool-reported; producers cannot self-certify a reproduction.
pub fn normalize(format: &str, value: &Value, source: &str) -> Result<Vec<Finding>, String> {
    let mut rows = Vec::new();
    match format {
        "json" => {
            let array = value
                .get("findings")
                .and_then(Value::as_array)
                .ok_or("tool JSON needs findings array")?;
            for f in array {
                rows.push((
                    f["file"].as_str().unwrap_or(""),
                    f["line"].as_u64().unwrap_or(0),
                    f["rule"].as_str().unwrap_or("unknown"),
                    f["message"].as_str().unwrap_or(""),
                    f["severity"].as_str().unwrap_or("warning"),
                ));
            }
        }
        "eslint" => {
            for file in value.as_array().ok_or("ESLint output must be an array")? {
                for f in file["messages"]
                    .as_array()
                    .ok_or("ESLint messages missing")?
                {
                    rows.push((
                        file["filePath"].as_str().unwrap_or(""),
                        f["line"].as_u64().unwrap_or(0),
                        f["ruleId"].as_str().unwrap_or("parse-error"),
                        f["message"].as_str().unwrap_or(""),
                        if f["severity"] == 2 {
                            "error"
                        } else {
                            "warning"
                        },
                    ));
                }
            }
        }
        "sarif" => {
            for run in value["runs"].as_array().ok_or("SARIF runs missing")? {
                let results = run["results"].as_array().ok_or("SARIF results missing")?;
                for f in results {
                    let loc = &f["locations"][0]["physicalLocation"];
                    rows.push((
                        loc["artifactLocation"]["uri"].as_str().unwrap_or(""),
                        loc["region"]["startLine"].as_u64().unwrap_or(0),
                        f["ruleId"].as_str().unwrap_or("unknown"),
                        f["message"]["text"].as_str().unwrap_or(""),
                        f["level"].as_str().unwrap_or("warning"),
                    ));
                }
            }
        }
        _ => return Err("unsupported review tool format".into()),
    }
    if rows.len() > 5000 {
        return Err("review finding limit exceeded".into());
    }
    let mut unique: BTreeMap<String, Finding> = BTreeMap::new();
    for (file, line, rule, message, severity) in rows {
        safe_path(file)?;
        if message.trim().is_empty() || line == 0 || message.len() > 16384 {
            return Err("finding requires a message and positive line".into());
        }
        let id = fingerprint(file, rule, message);
        unique.entry(id.clone()).or_insert(Finding {
            id,
            file: file.into(),
            line,
            rule: rule.into(),
            message: message.into(),
            severity: match severity {
                "critical" | "high" | "error" => "error",
                "low" | "info" | "note" => "note",
                _ => "warning",
            }
            .into(),
            verification: "tool-reported".into(),
            sources: vec![source.into()],
        });
    }
    Ok(unique.into_values().collect())
}

/// Incord owns the graph. An exact-revision snapshot may narrow checks; missing/stale coverage falls back.
pub fn impact(plan: &Plan, changed: &[String], commit: &str) -> Value {
    let mut affected: BTreeSet<String> = changed.iter().cloned().collect();
    let mut reason = "exact graph snapshot";
    let mut full = plan.graph_commit.as_deref() != Some(commit) || changed.is_empty();
    if full {
        reason = "missing or stale graph snapshot";
    }
    if !full {
        loop {
            let old = affected.len();
            for node in &plan.graph {
                if affected.contains(&node.file) {
                    affected.extend(node.callers.iter().cloned());
                }
            }
            if affected.len() == old {
                break;
            }
        }
        if affected.iter().any(|f| {
            !plan.graph.iter().any(|n| &n.file == f)
                || !plan.tests.iter().any(|r| {
                    r.paths
                        .iter()
                        .any(|p| f == p || f.starts_with(&format!("{p}/")))
                })
        }) {
            full = true;
            reason = "incomplete graph or test mapping";
        }
        if changed.iter().any(|f| {
            f.starts_with(".github/")
                || f.ends_with(".toml")
                || f.ends_with(".json")
                || f.ends_with(".lock")
        }) {
            full = true;
            reason = "configuration or dependency change";
        }
    }
    let commands = if full {
        plan.full_checks.clone()
    } else {
        plan.tests
            .iter()
            .filter(|r| {
                r.paths.iter().any(|p| {
                    affected
                        .iter()
                        .any(|f| f == p || f.starts_with(&format!("{p}/")))
                })
            })
            .map(|r| r.command.clone())
            .collect()
    };
    json!({"mode":if full{"full"}else{"selected"},"reason":reason,"affected_files":affected,"commands":commands,"graph_provenance":"producer-declared snapshot; not independently complete"})
}

fn run_command(
    repo: &Path,
    config: &Config,
    argv: &[String],
    secs: u64,
) -> Result<process::Outcome, String> {
    let wrapped = config.sandbox.as_ref().map(|s| s.wrap(argv, repo));
    process::run(
        wrapped.as_deref().unwrap_or(argv),
        repo,
        Duration::from_secs(secs),
    )
}

pub fn collect(
    repo: &Path,
    base: &str,
    plan: &Plan,
    output: &Path,
    config: &Config,
) -> Result<Value, String> {
    let started = std::time::Instant::now();
    let head = git(repo, &["rev-parse", "HEAD"])?;
    let base = git(
        repo,
        &["rev-parse", "--verify", &format!("{base}^{{commit}}")],
    )?;
    let changed = git(repo, &["diff", "--name-only", "-z", &base, &head])?
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    for path in &changed {
        safe_path(path)?;
    }
    let selected = impact(plan, &changed, &base);
    let mut tools = Vec::new();
    let mut findings: BTreeMap<String, Finding> = BTreeMap::new();
    let mut complete = true;
    for tool in &plan.tools {
        let out = run_command(repo, config, &tool.command, tool.timeout_secs);
        let outcome = match out {
            Ok(out) => {
                std::fs::write(
                    output.join(format!("{}.log", tool.name)),
                    format!("{}\n{}", out.stdout, out.stderr),
                )
                .map_err(|e| e.to_string())?;
                if out.timed_out || !out.code.is_some_and(|c| tool.accepted_exits.contains(&c)) {
                    json!({"name":tool.name,"status":"failed","code":out.code,"timed_out":out.timed_out})
                } else {
                    let parsed = serde_json::from_str::<Value>(&out.stdout)
                        .map_err(|e| e.to_string())
                        .and_then(|mut v| {
                            relative_paths(&mut v, repo)?;
                            normalize(&tool.format, &v, &tool.name)
                        });
                    match parsed {
                        Ok(rows) => {
                            for mut f in rows {
                                if let Some(prior) = findings.get_mut(&f.id) {
                                    prior.sources.append(&mut f.sources);
                                } else {
                                    findings.insert(f.id.clone(), f);
                                }
                            }
                            json!({"name":tool.name,"status":"completed","code":out.code})
                        }
                        Err(_) => json!({"name":tool.name,"status":"invalid-output"}),
                    }
                }
            }
            Err(_) => json!({"name":tool.name,"status":"unavailable"}),
        };
        complete &= outcome["status"] == "completed";
        tools.push(outcome);
    }
    let mut checks = Vec::new();
    for (index, argv) in selected["commands"]
        .as_array()
        .ok_or("missing impact commands")?
        .iter()
        .enumerate()
    {
        let argv: Vec<String> = serde_json::from_value(argv.clone()).map_err(|e| e.to_string())?;
        match run_command(repo, config, &argv, 600) {
            Ok(out) => {
                let passed = out.success();
                complete &= passed;
                std::fs::write(
                    output.join(format!("check-{index}.log")),
                    format!("{}\n{}", out.stdout, out.stderr),
                )
                .map_err(|e| e.to_string())?;
                checks.push(json!({"command":argv,"passed":passed,"code":out.code,"timed_out":out.timed_out}));
            }
            Err(_) => {
                complete = false;
                checks.push(json!({"command":argv,"passed":false,"unavailable":true}));
            }
        }
    }
    if git(repo, &["rev-parse", "HEAD"])? != head
        || !git(repo, &["status", "--porcelain"])?.is_empty()
    {
        return Err("review tools changed the checkout; report is not publishable".into());
    }
    Ok(
        json!({"version":1,"base":base,"head":head,"changed_files":changed,"complete":complete,"findings":findings.into_values().collect::<Vec<_>>(),"tools":tools,"checks":checks,"impact":selected,"elapsed_seconds":started.elapsed().as_secs_f64(),"evidence":output}),
    )
}

fn clean_text(s: &str) -> String {
    s.replace('@', "＠")
        .replace('`', "\\`")
        .replace('[', "\\[")
        .replace(']', "\\]")
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn public_report(value: &mut Value) -> bool {
    let mut withheld = false;
    match value {
        Value::String(text) => {
            if !insi_core::secrets::scan_diff(&format!("+{text}")).is_empty() {
                *text = "[withheld sensitive text]".into();
                withheld = true;
            }
        }
        Value::Array(items) => {
            for v in items {
                withheld |= public_report(v);
            }
        }
        Value::Object(fields) => {
            for v in fields.values_mut() {
                withheld |= public_report(v);
            }
        }
        _ => {}
    }
    withheld
}

fn relative_paths(value: &mut Value, repo: &Path) -> Result<(), String> {
    match value {
        Value::Object(fields) => {
            for (key, v) in fields {
                if matches!(key.as_str(), "filePath" | "file" | "uri") {
                    if let Some(raw) = v.as_str() {
                        let raw = raw.strip_prefix("file://").unwrap_or(raw);
                        let path = Path::new(raw);
                        if path.is_absolute() {
                            *v = json!(path
                                .strip_prefix(repo)
                                .map_err(|_| "tool finding points outside reviewed checkout")?
                                .to_str()
                                .ok_or("non-UTF8 finding path")?
                                .replace('\\', "/"));
                        }
                    }
                }
                relative_paths(v, repo)?;
            }
        }
        Value::Array(items) => {
            for v in items {
                relative_paths(v, repo)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn review_marker(report: &Value) -> String {
    let key=fingerprint("review",report["head"].as_str().unwrap_or(""),&json!({"findings":report["findings"],"tools":report["tools"],"checks":report["checks"],"complete":report["complete"],"policy_passed":report["policy_passed"]}).to_string());
    format!(
        "<!-- insi-review:{}:{key} -->",
        report["head"].as_str().unwrap_or("")
    )
}

/// Only comments anchored to added lines of the exact reviewed commit become inline comments.
pub fn review_payload(repo: &Path, report: &Value) -> Result<Value, String> {
    let head = report["head"].as_str().ok_or("missing reviewed head")?;
    let base = report["base"].as_str().ok_or("missing reviewed base")?;
    let mut comments = Vec::new();
    let mut summary = Vec::new();
    for finding in report["findings"]
        .as_array()
        .ok_or("missing findings")?
        .iter()
        .take(100)
    {
        let f: Finding = serde_json::from_value(finding.clone()).map_err(|e| e.to_string())?;
        let body = format!(
            "<!-- insi-finding:{} -->\n**{} · {} · {}**\n\n{}\n\nSources: {}",
            f.id,
            clean_text(&f.severity),
            clean_text(&f.rule),
            f.verification,
            clean_text(&f.message),
            f.sources.join(", ")
        );
        if !insi_core::secrets::scan_diff(&format!("+{body}")).is_empty() {
            summary.push("A finding was withheld by the sensitive-data scan.".into());
            continue;
        }
        let diff = git(
            repo,
            &[
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--unified=0",
                base,
                head,
                "--",
                &f.file,
            ],
        )?;
        if added_lines(&diff).contains(&f.line) {
            comments.push(json!({"path":f.file,"line":f.line,"side":"RIGHT","body":body}));
        } else {
            summary.push(format!(
                "- `{}`:{}: {} ({})",
                clean_text(&f.file),
                f.line,
                clean_text(&f.message),
                f.verification
            ));
        }
    }
    Ok(
        json!({"commit_id":head,"event":"COMMENT","body":format!("{}\nInsi review: {}. {} findings; {} checks.\n\n{}\n\nClaims are tool-reported unless separately reproduced. No automatic approval or merge.",review_marker(report),if report["complete"]==true{"completed"}else{"INCOMPLETE"},report["findings"].as_array().map_or(0,Vec::len),report["checks"].as_array().map_or(0,Vec::len),summary.join("\n")),"comments":comments}),
    )
}

fn added_lines(diff: &str) -> BTreeSet<u64> {
    let mut lines = BTreeSet::new();
    let mut at = 0;
    for line in diff.lines() {
        if line.starts_with("diff --git ") {
            at = 0;
        } else if line.starts_with("@@ ") {
            at = line
                .split_whitespace()
                .nth(2)
                .and_then(|s| s.trim_start_matches('+').split(',').next())
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
        } else if at > 0 && line.starts_with('+') {
            lines.insert(at);
            at += 1;
        } else if line.starts_with(' ') {
            at += 1;
        }
    }
    lines
}

/// A head marker makes retries quiet; findings already commented on are not repeated.
pub fn publish(repo: &Path, number: u64, report: &Value) -> Result<Value, String> {
    let configured = std::env::var("INSI_REVIEW_AUTHOR")
        .ok()
        .filter(|s| !s.trim().is_empty());
    let actor: Value = if let Some(login) = configured {
        json!({"login":login})
    } else {
        serde_json::from_str(&GhCli::gh(repo, &["api", "user"], None)?)
            .map_err(|e| e.to_string())?
    };
    let login = actor["login"]
        .as_str()
        .ok_or("cannot determine authenticated review author")?;
    let endpoint = format!("repos/{{owner}}/{{repo}}/pulls/{number}");
    let current: Value = serde_json::from_str(&GhCli::gh(repo, &["api", &endpoint], None)?)
        .map_err(|e| e.to_string())?;
    if current["head"]["sha"] != report["head"] || current["base"]["sha"] != report["base"] {
        return Err("PR revisions changed; rerun review".into());
    }
    let existing: Value = serde_json::from_str(&GhCli::gh(
        repo,
        &[
            "api",
            &format!("{endpoint}/reviews"),
            "--paginate",
            "--slurp",
        ],
        None,
    )?)
    .map_err(|e| e.to_string())?;
    let marker = review_marker(report);
    if existing
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|page| page.as_array().into_iter().flatten())
        .any(|r| {
            r["user"]["login"] == login && r["body"].as_str().is_some_and(|b| b.contains(&marker))
        })
    {
        return Ok(json!({"status":"already-reviewed"}));
    }
    let comments: Value = serde_json::from_str(&GhCli::gh(
        repo,
        &[
            "api",
            &format!("{endpoint}/comments"),
            "--paginate",
            "--slurp",
        ],
        None,
    )?)
    .map_err(|e| e.to_string())?;
    let previous = comments
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|page| page.as_array().into_iter().flatten())
        .filter(|c| c["user"]["login"] == login)
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    let mut payload = review_payload(repo, report)?;
    payload["body"] = json!(format!(
        "{}\n\nConfigured review policy passed: {}. Showing at most 100 finding details.",
        payload["body"].as_str().unwrap_or(""),
        report["policy_passed"]
    ));
    if let Some(items) = payload["comments"].as_array_mut() {
        items.retain(|c| {
            let marker = c["body"]
                .as_str()
                .unwrap_or("")
                .lines()
                .next()
                .unwrap_or("");
            !previous.contains(marker)
        });
    }
    let reply = GhCli::gh(
        repo,
        &[
            "api",
            "--method",
            "POST",
            &format!("{endpoint}/reviews"),
            "--input",
            "-",
        ],
        Some(&payload.to_string()),
    )?;
    serde_json::from_str(&reply).map_err(|e| e.to_string())
}

/// Clone first: local changes and the owner's checkout are never tool targets.
pub fn run(
    repo: &Path,
    plan_path: &Path,
    output: &Path,
    base: Option<&str>,
    pr: Option<u64>,
    post: bool,
    with_memory: bool,
) -> Result<(), String> {
    let review_started = std::time::Instant::now();
    let plan = read(plan_path)?;
    let mut config = Config::load(repo)?;
    if config.sandbox.is_none() {
        if let Ok(image) = std::env::var("INSI_REVIEW_IMAGE") {
            let digest = image.rsplit_once("@sha256:").map(|(_, digest)| digest);
            if image.starts_with('-')
                || !image
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"/._-:@".contains(&b))
                || !digest
                    .is_some_and(|d| d.len() == 64 && d.bytes().all(|b| b.is_ascii_hexdigit()))
            {
                return Err("INSI_REVIEW_IMAGE must name an immutable image@sha256:digest".into());
            }
            config.sandbox = Some(insi_core::config::Sandbox {
                command: vec![
                    "docker".into(),
                    "run".into(),
                    "--rm".into(),
                    "--network=none".into(),
                    "--cap-drop=ALL".into(),
                    "--security-opt=no-new-privileges".into(),
                    "--pids-limit=256".into(),
                    "--memory=4g".into(),
                    "--cpus=2".into(),
                    "-v".into(),
                    "{tree}:/work".into(),
                    "-w".into(),
                    "/work".into(),
                    image,
                ],
            });
        }
    }
    if pr.is_some() && config.sandbox.is_none() {
        return Err("PR tool execution requires a configured sandbox; fork code must not inherit host credentials".into());
    }
    if post && pr.is_none() {
        return Err("--publish requires --pr".into());
    }
    std::fs::create_dir(output).map_err(|e| format!("review output must be new: {e}"))?;
    let output = std::fs::canonicalize(output).map_err(|e| e.to_string())?;
    let checkout = output.join("checkout");
    git(
        &output,
        &[
            "clone",
            "--quiet",
            "--no-local",
            repo.to_str().ok_or("non-UTF8 repo")?,
            checkout.to_str().ok_or("non-UTF8 output")?,
        ],
    )?;
    let base = if let Some(number) = pr {
        let metadata: Value = serde_json::from_str(&GhCli::gh(
            repo,
            &["api", &format!("repos/{{owner}}/{{repo}}/pulls/{number}")],
            None,
        )?)
        .map_err(|e| e.to_string())?;
        let remote = git(repo, &["remote", "get-url", "origin"])?;
        git(&checkout, &["remote", "set-url", "origin", &remote])?;
        git(
            &checkout,
            &[
                "fetch",
                "--quiet",
                "origin",
                &format!("refs/pull/{number}/head"),
            ],
        )?;
        let head = metadata["head"]["sha"].as_str().ok_or("missing PR head")?;
        if git(&checkout, &["rev-parse", "FETCH_HEAD"])? != head {
            return Err("PR changed during fetch".into());
        }
        git(&checkout, &["checkout", "--quiet", "--detach", head])?;
        let base = metadata["base"]["sha"]
            .as_str()
            .ok_or("missing PR base")?
            .to_string();
        git(&checkout, &["fetch", "--quiet", "origin", &base])?;
        base
    } else {
        base.ok_or("local review requires --base <commit>")?
            .to_string()
    };
    std::fs::write(
        output.join("plan.json"),
        serde_json::to_vec_pretty(&plan).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let memory_impact = if with_memory {
        let diff = git(
            &checkout,
            &["diff", "--no-ext-diff", "--no-textconv", &base, "HEAD"],
        )?;
        if diff.len() > 1024 * 1024 {
            json!({"status":"unavailable","reason":"diff exceeds bounded context limit"})
        } else {
            match insi_memory::incord::address().and_then(|addr| {
                insi_memory::incord::tool_call(
                    addr,
                    "git_blast_radius",
                    json!({"cwd":repo,"diff":diff,"depth":3}),
                )
            }) {
                Ok(v) => v,
                Err(_) => {
                    json!({"status":"unavailable","reason":"memory impact lookup failed; full fallback retained"})
                }
            }
        }
    } else {
        json!({"status":"not-requested"})
    };
    std::fs::write(
        output.join("memory-impact.json"),
        serde_json::to_vec_pretty(&memory_impact).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let result = collect(&checkout, &base, &plan, &output, &config).and_then(|mut report| {
        report["memory_impact"] = memory_impact;
        if !plan.reproductions.is_empty() {
            let baseline = output.join("baseline");
            git(
                &output,
                &[
                    "clone",
                    "--quiet",
                    "--no-local",
                    checkout.to_str().ok_or("non-UTF8 checkout")?,
                    baseline.to_str().ok_or("non-UTF8 baseline")?,
                ],
            )?;
            let base = report["base"].as_str().ok_or("missing base")?.to_string();
            git(&baseline, &["checkout", "--quiet", "--detach", &base])?;
            let evidence = crate::review_evidence::run(
                &plan.reproductions,
                &baseline,
                &checkout,
                &config,
                &output,
            )?;
            for e in &evidence {
                if e["status"] == "inconclusive" {
                    report["complete"] = json!(false);
                }
                if e["status"] == "reproduced-regression" {
                    let file = e["file"].as_str().ok_or("missing evidence file")?;
                    let rule = e["rule"].as_str().ok_or("missing rule")?;
                    let f = Finding {
                        id: fingerprint(file, rule, e["message"].as_str().unwrap_or("")),
                        file: file.into(),
                        line: e["line"].as_u64().unwrap_or(1),
                        rule: rule.into(),
                        message: e["message"].as_str().unwrap_or("").into(),
                        severity: "error".into(),
                        verification: "reproduced-regression".into(),
                        sources: vec![format!("reproduction:{}", e["id"].as_str().unwrap_or(""))],
                    };
                    let items = report["findings"]
                        .as_array_mut()
                        .ok_or("missing findings")?;
                    if let Some(prior) = items.iter_mut().find(|v| v["id"] == f.id) {
                        prior["verification"] = json!("reproduced-regression");
                        prior["severity"] = json!("error");
                    } else {
                        items.push(serde_json::to_value(f).map_err(|e| e.to_string())?);
                    }
                }
            }
            for (tree, expected) in [
                (&checkout, report["head"].as_str().ok_or("missing head")?),
                (&baseline, base.as_str()),
            ] {
                if git(tree, &["rev-parse", "HEAD"])? != expected
                    || !git(tree, &["status", "--porcelain"])?.is_empty()
                {
                    return Err("reproduction changed tracked source or history".into());
                }
            }
            report["reproductions"] = json!(evidence);
        }
        Ok(report)
    });
    let mut report = match &result {
        Ok(v) => v.clone(),
        Err(e) => json!({"complete":false,"error":e}),
    };
    report["original_repo"] = json!(repo);
    report["elapsed_seconds"] = json!(review_started.elapsed().as_secs_f64());
    let rank = |s: &str| match s {
        "error" => 3,
        "warning" => 2,
        _ => 1,
    };
    report["policy_passed"] = json!(
        report["complete"] == true
            && !plan
                .block_on
                .as_deref()
                .is_some_and(|level| report["findings"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|f| rank(f["severity"].as_str().unwrap_or("warning")) >= rank(level)))
    );
    if public_report(&mut report) {
        report["complete"] = json!(false);
        report["policy_passed"] = json!(false);
        report["sensitive_output_withheld"] = json!(true);
    }
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    result?;
    if post {
        let receipt = publish(&checkout, pr.ok_or("missing PR")?, &report)?;
        std::fs::write(output.join("publication.json"), receipt.to_string())
            .map_err(|e| e.to_string())?;
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
    );
    if report["complete"] != true || report["policy_passed"] == false {
        return Err("review incomplete; see report.json".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    fn scratch() -> std::path::PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.incord/temp");
        std::fs::create_dir_all(&root).unwrap();
        let d = root.join(format!(
            "review-test-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&d).unwrap();
        std::fs::canonicalize(d).unwrap()
    }
    fn plan() -> Plan {
        serde_json::from_value(json!({"version":1,"tools":[],"full_checks":[["full"]],"tests":[{"paths":["src","tests"],"command":["selected"]}],"graph":[{"file":"src/a.rs","callers":["tests/a.rs"]},{"file":"tests/a.rs","callers":[]}],"graph_commit":"base"})).unwrap()
    }
    #[test]
    fn exact_graph_can_select_but_stale_or_incomplete_graph_uses_full_checks() {
        let mut p = plan();
        assert_eq!(impact(&p, &["src/a.rs".into()], "base")["mode"], "selected");
        assert_eq!(impact(&p, &["src/a.rs".into()], "new")["mode"], "full");
        p.graph.pop();
        assert_eq!(impact(&p, &["src/a.rs".into()], "base")["mode"], "full");
        assert_eq!(
            impact(&plan(), &["Cargo.toml".into()], "base")["mode"],
            "full"
        );
    }
    #[test]
    fn normalized_claims_are_deduplicated_and_never_self_certify() {
        let f = json!({"file":"src/a.rs","line":3,"rule":"bounds","message":"Index unchecked","verification":"proven"});
        let out = normalize("json", &json!({"findings":[f,f]}), "tool").unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].verification, "tool-reported");
        assert!(normalize(
            "json",
            &json!({"findings":[{"file":"../outside","line":1,"message":"bad"}]}),
            "t"
        )
        .is_err());
    }
    #[test]
    fn sarif_eslint_and_absolute_paths_share_the_same_schema() {
        let repo = std::env::current_dir().unwrap();
        let mut eslint = json!([{"filePath":repo.join("src/a.rs"),"messages":[{"line":2,"ruleId":"r","message":"bad","severity":2}]}]);
        relative_paths(&mut eslint, &repo).unwrap();
        let e = normalize("eslint", &eslint, "eslint").unwrap();
        assert_eq!(e[0].file, "src/a.rs");
        let sarif = json!({"runs":[{"results":[{"ruleId":"r","message":{"text":"bad"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"src/a.rs"},"region":{"startLine":2}}}]}]}]});
        assert_eq!(normalize("sarif", &sarif, "scan").unwrap()[0].id, e[0].id);
    }
    #[test]
    fn inline_anchors_handle_deletions_and_increment_operators() {
        let patch = "diff --git a/a b/a\n--- a/a\n+++ b/a\n@@ -2,1 +2,2 @@\n-old\n+++i;\n+new\n";
        assert_eq!(added_lines(patch), BTreeSet::from([2, 3]));
    }
    #[test]
    fn review_markers_ignore_duration_but_change_when_results_change() {
        let mut report = json!({"head":"abc","complete":false,"findings":[],"tools":[],"checks":[],"elapsed_seconds":1});
        let marker = review_marker(&report);
        report["elapsed_seconds"] = json!(900);
        assert_eq!(marker, review_marker(&report));
        report["complete"] = json!(true);
        assert_ne!(marker, review_marker(&report));
    }
    #[cfg(unix)]
    #[test]
    fn missing_tools_and_failed_checks_are_incomplete_not_clean_reviews() {
        let d = scratch();
        git(&d, &["init", "--quiet"]).unwrap();
        git(&d, &["config", "user.name", "Test"]).unwrap();
        git(&d, &["config", "user.email", "test@localhost"]).unwrap();
        std::fs::write(d.join("code"), "working").unwrap();
        git(&d, &["add", "code"]).unwrap();
        git(&d, &["commit", "--quiet", "-m", "baseline"]).unwrap();
        let base = git(&d, &["rev-parse", "HEAD"]).unwrap();
        let logs = d.join(".git/logs-review");
        std::fs::create_dir(&logs).unwrap();
        let mut p = plan();
        p.full_checks = vec![vec!["sh".into(), "-c".into(), "exit 1".into()]];
        p.tools.push(Tool {
            name: "missing".into(),
            command: vec!["insi-nonexistent-review-tool".into()],
            format: "json".into(),
            accepted_exits: vec![0],
            timeout_secs: 1,
        });
        let report = collect(&d, &base, &p, &logs, &Config::default()).unwrap();
        assert_eq!(report["complete"], false);
        assert_eq!(report["tools"][0]["status"], "unavailable");
        assert_eq!(report["checks"][0]["passed"], false);
        std::fs::remove_dir_all(d).unwrap();
    }
}
