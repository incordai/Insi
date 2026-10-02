//! A lesson: what a verified fix taught, in a form the next task can use.
//!
//! A lesson is written only after its fix is verified, never at the moment of
//! failure. In the benchmark the verified fix is the task's reference commit,
//! which passed the hidden check that the worker's attempt failed.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lesson {
    /// Unique per attempt it was learned from.
    pub id: String,
    /// Where it applies, e.g. the repository's name.
    pub topic: String,
    /// What went wrong, in a sentence.
    pub issue: String,
    pub root_cause: String,
    /// The action to take: "use X".
    #[serde(rename = "use")]
    pub use_: String,
    /// The action it replaces: "not Y".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avoid: Option<String>,
    /// The failure's normalized signature, to recognise it again.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub signature: String,
    /// The task it was learned from.
    pub evidence: String,
    /// When the evidence happened (RFC 3339): a lesson is only briefed to
    /// work that came after it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub learned_at: Option<String>,
    pub verified_by: Verification,
}

/// Who vouches for a lesson, strongest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verification {
    Human,
    /// An objective check passed: tests, a build.
    Check,
    AgentClaim,
}

impl Lesson {
    /// Whether this lesson may be briefed to a task dated `task_date`:
    /// only when it was learned strictly before. A lesson or a task with no
    /// readable date is never briefed — an undated lesson could be the
    /// task's own answer.
    pub fn precedes(&self, task_date: Option<&str>) -> bool {
        match (
            self.learned_at.as_deref().and_then(epoch_secs),
            task_date.and_then(epoch_secs),
        ) {
            (Some(learned), Some(task)) => learned < task,
            _ => false,
        }
    }
}

/// Seconds since 1970 for an RFC 3339 time such as `2026-10-02T09:15:00+05:30`
/// or `…Z`. Commit dates carry their committer's own offset, so they cannot
/// be compared as text.
pub fn epoch_secs(s: &str) -> Option<i64> {
    let s = s.trim();
    let num = |a: usize, b: usize| -> Option<i64> {
        let part = s.get(a..b)?;
        part.bytes()
            .all(|c| c.is_ascii_digit())
            .then(|| part.parse().ok())?
    };
    if s.len() < 20 || s.as_bytes()[4] != b'-' || !matches!(s.as_bytes()[10], b'T' | b't' | b' ') {
        return None;
    }
    let (y, mo, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (h, mi, se) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || se > 60 {
        return None;
    }
    // Skip fractional seconds, then read the offset.
    let mut rest = &s[19..];
    if let Some(frac) = rest.strip_prefix('.') {
        rest = frac.trim_start_matches(|c: char| c.is_ascii_digit());
    }
    let offset = match rest {
        "Z" | "z" => 0,
        _ if rest.len() == 6 && (rest.starts_with('+') || rest.starts_with('-')) => {
            let sign = if rest.starts_with('-') { -1 } else { 1 };
            let oh: i64 = rest.get(1..3)?.parse().ok()?;
            let om: i64 = rest.get(4..6)?.parse().ok()?;
            sign * (oh * 3600 + om * 60)
        }
        _ => return None,
    };
    Some(days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + se - offset)
}

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's
/// algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_are_read_with_their_offsets() {
        assert_eq!(epoch_secs("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(epoch_secs("2000-03-01T00:00:00Z"), Some(951_868_800));
        // The same instant written in two zones.
        assert_eq!(
            epoch_secs("2026-10-02T09:15:00+05:30"),
            epoch_secs("2026-10-02T03:45:00Z")
        );
        assert_eq!(
            epoch_secs("2026-10-01T22:00:00-07:00"),
            epoch_secs("2026-10-02T05:00:00.250Z")
        );
        for bad in [
            "",
            "2026-10-02",
            "2026-13-02T00:00:00Z",
            "2026-10-02T09:15:00",
            "yesterday",
        ] {
            assert_eq!(epoch_secs(bad), None, "{bad}");
        }
    }

    fn lesson(at: Option<&str>) -> Lesson {
        Lesson {
            id: "a".into(),
            topic: "repo".into(),
            issue: "i".into(),
            root_cause: "r".into(),
            use_: "u".into(),
            avoid: None,
            signature: String::new(),
            evidence: "t1".into(),
            learned_at: at.map(String::from),
            verified_by: Verification::Check,
        }
    }

    /// **NO LESSON FROM THE FUTURE.** A lesson reaches only tasks dated after
    /// it — compared as instants, not as text — and an undated one reaches
    /// none.
    #[test]
    fn a_lesson_is_briefed_only_to_later_work() {
        // 09:00 in +05:30 is 03:30Z: BEFORE 04:00Z, though "09" sorts after "04".
        let l = lesson(Some("2026-10-02T09:00:00+05:30"));
        assert!(l.precedes(Some("2026-10-02T04:00:00Z")));
        assert!(!l.precedes(Some("2026-10-02T03:00:00Z")));
        assert!(
            !l.precedes(Some("2026-10-02T03:30:00Z")),
            "the same instant is not after"
        );
        assert!(!l.precedes(None));
        assert!(!lesson(None).precedes(Some("2026-10-02T04:00:00Z")));
    }

    #[test]
    fn a_lesson_round_trips_with_use_as_its_key() {
        let json = serde_json::to_string(&lesson(Some("2026-10-02T09:00:00Z"))).expect("json");
        assert!(json.contains("\"use\":\"u\"") && json.contains("\"verified_by\":\"check\""));
        let back: Lesson = serde_json::from_str(&json).expect("parse");
        assert_eq!(back, lesson(Some("2026-10-02T09:00:00Z")));
    }
}
