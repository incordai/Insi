//! A source read by running a command that prints one JSON object per
//! line — an error tracker's export, a mailbox's, anything with a CLI:
//!
//! ```json
//! {"title": "TypeError in refund()", "body": "…", "url": "…", "paths": ["src/api/refund.ts"]}
//! ```
//!
//! `topic` may be given instead of `paths`; with neither, the item is the
//! repository's. The command holds whatever login it needs; Insi only reads
//! its output, and believes it only as far as the config's `trust`.

use std::path::PathBuf;
use std::time::Duration;

use insi_core::config::CommandConnector;
use insi_core::topic::Topics;
use serde::Deserialize;

use crate::{Item, Source};

/// How long the command may take.
pub const COMMAND_SECS: u64 = 120;
/// At most this many items are taken from one command.
pub const MAX_ITEMS: usize = 200;

#[derive(Debug, Deserialize)]
struct Line {
    title: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    paths: Vec<String>,
    #[serde(default)]
    topic: Option<String>,
}

pub struct Command {
    pub config: CommandConnector,
    /// Where the command runs: the repository.
    pub dir: PathBuf,
}

impl Source for Command {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn read(&self, topics: &Topics) -> Result<Vec<Item>, String> {
        let out = insi_eval::process::run(
            &self.config.command,
            &self.dir,
            Duration::from_secs(COMMAND_SECS),
        )?;
        if !out.success() {
            return Err(format!(
                "{} {}",
                self.config.command[0],
                if out.timed_out {
                    "timed out".to_string()
                } else {
                    format!("exited {:?}", out.code)
                }
            ));
        }
        parse(&out.stdout, &self.config, topics)
    }
}

/// The items in a command's output. A line that is not an item is an
/// error: a source that half-parses would quietly drop what it was for.
pub fn parse(
    stdout: &str,
    config: &CommandConnector,
    topics: &Topics,
) -> Result<Vec<Item>, String> {
    let mut items = Vec::new();
    for (i, line) in stdout.lines().filter(|l| !l.trim().is_empty()).enumerate() {
        if items.len() == MAX_ITEMS {
            break;
        }
        let l: Line = serde_json::from_str(line)
            .map_err(|e| format!("{} line {}: {e}", config.name, i + 1))?;
        let topic = match l.topic {
            // A topic it names outside this repository's is the repository's:
            // a source does not get to file items elsewhere.
            Some(t) if insi_core::topic::covers(topics.base, &t) => t,
            _ => topics.for_paths(&l.paths),
        };
        items.push(Item {
            source: config.name.clone(),
            trust: config.trust,
            topic,
            title: l.title,
            body: l.body,
            url: l.url,
        });
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;
    use insi_core::topic::TopicRule;
    use insi_core::Trust;

    fn connector() -> CommandConnector {
        CommandConnector {
            name: "errors".into(),
            command: vec!["export".into()],
            trust: Trust::Untrusted,
        }
    }

    /// **A COMMAND'S ITEMS CARRY ITS TRUST, AND ITS TOPIC CANNOT LEAVE THE
    /// REPOSITORY.** Paths resolve by the rules; a topic outside the
    /// repository's falls back to it; a line that is not an item is an error.
    #[test]
    fn command_output_becomes_items() {
        let rules = vec![TopicRule {
            topic: "shop/api".into(),
            paths: vec!["src/api".into()],
            commands: vec![],
        }];
        let topics = Topics {
            base: "shop",
            rules: &rules,
        };
        let out = "{\"title\": \"TypeError\", \"body\": \"at refund\", \"paths\": [\"src/api/refund.ts\"]}\n\
                   \n\
                   {\"title\": \"elsewhere\", \"topic\": \"other-repo/x\"}\n\
                   {\"title\": \"named\", \"topic\": \"shop/web\"}\n";
        let items = parse(out, &connector(), &topics).expect("parsed");
        let seen: Vec<(&str, &str)> = items
            .iter()
            .map(|i| (i.title.as_str(), i.topic.as_str()))
            .collect();
        assert_eq!(
            seen,
            [
                ("TypeError", "shop/api"),
                ("elsewhere", "shop"),
                ("named", "shop/web")
            ]
        );
        assert!(items
            .iter()
            .all(|i| i.trust == Trust::Untrusted && i.source == "errors"));
        assert!(parse("not json", &connector(), &topics)
            .expect_err("bad line")
            .contains("errors line 1"));
    }

    /// The command really runs: `git` stands in for an exporter.
    #[test]
    fn a_command_that_fails_is_a_problem_not_items() {
        let c = Command {
            config: CommandConnector {
                command: vec!["git".into(), "no-such-subcommand-x".into()],
                ..connector()
            },
            dir: std::env::current_dir().expect("cwd"),
        };
        let topics = Topics {
            base: "shop",
            rules: &[],
        };
        assert!(c.read(&topics).expect_err("fails").contains("exited"));
    }
}
