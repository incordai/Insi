//! The audit trail: every state change of the loop, one JSON line each,
//! appended and never rewritten.
//!
//! What a person reads to see why the harness is what it is: which strategy
//! proposed what and why, how it scored against the incumbent, and whether
//! it was kept.

use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    /// Seconds since 1970.
    pub at: u64,
    pub step: u32,
    /// `baseline`, `proposed`, `refused`, `scored`, `accepted`, `rejected`,
    /// `finished`.
    pub event: String,
    /// The harness it is about, by [`crate::harness::Harness::id`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy: Option<String>,
    /// What happened, in words.
    pub detail: String,
}

impl Event {
    pub fn new(step: u32, event: &str, detail: impl Into<String>) -> Event {
        Event {
            at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or_default(),
            step,
            event: event.to_string(),
            harness: None,
            strategy: None,
            detail: detail.into(),
        }
    }

    pub fn harness(mut self, id: &str) -> Event {
        self.harness = Some(id.to_string());
        self
    }

    pub fn strategy(mut self, name: &str) -> Event {
        self.strategy = Some(name.to_string());
        self
    }
}

/// Append `event` to `file`, one whole line, under the file's lock.
pub fn append(file: &Path, event: &Event) -> Result<(), String> {
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let line = serde_json::to_string(event).map_err(|e| e.to_string())?;
    let _held = insi_core::lock::lock_beside(file)?;
    let mut out = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(file)
        .map_err(|e| format!("{}: {e}", file.display()))?;
    writeln!(out, "{line}").map_err(|e| format!("{}: {e}", file.display()))
}

/// Every event in `file`; none when it does not exist.
pub fn read(file: &Path) -> Result<Vec<Event>, String> {
    let text = match std::fs::read_to_string(file) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{}: {e}", file.display())),
    };
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .enumerate()
        .map(|(i, l)| {
            serde_json::from_str(l).map_err(|e| format!("{} line {}: {e}", file.display(), i + 1))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_append_and_read_back_in_order() {
        let file = std::env::temp_dir()
            .join(format!("insi-audit-{}", std::process::id()))
            .join("audit.jsonl");
        let _ = std::fs::remove_file(&file);
        append(&file, &Event::new(0, "baseline", "2/4").harness("aa")).expect("one");
        append(
            &file,
            &Event::new(1, "rejected", "2/4 against 2/4")
                .harness("bb")
                .strategy("simplify"),
        )
        .expect("two");
        let events = read(&file).expect("read");
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].strategy.as_deref(), Some("simplify"));
        assert_eq!(events[0].event, "baseline");
        assert!(read(&file.with_file_name("none.jsonl"))
            .expect("absent")
            .is_empty());
    }
}
