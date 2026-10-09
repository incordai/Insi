//! The text put in front of a task: what earlier work taught.
//!
//! Only lessons learned strictly BEFORE the task, never from the task
//! itself, of the task's topic or a topic above it; within a size budget.
//! Each is phrased as an action, as the PRD asks: "Use X", "not Y".
//!
//! Lessons are consolidated as of the task's date — nothing about them is
//! stored, so the same lessons give every task the view it could have had:
//!
//! - **One item per action.** Lessons giving the same action are one item;
//!   how many times it was learned and when last are part of it.
//! - **Beliefs first.** An action learned twice or more is a belief, and
//!   comes before the single lessons.
//! - **A later contradiction supersedes.** A lesson whose action a later
//!   lesson names as the thing to avoid ("Not: X") is left out.
//! - **Recurring in siblings reaches the parent.** An action learned in two
//!   or more topics applies to every task under the topic they share
//!   (`shop/api` and `shop/web` → `shop`), not only to those topics.
//!
//! Called by `insi eval --lessons` and `insi issue --lessons`
//! (crates/insi/src/main.rs) and the MCP tool `insi_briefing`
//! (crates/insi/src/mcp.rs). Siblings that choose lessons by other rules:
//! `insi hook` (crates/insi/src/hook.rs, by file and failure signature) and
//! `insi_lessons_for_error` (mcp.rs `matching`) — they do not consolidate.

use std::collections::BTreeMap;

use insi_core::topic::{covers, related};
use insi_core::Lesson;

/// Times an action must be learned to be a belief.
pub const BELIEF_AT: usize = 2;
/// Distinct topics an action must be learned in to reach their parent.
pub const PROMOTE_AT: usize = 2;

/// A briefing for a task, and the lessons it holds.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Briefing {
    pub text: String,
    pub lesson_ids: Vec<String>,
}

/// One action as a task sees it: every lesson that gave it before the
/// task, newest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item<'a> {
    pub lessons: Vec<&'a Lesson>,
    /// The parent topic it reached by recurring in its children, when it
    /// applies to the task only that way.
    pub promoted_to: Option<String>,
}

impl Item<'_> {
    pub fn count(&self) -> usize {
        self.lessons.len()
    }
    pub fn is_belief(&self) -> bool {
        self.count() >= BELIEF_AT
    }
    pub fn newest(&self) -> &Lesson {
        self.lessons[0]
    }
}

fn when(l: &Lesson) -> Option<i64> {
    l.learned_at
        .as_deref()
        .and_then(insi_core::lesson::epoch_secs)
}

/// An action as text is compared: case, spacing and a closing full stop
/// do not make two actions different.
fn normal(action: &str) -> String {
    action
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_end_matches('.')
        .to_lowercase()
}

/// The items a task of `topic`, dated `task_date`, with id `task_id`, may be
/// briefed with: beliefs first, then the newest.
pub fn items<'a>(
    lessons: &'a [Lesson],
    topic: &str,
    task_id: &str,
    task_date: Option<&str>,
) -> Vec<Item<'a>> {
    let known: Vec<&Lesson> = lessons
        .iter()
        .filter(|l| l.evidence != task_id && l.precedes(task_date))
        .collect();
    // Superseded: a later lesson, on the same line of topics, says to avoid
    // exactly what this one says to do.
    let current: Vec<&Lesson> = known
        .iter()
        .copied()
        .filter(|l| {
            !known.iter().any(|m| {
                when(m) > when(l)
                    && related(&m.topic, &l.topic)
                    && m.avoid.as_deref().map(normal) == Some(normal(&l.use_))
            })
        })
        .collect();
    let mut by_action: BTreeMap<String, Vec<&Lesson>> = BTreeMap::new();
    for l in current {
        by_action.entry(normal(&l.use_)).or_default().push(l);
    }
    let mut out: Vec<Item> = Vec::new();
    for (_, mut group) in by_action {
        group.sort_by_key(|l| std::cmp::Reverse(when(l)));
        let here: Vec<&Lesson> = group
            .iter()
            .copied()
            .filter(|l| covers(&l.topic, topic))
            .collect();
        let mut topics: Vec<&str> = group.iter().map(|l| l.topic.as_str()).collect();
        topics.sort();
        topics.dedup();
        let shared = topics
            .iter()
            .skip(1)
            .fold(topics[0].to_string(), |acc, t| common(&acc, t));
        let reaches = topics.len() >= PROMOTE_AT && !shared.is_empty() && covers(&shared, topic);
        if reaches {
            let promoted_to = (here.is_empty()).then(|| shared.clone());
            out.push(Item {
                lessons: group,
                promoted_to,
            });
        } else if !here.is_empty() {
            out.push(Item {
                lessons: here,
                promoted_to: None,
            });
        }
    }
    out.sort_by(|a, b| {
        b.is_belief()
            .cmp(&a.is_belief())
            .then(when(b.newest()).cmp(&when(a.newest())))
    });
    out
}

/// The longest run of leading topic parts `a` and `b` share.
fn common(a: &str, b: &str) -> String {
    a.split('/')
        .zip(b.split('/'))
        .take_while(|(x, y)| x == y)
        .map(|(x, _)| x)
        .collect::<Vec<_>>()
        .join("/")
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
    const HEAD: &str = "Lessons from earlier work in this repository, each learned from a fix \
that passed its tests. Apply one only where it fits this task.\n";
    let mut text = String::from(HEAD);
    let mut ids = Vec::new();
    for item in items(lessons, topic, task_id, task_date) {
        let l = item.newest();
        let mut line = format!("- {}", l.use_.trim());
        if let Some(avoid) = &l.avoid {
            line.push_str(&format!(" Not: {}", avoid.trim()));
        }
        if item.is_belief() {
            line.push_str(&format!(
                " (Learned {} times; last when: {})",
                item.count(),
                l.issue.trim()
            ));
        } else {
            line.push_str(&format!(" (Seen when: {})", l.issue.trim()));
        }
        if let Some(parent) = &item.promoted_to {
            line.push_str(&format!(" [across {parent}]"));
        }
        line.push('\n');
        if text.len() + line.len() > max_chars {
            break;
        }
        text.push_str(&line);
        ids.extend(item.lessons.iter().map(|l| l.id.clone()));
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
        // The store was learned twice: a belief, first, carrying both.
        assert_eq!(b.lesson_ids, ["dup", "old", "new"]);
        assert!(
            b.text
                .contains("- use the store. (Learned 2 times; last when: issue dup)"),
            "{}",
            b.text
        );
        assert!(b
            .text
            .contains("- Read the config. Not: guessing (Seen when: issue new)"));
        assert!(!b.text.contains("future") && !b.text.contains("Other topic"));
    }

    fn avoiding(mut l: Lesson, avoid: &str) -> Lesson {
        l.avoid = Some(avoid.into());
        l
    }

    /// **A BELIEF IS COUNTED AS OF THE TASK.** A task between the first and
    /// the second learning sees one lesson, not a belief: what came later is
    /// not known yet.
    #[test]
    fn a_belief_is_only_what_was_known_before_the_task() {
        let lessons = vec![
            lesson("a", "app", "t1", "2026-01-01T00:00:00Z", "Use the store."),
            lesson("b", "app", "t2", "2026-03-01T00:00:00Z", "Use the store"),
        ];
        let feb = build(&lessons, "app", "x", Some("2026-02-01T00:00:00Z"), 4_000);
        assert_eq!(feb.lesson_ids, ["a"]);
        assert!(feb.text.contains("(Seen when: issue a)"), "{}", feb.text);
        let apr = build(&lessons, "app", "x", Some("2026-04-01T00:00:00Z"), 4_000);
        assert_eq!(apr.lesson_ids, ["b", "a"]);
        assert!(apr.text.contains("Learned 2 times"), "{}", apr.text);
    }

    /// **A LATER LESSON THAT SAYS "NOT X" RETIRES THE ONE THAT SAID "X"** —
    /// for tasks after it; tasks before it still get the old advice.
    #[test]
    fn a_later_contradiction_supersedes_the_earlier_lesson() {
        let lessons = vec![
            lesson(
                "old",
                "app",
                "t1",
                "2026-01-01T00:00:00Z",
                "Mock the clock.",
            ),
            avoiding(
                lesson(
                    "new",
                    "app",
                    "t2",
                    "2026-03-01T00:00:00Z",
                    "Use the real clock.",
                ),
                "mock the clock",
            ),
        ];
        let after = build(&lessons, "app", "x", Some("2026-06-01T00:00:00Z"), 4_000);
        assert_eq!(after.lesson_ids, ["new"]);
        assert!(!after.text.contains("- Mock the clock."), "{}", after.text);
        let before = build(&lessons, "app", "x", Some("2026-02-01T00:00:00Z"), 4_000);
        assert_eq!(before.lesson_ids, ["old"]);
    }

    /// **WHAT RECURS IN SIBLINGS REACHES THEIR PARENT.** Learned in
    /// `shop/api` and `shop/web`, an action applies to `shop` and to its
    /// other children; learned in one of them only, it stays there.
    #[test]
    fn an_action_recurring_in_siblings_reaches_their_parent() {
        let at = |m: u8| format!("2026-0{m}-01T00:00:00Z");
        let lessons = vec![
            lesson("api", "shop/api", "t1", &at(1), "Run the linter first."),
            lesson("web", "shop/web", "t2", &at(2), "Run the linter first."),
            lesson("one", "shop/api", "t3", &at(3), "Api only."),
        ];
        let jobs = build(&lessons, "shop/jobs", "x", Some(&at(6)), 4_000);
        assert_eq!(jobs.lesson_ids, ["web", "api"]);
        assert!(
            jobs.text.contains("Learned 2 times") && jobs.text.contains("[across shop]"),
            "{}",
            jobs.text
        );
        assert!(!jobs.text.contains("Api only"));
        let api = build(&lessons, "shop/api", "x", Some(&at(6)), 4_000);
        assert!(
            !api.text.contains("[across"),
            "it applies here directly: {}",
            api.text
        );
        let other = build(&lessons, "other", "x", Some(&at(6)), 4_000);
        assert_eq!(other, Briefing::default(), "not outside the shared parent");
    }

    /// **A TOPIC INHERITS ITS PARENTS' LESSONS, NOT ITS CHILDREN'S OR ITS
    /// SIBLINGS'.**
    #[test]
    fn a_briefing_holds_the_lessons_of_the_topics_above() {
        let at = "2026-01-01T00:00:00Z";
        let lessons = vec![
            lesson("repo", "shop", "t1", at, "Repo-wide."),
            lesson("api", "shop/api", "t2", at, "Api only."),
            lesson("billing", "shop/api/billing", "t3", at, "Billing only."),
            lesson("web", "shop/web", "t4", at, "Web only."),
        ];
        let ids = |topic: &str| {
            let mut ids =
                build(&lessons, topic, "x", Some("2026-06-01T00:00:00Z"), 4_000).lesson_ids;
            ids.sort();
            ids
        };
        assert_eq!(ids("shop/api"), ["api", "repo"]);
        assert_eq!(ids("shop"), ["repo"]);
        assert_eq!(ids("shop/api/billing"), ["api", "billing", "repo"]);
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
