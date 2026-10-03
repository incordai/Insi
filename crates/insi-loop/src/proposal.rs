//! The ONE change a proposer answers with, and the checks it passes before
//! it touches anything.
//!
//! A proposal replaces one harness file with new text:
//!
//! ```json
//! {"file": "prompt.md", "content": "…the whole new file…", "why": "…"}
//! ```
//!
//! Whole-file replacement, not a diff: nothing to misapply, and the
//! candidate is exactly what was proposed. The file must be a plain relative
//! path inside the harness, the text bounded, and the change real.

use serde::Deserialize;

use crate::harness::{Harness, MAX_FILE_BYTES};

/// The longest reason accepted.
pub const MAX_WHY: usize = 1_000;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Proposal {
    pub file: String,
    pub content: String,
    pub why: String,
}

/// The proposal in a model's answer: its first JSON object, checked against
/// the harness it would change.
pub fn parse(answer: &str, incumbent: &Harness) -> Result<Proposal, String> {
    let start = answer
        .find('{')
        .ok_or_else(|| "the answer has no JSON object".to_string())?;
    let end = answer
        .rfind('}')
        .filter(|&e| e > start)
        .ok_or_else(|| "the answer's JSON object is not closed".to_string())?;
    let p: Proposal = serde_json::from_str(&answer[start..=end])
        .map_err(|e| format!("the answer is not a proposal: {e}"))?;
    if let Some(why) = path_problem(&p.file) {
        return Err(why);
    }
    if p.content.len() > MAX_FILE_BYTES {
        return Err(format!(
            "the proposed {} is {} bytes; at most {MAX_FILE_BYTES}",
            p.file,
            p.content.len()
        ));
    }
    if p.why.trim().is_empty() || p.why.len() > MAX_WHY {
        return Err(format!(
            "the proposal's \"why\" must say why, in at most {MAX_WHY} bytes"
        ));
    }
    if incumbent.files.get(&p.file) == Some(&p.content) {
        return Err(format!("the proposal leaves {} as it is", p.file));
    }
    Ok(p)
}

/// Why `file` is not a plain path inside the harness, if it is not.
fn path_problem(file: &str) -> Option<String> {
    let bad = file.is_empty()
        || file.len() > 200
        || file.starts_with('/')
        || file.contains('\\')
        || file.contains(':')
        || file
            .split('/')
            .any(|part| part.is_empty() || part.starts_with('.'));
    bad.then(|| format!("\"{file}\" is not a plain relative path inside the harness (a/b.md)"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn incumbent() -> Harness {
        Harness::default().with("prompt.md", "old")
    }

    #[test]
    fn a_proposal_is_read_from_the_answer() {
        let p = parse(
            "Here it is:\n{\"file\": \"prompt.md\", \"content\": \"new\", \"why\": \"clearer\"}\nDone.",
            &incumbent(),
        )
        .expect("valid");
        assert_eq!(p.file, "prompt.md");
        let added = parse(
            r#"{"file": "checklists/stop.md", "content": "x", "why": "w"}"#,
            &incumbent(),
        );
        assert!(added.is_ok(), "a new file is a change");
    }

    /// **NOTHING OUTSIDE THE HARNESS, NOTHING HIDDEN, NOTHING EMPTY.**
    #[test]
    fn a_proposal_that_reaches_outside_or_changes_nothing_is_refused() {
        let with = |file: &str| {
            parse(
                &format!(r#"{{"file": "{file}", "content": "x", "why": "w"}}"#),
                &incumbent(),
            )
        };
        for bad in [
            "../evil.md",
            "/etc/passwd",
            "a/../../b",
            ".git/config",
            "a//b",
            "C:x",
            "",
        ] {
            assert!(with(bad).is_err(), "{bad}");
        }
        assert!(with("a\\\\b").is_err());
        assert!(parse(
            r#"{"file":"prompt.md","content":"old","why":"w"}"#,
            &incumbent()
        )
        .expect_err("same")
        .contains("as it is"));
        assert!(parse(
            r#"{"file":"prompt.md","content":"n","why":" "}"#,
            &incumbent()
        )
        .is_err());
        assert!(parse(
            r#"{"file":"prompt.md","content":"n","why":"w","also":"x"}"#,
            &incumbent()
        )
        .is_err());
        assert!(parse("no json here", &incumbent()).is_err());
    }
}
