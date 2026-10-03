//! `insi context` and the MCP tool `insi_context`: a repository's outside
//! sources, for one topic.
//!
//! Its own docs (trusted), its failed CI runs through `gh` (internal), and
//! the `[[connectors]]` its config names (untrusted unless it says
//! otherwise). Read-only; a source that cannot be read is named at the end,
//! and the others still answer.

use std::path::Path;

use insi_connectors::{ci, command, docs, gather, render, Query, Source};
use insi_core::Config;

/// Items shown when no limit is given.
pub const LIMIT: usize = 10;
/// How many recent failed CI runs are read.
pub const CI_RUNS: usize = 5;

/// What a request reads.
pub struct Ask<'a> {
    pub repo: &'a Path,
    pub config: &'a Config,
    /// The repository's topic, when the config names none.
    pub base: &'a str,
    pub topic: Option<&'a str>,
    pub words: &'a [String],
    pub limit: usize,
    pub with_ci: bool,
}

/// The rendered context.
pub fn text(a: &Ask) -> String {
    let topics = a.config.topics(a.base);
    let topic = a.topic.unwrap_or(topics.base);
    let docs = docs::Docs::new(a.repo);
    let gh = ci::GhRuns {
        repo: a.repo.to_path_buf(),
    };
    let ci = ci::Ci {
        runs: &gh,
        repo: a.repo.to_path_buf(),
        limit: CI_RUNS,
    };
    let commands: Vec<command::Command> = a
        .config
        .connectors
        .iter()
        .map(|c| command::Command {
            config: c.clone(),
            dir: a.repo.to_path_buf(),
        })
        .collect();
    let mut sources: Vec<&dyn Source> = vec![&docs];
    if a.with_ci {
        sources.push(&ci);
    }
    sources.extend(commands.iter().map(|c| c as &dyn Source));
    let got = gather(
        &sources,
        &topics,
        &Query {
            topic: topic.to_string(),
            words: a.words.to_vec(),
            limit: a.limit,
        },
    );
    let mut out = render(topic, &got.items);
    if !got.problems.is_empty() {
        out.push_str("\nNot read:\n");
        for p in &got.problems {
            out.push_str(&format!("- {}\n", p.lines().next().unwrap_or_default()));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **THE REPOSITORY'S DOCS AND ITS CONFIGURED SOURCES ANSWER TOGETHER;
    /// ONE THAT FAILS IS NAMED.** The stand-in source is `git`, which prints
    /// nothing for `config --get` of an unset key and exits 1.
    #[test]
    fn docs_and_configured_sources_are_read_for_a_topic() {
        let repo = std::env::temp_dir().join(format!(
            "insi-context-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::create_dir_all(repo.join("docs")).expect("dirs");
        std::fs::write(
            repo.join("README.md"),
            "# Shop\nRefunds round to the cent.\n",
        )
        .expect("w");
        let config = Config::parse(
            "[[connectors]]\nname = \"errors\"\ncommand = [\"git\", \"config\", \"--get\", \"insi.no-such-key\"]\n",
        )
        .expect("config");
        let out = text(&Ask {
            repo: &repo,
            config: &config,
            base: "shop",
            topic: None,
            words: &["refunds".to_string()],
            limit: LIMIT,
            with_ci: false,
        });
        assert!(out.contains("[1] docs · trusted · shop — Shop"), "{out}");
        assert!(out.contains("Refunds round to the cent."));
        assert!(
            out.contains("Not read:\n- errors: git exited Some(1)"),
            "{out}"
        );
    }
}
