//! Setup output reused between attempts instead of rebuilt.
//!
//! Every attempt gets a fresh worktree, and setup (`npm ci`) used to run in
//! each one: measured on Watchman, 4 to 5.5 minutes of every 13 to 20 minute
//! attempt, installing the same lockfile each time. With a
//! [`SetupCache`], what setup produced (`node_modules`) is kept between
//! attempts under `<work>/deps/<key>/` and moved into the next worktree whose
//! key matches.
//!
//! - **Moved, not copied.** A rename on one volume is instant; copying tens
//!   of thousands of files is minutes.
//! - **The key** is the setup command plus the contents of the key files
//!   (the lockfile) at the attempt's commit. A different lockfile is a
//!   different key, and setup runs.
//! - **Returned only when still right.** After the attempt the key is read
//!   again from the worktree; a worker that changed the lockfile (installed
//!   something) changed what is in the folder, and it is not returned.
//! - **One holder at a time.** A slot is taken and returned under a file
//!   lock; an attempt that finds the slot empty — another attempt holds it —
//!   runs setup itself.

use std::path::{Path, PathBuf};

use insi_core::task::SetupCache;

/// The key for `cache` in `tree`: the setup argv and each key file's bytes,
/// hashed. `None` when a key file is missing — nothing to key on.
pub fn key(cache: &SetupCache, setup: &[String], tree: &Path) -> Option<String> {
    // FNV-1a, 64-bit: stable across runs and platforms.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |bytes: &[u8]| {
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        // A separator, so ["ab","c"] and ["a","bc"] differ.
        hash ^= 0xff;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    };
    for arg in setup {
        feed(arg.as_bytes());
    }
    for file in &cache.key {
        feed(file.as_bytes());
        feed(&std::fs::read(tree.join(file)).ok()?);
    }
    Some(format!("{hash:016x}"))
}

fn slot(root: &Path, key: &str) -> PathBuf {
    root.join("deps").join(key)
}

/// Move the kept folders for `key` from the cache into `tree`. `true` when
/// every one was there and moved; otherwise nothing is left half-moved and
/// setup must run.
pub fn take(root: &Path, cache: &SetupCache, key: &str, tree: &Path) -> bool {
    let slot = slot(root, key);
    if !slot.is_dir() {
        return false;
    }
    let Ok(_held) = insi_core::lock::lock_beside(&slot) else {
        return false;
    };
    if cache
        .keep
        .iter()
        .any(|k| !slot.join(k).exists() || tree.join(k).exists())
    {
        return false;
    }
    let mut moved: Vec<&String> = Vec::new();
    for k in &cache.keep {
        let to = tree.join(k);
        if let Some(parent) = to.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if std::fs::rename(slot.join(k), &to).is_err() {
            // Put back what already moved: the slot stays whole for the
            // next attempt, and this one runs setup.
            for done in moved {
                let _ = std::fs::rename(tree.join(done), slot.join(done));
            }
            return false;
        }
        moved.push(k);
    }
    true
}

/// Move the kept folders back from `tree` into the cache for `key` — only
/// when the key read from `tree` now is still `key`, and only into a slot
/// that is empty. `true` when they went back.
pub fn give_back(
    root: &Path,
    cache: &SetupCache,
    setup: &[String],
    key: &str,
    tree: &Path,
) -> bool {
    if self::key(cache, setup, tree).as_deref() != Some(key) {
        return false;
    }
    let slot = slot(root, key);
    if std::fs::create_dir_all(&slot).is_err() {
        return false;
    }
    let Ok(_held) = insi_core::lock::lock_beside(&slot) else {
        return false;
    };
    if cache
        .keep
        .iter()
        .any(|k| slot.join(k).exists() || !tree.join(k).is_dir())
    {
        return false;
    }
    let mut moved: Vec<&String> = Vec::new();
    for k in &cache.keep {
        let to = slot.join(k);
        if let Some(parent) = to.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if std::fs::rename(tree.join(k), &to).is_err() {
            // A folder still in use (a test process holding a file) is left
            // with the tree; the slot must hold all or none.
            for done in moved {
                let _ = std::fs::rename(slot.join(done), tree.join(done));
            }
            return false;
        }
        moved.push(k);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(label: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "insi-deps-{}-{}-{label}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::create_dir_all(&d).expect("dir");
        d
    }

    fn cache() -> SetupCache {
        SetupCache {
            key: vec!["package-lock.json".into()],
            keep: vec!["node_modules".into()],
        }
    }

    fn tree(label: &str, lock: &str) -> PathBuf {
        let t = dir(label);
        std::fs::write(t.join("package-lock.json"), lock).expect("lock");
        t
    }

    /// **WHAT ONE ATTEMPT INSTALLED, THE NEXT ONE REUSES — WHILE THE
    /// LOCKFILE IS THE SAME.** Returned to the cache after the first
    /// attempt, moved into the second; a different lockfile is a different
    /// key and finds nothing.
    #[test]
    fn kept_output_moves_between_attempts_with_the_same_key() {
        let root = dir("root");
        let setup = vec!["npm".to_string(), "ci".to_string()];
        let first = tree("first", "{\"v\":1}");
        std::fs::create_dir_all(first.join("node_modules/pkg")).expect("installed");
        std::fs::write(first.join("node_modules/pkg/index.js"), "x").expect("file");
        let k = key(&cache(), &setup, &first).expect("key");

        assert!(give_back(&root, &cache(), &setup, &k, &first));
        assert!(!first.join("node_modules").exists(), "moved, not copied");

        let second = tree("second", "{\"v\":1}");
        assert_eq!(key(&cache(), &setup, &second).as_deref(), Some(k.as_str()));
        assert!(take(&root, &cache(), &k, &second));
        assert!(second.join("node_modules/pkg/index.js").is_file());
        assert!(
            !take(&root, &cache(), &k, &dir("third")),
            "one holder at a time"
        );

        let other = tree("other", "{\"v\":2}");
        let k2 = key(&cache(), &setup, &other).expect("key");
        assert_ne!(k, k2, "a different lockfile is a different key");
        assert!(!take(&root, &cache(), &k2, &other));
        assert_ne!(
            Some(k.clone()),
            key(
                &cache(),
                &["pnpm".to_string(), "install".to_string()],
                &second
            ),
            "a different setup command is a different key"
        );
    }

    /// **A WORKER THAT CHANGED THE LOCKFILE DOES NOT RETURN ITS FOLDER.**
    /// What it installed is not what the key says.
    #[test]
    fn a_changed_lockfile_keeps_the_folder_out_of_the_cache() {
        let root = dir("root2");
        let setup = vec!["npm".to_string(), "ci".to_string()];
        let t = tree("changed", "{\"v\":1}");
        std::fs::create_dir_all(t.join("node_modules")).expect("installed");
        let k = key(&cache(), &setup, &t).expect("key");
        std::fs::write(t.join("package-lock.json"), "{\"v\":1,\"extra\":true}").expect("w");
        assert!(!give_back(&root, &cache(), &setup, &k, &t));
        assert!(t.join("node_modules").is_dir(), "left with the tree");
        assert_eq!(
            key(&cache(), &setup, &dir("nolock")),
            None,
            "no lockfile, no key"
        );
    }
}
