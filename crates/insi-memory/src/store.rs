//! Where lessons are kept.

use std::io::Write;
use std::path::PathBuf;

use insi_core::Lesson;

pub trait LessonStore {
    /// Keep a lesson. A lesson whose id is already kept is not added twice.
    fn add(&self, lesson: &Lesson) -> Result<(), String>;
    /// Every kept lesson, oldest first.
    fn all(&self) -> Result<Vec<Lesson>, String>;
}

/// Lessons as JSON lines in one file, appended.
pub struct FileLessons {
    pub path: PathBuf,
}

impl LessonStore for FileLessons {
    fn add(&self, lesson: &Lesson) -> Result<(), String> {
        // The check and the append are one step, across processes too.
        let _held = insi_core::lock::lock_beside(&self.path)?;
        if self.all()?.iter().any(|l| l.id == lesson.id) {
            return Ok(());
        }
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        let line = serde_json::to_string(lesson).map_err(|e| e.to_string())?;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|e| format!("{}: {e}", self.path.display()))?;
        writeln!(file, "{line}").map_err(|e| format!("{}: {e}", self.path.display()))
    }

    fn all(&self) -> Result<Vec<Lesson>, String> {
        let text = match std::fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(format!("{}: {e}", self.path.display())),
        };
        text.lines()
            .enumerate()
            .filter(|(_, l)| !l.trim().is_empty())
            .map(|(i, l)| {
                serde_json::from_str(l)
                    .map_err(|e| format!("{} line {}: {e}", self.path.display(), i + 1))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use insi_core::Verification;

    fn lesson(id: &str) -> Lesson {
        Lesson {
            id: id.into(),
            topic: "repo".into(),
            issue: "i".into(),
            root_cause: "r".into(),
            use_: "u".into(),
            avoid: None,
            signature: String::new(),
            evidence: "t".into(),
            learned_at: None,
            verified_by: Verification::Check,
        }
    }

    #[test]
    fn lessons_are_kept_once_each() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let path = std::env::temp_dir()
            .join(format!("insi-lessons-{}-{nanos}", std::process::id()))
            .join("lessons.jsonl");
        let store = FileLessons { path: path.clone() };
        assert!(store.all().expect("absent is empty").is_empty());
        store.add(&lesson("a")).expect("add");
        store.add(&lesson("b")).expect("add");
        store.add(&lesson("a")).expect("again");
        let ids: Vec<String> = store
            .all()
            .expect("read")
            .into_iter()
            .map(|l| l.id)
            .collect();
        assert_eq!(ids, ["a", "b"]);
        let _ = std::fs::remove_file(&path);
    }
}
