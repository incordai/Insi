// Copyright (c) 2026-2027 Incord Ai.
// https://incord.ai
//
// SPDX-License-Identifier: BUSL-1.1

//! Lightweight per-project error store, Sentry envelope parser, and `Source`
//! implementation for `insi errors` and `insi context`.
//!
//! Stores grouped runtime errors on disk under `<store>/projects.json` and
//! `<store>/issues/<project_id>.json`, scrubbed with [`insi_core::secrets`]
//! before persistence. Read-only [`ErrorSource`] feeds open issues for a
//! repository into [`crate::gather`] as [`Trust::Untrusted`] data.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use insi_core::secrets::{pii_in, shape_in};
use insi_core::signature::normalize;
use insi_core::topic::Topics;
use insi_core::Trust;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Item, Source};

/// Environment variable overriding where Insi stores collected runtime errors.
pub const STORE_DIR_ENV: &str = "INSI_ERRORS_DIR";
const MAX_MESSAGE: usize = 4_000;
const MAX_FRAMES: usize = 32;
const MAX_BREADCRUMBS: usize = 20;
const MAX_ISSUES_PER_PROJECT: usize = 500;

static KEY_SEQ: AtomicU64 = AtomicU64::new(1);

/// Status of a grouped runtime error issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum IssueStatus {
    #[default]
    Open,
    Resolved,
    Ignored,
}

impl IssueStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Resolved => "resolved",
            Self::Ignored => "ignored",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, String> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "open" => Ok(Self::Open),
            "resolved" => Ok(Self::Resolved),
            "ignored" => Ok(Self::Ignored),
            other => Err(format!(
                "unknown status \"{other}\" (expected open, resolved, or ignored)"
            )),
        }
    }
}

/// One registered codebase receiving runtime errors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub key: String,
    pub repo: String,
    pub name: String,
    pub created_at: u64,
}

impl Project {
    /// Standard Sentry-protocol DSN targeting `addr` (e.g. `127.0.0.1:9185`).
    pub fn dsn(&self, addr: &str) -> String {
        format!("http://{}@{addr}/{}", self.key, self.id)
    }
}

/// One stack frame in a captured error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct StackFrame {
    pub filename: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub function: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lineno: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colno: Option<u64>,
    #[serde(default)]
    pub in_app: bool,
}

/// One captured runtime error event (redacted).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorEvent {
    pub event_id: String,
    pub timestamp: u64,
    pub level: String,
    pub title: String,
    pub message: String,
    pub culprit: String,
    pub process: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    #[serde(default)]
    pub frames: Vec<StackFrame>,
    #[serde(default)]
    pub breadcrumbs: Vec<String>,
    #[serde(default)]
    pub tags: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// Grouped runtime error issue for a project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorIssue {
    pub id: String,
    pub project_id: String,
    pub title: String,
    pub culprit: String,
    pub level: String,
    pub process: String,
    pub status: IssueStatus,
    pub count: u64,
    pub first_seen: u64,
    pub last_seen: u64,
    #[serde(default)]
    pub paths: Vec<String>,
    pub latest: ErrorEvent,
}

/// Direct error payload accepted by `/v1/errors/ingest` and tests.
#[derive(Debug, Clone, Deserialize)]
pub struct DirectEvent {
    #[serde(default)]
    pub repo: Option<String>,
    #[serde(default)]
    pub project_id: Option<String>,
    pub title: String,
    #[serde(default)]
    pub level: Option<String>,
    #[serde(default)]
    pub process: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub culprit: Option<String>,
    #[serde(default)]
    pub release: Option<String>,
    #[serde(default)]
    pub environment: Option<String>,
    #[serde(default)]
    pub platform: Option<String>,
    #[serde(default)]
    pub frames: Vec<StackFrame>,
    #[serde(default)]
    pub breadcrumbs: Vec<String>,
    #[serde(default)]
    pub tags: BTreeMap<String, String>,
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ProjectsFile {
    #[serde(default)]
    projects: Vec<Project>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct IssuesFile {
    #[serde(default)]
    issues: Vec<ErrorIssue>,
}

/// Default location for the local error store (`INSI_ERRORS_DIR`, else `<home>/.insi/errors`).
pub fn default_store_dir() -> PathBuf {
    if let Ok(raw) = std::env::var(STORE_DIR_ENV) {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".insi")
        .join("errors")
}

fn home_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    let keys = ["USERPROFILE", "HOME"];
    #[cfg(not(windows))]
    let keys = ["HOME", "USERPROFILE"];
    keys.iter().find_map(|k| {
        std::env::var_os(k)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    })
}

/// File-backed error store shared by `insi errors` and `insi context`.
#[derive(Debug, Clone)]
pub struct Store {
    pub dir: PathBuf,
}

impl Store {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn default_store() -> Self {
        Self::new(default_store_dir())
    }

    fn lock(&self) -> Result<File, String> {
        std::fs::create_dir_all(&self.dir).map_err(|e| format!("errors store dir: {e}"))?;
        let lock_path = self.dir.join("store.lock");
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)
            .map_err(|e| format!("open lock {}: {e}", lock_path.display()))?;
        file.lock()
            .map_err(|e| format!("lock {}: {e}", lock_path.display()))?;
        Ok(file)
    }

    fn projects_path(&self) -> PathBuf {
        self.dir.join("projects.json")
    }

    fn issues_path(&self, project_id: &str) -> PathBuf {
        self.dir.join("issues").join(format!("{project_id}.json"))
    }

    fn read_projects_unlocked(&self) -> Result<ProjectsFile, String> {
        let path = self.projects_path();
        if !path.is_file() {
            return Ok(ProjectsFile::default());
        }
        let raw =
            std::fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
        serde_json::from_str(&raw).map_err(|e| format!("parse {}: {e}", path.display()))
    }

    fn read_issues_unlocked(&self, project_id: &str) -> Result<IssuesFile, String> {
        let path = self.issues_path(project_id);
        if !path.is_file() {
            return Ok(IssuesFile::default());
        }
        let raw =
            std::fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
        serde_json::from_str(&raw).map_err(|e| format!("parse {}: {e}", path.display()))
    }

    /// Every registered project.
    pub fn projects(&self) -> Result<Vec<Project>, String> {
        if !self.projects_path().is_file() {
            return Ok(Vec::new());
        }
        let _lock = self.lock()?;
        Ok(self.read_projects_unlocked()?.projects)
    }

    /// Look up one project by id.
    pub fn project_by_id(&self, id: &str) -> Result<Option<Project>, String> {
        Ok(self.projects()?.into_iter().find(|p| p.id == id))
    }

    /// Look up one project by repository path.
    pub fn project_by_repo(&self, repo: &Path) -> Result<Option<Project>, String> {
        let target = normalize_repo_key(repo);
        Ok(self
            .projects()?
            .into_iter()
            .find(|p| same_repo_key(&p.repo, &target)))
    }

    /// Register `repo` if absent and return its `Project` (idempotent).
    pub fn register_project(&self, repo: &Path, name: Option<&str>) -> Result<Project, String> {
        let _lock = self.lock()?;
        let mut file = self.read_projects_unlocked()?;
        let normalized_repo = normalize_repo_key(repo);
        if let Some(existing) = file
            .projects
            .iter()
            .find(|p| same_repo_key(&p.repo, &normalized_repo))
            .cloned()
        {
            return Ok(existing);
        }
        let folder_name = repo
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| "project".to_string());
        let display_name = name
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .unwrap_or(&folder_name)
            .to_string();
        let base_slug = slugify(&display_name);
        let id = if file.projects.iter().any(|p| p.id == base_slug) {
            let suffix = &fnv1a_hex(normalized_repo.as_bytes())[..6];
            let candidate = format!("{base_slug}-{suffix}");
            if file.projects.iter().any(|p| p.id == candidate) {
                format!("{base_slug}-{}", next_seq())
            } else {
                candidate
            }
        } else {
            base_slug
        };
        let project = Project {
            id,
            key: generate_key(&normalized_repo),
            repo: normalized_repo,
            name: display_name,
            created_at: now_secs(),
        };
        file.projects.push(project.clone());
        write_json_atomic(&self.projects_path(), &file)?;
        Ok(project)
    }

    /// List issues for `project_id` (optionally filtered by status), newest first.
    pub fn list_issues(
        &self,
        project_id: &str,
        status: Option<IssueStatus>,
    ) -> Result<Vec<ErrorIssue>, String> {
        if !self.issues_path(project_id).is_file() {
            return Ok(Vec::new());
        }
        let _lock = self.lock()?;
        let mut issues = self.read_issues_unlocked(project_id)?.issues;
        if let Some(want) = status {
            issues.retain(|i| i.status == want);
        }
        issues.sort_by(|a, b| b.last_seen.cmp(&a.last_seen).then(a.id.cmp(&b.id)));
        Ok(issues)
    }

    /// Update an issue's status (`open` | `resolved` | `ignored`).
    pub fn set_status(
        &self,
        project_id: &str,
        issue_id: &str,
        status: IssueStatus,
    ) -> Result<ErrorIssue, String> {
        let _lock = self.lock()?;
        let mut file = self.read_issues_unlocked(project_id)?;
        let Some(issue) = file.issues.iter_mut().find(|i| i.id == issue_id) else {
            return Err(format!(
                "issue \"{issue_id}\" not found in project \"{project_id}\""
            ));
        };
        issue.status = status;
        let updated = issue.clone();
        write_json_atomic(&self.issues_path(project_id), &file)?;
        Ok(updated)
    }

    /// Ingest a Sentry envelope for `project_id`, authenticating `auth_key` if supplied or required.
    pub fn ingest_sentry_envelope(
        &self,
        project_id: &str,
        auth_key: Option<&str>,
        body: &[u8],
    ) -> Result<Vec<ErrorIssue>, String> {
        let project = self.authorized_project(project_id, auth_key)?;
        let events = parse_sentry_envelope(body)?;
        self.record_events(&project, events)
    }

    /// Ingest a legacy Sentry `/store/` JSON event for `project_id`.
    pub fn ingest_sentry_store(
        &self,
        project_id: &str,
        auth_key: Option<&str>,
        body: &[u8],
    ) -> Result<Vec<ErrorIssue>, String> {
        let project = self.authorized_project(project_id, auth_key)?;
        let event = parse_sentry_store(body)?;
        self.record_events(&project, vec![event])
    }

    /// Ingest one direct JSON event, registering `repo` automatically if given.
    pub fn ingest_direct(&self, direct: DirectEvent) -> Result<ErrorIssue, String> {
        let project = match (&direct.project_id, &direct.repo) {
            (Some(id), _) if !id.trim().is_empty() => self
                .project_by_id(id.trim())?
                .ok_or_else(|| format!("unknown project \"{id}\""))?,
            (_, Some(repo)) if !repo.trim().is_empty() => {
                self.register_project(Path::new(repo.trim()), None)?
            }
            _ => return Err("ingest requires either project_id or repo".into()),
        };
        let event = event_from_direct(direct);
        let mut issues = self.record_events(&project, vec![event])?;
        issues
            .pop()
            .ok_or_else(|| "direct event produced no issue".into())
    }

    fn authorized_project(
        &self,
        project_id: &str,
        auth_key: Option<&str>,
    ) -> Result<Project, String> {
        let project = self
            .project_by_id(project_id)?
            .ok_or_else(|| format!("unknown project \"{project_id}\""))?;
        match auth_key {
            Some(key) if key == project.key => Ok(project),
            Some(_) => Err("invalid sentry key".into()),
            None => Err("missing sentry key".into()),
        }
    }

    fn record_events(
        &self,
        project: &Project,
        events: Vec<ErrorEvent>,
    ) -> Result<Vec<ErrorIssue>, String> {
        if events.is_empty() {
            return Ok(Vec::new());
        }
        let _lock = self.lock()?;
        let mut file = self.read_issues_unlocked(&project.id)?;
        let mut touched = Vec::with_capacity(events.len());
        for raw_event in events {
            let event = scrub_event(raw_event);
            let fp = fingerprint(&event);
            let paths = extract_paths(&event);
            let now = event.timestamp;
            if let Some(existing) = file.issues.iter_mut().find(|i| i.id == fp) {
                existing.count = existing.count.saturating_add(1);
                existing.last_seen = existing.last_seen.max(now);
                existing.title = event.title.clone();
                if !event.culprit.is_empty() {
                    existing.culprit = event.culprit.clone();
                }
                existing.level = event.level.clone();
                existing.process = event.process.clone();
                if existing.status == IssueStatus::Resolved {
                    existing.status = IssueStatus::Open;
                }
                for path in paths {
                    if !existing.paths.contains(&path) {
                        existing.paths.push(path);
                    }
                }
                existing.latest = event;
                touched.push(existing.clone());
            } else {
                let issue = ErrorIssue {
                    id: fp,
                    project_id: project.id.clone(),
                    title: event.title.clone(),
                    culprit: event.culprit.clone(),
                    level: event.level.clone(),
                    process: event.process.clone(),
                    status: IssueStatus::Open,
                    count: 1,
                    first_seen: now,
                    last_seen: now,
                    paths,
                    latest: event,
                };
                file.issues.push(issue.clone());
                touched.push(issue);
            }
        }
        if file.issues.len() > MAX_ISSUES_PER_PROJECT {
            file.issues
                .sort_by(|a, b| b.last_seen.cmp(&a.last_seen).then(a.id.cmp(&b.id)));
            file.issues.truncate(MAX_ISSUES_PER_PROJECT);
        }
        write_json_atomic(&self.issues_path(&project.id), &file)?;
        Ok(touched)
    }
}

/// Outside context source backed by Insi's error store for one repository.
pub struct ErrorSource {
    pub store: Store,
    pub repo: PathBuf,
}

impl Source for ErrorSource {
    fn name(&self) -> &str {
        "errors"
    }

    fn read(&self, topics: &Topics) -> Result<Vec<Item>, String> {
        let Some(project) = self.store.project_by_repo(&self.repo)? else {
            return Ok(Vec::new());
        };
        let issues = self
            .store
            .list_issues(&project.id, Some(IssueStatus::Open))?;
        Ok(issues
            .into_iter()
            .map(|issue| Item {
                source: "errors".into(),
                trust: Trust::Untrusted,
                topic: topics.for_paths(&issue.paths),
                title: format!(
                    "[{} · {} ×{}] {}",
                    issue.level, issue.process, issue.count, issue.title
                ),
                body: format_issue_body(&issue),
                url: issue.latest.url.clone(),
            })
            .collect())
    }
}

fn format_issue_body(issue: &ErrorIssue) -> String {
    let mut lines = Vec::new();
    if !issue.culprit.is_empty() {
        lines.push(format!("where: {}", issue.culprit));
    }
    if let Some(rel) = &issue.latest.release {
        lines.push(format!("release: {rel}"));
    }
    if !issue.latest.message.is_empty() && issue.latest.message != issue.title {
        lines.push(issue.latest.message.clone());
    }
    if !issue.latest.frames.is_empty() {
        lines.push("stack:".into());
        for f in issue.latest.frames.iter().rev().take(8) {
            let func = f.function.as_deref().unwrap_or("<anon>");
            let loc = match (f.lineno, f.colno) {
                (Some(l), Some(c)) => format!("{}:{l}:{c}", f.filename),
                (Some(l), None) => format!("{}:{l}", f.filename),
                _ => f.filename.clone(),
            };
            lines.push(format!("  at {func} ({loc})"));
        }
    }
    if !issue.latest.breadcrumbs.is_empty() {
        lines.push("breadcrumbs:".into());
        for b in issue.latest.breadcrumbs.iter().rev().take(5).rev() {
            lines.push(format!("  - {b}"));
        }
    }
    lines.join("\n")
}

/// Parse a Sentry envelope body (`header\n(item_header\npayload\n)*`) into events.
pub fn parse_sentry_envelope(body: &[u8]) -> Result<Vec<ErrorEvent>, String> {
    let mut pos = find_newline(body, 0).ok_or("empty sentry envelope")?;
    let header_slice = trim_cr(&body[..pos]);
    let _header: Value = serde_json::from_slice(header_slice)
        .map_err(|e| format!("invalid envelope header: {e}"))?;
    pos += 1;

    let mut events = Vec::new();
    while pos < body.len() {
        while pos < body.len() && (body[pos] == b'\n' || body[pos] == b'\r') {
            pos += 1;
        }
        if pos >= body.len() {
            break;
        }
        let item_end = find_newline(body, pos).unwrap_or(body.len());
        let item_header_slice = trim_cr(&body[pos..item_end]);
        pos = if item_end < body.len() {
            item_end + 1
        } else {
            body.len()
        };
        if item_header_slice.is_empty() {
            continue;
        }
        let item_header: Value = serde_json::from_slice(item_header_slice)
            .map_err(|e| format!("invalid envelope item header: {e}"))?;
        let length = item_header.get("length").and_then(Value::as_u64);
        let payload = if let Some(len) = length {
            let len = usize::try_from(len).map_err(|_| "envelope item too large")?;
            if pos.saturating_add(len) > body.len() {
                return Err("truncated envelope item payload".into());
            }
            let slice = &body[pos..pos + len];
            pos += len;
            if pos < body.len() && body[pos] == b'\r' {
                pos += 1;
            }
            if pos < body.len() && body[pos] == b'\n' {
                pos += 1;
            }
            slice
        } else {
            let payload_end = find_newline(body, pos).unwrap_or(body.len());
            let slice = trim_cr(&body[pos..payload_end]);
            pos = if payload_end < body.len() {
                payload_end + 1
            } else {
                body.len()
            };
            slice
        };

        let item_type = item_header
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match item_type {
            "event" => {
                let value: Value = serde_json::from_slice(payload)
                    .map_err(|e| format!("invalid sentry event payload: {e}"))?;
                events.push(extract_event_from_value(&value));
            }
            "log" => {
                if let Ok(value) = serde_json::from_slice::<Value>(payload) {
                    if let Some(items) = value.get("items").and_then(Value::as_array) {
                        for entry in items {
                            let level =
                                entry.get("level").and_then(Value::as_str).unwrap_or("info");
                            if matches!(level, "error" | "fatal") {
                                events.push(extract_log_entry(entry, level));
                            }
                        }
                    }
                }
            }
            // Sessions, client reports, transactions, attachments: accept envelope without erroring.
            _ => {}
        }
    }
    Ok(events)
}

/// Parse a legacy Sentry `/store/` JSON event body.
pub fn parse_sentry_store(body: &[u8]) -> Result<ErrorEvent, String> {
    let value: Value =
        serde_json::from_slice(body).map_err(|e| format!("invalid sentry event json: {e}"))?;
    Ok(extract_event_from_value(&value))
}

fn extract_log_entry(entry: &Value, level: &str) -> ErrorEvent {
    let body = entry
        .get("body")
        .and_then(Value::as_str)
        .unwrap_or("logged error")
        .to_string();
    let title = first_line(&body);
    ErrorEvent {
        event_id: format!("log-{}", next_seq()),
        timestamp: now_secs(),
        level: level.to_string(),
        title,
        message: body,
        culprit: String::new(),
        process: "log".to_string(),
        release: None,
        environment: None,
        platform: None,
        frames: Vec::new(),
        breadcrumbs: Vec::new(),
        tags: BTreeMap::new(),
        url: None,
    }
}

fn extract_event_from_value(value: &Value) -> ErrorEvent {
    let event_id = value
        .get("event_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("evt-{}", next_seq()));
    let timestamp = parse_timestamp(value.get("timestamp")).unwrap_or_else(now_secs);
    let level = value
        .get("level")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or("error")
        .to_string();
    let tags = parse_tags(value.get("tags"));
    let platform = value
        .get("platform")
        .and_then(Value::as_str)
        .map(str::to_string);
    let process = tags
        .get("event.process")
        .cloned()
        .or_else(|| platform.clone())
        .unwrap_or_else(|| "app".to_string());

    let exceptions = value
        .get("exception")
        .and_then(|e| e.get("values"))
        .and_then(Value::as_array);

    let (title, message, mut frames) = if let Some(first_exc) = exceptions.and_then(|v| v.first()) {
        let exc_type = first_exc
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("Error");
        let exc_value = first_exc
            .get("value")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let title = if exc_value.is_empty() {
            exc_type.to_string()
        } else {
            format!("{exc_type}: {}", first_line(exc_value))
        };
        let frames = parse_frames(
            first_exc
                .get("stacktrace")
                .and_then(|s| s.get("frames"))
                .and_then(Value::as_array),
        );
        (title, exc_value.to_string(), frames)
    } else {
        let msg = value
            .get("logentry")
            .and_then(|l| l.get("formatted").or_else(|| l.get("message")))
            .and_then(Value::as_str)
            .or_else(|| {
                value.get("message").and_then(|m| {
                    m.as_str()
                        .or_else(|| m.get("formatted").and_then(Value::as_str))
                })
            })
            .unwrap_or("Unnamed error")
            .to_string();
        let frames = parse_frames(
            value
                .get("threads")
                .and_then(|t| t.get("values"))
                .and_then(Value::as_array)
                .and_then(|v| v.first())
                .and_then(|t| t.get("stacktrace"))
                .and_then(|s| s.get("frames"))
                .and_then(Value::as_array),
        );
        (first_line(&msg), msg, frames)
    };

    if frames.len() > MAX_FRAMES {
        let start = frames.len() - MAX_FRAMES;
        frames = frames.split_off(start);
    }

    let culprit = value
        .get("culprit")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| {
            value
                .get("transaction")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        })
        .or_else(|| culprit_from_frames(&frames))
        .unwrap_or_default();

    let breadcrumbs = parse_breadcrumbs(value.get("breadcrumbs"));
    let url = value
        .get("request")
        .and_then(|r| r.get("url"))
        .and_then(Value::as_str)
        .map(str::to_string);

    ErrorEvent {
        event_id,
        timestamp,
        level,
        title,
        message,
        culprit,
        process,
        release: value
            .get("release")
            .and_then(Value::as_str)
            .map(str::to_string),
        environment: value
            .get("environment")
            .and_then(Value::as_str)
            .map(str::to_string),
        platform,
        frames,
        breadcrumbs,
        tags,
        url,
    }
}

fn event_from_direct(direct: DirectEvent) -> ErrorEvent {
    let level = direct.level.unwrap_or_else(|| "error".into());
    let process = direct
        .process
        .or_else(|| direct.tags.get("event.process").cloned())
        .or_else(|| direct.platform.clone())
        .unwrap_or_else(|| "app".into());
    let message = direct.message.unwrap_or_else(|| direct.title.clone());
    let mut frames = direct.frames;
    if frames.len() > MAX_FRAMES {
        let start = frames.len() - MAX_FRAMES;
        frames = frames.split_off(start);
    }
    let culprit = direct
        .culprit
        .or_else(|| culprit_from_frames(&frames))
        .unwrap_or_default();
    ErrorEvent {
        event_id: format!("evt-{}", next_seq()),
        timestamp: now_secs(),
        level,
        title: first_line(&direct.title),
        message,
        culprit,
        process,
        release: direct.release,
        environment: direct.environment,
        platform: direct.platform,
        frames,
        breadcrumbs: direct.breadcrumbs,
        tags: direct.tags,
        url: direct.url,
    }
}

fn parse_frames(arr: Option<&Vec<Value>>) -> Vec<StackFrame> {
    let Some(arr) = arr else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|f| {
            let filename = f
                .get("filename")
                .or_else(|| f.get("abs_path"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let function = f
                .get("function")
                .and_then(Value::as_str)
                .map(str::to_string);
            if filename.is_empty() && function.is_none() {
                return None;
            }
            Some(StackFrame {
                filename,
                function,
                lineno: f.get("lineno").and_then(Value::as_u64),
                colno: f.get("colno").and_then(Value::as_u64),
                in_app: f.get("in_app").and_then(Value::as_bool).unwrap_or(true),
            })
        })
        .collect()
}

fn parse_tags(raw: Option<&Value>) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    match raw {
        Some(Value::Object(map)) => {
            for (k, v) in map {
                if let Some(s) = scalar_string(v) {
                    out.insert(k.clone(), s);
                }
            }
        }
        Some(Value::Array(items)) => {
            for pair in items {
                if let Some(arr) = pair.as_array() {
                    if let (Some(k), Some(v)) = (
                        arr.first().and_then(Value::as_str),
                        arr.get(1).and_then(scalar_string),
                    ) {
                        out.insert(k.to_string(), v);
                    }
                }
            }
        }
        _ => {}
    }
    out
}

fn parse_breadcrumbs(raw: Option<&Value>) -> Vec<String> {
    let arr = raw.and_then(|v| {
        v.get("values")
            .and_then(Value::as_array)
            .or_else(|| v.as_array())
    });
    let Some(arr) = arr else {
        return Vec::new();
    };
    let mut out: Vec<String> = arr
        .iter()
        .filter_map(|b| {
            let category = b.get("category").and_then(Value::as_str).unwrap_or("crumb");
            let msg = b
                .get("message")
                .and_then(Value::as_str)
                .or_else(|| b.get("type").and_then(Value::as_str))?;
            Some(format!("[{category}] {}", first_line(msg)))
        })
        .collect();
    if out.len() > MAX_BREADCRUMBS {
        out = out.split_off(out.len() - MAX_BREADCRUMBS);
    }
    out
}

fn culprit_from_frames(frames: &[StackFrame]) -> Option<String> {
    let preferred = frames
        .iter()
        .rev()
        .find(|f| f.in_app && !f.filename.is_empty())
        .or_else(|| frames.last());
    preferred.map(|f| match (&f.function, f.lineno) {
        (Some(func), Some(line)) => format!("{func} ({}:{line})", f.filename),
        (Some(func), None) if !f.filename.is_empty() => format!("{func} ({})", f.filename),
        (Some(func), None) => func.clone(),
        (None, Some(line)) => format!("{}:{line}", f.filename),
        (None, None) => f.filename.clone(),
    })
}

fn extract_paths(event: &ErrorEvent) -> Vec<String> {
    let mut paths = Vec::new();
    for frame in &event.frames {
        let p = frame
            .filename
            .trim()
            .trim_start_matches("app:///")
            .trim_start_matches("./")
            .replace('\\', "/");
        if !p.is_empty() && !p.starts_with('<') && !paths.contains(&p) {
            paths.push(p);
        }
    }
    paths
}

fn fingerprint(event: &ErrorEvent) -> String {
    let norm_title = normalize(&event.title);
    let norm_culprit = normalize(&event.culprit);
    let key = format!("{}\n{}\n{}", event.process, norm_title, norm_culprit);
    fnv1a_hex(key.as_bytes())[..16].to_string()
}

fn scrub_event(mut event: ErrorEvent) -> ErrorEvent {
    event.title = clip(&redact_text(&event.title), 300);
    event.message = clip(&redact_text(&event.message), MAX_MESSAGE);
    event.culprit = clip(&redact_text(&event.culprit), 300);
    event.process = clip(&redact_text(&event.process), 64);
    event.url = event.url.map(|u| clip(&redact_text(&u), 500));
    event.breadcrumbs = event
        .breadcrumbs
        .into_iter()
        .map(|b| clip(&redact_text(&b), 300))
        .collect();
    for frame in &mut event.frames {
        frame.filename = clip(&redact_text(&frame.filename), 300);
        frame.function = frame.function.take().map(|f| clip(&redact_text(&f), 200));
    }
    event.tags = event
        .tags
        .into_iter()
        .map(|(k, v)| (clip(&redact_text(&k), 80), clip(&redact_text(&v), 300)))
        .collect();
    event
}

/// Scrub credential and PII shapes line by line via `insi_core::secrets`.
pub fn redact_text(text: &str) -> String {
    text.lines()
        .map(|line| {
            if shape_in(line).is_some() || pii_in(line).is_some() {
                "[redacted]".to_string()
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn clip(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

fn first_line(s: &str) -> String {
    s.lines()
        .find(|l| !l.trim().is_empty())
        .map(|l| l.trim().to_string())
        .unwrap_or_default()
}

fn scalar_string(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn parse_timestamp(raw: Option<&Value>) -> Option<u64> {
    match raw? {
        Value::Number(n) => n.as_u64().or_else(|| n.as_f64().map(|f| f.max(0.0) as u64)),
        _ => None,
    }
}

fn find_newline(bytes: &[u8], start: usize) -> Option<usize> {
    bytes
        .get(start..)?
        .iter()
        .position(|&b| b == b'\n')
        .map(|offset| start + offset)
}

fn trim_cr(slice: &[u8]) -> &[u8] {
    slice.strip_suffix(b"\r").unwrap_or(slice)
}

fn normalize_repo_key(repo: &Path) -> String {
    let abs = std::path::absolute(repo).unwrap_or_else(|_| repo.to_path_buf());
    let s = abs.display().to_string().replace('\\', "/");
    let trimmed = s.trim_end_matches('/');
    if trimmed.is_empty() {
        "/".to_string()
    } else {
        trimmed.to_string()
    }
}

fn same_repo_key(a: &str, b: &str) -> bool {
    #[cfg(windows)]
    {
        a.eq_ignore_ascii_case(b)
    }
    #[cfg(not(windows))]
    {
        a == b
    }
}

fn slugify(name: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "project".to_string()
    } else {
        trimmed
    }
}

fn fnv1a_hex(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

fn next_seq() -> u64 {
    KEY_SEQ.fetch_add(1, Ordering::Relaxed)
}

fn generate_key(seed: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let a = fnv1a_hex(format!("{seed}:{nanos}:{}", next_seq()).as_bytes());
    let b = fnv1a_hex(format!("{}:{nanos}:{seed}", std::process::id()).as_bytes());
    format!("{a}{b}")
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("create dir {}: {e}", parent.display()))?;
    }
    let tmp = path.with_extension(format!("json.tmp-{}", next_seq()));
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    {
        let mut f = File::create(&tmp).map_err(|e| format!("create {}: {e}", tmp.display()))?;
        f.write_all(&bytes)
            .and_then(|()| f.sync_all())
            .map_err(|e| format!("write {}: {e}", tmp.display()))?;
    }
    std::fs::rename(&tmp, path).map_err(|e| format!("rename {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use insi_core::topic::TopicRule;

    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "insi-errors-test-{}-{}",
            std::process::id(),
            next_seq()
        ));
        std::fs::create_dir_all(&dir).expect("mkdir");
        dir
    }

    #[test]
    fn registers_projects_parses_sentry_envelope_groups_scrubs_and_reads_as_source() {
        let dir = scratch();
        let store = Store::new(&dir);
        let repo = dir.join("watchman");
        std::fs::create_dir_all(&repo).expect("repo");

        let p1 = store.register_project(&repo, None).expect("register");
        let p2 = store.register_project(&repo, None).expect("idempotent");
        assert_eq!(p1.id, p2.id);
        assert_eq!(p1.key, p2.key);
        assert!(p1.dsn("127.0.0.1:9185").contains(&p1.key));

        let envelope = concat!(
            "{\"event_id\":\"e1\",\"sent_at\":\"2026-11-26T19:00:00Z\"}\n",
            "{\"type\":\"session\"}\n",
            "{\"sid\":\"abc\"}\n",
            "{\"type\":\"event\"}\n",
            "{\"event_id\":\"e1\",\"level\":\"error\",\"platform\":\"javascript\",\"tags\":{\"event.process\":\"webview\"},\"exception\":{\"values\":[{\"type\":\"TypeError\",\"value\":\"Cannot read x\\nAuthorization: Bearer ghp_12345678901234567890123456\",\"stacktrace\":{\"frames\":[{\"filename\":\"app:///src/features/automations/ui/AuditorEditor.tsx\",\"function\":\"render\",\"lineno\":42,\"colno\":7,\"in_app\":true}]}}]},\"breadcrumbs\":{\"values\":[{\"category\":\"ui.click\",\"message\":\"Clicked Save\"}]}}\n"
        );
        // Wrong key is rejected.
        assert!(store
            .ingest_sentry_envelope(&p1.id, Some("wrong"), envelope.as_bytes())
            .is_err());

        // Same event twice increments count and groups under one issue.
        store
            .ingest_sentry_envelope(&p1.id, Some(&p1.key), envelope.as_bytes())
            .expect("ingest 1");
        store
            .ingest_sentry_envelope(&p1.id, Some(&p1.key), envelope.as_bytes())
            .expect("ingest 2");

        let open = store
            .list_issues(&p1.id, Some(IssueStatus::Open))
            .expect("list");
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].count, 2);
        assert_eq!(open[0].process, "webview");
        assert!(
            !open[0].latest.message.contains("ghp_"),
            "secret must be scrubbed: {}",
            open[0].latest.message
        );
        assert!(open[0].latest.message.contains("[redacted]"));

        // Feeds `ErrorSource` for `insi context` with topic matching and Untrusted fence.
        let src = ErrorSource {
            store: store.clone(),
            repo: repo.clone(),
        };
        let topics = Topics {
            base: "watchman",
            rules: &[TopicRule {
                topic: "watchman/automations".into(),
                paths: vec!["src/features/automations/".into()],
                commands: vec![],
            }],
        };
        let items = src.read(&topics).expect("read");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].trust, Trust::Untrusted);
        assert_eq!(items[0].topic, "watchman/automations");
        assert!(items[0].body.contains("Clicked Save"));

        // Resolving clears open errors; a later identical event reopens it (regression).
        let resolved = store
            .set_status(&p1.id, &open[0].id, IssueStatus::Resolved)
            .expect("resolve");
        assert_eq!(resolved.status, IssueStatus::Resolved);
        assert!(store
            .list_issues(&p1.id, Some(IssueStatus::Open))
            .expect("open")
            .is_empty());

        store
            .ingest_sentry_envelope(&p1.id, Some(&p1.key), envelope.as_bytes())
            .expect("regress");
        let after = store
            .list_issues(&p1.id, Some(IssueStatus::Open))
            .expect("reopened");
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].count, 3);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
