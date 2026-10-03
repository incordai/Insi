//! A harness: the folder of prompts and settings a worker runs with.
//!
//! Held as its text files by repository-relative path, so two harnesses
//! compare by content and a candidate is the incumbent with one file
//! replaced. Hidden files and folders (`.git`) are not part of it.

use std::collections::BTreeMap;
use std::path::Path;

/// At most this many files in a harness.
pub const MAX_FILES: usize = 200;
/// At most this many bytes in one harness file.
pub const MAX_FILE_BYTES: usize = 64 * 1024;
/// The file whose text is put before every task's prompt.
pub const PROMPT_FILE: &str = "prompt.md";

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Harness {
    pub files: BTreeMap<String, String>,
}

impl Harness {
    /// Read `dir`'s text files. A file that is not UTF-8, too large, or one
    /// too many is an error: the loop must see the whole harness it changes.
    pub fn read(dir: &Path) -> Result<Harness, String> {
        let mut files = BTreeMap::new();
        read_into(dir, dir, &mut files)?;
        Ok(Harness { files })
    }

    /// Write every file under `dir`, which must not exist yet.
    pub fn write(&self, dir: &Path) -> Result<(), String> {
        if dir.exists() {
            return Err(format!("{} already exists", dir.display()));
        }
        for (rel, text) in &self.files {
            let path = dir.join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("{}: {e}", parent.display()))?;
            }
            std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
        }
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))
    }

    /// A short, stable name for this content: FNV-1a over every path and
    /// text, in order.
    pub fn id(&self) -> String {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for (rel, text) in &self.files {
            for part in [rel.as_bytes(), &[0xff], text.as_bytes(), &[0xfe]] {
                for byte in part {
                    hash ^= u64::from(*byte);
                    hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
                }
            }
        }
        format!("{hash:016x}")
    }

    /// The text put before every task's prompt; empty when there is none.
    pub fn prompt(&self) -> &str {
        self.files.get(PROMPT_FILE).map_or("", |t| t.trim())
    }

    /// This harness with `file` set to `content`.
    pub fn with(&self, file: &str, content: &str) -> Harness {
        let mut next = self.clone();
        next.files.insert(file.to_string(), content.to_string());
        next
    }
}

fn read_into(root: &Path, dir: &Path, files: &mut BTreeMap<String, String>) -> Result<(), String> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .flatten()
        .collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        let kind = entry
            .file_type()
            .map_err(|e| format!("{}: {e}", path.display()))?;
        if kind.is_dir() {
            read_into(root, &path, files)?;
        } else if kind.is_file() {
            if files.len() == MAX_FILES {
                return Err(format!(
                    "{} has more than {MAX_FILES} files",
                    root.display()
                ));
            }
            let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            if bytes.len() > MAX_FILE_BYTES {
                return Err(format!(
                    "{} is {} bytes; a harness file is at most {MAX_FILE_BYTES}",
                    path.display(),
                    bytes.len()
                ));
            }
            let text = String::from_utf8(bytes)
                .map_err(|_| format!("{} is not UTF-8 text", path.display()))?;
            let rel = path
                .strip_prefix(root)
                .map_err(|e| e.to_string())?
                .to_string_lossy()
                .replace('\\', "/");
            files.insert(rel, text);
        }
        // A link is neither: not followed out of the harness.
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "insi-harness-{}-{}-{label}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        ))
    }

    /// **A HARNESS IS ITS TEXT FILES, AND ITS ID IS ITS CONTENT.** Written
    /// out and read back it is the same; one changed file is another id;
    /// `.git` is not part of it.
    #[test]
    fn a_harness_round_trips_and_is_named_by_its_content() {
        let h = Harness::default()
            .with("prompt.md", "Keep the marker.\n")
            .with("routing/roles.toml", "coder = \"a\"\n");
        let d = dir("rt");
        h.write(&d).expect("write");
        std::fs::create_dir_all(d.join(".git")).expect("git");
        std::fs::write(d.join(".git/HEAD"), "ref").expect("head");
        let back = Harness::read(&d).expect("read");
        assert_eq!(back, h);
        assert_eq!(back.id(), h.id());
        assert_eq!(back.prompt(), "Keep the marker.");
        assert_ne!(h.with("prompt.md", "Other.").id(), h.id());
        assert!(h.write(&d).is_err(), "never over an existing folder");

        let big = dir("big");
        std::fs::create_dir_all(&big).expect("dir");
        std::fs::write(big.join("x.md"), vec![b'a'; MAX_FILE_BYTES + 1]).expect("w");
        assert!(Harness::read(&big)
            .expect_err("too big")
            .contains("at most"));
        std::fs::write(big.join("x.md"), [0xff, 0xfe]).expect("w");
        assert!(Harness::read(&big).expect_err("binary").contains("UTF-8"));
    }
}
