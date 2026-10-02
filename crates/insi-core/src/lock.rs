//! Mutual exclusion across processes, for files two `insi` runs may share.
//!
//! Two runs can share a work folder — a validation and an eval at the same
//! time — and with it the clone they check out from, the results file and
//! the lesson file. A lock inside one process does not reach the other, so
//! the lock is a file.
//!
//! The lock is taken on a SIDECAR file (`<name>.lock`), never on the data
//! itself: on Windows a locked range cannot even be read by another handle,
//! so locking the results file would make `insi report` fail while an eval
//! writes.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

/// Held while it lives; dropping it releases the lock.
pub struct FileLock {
    _file: File,
}

/// Take the lock beside `path`, waiting for any other holder.
pub fn lock_beside(path: &Path) -> Result<FileLock, String> {
    let lock_path = sidecar(path);
    if let Some(parent) = lock_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .map_err(|e| format!("{}: {e}", lock_path.display()))?;
    file.lock()
        .map_err(|e| format!("locking {}: {e}", lock_path.display()))?;
    Ok(FileLock { _file: file })
}

fn sidecar(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".lock");
    path.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    /// **ONE HOLDER AT A TIME, AND THE DATA STAYS READABLE.** Threads that
    /// each open their own handle never overlap inside the lock, and the
    /// guarded file can be read while the lock is held.
    #[test]
    fn holders_take_turns_and_the_data_stays_readable() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let dir = std::env::temp_dir().join(format!("insi-lock-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let data = dir.join("results.jsonl");
        std::fs::write(&data, "line\n").expect("w");

        let inside = Arc::new(AtomicUsize::new(0));
        let most = Arc::new(AtomicUsize::new(0));
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let (data, inside, most) = (data.clone(), inside.clone(), most.clone());
                std::thread::spawn(move || {
                    for _ in 0..5 {
                        let _held = lock_beside(&data).expect("lock");
                        let now = inside.fetch_add(1, Ordering::SeqCst) + 1;
                        most.fetch_max(now, Ordering::SeqCst);
                        assert_eq!(std::fs::read_to_string(&data).expect("readable"), "line\n");
                        std::thread::sleep(Duration::from_millis(2));
                        inside.fetch_sub(1, Ordering::SeqCst);
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().expect("thread");
        }
        assert_eq!(most.load(Ordering::SeqCst), 1, "two holders at once");
        assert!(dir.join("results.jsonl.lock").is_file());
    }
}
