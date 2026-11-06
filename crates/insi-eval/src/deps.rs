//! Pristine setup snapshots, published before workers or checks run.
//! Attempts receive private copies and never return worker-exposed output.

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
    feed(std::env::consts::OS.as_bytes());
    feed(std::env::consts::ARCH.as_bytes());
    for folder in &cache.keep {
        feed(folder.as_bytes());
    }
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
    root.join("deps-pristine-v1").join(key)
}

/// Copy pristine folders for `key` from the cache into `tree`. `Ok` when
/// every one was copied; otherwise partial copies are discarded and setup runs.
pub fn take(root: &Path, cache: &SetupCache, key: &str, tree: &Path) -> Result<(), String> {
    let slot = slot(root, key);
    if !slot.is_dir() {
        return Err(format!("no cache for key {key} yet"));
    }
    let _held = insi_core::lock::lock_beside(&slot)
        .map_err(|e| format!("the cache for {key} could not be locked: {e}"))?;
    if !slot.join(".complete").is_file() {
        return Err(format!("cache for {key} was not completely published"));
    }
    if let Some(k) = cache.keep.iter().find(|k| !slot.join(k).exists()) {
        return Err(format!("the cache for {key} holds no {k}"));
    }
    if let Some(k) = cache.keep.iter().find(|k| tree.join(k).exists()) {
        return Err(format!("the worktree already has {k}"));
    }
    for k in &cache.keep {
        if let Err(e) = copy_private(&slot.join(k), &tree.join(k), &slot) {
            for keep in &cache.keep {
                let _ = std::fs::remove_dir_all(tree.join(keep));
            }
            return Err(e);
        }
    }
    Ok(())
}

/// Publish pristine setup folders before any worker/check runs — only
/// when the key read from `tree` now is still `key`, and only into a slot
/// that has no completed snapshot. The caller must not have run a worker yet.
pub fn give_back(
    root: &Path,
    cache: &SetupCache,
    setup: &[String],
    key: &str,
    tree: &Path,
) -> Result<(), String> {
    if self::key(cache, setup, tree).as_deref() != Some(key) {
        return Err("the key files changed during the attempt".to_string());
    }
    let slot = slot(root, key);
    std::fs::create_dir_all(&slot).map_err(|e| format!("{}: {e}", slot.display()))?;
    let _held = insi_core::lock::lock_beside(&slot)
        .map_err(|e| format!("the cache for {key} could not be locked: {e}"))?;
    if slot.join(".complete").is_file() {
        return Ok(());
    }
    // A crash during publication leaves an incomplete snapshot. No reader
    // may use it, and the next publisher rebuilds it while holding the lock.
    for k in &cache.keep {
        let _ = std::fs::remove_dir_all(slot.join(k));
    }
    if let Some(k) = cache.keep.iter().find(|k| !tree.join(k).is_dir()) {
        return Err(format!("the worktree has no {k} to return"));
    }
    for k in &cache.keep {
        if let Err(e) = copy_private(&tree.join(k), &slot.join(k), tree) {
            for keep in &cache.keep {
                let _ = std::fs::remove_dir_all(slot.join(keep));
            }
            return Err(e);
        }
    }
    std::fs::write(slot.join(".complete"), b"pristine-v1").map_err(|e| e.to_string())?;
    Ok(())
}

// Preserve internal dependency links (e.g. node_modules/.bin), but refuse
// links reaching outside the snapshot and special files.
fn copy_private(from: &Path, to: &Path, boundary: &Path) -> Result<(), String> {
    let copy = || -> std::io::Result<()> {
        let meta = std::fs::symlink_metadata(from)?;
        if meta.file_type().is_symlink() {
            let target = std::fs::read_link(from)?;
            if target.is_absolute() || !from.canonicalize()?.starts_with(boundary.canonicalize()?) {
                return Err(std::io::Error::other("dependency link escapes snapshot"));
            }
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent)?;
            }
            #[cfg(unix)]
            std::os::unix::fs::symlink(target, to)?;
            #[cfg(windows)]
            if from.is_dir() {
                std::os::windows::fs::symlink_dir(target, to)?;
            } else {
                std::os::windows::fs::symlink_file(target, to)?;
            }
        } else if meta.is_dir() {
            std::fs::create_dir_all(to)?;
            for entry in std::fs::read_dir(from)? {
                let entry = entry?;
                copy_private(&entry.path(), &to.join(entry.file_name()), boundary)
                    .map_err(std::io::Error::other)?;
            }
        } else if meta.is_file() {
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(from, to)?;
        } else {
            return Err(std::io::Error::other("special file in dependency snapshot"));
        }
        Ok(())
    };
    copy().map_err(|e| format!("{}: {e}", from.display()))
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

    /// Worker edits never reach later attempts with the same setup key.
    #[test]
    fn attempts_get_private_copies_of_pristine_setup() {
        let root = dir("root");
        let setup = vec!["npm".to_string(), "ci".to_string()];
        let first = tree("first", "{\"v\":1}");
        std::fs::create_dir_all(first.join("node_modules/pkg")).expect("installed");
        std::fs::write(first.join("node_modules/pkg/index.js"), "x").expect("file");
        let k = key(&cache(), &setup, &first).expect("key");

        give_back(&root, &cache(), &setup, &k, &first).expect("returned");
        assert!(
            first.join("node_modules").exists(),
            "snapshot leaves setup intact"
        );

        let second = tree("second", "{\"v\":1}");
        assert_eq!(key(&cache(), &setup, &second).as_deref(), Some(k.as_str()));
        take(&root, &cache(), &k, &second).expect("reused");
        assert!(second.join("node_modules/pkg/index.js").is_file());
        std::fs::write(second.join("node_modules/pkg/index.js"), "worker edit").expect("edit");
        let third = dir("third");
        take(&root, &cache(), &k, &third).expect("pristine copy");
        assert_eq!(
            std::fs::read_to_string(third.join("node_modules/pkg/index.js")).expect("read"),
            "x"
        );

        let other = tree("other", "{\"v\":2}");
        let k2 = key(&cache(), &setup, &other).expect("key");
        assert_ne!(k, k2, "a different lockfile is a different key");
        let why = take(&root, &cache(), &k2, &other).expect_err("nothing for that key");
        assert!(why.contains("no cache for key"), "{why}");
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
        let why = give_back(&root, &cache(), &setup, &k, &t).expect_err("changed");
        assert!(why.contains("key files changed"), "{why}");
        assert!(t.join("node_modules").is_dir(), "left with the tree");
        assert_eq!(
            key(&cache(), &setup, &dir("nolock")),
            None,
            "no lockfile, no key"
        );
    }
}
