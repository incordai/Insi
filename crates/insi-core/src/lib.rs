#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        reason = "test fixtures fail immediately on broken setup or assertions"
    )
)]
//! Insi's shared vocabulary: the per-repo config, a benchmark task and its
//! budget, the result of one run, and the error-signature normalizer that
//! lets one failure be recognised the next time it happens.
//!
//! Nothing here touches the network, git or a process: it is the part every
//! other crate (and, later, any project that adds Insi as a library) agrees
//! on. The one piece of filesystem machinery, [`lock`], is here because every
//! crate that writes a shared file needs the same one.

pub mod config;
pub mod lesson;
pub mod lock;
pub mod secrets;
pub mod signature;
pub mod task;
pub mod topic;
pub mod trust;

pub use config::{Config, Places, Price, Worker};
pub use lesson::{Lesson, Verification};
pub use signature::normalize;
pub use task::{Budget, EvalResult, Isolation, Task, Usage};
pub use trust::Trust;

/// Extract the first complete object of the requested type, ignoring braces
/// in surrounding prose. Proposal and lesson parsers retain their own validation.
pub fn first_json_object<T: serde::de::DeserializeOwned>(text: &str) -> Result<T, String> {
    for (start, _) in text.match_indices('{') {
        if let Some(Ok(value)) = serde_json::Deserializer::from_str(&text[start..])
            .into_iter::<T>()
            .next()
        {
            return Ok(value);
        }
    }
    // Retain the established missing-object diagnostic for callers and existing tooling.
    Err("the answer has no JSON object".into())
}

/// Normalize machine output once for eval, lesson writing and proposal generation.
/// Incomplete event streams never become plain-text answers.
pub fn worker_answer(stdout: &str) -> Option<String> {
    let events: Vec<serde_json::Value> = stdout
        .lines()
        .filter_map(|line| serde_json::from_str(line.trim()).ok())
        .collect();
    if events
        .iter()
        .any(|v| v["type"] == "turn.failed" || v["type"] == "error" || v["is_error"] == true)
    {
        return None;
    }
    let codex = events.iter().any(|v| {
        v["type"].as_str().is_some_and(|t| {
            t.starts_with("thread.") || t.starts_with("turn.") || t.starts_with("item.")
        })
    });
    if codex {
        if !events.iter().any(|v| v["type"] == "turn.completed") {
            return None;
        }
        return events.iter().rev().find_map(|v| {
            (v["type"] == "item.completed" && v["item"]["type"] == "agent_message")
                .then(|| v["item"]["text"].as_str().map(String::from))
                .flatten()
        });
    }
    for v in events.iter().rev() {
        if v["event"] == "result" || v.get("status").is_some() {
            let result = if v["event"] == "result" {
                &v["result"]
            } else {
                v
            };
            return (result["status"] == "SUCCESS")
                .then(|| result["response"].as_str().map(String::from))
                .flatten();
        }
        if v["type"] == "result" {
            return v["result"].as_str().map(String::from);
        }
    }
    if events
        .iter()
        .any(|v| v.get("type").is_some() || v.get("event").is_some())
    {
        return None;
    }
    let text = stdout.trim();
    (!text.is_empty()).then(|| text.into())
}

/// Detect explicit service/auth failures even when the CLI exits zero or leaves a diff.
/// Max-turn exhaustion remains an agent failure, so it can still be judged by tests.
pub fn worker_infrastructure_error(stdout: &str) -> bool {
    stdout
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .any(|v| {
            let failed = v["is_error"] == true
                || v["type"] == "turn.failed"
                || v["type"] == "error"
                || v["status"] == "ERROR"
                || v["result"]["status"] == "ERROR";
            if !failed || v["subtype"] == "error_max_turns" {
                return false;
            }
            let text = v.to_string().to_ascii_lowercase();
            [
                "rate_limit",
                "rate limit",
                "authentication",
                "unauthorized",
                "not logged in",
                "login",
                "overloaded",
                "api_error",
                "invalid_api_key",
                "unknown option",
                "unknown flag",
            ]
            .iter()
            .any(|needle| text.contains(needle))
        })
}

/// Service failures are infrastructure; exhaustion and incomplete output cannot produce lessons.
#[test]
fn worker_failures_and_completion_are_distinguished() {
    for line in [
        r#"{"type":"result","is_error":true,"subtype":"error_during_execution","error":"rate limit"}"#,
        r#"{"type":"turn.failed","error":{"message":"authentication required"}}"#,
        r#"{"event":"result","result":{"status":"ERROR","error":"API overloaded"}}"#,
    ] {
        assert!(worker_infrastructure_error(line));
        assert!(worker_answer(line).is_none());
    }
    assert!(!worker_infrastructure_error(
        r#"{"type":"result","is_error":true,"subtype":"error_max_turns"}"#
    ));
    assert!(worker_answer(
        r#"{"type":"item.completed","item":{"type":"agent_message","text":"preamble"}}"#
    )
    .is_none());
}

/// Prose braces and fenced output must not hide the first typed object.
#[test]
fn json_extraction_ignores_prose_and_keeps_the_first_object() {
    #[derive(Debug, serde::Deserialize, PartialEq)]
    struct Answer {
        value: u32,
    }
    for text in [
        "before {placeholder} {\"value\":1} after {anything}",
        "```json\n{\"value\":1}\n```",
        "{\"value\":1} {\"value\":2}",
    ] {
        assert_eq!(first_json_object::<Answer>(text), Ok(Answer { value: 1 }));
    }
    assert!(first_json_object::<Answer>("no object").is_err());
}
