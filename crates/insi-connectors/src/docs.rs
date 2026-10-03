//! The repository's own documentation: trusted, one item per section.
//!
//! Markdown files at the paths named (by default `README.md` and `docs/`),
//! split at their headings outside code blocks. A section's topic is its
//! file's, by the repository's topic rules.

use std::path::{Path, PathBuf};

use insi_core::topic::Topics;
use insi_core::Trust;

use crate::{Item, Source};

/// The paths read when none are named.
pub const DEFAULT_PATHS: [&str; 2] = ["README.md", "docs"];
/// At most this many files are read.
pub const MAX_FILES: usize = 500;
/// A file larger than this is not documentation to split into context.
pub const MAX_FILE_BYTES: u64 = 512 * 1024;

pub struct Docs {
    pub root: PathBuf,
    pub paths: Vec<String>,
}

impl Docs {
    pub fn new(root: &Path) -> Docs {
        Docs {
            root: root.to_path_buf(),
            paths: DEFAULT_PATHS.map(String::from).to_vec(),
        }
    }
}

impl Source for Docs {
    fn name(&self) -> &str {
        "docs"
    }

    fn read(&self, topics: &Topics) -> Result<Vec<Item>, String> {
        let mut files = Vec::new();
        for p in &self.paths {
            collect(&self.root.join(p), &mut files);
        }
        files.sort();
        files.truncate(MAX_FILES);
        let mut items = Vec::new();
        for file in files {
            let Ok(text) = std::fs::read_to_string(&file) else {
                continue;
            };
            let rel = file
                .strip_prefix(&self.root)
                .unwrap_or(&file)
                .to_string_lossy()
                .replace('\\', "/");
            let topic = topics.for_paths(std::slice::from_ref(&rel));
            for (title, body) in sections(&text) {
                items.push(Item {
                    source: "docs".into(),
                    trust: Trust::Trusted,
                    topic: topic.clone(),
                    title: if title.is_empty() { rel.clone() } else { title },
                    body,
                    url: Some(rel.clone()),
                });
            }
        }
        Ok(items)
    }
}

/// Markdown files at `path`: the file itself, or every one under the folder.
/// Hidden entries and links are not followed.
fn collect(path: &Path, files: &mut Vec<PathBuf>) {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return;
    };
    if meta.is_file() {
        if path.extension().is_some_and(|e| e == "md") && meta.len() <= MAX_FILE_BYTES {
            files.push(path.to_path_buf());
        }
    } else if meta.is_dir() {
        let Ok(entries) = std::fs::read_dir(path) else {
            return;
        };
        for entry in entries.flatten() {
            if files.len() >= MAX_FILES {
                return;
            }
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            collect(&entry.path(), files);
        }
    }
}

/// `(heading, text)` per section; the text before the first heading has an
/// empty heading. Sections with no text are left out.
pub fn sections(markdown: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut title = String::new();
    let mut body = String::new();
    let mut in_code = false;
    for line in markdown.lines() {
        if line.trim_start().starts_with("```") {
            in_code = !in_code;
        }
        let heading = !in_code && line.starts_with('#');
        if heading {
            if !body.trim().is_empty() {
                out.push((title.clone(), body.trim().to_string()));
            }
            title = line.trim_start_matches('#').trim().to_string();
            body.clear();
        } else {
            body.push_str(line);
            body.push('\n');
        }
    }
    if !body.trim().is_empty() {
        out.push((title, body.trim().to_string()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use insi_core::topic::TopicRule;

    #[test]
    fn markdown_splits_at_headings_outside_code() {
        let s =
            sections("intro\n# One\nfirst\n```\n# not a heading\n```\n## Two\nsecond\n# Empty\n");
        assert_eq!(s.len(), 3, "{s:?}");
        assert_eq!(s[0], ("".to_string(), "intro".to_string()));
        assert_eq!(s[1].0, "One");
        assert!(s[1].1.contains("# not a heading"));
        assert_eq!(s[2], ("Two".to_string(), "second".to_string()));
    }

    /// **A SECTION'S TOPIC IS ITS FILE'S.** Docs under a topic rule's folder
    /// are that topic's; hidden folders are not read.
    #[test]
    fn docs_are_read_with_the_topic_of_their_path() {
        let root = std::env::temp_dir().join(format!(
            "insi-docs-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::create_dir_all(root.join("docs/api")).expect("dirs");
        std::fs::create_dir_all(root.join("docs/.drafts")).expect("dirs");
        std::fs::write(root.join("README.md"), "# Shop\nSells things.\n").expect("w");
        std::fs::write(root.join("docs/api/orders.md"), "# Orders\nIdempotent.\n").expect("w");
        std::fs::write(root.join("docs/.drafts/x.md"), "# Draft\nsecret\n").expect("w");
        std::fs::write(root.join("docs/notes.txt"), "not markdown").expect("w");
        let rules = vec![TopicRule {
            topic: "shop/api".into(),
            paths: vec!["docs/api".into()],
            commands: vec![],
        }];
        let items = Docs::new(&root)
            .read(&Topics {
                base: "shop",
                rules: &rules,
            })
            .expect("read");
        let seen: Vec<(String, String)> = items
            .iter()
            .map(|i| (i.title.clone(), i.topic.clone()))
            .collect();
        assert_eq!(
            seen,
            [
                ("Shop".to_string(), "shop".to_string()),
                ("Orders".to_string(), "shop/api".to_string())
            ]
        );
        assert!(items.iter().all(|i| i.trust == Trust::Trusted));
    }
}
