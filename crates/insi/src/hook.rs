//! `insi hook`: a lesson at the moment it applies, through Claude Code's
//! hooks.
//!
//! The briefing puts lessons in front of a task once, at the start. This puts
//! one back in front of the agent at the step it is about:
//!
//! - **PreToolUse**, before an edit (`Edit`, `Write`, `MultiEdit`,
//!   `NotebookEdit`): lessons whose text names the file being edited — by
//!   its path in the repository, or by its file name.
//! - **PostToolUseFailure**, after a `Bash` command that failed: lessons
//!   whose failure signature shares lines with its output (the same match as
//!   `insi_lessons_for_error`). A failed command does not raise PostToolUse
//!   at all; Claude Code (measured on 2.1.288) raises PostToolUseFailure with
//!   `error` = `"Exit code N\n<output>"` and no `tool_response`.
//!
//! Advisory, never a gate: it adds context and cannot block, allow or change
//! a call. Each lesson is shown at most once per session (the ids already
//! shown are kept under the work folder), and at most [`MAX_SHOWN`] at a
//! time, so an agent editing the same file twenty times is told once.
//! Anything it cannot read — no lessons, an event it does not handle, a
//! malformed event — is no output and exit 0: a hook that fails must not
//! stop the agent's work.

use std::path::{Path, PathBuf};

use insi_core::Lesson;
use serde_json::{json, Value};

/// The most lessons one hook call shows.
pub const MAX_SHOWN: usize = 2;
/// How much of a failed command's output is matched: its end, where the
/// error is.
pub const OUTPUT_TAIL: usize = 16 * 1024;

/// The tools whose input names a file about to be changed.
const EDIT_TOOLS: [&str; 4] = ["Edit", "Write", "MultiEdit", "NotebookEdit"];

/// What one hook event asks about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Moment {
    /// About to change this file, relative to the repository (`/`-separated).
    Editing(String),
    /// A command just failed with this output.
    Failed(String),
}

/// What a hook event is about, if it is one this answers.
pub fn moment(event: &Value) -> Option<Moment> {
    let tool = event["tool_name"].as_str()?;
    match event["hook_event_name"].as_str()? {
        "PreToolUse" if EDIT_TOOLS.contains(&tool) => {
            let input = &event["tool_input"];
            let file = input["file_path"]
                .as_str()
                .or_else(|| input["notebook_path"].as_str())?;
            let cwd = event["cwd"].as_str().map(Path::new);
            Some(Moment::Editing(relative(Path::new(file), cwd)))
        }
        // An interrupt is the person stopping the command, not a failure the
        // command had.
        "PostToolUseFailure" if tool == "Bash" && event["is_interrupt"] != true => event["error"]
            .as_str()
            .and_then(output_tail)
            .map(Moment::Failed),
        _ => None,
    }
}

/// `file` relative to `cwd` when it is under it, `/`-separated.
fn relative(file: &Path, cwd: Option<&Path>) -> String {
    let rel = cwd
        .and_then(|cwd| file.strip_prefix(cwd).ok())
        .unwrap_or(file);
    rel.to_string_lossy().replace('\\', "/")
}

/// The last [`OUTPUT_TAIL`] bytes of a failed command's output, where the
/// error is; `None` when there is nothing to match.
fn output_tail(text: &str) -> Option<String> {
    let start = text
        .char_indices()
        .map(|(i, _)| i)
        .find(|&i| text.len() - i <= OUTPUT_TAIL)
        .unwrap_or(0);
    let tail = text[start..].trim().to_string();
    (!tail.is_empty()).then_some(tail)
}

/// The lessons of `topic` that bear on `moment`, best first, not counting
/// the ids in `shown`.
pub fn lessons_for<'a>(
    moment: &Moment,
    lessons: &'a [Lesson],
    topic: &str,
    shown: &[String],
) -> Vec<&'a Lesson> {
    let fresh = |l: &&Lesson| !shown.contains(&l.id);
    match moment {
        Moment::Editing(file) => {
            let name = file.rsplit('/').next().unwrap_or(file);
            let mut scored: Vec<(u8, &Lesson)> = lessons
                .iter()
                .filter(|l| l.topic == topic)
                .filter(fresh)
                .filter_map(|l| {
                    let text = [
                        l.issue.as_str(),
                        l.root_cause.as_str(),
                        l.use_.as_str(),
                        l.avoid.as_deref().unwrap_or(""),
                    ]
                    .join("\n");
                    if file.contains('/') && text.contains(file.as_str()) {
                        Some((2, l))
                    } else if names_file(&text, name) {
                        Some((1, l))
                    } else {
                        None
                    }
                })
                .collect();
            scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
            scored.into_iter().map(|(_, l)| l).take(MAX_SHOWN).collect()
        }
        Moment::Failed(output) => crate::mcp::matching(lessons, output, Some(topic))
            .into_iter()
            .filter(fresh)
            .take(MAX_SHOWN)
            .collect(),
    }
}

/// Whether `text` names the file `name` as a word of its own: `pty.ts` in
/// "exported from src/pty.ts." but not in "mypty.ts". A name without an
/// extension (`Makefile`, `README`) is too likely to be an ordinary word and
/// does not count.
fn names_file(text: &str, name: &str) -> bool {
    if name.len() < 4 || !name.contains('.') {
        return false;
    }
    let part_of_name = |c: char| c.is_alphanumeric() || c == '_' || c == '-' || c == '.';
    text.match_indices(name).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + name.len()..].chars().next();
        before.is_none_or(|c| !part_of_name(c) || c == '/')
            && after.is_none_or(|c| !part_of_name(c) || c == '.')
    })
}

/// The context to add for `chosen`, in the words the briefing uses.
pub fn context(moment: &Moment, chosen: &[&Lesson]) -> String {
    let head = match moment {
        Moment::Editing(file) => format!("Insi: earlier work on `{file}` taught:"),
        Moment::Failed(_) => "Insi: this failure has been seen before:".to_string(),
    };
    let mut out = head;
    for l in chosen {
        out.push_str(&format!(
            "\n- {} (Seen when: {})",
            l.use_.trim(),
            l.issue.trim()
        ));
        if let Some(avoid) = &l.avoid {
            out.push_str(&format!(" Not: {}", avoid.trim()));
        }
    }
    out
}

/// The hook's answer: `additionalContext` for the event, as Claude Code
/// reads it from stdout.
pub fn answer(event_name: &str, context: &str) -> Value {
    json!({
        "hookSpecificOutput": {
            "hookEventName": event_name,
            "additionalContext": context,
        }
    })
}

/// Where the ids already shown in `session` are kept: one file per session
/// under `work/hook-shown/`. The session id comes from the event, so only
/// letters, digits, `-` and `_` are kept of it — never a path.
pub fn shown_file(work: &Path, session: &str) -> Option<PathBuf> {
    let safe: String = session
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(128)
        .collect();
    (!safe.is_empty()).then(|| work.join("hook-shown").join(safe))
}

/// Answer one event. `lessons` is read only when the event is one this
/// answers. `Ok(None)`: nothing to say. `Err`: why nothing could be said —
/// for stderr, never for the agent.
pub fn respond(
    event: &Value,
    lessons: &dyn Fn() -> Result<Vec<Lesson>, String>,
    topic: &str,
    work: &Path,
) -> Result<Option<Value>, String> {
    let Some(moment) = moment(event) else {
        return Ok(None);
    };
    let all = lessons().map_err(|e| format!("the lessons could not be read: {e}"))?;
    let shown_path = event["session_id"]
        .as_str()
        .and_then(|s| shown_file(work, s));
    let shown: Vec<String> = shown_path
        .as_ref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|text| text.lines().map(String::from).collect())
        .unwrap_or_default();
    let chosen = lessons_for(&moment, &all, topic, &shown);
    if chosen.is_empty() {
        return Ok(None);
    }
    if let Some(path) = &shown_path {
        // Best effort: a lesson shown twice costs a few lines, and a hook
        // that failed over its own bookkeeping would cost the agent its call.
        let _ = remember_shown(path, &chosen);
    }
    let name = event["hook_event_name"].as_str().unwrap_or_default();
    Ok(Some(answer(name, &context(&moment, &chosen))))
}

fn remember_shown(path: &Path, chosen: &[&Lesson]) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    for l in chosen {
        writeln!(file, "{}", l.id)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use insi_core::Verification;

    fn lesson(id: &str, root_cause: &str, signature: &str) -> Lesson {
        Lesson {
            id: id.into(),
            topic: "app".into(),
            issue: format!("issue {id}"),
            root_cause: root_cause.into(),
            use_: format!("use {id}"),
            avoid: None,
            signature: signature.into(),
            evidence: format!("t-{id}"),
            learned_at: Some("2026-01-01T00:00:00Z".into()),
            verified_by: Verification::Check,
        }
    }

    fn edit(file: &str) -> Value {
        json!({
            "session_id": "s-1",
            "cwd": "/repo",
            "hook_event_name": "PreToolUse",
            "tool_name": "Edit",
            "tool_input": {"file_path": file, "old_string": "a", "new_string": "b"}
        })
    }

    #[test]
    fn an_edit_and_a_failed_command_are_the_moments_answered() {
        assert_eq!(
            moment(&edit("/repo/src/pty.ts")),
            Some(Moment::Editing("src/pty.ts".into()))
        );
        let read = json!({"hook_event_name": "PreToolUse", "tool_name": "Read",
                          "tool_input": {"file_path": "/repo/a.ts"}});
        assert_eq!(moment(&read), None, "reading changes nothing");
        // As Claude Code 2.1.288 sent them (paths shortened).
        let windows = json!({"hook_event_name": "PreToolUse", "tool_name": "Write",
                             "cwd": "C:\\work\\probe",
                             "tool_input": {"file_path": "C:\\work\\probe\\src\\a.txt", "content": "hi"}});
        if cfg!(windows) {
            assert_eq!(moment(&windows), Some(Moment::Editing("src/a.txt".into())));
        }
        let passed = json!({"hook_event_name": "PostToolUse", "tool_name": "Bash",
                            "tool_response": {"stdout": "ok", "stderr": ""}});
        assert_eq!(moment(&passed), None, "a command that passed");
        let failed = json!({"hook_event_name": "PostToolUseFailure", "tool_name": "Bash",
                            "tool_input": {"command": "ls /nope"},
                            "error": "Exit code 2\nls: cannot access '/nope': No such file or directory",
                            "is_interrupt": false});
        assert_eq!(
            moment(&failed),
            Some(Moment::Failed(
                "Exit code 2\nls: cannot access '/nope': No such file or directory".into()
            ))
        );
        let stopped = json!({"hook_event_name": "PostToolUseFailure", "tool_name": "Bash",
                             "error": "Interrupted", "is_interrupt": true});
        assert_eq!(moment(&stopped), None, "the person stopped it");
        assert_eq!(moment(&json!({"nonsense": true})), None);
    }

    /// **A LESSON THAT NAMES THE FILE COMES BACK BEFORE IT IS EDITED** — by
    /// path first, then by file name as a word of its own; a name inside a
    /// longer one, another topic's lesson, and one already shown do not.
    #[test]
    fn lessons_naming_the_file_are_chosen() {
        let all = vec![
            lesson("by-name", "Tests import decodePtyChunk from pty.ts.", ""),
            lesson("by-path", "src/pty.ts must export its helpers.", ""),
            lesson("inside", "mypty.ts is unrelated.", ""),
            Lesson {
                topic: "other".into(),
                ..lesson("other", "src/pty.ts again.", "")
            },
        ];
        let at = Moment::Editing("src/pty.ts".into());
        let ids: Vec<&str> = lessons_for(&at, &all, "app", &[])
            .iter()
            .map(|l| l.id.as_str())
            .collect();
        assert_eq!(ids, ["by-path", "by-name"]);
        let ids: Vec<&str> = lessons_for(&at, &all, "app", &["by-path".into()])
            .iter()
            .map(|l| l.id.as_str())
            .collect();
        assert_eq!(ids, ["by-name"]);
        assert!(!names_file("see Makefile", "Makefile"));
    }

    /// **A FAILURE SEEN BEFORE BRINGS ITS LESSON BACK, ONCE PER SESSION.**
    #[test]
    fn a_failed_command_is_answered_once_per_session() {
        let dir = std::env::temp_dir().join(format!(
            "insi-hook-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        ));
        let all = vec![lesson(
            "pty",
            "r",
            "TypeError: decodePtyChunk is not a function",
        )];
        let event = json!({
            "session_id": "s-9",
            "hook_event_name": "PostToolUseFailure",
            "tool_name": "Bash",
            "error": "Exit code 1\nFAIL src/pty.test.ts\nTypeError: decodePtyChunk is not a function",
            "is_interrupt": false
        });
        let lessons = || Ok(all.clone());
        let first = respond(&event, &lessons, "app", &dir)
            .expect("read")
            .expect("answered");
        assert_eq!(
            first["hookSpecificOutput"]["hookEventName"],
            "PostToolUseFailure"
        );
        let text = first["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .expect("context");
        assert!(
            text.contains("use pty") && text.contains("seen before"),
            "{text}"
        );
        assert_eq!(
            respond(&event, &lessons, "app", &dir),
            Ok(None),
            "shown again"
        );
        let unreadable = || Err("bad line 3".to_string());
        assert!(respond(&event, &unreadable, "app", &dir)
            .expect_err("said why")
            .contains("bad line 3"));
    }

    #[test]
    fn a_session_id_never_becomes_a_path() {
        let work = Path::new("/w");
        assert_eq!(
            shown_file(work, "../../etc/passwd"),
            Some(work.join("hook-shown").join("etcpasswd"))
        );
        assert_eq!(shown_file(work, "../.."), None);
    }
}
