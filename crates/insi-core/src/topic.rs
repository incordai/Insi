//! Topics: where a lesson applies, resolved by rules, never by a model.
//!
//! A topic is a `/`-separated path, broadest first: `shop` → `shop/api` →
//! `shop/api/billing`. An item filed on a topic applies to it and to every
//! topic below it, so a task in `shop/api` sees the lessons of `shop/api`
//! and of `shop`, and not those of `shop/web`.
//!
//! The repository's topic (`topic` in `.insi/config.toml`) is the base.
//! `[[topic_rules]]` name narrower topics by the files and commands they
//! cover:
//!
//! ```toml
//! [[topic_rules]]
//! topic = "shop/api"
//! paths = ["src/api"]            # a folder covers everything under it
//! commands = ["npm run test:api"]
//! ```
//!
//! - **A path** matches a pattern when the pattern matches it or a folder
//!   above it. `*` and `?` stay inside one folder name; `**` spans folders.
//! - **A command** matches when the rule's words appear in it in order, side
//!   by side: `npm run test:api` matches `cd x && npm run test:api -- -u`.
//! - **The most specific rule wins**: the deepest topic, then the first rule
//!   in the file. Several paths resolve to the narrowest topic covering all
//!   of them; one outside every rule makes it the base.
//!
//! The same files give the same topic, every time.

use serde::{Deserialize, Serialize};

/// A narrower topic for the files and commands it names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TopicRule {
    pub topic: String,
    /// Repository-relative path patterns, `/`-separated.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    /// Command word sequences.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<String>,
}

impl TopicRule {
    /// Why this rule cannot be used, if it cannot.
    pub fn problem(&self) -> Option<String> {
        if let Some(why) = topic_problem(&self.topic) {
            return Some(why);
        }
        if self.paths.is_empty() && self.commands.is_empty() {
            return Some(format!(
                "topic rule \"{}\" names no paths and no commands, so it can never apply",
                self.topic
            ));
        }
        if self.commands.iter().any(|c| words(c).is_empty()) {
            return Some(format!(
                "topic rule \"{}\" has an empty command",
                self.topic
            ));
        }
        None
    }
}

/// Why `topic` is not a usable topic name, if it is not.
pub fn topic_problem(topic: &str) -> Option<String> {
    if topic.split('/').any(|seg| seg.trim().is_empty()) {
        Some(format!(
            "topic \"{topic}\" has an empty part; write it as a/b/c"
        ))
    } else if topic.chars().any(char::is_whitespace) {
        Some(format!("topic \"{topic}\" contains whitespace"))
    } else {
        None
    }
}

/// Whether an item filed on `parent` applies in `child`: the same topic, or
/// one below it. `shop` covers `shop/api`; `shop/a` does not cover `shop/ab`.
pub fn covers(parent: &str, child: &str) -> bool {
    child == parent
        || child
            .strip_prefix(parent)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// Whether `a` and `b` are on one line of descent: either covers the other.
pub fn related(a: &str, b: &str) -> bool {
    covers(a, b) || covers(b, a)
}

fn depth(topic: &str) -> usize {
    topic.split('/').count()
}

/// The base topic and the rules that narrow it.
#[derive(Debug, Clone, Copy)]
pub struct Topics<'a> {
    pub base: &'a str,
    pub rules: &'a [TopicRule],
}

impl Topics<'_> {
    /// The topic of work on `paths`: the narrowest one covering all of them.
    /// No paths, or one no rule covers, is the base.
    pub fn for_paths(&self, paths: &[String]) -> String {
        let mut each = paths.iter().map(|p| self.for_path(p));
        let Some(first) = each.next() else {
            return self.base.to_string();
        };
        let common = each.fold(first, |acc, t| common_topic(&acc, &t));
        if common.is_empty() {
            self.base.to_string()
        } else {
            common
        }
    }

    fn for_path(&self, path: &str) -> String {
        let path = path.replace('\\', "/");
        self.most_specific(|r| r.paths.iter().any(|p| path_matches(p, &path)))
    }

    /// The topic of a shell command.
    pub fn for_command(&self, command: &str) -> String {
        let said = words(command);
        self.most_specific(|r| {
            r.commands.iter().any(|c| {
                let want = words(c);
                !want.is_empty() && said.windows(want.len()).any(|w| w == want.as_slice())
            })
        })
    }

    fn most_specific(&self, applies: impl Fn(&TopicRule) -> bool) -> String {
        let mut best: Option<&TopicRule> = None;
        for rule in self.rules.iter().filter(|r| applies(r)) {
            if best.is_none_or(|b| depth(&rule.topic) > depth(&b.topic)) {
                best = Some(rule);
            }
        }
        best.map_or_else(|| self.base.to_string(), |r| r.topic.clone())
    }
}

/// The longest run of leading parts `a` and `b` share.
fn common_topic(a: &str, b: &str) -> String {
    a.split('/')
        .zip(b.split('/'))
        .take_while(|(x, y)| x == y)
        .map(|(x, _)| x)
        .collect::<Vec<_>>()
        .join("/")
}

fn words(text: &str) -> Vec<&str> {
    text.split_whitespace().collect()
}

/// Whether `pattern` matches `path` or a folder above it.
pub fn path_matches(pattern: &str, path: &str) -> bool {
    let pat: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();
    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    !pat.is_empty() && (1..=parts.len()).any(|n| segments(&pat, &parts[..n]))
}

fn segments(pat: &[&str], parts: &[&str]) -> bool {
    match pat.split_first() {
        None => parts.is_empty(),
        Some((&"**", rest)) => (0..=parts.len()).any(|i| segments(rest, &parts[i..])),
        Some((first, rest)) => parts
            .split_first()
            .is_some_and(|(head, tail)| segment(first, head) && segments(rest, tail)),
    }
}

/// One folder or file name against one pattern part: `*` is any run of
/// characters, `?` any one.
fn segment(pat: &str, name: &str) -> bool {
    let (p, n): (Vec<char>, Vec<char>) = (pat.chars().collect(), name.chars().collect());
    let (mut pi, mut ni) = (0, 0);
    // Where the last `*` was, and where in the name it last resumed.
    let mut star: Option<(usize, usize)> = None;
    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some((pi, ni));
            pi += 1;
        } else if let Some((sp, sn)) = star {
            pi = sp + 1;
            ni = sn + 1;
            star = Some((sp, sn + 1));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|c| *c == '*')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(topic: &str, paths: &[&str], commands: &[&str]) -> TopicRule {
        TopicRule {
            topic: topic.into(),
            paths: paths.iter().map(|s| s.to_string()).collect(),
            commands: commands.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn paths(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_topic_covers_itself_and_what_is_below_it() {
        assert!(covers("shop", "shop"));
        assert!(covers("shop", "shop/api"));
        assert!(covers("shop", "shop/api/billing"));
        assert!(!covers("shop/api", "shop"));
        assert!(!covers("shop/a", "shop/ab"), "a prefix is not a parent");
        assert!(!covers("shop", "shopping"));
        assert!(related("shop/api", "shop") && !related("shop/api", "shop/web"));
    }

    #[test]
    fn path_patterns_match_files_and_the_folders_above_them() {
        assert!(path_matches("src/api", "src/api/orders.ts"));
        assert!(path_matches("src/api/", "src/api/v1/orders.ts"));
        assert!(!path_matches("src/api", "src/apis/orders.ts"));
        assert!(path_matches("src/*.ts", "src/main.ts"));
        assert!(
            !path_matches("src/*.ts", "src/api/main.ts"),
            "* stays in a folder"
        );
        assert!(path_matches("**/*.test.ts", "src/api/a.test.ts"));
        assert!(
            path_matches("**/*.test.ts", "a.test.ts"),
            "** spans no folder too"
        );
        assert!(path_matches("src/**/db", "src/a/b/db/x.rs"));
        assert!(path_matches("src/?pi", "src/api/x"));
        assert!(path_matches("*", "anything/at/all"));
        assert!(!path_matches("", "src/a"));
        assert!(segment("a*b*c", "aXXbYYc") && !segment("a*b*c", "aXXbYY"));
    }

    /// **THE SAME FILES GIVE THE SAME TOPIC: THE MOST SPECIFIC RULE, THEN
    /// THE NARROWEST TOPIC COVERING ALL OF THEM.** A path outside every rule
    /// is the repository's own topic; so is work with no paths at all.
    #[test]
    fn paths_resolve_to_the_narrowest_topic_covering_them() {
        let rules = vec![
            rule("shop/api", &["src/api"], &["npm run test:api"]),
            rule("shop/api/billing", &["src/api/billing"], &[]),
            rule("shop/web", &["src/web", "public"], &["npm run dev"]),
        ];
        let t = Topics {
            base: "shop",
            rules: &rules,
        };
        assert_eq!(
            t.for_paths(&paths(&["src/api/billing/charge.ts"])),
            "shop/api/billing"
        );
        assert_eq!(t.for_paths(&paths(&["src/api/orders.ts"])), "shop/api");
        assert_eq!(
            t.for_paths(&paths(&["src/api/orders.ts", "src/api/billing/charge.ts"])),
            "shop/api"
        );
        assert_eq!(
            t.for_paths(&paths(&["src/api/orders.ts", "public/index.html"])),
            "shop"
        );
        assert_eq!(t.for_paths(&paths(&["README.md"])), "shop");
        assert_eq!(t.for_paths(&[]), "shop");
        assert_eq!(t.for_paths(&paths(&["src\\web\\a.tsx"])), "shop/web");

        assert_eq!(
            t.for_command("cd app && npm run test:api -- -u"),
            "shop/api"
        );
        assert_eq!(
            t.for_command("npm run test"),
            "shop",
            "words, not a prefix of one"
        );
        assert_eq!(t.for_command("npm  run   dev"), "shop/web");

        let unrelated = vec![rule("tools/lint", &["lint"], &[])];
        let t = Topics {
            base: "shop",
            rules: &unrelated,
        };
        assert_eq!(
            t.for_paths(&paths(&["lint/a.js", "src/b.js"])),
            "shop",
            "nothing in common is the base"
        );
    }

    #[test]
    fn a_rule_that_can_never_apply_is_refused() {
        assert_eq!(rule("shop/api", &["src/api"], &[]).problem(), None);
        assert!(rule("shop//api", &["x"], &[]).problem().is_some());
        assert!(rule("shop/api/", &["x"], &[]).problem().is_some());
        assert!(rule("shop api", &["x"], &[]).problem().is_some());
        assert!(rule("shop/api", &[], &[]).problem().is_some());
        assert!(rule("shop/api", &[], &["  "]).problem().is_some());
    }
}
