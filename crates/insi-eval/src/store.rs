//! Where tasks, hidden tests and results live on disk.
//!
//! - `<tasks>/<id>.toml` — the public part of each task.
//! - `<private>/<id>/<path>` — its hidden test files, kept outside every
//!   checkout a worker can read.
//! - a results file — one JSON line per attempt, appended, never rewritten.

use std::io::Write;
use std::path::{Path, PathBuf};

use insi_core::task::is_safe_relative;
use insi_core::{EvalResult, Task};

use crate::mine::Mined;

pub struct Store {
    pub tasks: PathBuf,
    pub private: PathBuf,
}

impl Store {
    /// Write a mined task: its public file and its hidden tests.
    pub fn save(&self, mined: &Mined) -> Result<(), String> {
        mined.task.validate()?;
        std::fs::create_dir_all(&self.tasks)
            .map_err(|e| format!("{}: {e}", self.tasks.display()))?;
        let text = toml::to_string(&mined.task).map_err(|e| e.to_string())?;
        let file = self.tasks.join(format!("{}.toml", mined.task.id));
        std::fs::write(&file, text).map_err(|e| format!("{}: {e}", file.display()))?;
        for (path, bytes) in &mined.hidden {
            let target = self.hidden_path(&mined.task.id, path)?;
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("{}: {e}", parent.display()))?;
            }
            std::fs::write(&target, bytes).map_err(|e| format!("{}: {e}", target.display()))?;
        }
        Ok(())
    }

    /// Every task, by id.
    pub fn load(&self) -> Result<Vec<Task>, String> {
        let mut tasks = Vec::new();
        let entries =
            std::fs::read_dir(&self.tasks).map_err(|e| format!("{}: {e}", self.tasks.display()))?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "toml") {
                let text = std::fs::read_to_string(&path)
                    .map_err(|e| format!("{}: {e}", path.display()))?;
                let task: Task =
                    toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
                task.validate()?;
                tasks.push(task);
            }
        }
        tasks.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(tasks)
    }

    /// A hidden test's contents.
    pub fn hidden(&self, task: &Task, path: &str) -> Result<Vec<u8>, String> {
        let file = self.hidden_path(&task.id, path)?;
        std::fs::read(&file).map_err(|e| format!("hidden test {}: {e}", file.display()))
    }

    fn hidden_path(&self, id: &str, path: &str) -> Result<PathBuf, String> {
        if !is_safe_relative(id) || !is_safe_relative(path) {
            return Err(format!(
                "refused a hidden test path outside the store: {id}/{path}"
            ));
        }
        Ok(self.private.join(id).join(path))
    }
}

/// Append one attempt to a results file.
pub fn append_result(file: &Path, result: &EvalResult) -> Result<(), String> {
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let line = serde_json::to_string(result).map_err(|e| e.to_string())?;
    let mut out = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(file)
        .map_err(|e| format!("{}: {e}", file.display()))?;
    writeln!(out, "{line}").map_err(|e| format!("{}: {e}", file.display()))
}

/// Every attempt in a results file; a line that does not parse is an error,
/// not a silently smaller sample.
pub fn read_results(file: &Path) -> Result<Vec<EvalResult>, String> {
    let text = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .enumerate()
        .map(|(i, l)| {
            serde_json::from_str(l).map_err(|e| format!("{} line {}: {e}", file.display(), i + 1))
        })
        .collect()
}
