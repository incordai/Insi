// Copyright (c) 2026-2027 Incord Ai.
// https://incord.ai
//
// SPDX-License-Identifier: BUSL-1.1

//! Lightweight loopback Sentry-compatible error-list server and CLI (`insi errors`).
//!
//! Receives real-time errors from `sentry-tauri` (Watchman), standard Sentry
//! SDKs, or direct JSON posts, groups them per project on disk via
//! [`insi_connectors::errors::Store`], and serves both JSON routes for Watchman
//! and a minimal built-in HTML visual dashboard at `/`.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use flate2::read::GzDecoder;
use insi_connectors::errors::{DirectEvent, ErrorIssue, IssueStatus, Project, Store};
use serde::Deserialize;
use serde_json::{json, Value};

/// Default loopback address for `insi errors serve`.
pub const DEFAULT_ADDR: &str = "127.0.0.1:9185";
/// Environment variable overriding the error-server address.
pub const ADDR_ENV: &str = "INSI_ERRORS_ADDR";
const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(10);

const ERRORS_USAGE: &str = "usage:
  insi errors serve   [--addr 127.0.0.1:9185] [--store <dir>]
  insi errors dsn     [--repo <dir>] [--addr 127.0.0.1:9185] [--store <dir>]
  insi errors list    [--repo <dir>] [--all] [--json] [--store <dir>]
  insi errors check   [--repo <dir>] [--json] [--store <dir>]
  insi errors resolve --id <issue-id> [--repo <dir>] [--store <dir>]
  insi errors ignore  --id <issue-id> [--repo <dir>] [--store <dir>]
  insi errors reopen  --id <issue-id> [--repo <dir>] [--store <dir>]";

/// Parse and validate a loopback socket address.
pub fn parse_loopback_addr(raw: Option<&str>) -> Result<SocketAddr, String> {
    let text = raw
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| {
            std::env::var(ADDR_ENV)
                .ok()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        })
        .unwrap_or_else(|| DEFAULT_ADDR.to_string());
    let addr: SocketAddr = text
        .parse()
        .map_err(|_| format!("invalid address \"{text}\" (expected e.g. {DEFAULT_ADDR})"))?;
    if !addr.ip().is_loopback() {
        return Err(format!(
            "refusing non-loopback address {addr}; the error server listens on localhost only"
        ));
    }
    Ok(addr)
}

/// Entrypoint for `insi errors <subcommand> ...`.
pub fn cli(args: &[String]) -> Result<(), String> {
    let Some((sub, rest)) = args.split_first() else {
        return Err(ERRORS_USAGE.to_string());
    };
    if matches!(sub.as_str(), "-h" | "--help" | "help") {
        println!("{ERRORS_USAGE}");
        return Ok(());
    }
    let flags = parse_flags(rest)?;
    let store = flags
        .get("store")
        .map(Store::new)
        .unwrap_or_else(Store::default_store);
    match sub.as_str() {
        "serve" => {
            let addr = parse_loopback_addr(flags.get("addr"))?;
            serve(addr, &store)
        }
        "dsn" => {
            let repo = repo_arg(flags.get("repo"))?;
            let addr = parse_loopback_addr(flags.get("addr"))?;
            let project = store.register_project(&repo, flags.get("name"))?;
            println!("{}", project.dsn(&addr.to_string()));
            Ok(())
        }
        "list" => list_cmd(&store, &flags),
        "check" => check_cmd(&store, &flags),
        "resolve" => status_cmd(&store, &flags, IssueStatus::Resolved),
        "ignore" => status_cmd(&store, &flags, IssueStatus::Ignored),
        "reopen" => status_cmd(&store, &flags, IssueStatus::Open),
        other => Err(format!(
            "unknown errors subcommand \"{other}\"\n\n{ERRORS_USAGE}"
        )),
    }
}

struct SimpleFlags(BTreeMap<String, Option<String>>);

impl SimpleFlags {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|v| v.as_deref())
    }
    fn has(&self, key: &str) -> bool {
        self.0.contains_key(key)
    }
}

fn parse_flags(args: &[String]) -> Result<SimpleFlags, String> {
    let mut map = BTreeMap::new();
    let mut i = 0;
    while i < args.len() {
        let key = args[i]
            .strip_prefix("--")
            .ok_or_else(|| format!("unexpected argument \"{}\"\n\n{ERRORS_USAGE}", args[i]))?;
        let value = args.get(i + 1).filter(|v| !v.starts_with("--")).cloned();
        i += if value.is_some() { 2 } else { 1 };
        map.insert(key.to_string(), value);
    }
    Ok(SimpleFlags(map))
}

fn repo_arg(raw: Option<&str>) -> Result<PathBuf, String> {
    std::path::absolute(raw.unwrap_or(".")).map_err(|e| format!("--repo: {e}"))
}

fn list_cmd(store: &Store, flags: &SimpleFlags) -> Result<(), String> {
    let filter = if flags.has("all") {
        None
    } else {
        Some(IssueStatus::Open)
    };
    let issues = if let Some(raw_repo) = flags.get("repo") {
        let repo = repo_arg(Some(raw_repo))?;
        match store.project_by_repo(&repo)? {
            Some(p) => store.list_issues(&p.id, filter)?,
            None => Vec::new(),
        }
    } else {
        let mut all = Vec::new();
        for p in store.projects()? {
            all.extend(store.list_issues(&p.id, filter)?);
        }
        all.sort_by(|a, b| b.last_seen.cmp(&a.last_seen).then(a.id.cmp(&b.id)));
        all
    };
    if flags.has("json") {
        println!(
            "{}",
            serde_json::to_string_pretty(&issues).map_err(|e| e.to_string())?
        );
        return Ok(());
    }
    if issues.is_empty() {
        println!("no runtime errors");
        return Ok(());
    }
    for issue in issues {
        println!(
            "{}  [{} · {} · {} ×{}] {}{}",
            issue.id,
            issue.status.as_str(),
            issue.level,
            issue.process,
            issue.count,
            issue.title,
            if issue.culprit.is_empty() {
                String::new()
            } else {
                format!(" ({})", issue.culprit)
            }
        );
    }
    Ok(())
}

fn check_cmd(store: &Store, flags: &SimpleFlags) -> Result<(), String> {
    let repo = repo_arg(flags.get("repo"))?;
    let project = store.project_by_repo(&repo)?;
    let open = match &project {
        Some(p) => store.list_issues(&p.id, Some(IssueStatus::Open))?,
        None => Vec::new(),
    };
    if flags.has("json") {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "project": project,
                "open": open.len(),
                "errors": open,
            }))
            .map_err(|e| e.to_string())?
        );
    } else if open.is_empty() {
        println!("no open runtime errors");
    } else {
        println!("{} open runtime error(s):", open.len());
        for issue in open.iter().take(10) {
            println!(
                "  - [{}] [{} · {} ×{}] {}{}",
                issue.id,
                issue.level,
                issue.process,
                issue.count,
                issue.title,
                if issue.culprit.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", issue.culprit)
                }
            );
        }
    }
    if open.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{} open runtime error(s): {}",
            open.len(),
            summarize_open(&open)
        ))
    }
}

fn status_cmd(store: &Store, flags: &SimpleFlags, status: IssueStatus) -> Result<(), String> {
    let id = flags
        .get("id")
        .ok_or_else(|| format!("--id <issue-id> is required\n\n{ERRORS_USAGE}"))?;
    let repo = repo_arg(flags.get("repo"))?;
    let project = store
        .project_by_repo(&repo)?
        .ok_or_else(|| format!("no error project registered for {}", repo.display()))?;
    let updated = store.set_status(&project.id, id, status)?;
    println!("{} -> {}", updated.id, updated.status.as_str());
    Ok(())
}

/// Start the loopback HTTP error-list server and block until the process ends.
pub fn serve(addr: SocketAddr, store: &Store) -> Result<(), String> {
    let listener = TcpListener::bind(addr).map_err(|e| format!("bind {addr}: {e}"))?;
    let bound = listener.local_addr().map_err(|e| e.to_string())?;
    eprintln!("insi errors serving on http://{bound}");
    serve_listener(listener, store)
}

fn serve_listener(listener: TcpListener, store: &Store) -> Result<(), String> {
    let bound = listener
        .local_addr()
        .map_err(|e| e.to_string())?
        .to_string();
    for incoming in listener.incoming() {
        let Ok(stream) = incoming else {
            continue;
        };
        let store = store.clone();
        let host = bound.clone();
        std::thread::spawn(move || {
            let _ = handle_connection(stream, &store, &host);
        });
    }
    Ok(())
}

struct Request {
    method: String,
    target: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

struct Response {
    status: u16,
    reason: &'static str,
    content_type: &'static str,
    body: Vec<u8>,
}

impl Response {
    fn json(status: u16, reason: &'static str, value: Value) -> Self {
        Self {
            status,
            reason,
            content_type: "application/json; charset=utf-8",
            body: serde_json::to_vec(&value).unwrap_or_else(|_| b"{}".to_vec()),
        }
    }

    fn html(body: &'static str) -> Self {
        Self {
            status: 200,
            reason: "OK",
            content_type: "text/html; charset=utf-8",
            body: body.as_bytes().to_vec(),
        }
    }

    fn empty(status: u16, reason: &'static str) -> Self {
        Self {
            status,
            reason,
            content_type: "text/plain; charset=utf-8",
            body: Vec::new(),
        }
    }
}

fn handle_connection(mut stream: TcpStream, store: &Store, host: &str) -> Result<(), String> {
    let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
    let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
    let req = read_request(&stream)?;
    let resp = dispatch(&req, store, host);
    write_response(&mut stream, &resp)
}

fn read_request(stream: &TcpStream) -> Result<Request, String> {
    let mut reader = BufReader::new(stream);
    let mut request_line = String::new();
    let n = reader
        .read_line(&mut request_line)
        .map_err(|e| format!("read request line: {e}"))?;
    if n == 0 {
        return Err("empty connection".into());
    }
    let mut parts = request_line
        .trim_end_matches(['\r', '\n'])
        .split_whitespace();
    let method = parts
        .next()
        .ok_or("missing HTTP method")?
        .to_ascii_uppercase();
    let target = parts.next().ok_or("missing HTTP target")?.to_string();

    let mut headers = BTreeMap::new();
    loop {
        let mut line = String::new();
        let len = reader
            .read_line(&mut line)
            .map_err(|e| format!("read header: {e}"))?;
        if len == 0 {
            break;
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some((k, v)) = trimmed.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }

    let content_len = headers
        .get("content-length")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);
    if content_len > MAX_BODY_BYTES {
        return Err("request body too large".into());
    }
    let mut body = vec![0u8; content_len];
    if content_len > 0 {
        reader
            .read_exact(&mut body)
            .map_err(|e| format!("read body: {e}"))?;
    }
    if headers
        .get("content-encoding")
        .is_some_and(|e| e.eq_ignore_ascii_case("gzip"))
    {
        let mut decoded = Vec::new();
        let mut decoder = GzDecoder::new(&body[..]).take(MAX_BODY_BYTES as u64 + 1);
        decoder
            .read_to_end(&mut decoded)
            .map_err(|e| format!("gzip decode: {e}"))?;
        if decoded.len() > MAX_BODY_BYTES {
            return Err("decompressed body too large".into());
        }
        body = decoded;
    }

    Ok(Request {
        method,
        target,
        headers,
        body,
    })
}

fn write_response(stream: &mut TcpStream, resp: &Response) -> Result<(), String> {
    let head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type, X-Sentry-Auth, Authorization\r\nConnection: close\r\n\r\n",
        resp.status,
        resp.reason,
        resp.content_type,
        resp.body.len()
    );
    stream
        .write_all(head.as_bytes())
        .and_then(|()| stream.write_all(&resp.body))
        .and_then(|()| stream.flush())
        .map_err(|e| format!("write response: {e}"))
}

fn dispatch(req: &Request, store: &Store, host: &str) -> Response {
    if req.method == "OPTIONS" {
        return Response::empty(204, "No Content");
    }
    let (path, query_str) = req
        .target
        .split_once('?')
        .unwrap_or((req.target.as_str(), ""));
    let query = parse_query(query_str);
    let path_trimmed = path.trim_end_matches('/');
    let path_norm = if path_trimmed.is_empty() {
        "/"
    } else {
        path_trimmed
    };

    match (req.method.as_str(), path_norm) {
        ("GET", "/") => Response::html(DASHBOARD_HTML),
        ("GET", "/v1/health") => Response::json(
            200,
            "OK",
            json!({
                "ok": true,
                "service": "insi-errors",
                "version": env!("CARGO_PKG_VERSION"),
            }),
        ),
        ("GET", "/v1/projects") => match store.projects() {
            Ok(projects) => {
                let with_dsn: Vec<Value> = projects
                    .into_iter()
                    .map(|p| project_json(&p, host))
                    .collect();
                Response::json(200, "OK", json!({ "projects": with_dsn }))
            }
            Err(e) => Response::json(500, "Internal Server Error", json!({ "error": e })),
        },
        ("POST", "/v1/projects") => handle_register_project(&req.body, store, host),
        ("GET", "/v1/errors") => handle_list_errors(&query, store, host),
        ("POST", "/v1/errors/ingest") => handle_ingest_direct(&req.body, store),
        _ => dispatch_dynamic(req, path_norm, &query, store),
    }
}

fn dispatch_dynamic(
    req: &Request,
    path: &str,
    query: &BTreeMap<String, String>,
    store: &Store,
) -> Response {
    if req.method == "POST" {
        if let Some(rest) = path.strip_prefix("/api/") {
            if let Some((project_id, endpoint)) = rest.split_once('/') {
                let auth_key = extract_sentry_key(&req.headers, query);
                let result = match endpoint {
                    "envelope" => {
                        store.ingest_sentry_envelope(project_id, auth_key.as_deref(), &req.body)
                    }
                    "store" => {
                        store.ingest_sentry_store(project_id, auth_key.as_deref(), &req.body)
                    }
                    _ => return Response::json(404, "Not Found", json!({ "error": "not found" })),
                };
                return match result {
                    Ok(issues) => {
                        let id = issues
                            .first()
                            .map(|i| i.latest.event_id.as_str())
                            .unwrap_or("ok");
                        Response::json(200, "OK", json!({ "id": id }))
                    }
                    Err(e) if e.contains("sentry key") => {
                        Response::json(401, "Unauthorized", json!({ "error": e }))
                    }
                    Err(e) if e.starts_with("unknown project") => {
                        Response::json(404, "Not Found", json!({ "error": e }))
                    }
                    Err(e) => Response::json(400, "Bad Request", json!({ "error": e })),
                };
            }
        }
        if let Some(rest) = path.strip_prefix("/v1/errors/") {
            if let Some((issue_id, action)) = rest.rsplit_once('/') {
                let status = match action {
                    "resolve" => IssueStatus::Resolved,
                    "ignore" => IssueStatus::Ignored,
                    "reopen" => IssueStatus::Open,
                    _ => return Response::json(404, "Not Found", json!({ "error": "not found" })),
                };
                return handle_set_status(issue_id, status, &req.body, store);
            }
        }
    }
    Response::json(404, "Not Found", json!({ "error": "not found" }))
}

#[derive(Deserialize)]
struct RegisterProjectBody {
    repo: String,
    #[serde(default)]
    name: Option<String>,
}

fn handle_register_project(body: &[u8], store: &Store, host: &str) -> Response {
    let parsed: RegisterProjectBody = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(e) => return Response::json(400, "Bad Request", json!({ "error": e.to_string() })),
    };
    if parsed.repo.trim().is_empty() {
        return Response::json(400, "Bad Request", json!({ "error": "repo is required" }));
    }
    match store.register_project(Path::new(parsed.repo.trim()), parsed.name.as_deref()) {
        Ok(project) => Response::json(
            200,
            "OK",
            json!({
                "project": project,
                "dsn": project.dsn(host),
            }),
        ),
        Err(e) => Response::json(500, "Internal Server Error", json!({ "error": e })),
    }
}

fn handle_list_errors(query: &BTreeMap<String, String>, store: &Store, host: &str) -> Response {
    let filter = match query.get("status").map(String::as_str) {
        None | Some("open") | Some("") => Some(IssueStatus::Open),
        Some("all") => None,
        Some(other) => match IssueStatus::parse(other) {
            Ok(s) => Some(s),
            Err(e) => return Response::json(400, "Bad Request", json!({ "error": e })),
        },
    };
    let project = if let Some(id) = query.get("project").filter(|s| !s.is_empty()) {
        match store.project_by_id(id) {
            Ok(p) => p,
            Err(e) => return Response::json(500, "Internal Server Error", json!({ "error": e })),
        }
    } else if let Some(cwd) = query.get("cwd").filter(|s| !s.is_empty()) {
        match store.project_by_repo(Path::new(cwd)) {
            Ok(p) => p,
            Err(e) => return Response::json(500, "Internal Server Error", json!({ "error": e })),
        }
    } else {
        let projects = match store.projects() {
            Ok(p) => p,
            Err(e) => return Response::json(500, "Internal Server Error", json!({ "error": e })),
        };
        let mut all = Vec::new();
        for p in &projects {
            if let Ok(list) = store.list_issues(&p.id, filter) {
                all.extend(list);
            }
        }
        all.sort_by(|a, b| b.last_seen.cmp(&a.last_seen).then(a.id.cmp(&b.id)));
        let with_dsn: Vec<Value> = projects.iter().map(|p| project_json(p, host)).collect();
        return Response::json(
            200,
            "OK",
            json!({
                "projects": with_dsn,
                "errors": all,
            }),
        );
    };

    let Some(project) = project else {
        return Response::json(
            200,
            "OK",
            json!({
                "project": null,
                "dsn": null,
                "errors": [],
            }),
        );
    };
    match store.list_issues(&project.id, filter) {
        Ok(errors) => Response::json(
            200,
            "OK",
            json!({
                "project": project,
                "dsn": project.dsn(host),
                "errors": errors,
            }),
        ),
        Err(e) => Response::json(500, "Internal Server Error", json!({ "error": e })),
    }
}

fn handle_ingest_direct(body: &[u8], store: &Store) -> Response {
    let direct: DirectEvent = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(e) => return Response::json(400, "Bad Request", json!({ "error": e.to_string() })),
    };
    match store.ingest_direct(direct) {
        Ok(issue) => Response::json(200, "OK", json!({ "ok": true, "issue": issue })),
        Err(e) => Response::json(400, "Bad Request", json!({ "error": e })),
    }
}

#[derive(Default, Deserialize)]
struct StatusBody {
    #[serde(default)]
    project_id: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
}

fn handle_set_status(issue_id: &str, status: IssueStatus, body: &[u8], store: &Store) -> Response {
    let parsed: StatusBody = if body.is_empty() {
        StatusBody::default()
    } else {
        match serde_json::from_slice(body) {
            Ok(v) => v,
            Err(e) => return Response::json(400, "Bad Request", json!({ "error": e.to_string() })),
        }
    };
    let project_id = if let Some(id) = parsed.project_id.filter(|s| !s.is_empty()) {
        Some(id)
    } else if let Some(cwd) = parsed.cwd.filter(|s| !s.is_empty()) {
        store
            .project_by_repo(Path::new(&cwd))
            .ok()
            .flatten()
            .map(|p| p.id)
    } else {
        find_project_for_issue(store, issue_id)
    };
    let Some(project_id) = project_id else {
        return Response::json(
            404,
            "Not Found",
            json!({ "error": format!("issue \"{issue_id}\" not found") }),
        );
    };
    match store.set_status(&project_id, issue_id, status) {
        Ok(issue) => Response::json(200, "OK", json!({ "ok": true, "issue": issue })),
        Err(e) => Response::json(404, "Not Found", json!({ "error": e })),
    }
}

fn find_project_for_issue(store: &Store, issue_id: &str) -> Option<String> {
    let projects = store.projects().ok()?;
    for p in projects {
        if let Ok(issues) = store.list_issues(&p.id, None) {
            if issues.iter().any(|i| i.id == issue_id) {
                return Some(p.id);
            }
        }
    }
    None
}

fn project_json(p: &Project, host: &str) -> Value {
    json!({
        "id": p.id,
        "key": p.key,
        "repo": p.repo,
        "name": p.name,
        "created_at": p.created_at,
        "dsn": p.dsn(host),
    })
}

fn extract_sentry_key(
    headers: &BTreeMap<String, String>,
    query: &BTreeMap<String, String>,
) -> Option<String> {
    for name in ["x-sentry-auth", "authorization"] {
        if let Some(raw) = headers.get(name) {
            for part in raw.split([',', ' ']) {
                if let Some(val) = part.trim().strip_prefix("sentry_key=") {
                    let key = val.trim_matches('"').trim();
                    if !key.is_empty() {
                        return Some(key.to_string());
                    }
                }
            }
        }
    }
    query
        .get("sentry_key")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn parse_query(raw: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for pair in raw.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        out.insert(percent_decode(k), percent_decode(v));
    }
    out
}

fn percent_decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'+' {
            out.push(b' ');
            i += 1;
        } else if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
            out.push(bytes[i]);
            i += 1;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Format open issues summary for CLI callers (`ErrorIssue` list).
pub fn summarize_open(issues: &[ErrorIssue]) -> String {
    issues
        .iter()
        .take(5)
        .map(|i| format!("[{}] {} (×{})", i.process, i.title, i.count))
        .collect::<Vec<_>>()
        .join("; ")
}

const DASHBOARD_HTML: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width,initial-scale=1" />
<title>Insi Runtime Errors</title>
<style>
  :root { color-scheme: dark; --bg:#141416; --card:#1c1c20; --border:#2b2b31; --fg:#ececf1; --muted:#9a9aa6; --red:#ef4444; --green:#10b981; }
  body { margin:0; font:13px/1.5 ui-sans-serif,system-ui,sans-serif; background:var(--bg); color:var(--fg); }
  header { display:flex; gap:12px; align-items:center; padding:14px 20px; border-bottom:1px solid var(--border); }
  h1 { font-size:15px; margin:0; font-weight:600; }
  select, button { background:var(--card); color:var(--fg); border:1px solid var(--border); border-radius:6px; padding:5px 10px; font:inherit; cursor:pointer; }
  main { max-width:980px; margin:20px auto; padding:0 20px; }
  .dsn { background:var(--card); border:1px solid var(--border); border-radius:8px; padding:10px 12px; margin-bottom:16px; display:flex; align-items:center; justify-content:space-between; gap:12px; }
  .dsn code { font:12px ui-monospace,monospace; color:var(--muted); overflow-wrap:anywhere; }
  .item { background:var(--card); border:1px solid var(--border); border-radius:8px; padding:12px 14px; margin-bottom:10px; }
  .row { display:flex; justify-content:space-between; align-items:center; gap:10px; }
  .meta { color:var(--muted); font-size:12px; margin-top:4px; }
  pre { background:#111113; border:1px solid var(--border); border-radius:6px; padding:8px; overflow:auto; font:11.5px ui-monospace,monospace; color:#c8c8d2; margin:8px 0 0; }
  .badge { font-size:11px; padding:1px 6px; border-radius:999px; border:1px solid var(--border); color:var(--muted); }
  .badge.open { color:var(--red); border-color:rgba(239,68,68,.35); }
  .badge.resolved { color:var(--green); border-color:rgba(16,185,129,.35); }
</style>
</head>
<body>
<header>
  <h1>Insi Runtime Errors</h1>
  <select id="project"></select>
  <select id="status">
    <option value="open">Open</option>
    <option value="resolved">Resolved</option>
    <option value="ignored">Ignored</option>
    <option value="all">All</option>
  </select>
  <button id="refresh">Refresh</button>
</header>
<main>
  <div id="dsnBox" class="dsn" style="display:none"><code id="dsn"></code><button id="copyDsn">Copy DSN</button></div>
  <div id="list">Loading…</div>
</main>
<script>
const projSel = document.getElementById('project');
const statusSel = document.getElementById('status');
const listEl = document.getElementById('list');
const dsnBox = document.getElementById('dsnBox');
const dsnEl = document.getElementById('dsn');

async function loadProjects() {
  const res = await fetch('/v1/projects').then(r => r.json());
  const cur = projSel.value;
  projSel.innerHTML = '<option value="">All projects</option>' + (res.projects || []).map(p => `<option value="${p.id}">${p.name} (${p.id})</option>`).join('');
  if (cur) projSel.value = cur;
}

async function loadErrors() {
  const q = new URLSearchParams();
  if (projSel.value) q.set('project', projSel.value);
  q.set('status', statusSel.value);
  const res = await fetch('/v1/errors?' + q.toString()).then(r => r.json());
  if (res.dsn) { dsnBox.style.display = 'flex'; dsnEl.textContent = res.dsn; } else { dsnBox.style.display = 'none'; }
  const errs = res.errors || [];
  if (!errs.length) { listEl.innerHTML = '<p class="meta">No matching errors.</p>'; return; }
  listEl.innerHTML = errs.map(e => {
    const frames = (e.latest && e.latest.frames || []).slice(-6).reverse().map(f => `${f.function || '<anon>'} (${f.filename}${f.lineno ? ':' + f.lineno : ''})`).join('\n');
    return `<div class="item"><div class="row"><strong>${esc(e.title)}</strong><div><span class="badge ${e.status}">${e.status}</span> <span class="badge">${esc(e.process)}</span> <span class="badge">×${e.count}</span></div></div><div class="meta">${esc(e.project_id)} · ${esc(e.culprit || '')} · last seen ${new Date(e.last_seen * 1000).toLocaleString()}</div>${frames ? `<pre>${esc(frames)}</pre>` : ''}<div style="margin-top:8px;display:flex;gap:6px">${e.status !== 'resolved' ? `<button onclick="act('${e.project_id}','${e.id}','resolve')">Resolve</button>` : `<button onclick="act('${e.project_id}','${e.id}','reopen')">Reopen</button>`}${e.status !== 'ignored' ? `<button onclick="act('${e.project_id}','${e.id}','ignore')">Ignore</button>` : ''}</div></div>`;
  }).join('');
}

function esc(s) { return String(s).replace(/[&<>"]/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;'}[c])); }
async function act(project_id, id, op) {
  await fetch(`/v1/errors/${encodeURIComponent(id)}/${op}`, { method: 'POST', headers: {'content-type':'application/json'}, body: JSON.stringify({ project_id }) });
  await loadErrors();
}
document.getElementById('copyDsn').onclick = () => navigator.clipboard?.writeText(dsnEl.textContent);
document.getElementById('refresh').onclick = () => loadErrors();
projSel.onchange = loadErrors;
statusSel.onchange = loadErrors;
loadProjects().then(loadErrors);
</script>
</body>
</html>
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::GzEncoder;
    use flate2::Compression;

    fn temp_store() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "insi-errors-http-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    fn http_call(addr: SocketAddr, req: &[u8]) -> String {
        let mut stream = TcpStream::connect(addr).expect("connect");
        stream.write_all(req).expect("write");
        let _ = stream.shutdown(std::net::Shutdown::Write);
        let mut resp = Vec::new();
        stream.read_to_end(&mut resp).expect("read");
        String::from_utf8_lossy(&resp).into_owned()
    }

    #[test]
    fn refuses_non_loopback_address() {
        assert!(parse_loopback_addr(Some("0.0.0.0:9185")).is_err());
        assert!(parse_loopback_addr(Some("192.168.1.5:9185")).is_err());
        assert!(parse_loopback_addr(Some("127.0.0.1:9185")).is_ok());
    }

    #[test]
    fn http_server_round_trips_project_gzip_envelope_list_and_resolve() {
        let dir = temp_store();
        let store = Store::new(&dir);
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let server_store = store.clone();
        std::thread::spawn(move || {
            let _ = serve_listener(listener, &server_store);
        });

        let repo = dir.join("demo-app");
        std::fs::create_dir_all(&repo).expect("repo");
        let reg_body = serde_json::to_vec(&json!({ "repo": repo.display().to_string() })).unwrap();
        let reg_req = format!(
            "POST /v1/projects HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
            reg_body.len()
        );
        let mut reg_bytes = reg_req.into_bytes();
        reg_bytes.extend_from_slice(&reg_body);
        let reg_resp = http_call(addr, &reg_bytes);
        assert!(reg_resp.starts_with("HTTP/1.1 200 OK"), "{reg_resp}");
        let reg_json: Value =
            serde_json::from_str(reg_resp.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        let project_id = reg_json["project"]["id"].as_str().unwrap().to_string();
        let key = reg_json["project"]["key"].as_str().unwrap().to_string();

        // Gzipped Sentry envelope (matches sentry-rust / sentry-tauri default transport).
        let envelope = concat!(
            "{\"event_id\":\"evt99\"}\n",
            "{\"type\":\"event\"}\n",
            "{\"event_id\":\"evt99\",\"level\":\"error\",\"platform\":\"rust\",\"tags\":{\"event.process\":\"rust\"},\"exception\":{\"values\":[{\"type\":\"Panic\",\"value\":\"boom\",\"stacktrace\":{\"frames\":[{\"filename\":\"src/main.rs\",\"function\":\"run\",\"lineno\":12,\"in_app\":true}]}}]}}\n"
        );
        let mut gz = GzEncoder::new(Vec::new(), Compression::default());
        gz.write_all(envelope.as_bytes()).unwrap();
        let gz_bytes = gz.finish().unwrap();

        let env_head = format!(
            "POST /api/{project_id}/envelope/ HTTP/1.1\r\nHost: {addr}\r\nContent-Encoding: gzip\r\nX-Sentry-Auth: Sentry sentry_version=7, sentry_key={key}\r\nContent-Length: {}\r\n\r\n",
            gz_bytes.len()
        );
        let mut env_req = env_head.into_bytes();
        env_req.extend_from_slice(&gz_bytes);
        let env_resp = http_call(addr, &env_req);
        assert!(env_resp.starts_with("HTTP/1.1 200 OK"), "{env_resp}");

        let list_req = format!(
            "GET /v1/errors?project={project_id}&status=open HTTP/1.1\r\nHost: {addr}\r\n\r\n"
        );
        let list_resp = http_call(addr, list_req.as_bytes());
        let list_json: Value =
            serde_json::from_str(list_resp.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        let errors = list_json["errors"].as_array().unwrap();
        assert_eq!(errors.len(), 1);
        let issue_id = errors[0]["id"].as_str().unwrap();
        assert_eq!(errors[0]["process"].as_str(), Some("rust"));

        let res_body = serde_json::to_vec(&json!({ "project_id": project_id })).unwrap();
        let res_req = format!(
            "POST /v1/errors/{issue_id}/resolve HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
            res_body.len()
        );
        let mut res_bytes = res_req.into_bytes();
        res_bytes.extend_from_slice(&res_body);
        let res_resp = http_call(addr, &res_bytes);
        assert!(res_resp.starts_with("HTTP/1.1 200 OK"), "{res_resp}");

        let after_resp = http_call(addr, list_req.as_bytes());
        let after_json: Value =
            serde_json::from_str(after_resp.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert!(after_json["errors"].as_array().unwrap().is_empty());

        let html_resp = http_call(
            addr,
            format!("GET / HTTP/1.1\r\nHost: {addr}\r\n\r\n").as_bytes(),
        );
        assert!(html_resp.contains("Insi Runtime Errors"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
