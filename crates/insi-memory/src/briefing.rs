//! The text put in front of a task: what earlier work taught.
//!
//! Only lessons of the task's topic, learned strictly BEFORE the task, and
//! not from the task itself; newest first, one per action, within a size
//! budget. Each is phrased as an action, as the PRD asks: "Use X", "not Y".

use insi_core::Lesson;

/// A briefing for a task, and the lessons it holds.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Briefing {
    pub text: String,
    pub lesson_ids: Vec<String>,
}

/// Build the briefing for a task of `topic`, dated `task_date`, with id
/// `task_id`, from `lessons`, in at most `max_chars`. Empty when no lesson
/// applies.
pub fn build(
    lessons: &[Lesson],
    topic: &str,
    task_id: &str,
    task_date: Option<&str>,
    max_chars: usize,
) -> Briefing {
    let mut usable: Vec<&Lesson> = lessons
        .iter()
        .filter(|l| l.topic == topic && l.evidence != task_id && l.precedes(task_date))
        .collect();
    // Newest first: the code the task starts from is closest to them.
    usable.sort_by_key(|l| {
        std::cmp::Reverse(
            l.learned_at
                .as_deref()
                .and_then(insi_core::lesson::epoch_secs),
        )
    });
    const HEAD: &str = "Lessons from earlier work in this repository, each learned from a fix \
that passed its tests. Apply one only where it fits this task.\n";
    let mut text = String::from(HEAD);
    let mut ids = Vec::new();
    let mut seen_actions: Vec<String> = Vec::new();
    for l in usable {
        let action = l.use_.trim().to_lowercase();
        if seen_actions.contains(&action) {
            continue;
        }
        let line = match &l.avoid {
            Some(avoid) => format!(
                "- {} Not: {} (Seen when: {})\n",
                l.use_.trim(),
                avoid.trim(),
                l.issue.trim()
            ),
            None => format!("- {} (Seen when: {})\n", l.use_.trim(), l.issue.trim()),
        };
        if text.len() + line.len() > max_chars {
            break;
        }
        text.push_str(&line);
        ids.push(l.id.clone());
        seen_actions.push(action);
    }
    if ids.is_empty() {
        return Briefing::default();
    }
    Briefing {
        text,
        lesson_ids: ids,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use insi_core::Verification;

    fn lesson(id: &str, topic: &str, evidence: &str, at: &str, use_: &str) -> Lesson {
        Lesson {
            id: id.into(),
            topic: topic.into(),
            issue: format!("issue {id}"),
            root_cause: "r".into(),
            use_: use_.into(),
            avoid: (id == "new").then(|| "guessing".to_string()),
            signature: String::new(),
            evidence: evidence.into(),
            learned_at: Some(at.into()),
            verified_by: Verification::Check,
        }
    }

    /// **ONLY THE PAST, ONLY THIS TOPIC, NEVER ITS OWN ANSWER.** A lesson
    /// from a later task, another topic, or the task itself stays out;
    /// repeated actions appear once; the newest comes first.
    #[test]
    fn a_briefing_holds_only_earlier_lessons_of_the_topic() {
        let lessons = vec![
            lesson("old", "app", "t1", "2026-01-01T00:00:00Z", "Use the store."),
            lesson(
                "new",
                "app",
                "t2",
                "2026-03-01T00:00:00Z",
                "Read the config.",
            ),
            lesson("dup", "app", "t3", "2026-02-01T00:00:00Z", "use the store."),
            lesson(
                "later",
                "app",
                "t9",
                "2026-12-01T00:00:00Z",
                "From the future.",
            ),
            lesson("other", "web", "t4", "2026-01-01T00:00:00Z", "Other topic."),
            lesson(
                "self",
                "app",
                "t5",
                "2026-01-01T00:00:00Z",
                "Its own answer.",
            ),
        ];
        let b = build(&lessons, "app", "t5", Some("2026-06-01T00:00:00Z"), 4_000);
        assert_eq!(b.lesson_ids, ["new", "dup"]);
        assert!(b
            .text
            .contains("- Read the config. Not: guessing (Seen when: issue new)"));
        assert!(!b.text.contains("future") && !b.text.contains("Other topic"));
    }

    #[test]
    fn a_briefing_fits_its_budget_and_is_empty_without_lessons() {
        let lessons: Vec<Lesson> = (0..50)
            .map(|i| {
                lesson(
                    &format!("l{i}"),
                    "app",
                    &format!("t{i}"),
                    &format!("2026-01-{:02}T00:00:00Z", 1 + i % 28),
                    &format!("Action number {i} with some words."),
                )
            })
            .collect();
        let b = build(&lessons, "app", "x", Some("2026-06-01T00:00:00Z"), 600);
        assert!(
            b.text.len() <= 600 && !b.lesson_ids.is_empty(),
            "{}",
            b.text.len()
        );
        assert_eq!(
            build(&[], "app", "x", Some("2026-06-01T00:00:00Z"), 600),
            Briefing::default()
        );
    }
}
