// Copyright (c) 2026-2027 Incord Ai.
// https://incord.ai
//
// SPDX-License-Identifier: BUSL-1.1

//! EVERY OWN RUST FILE CARRIES THE LICENSE HEADER (the owner, 2026-11-18): Insi is under the Business Source
//! License 1.1, and each of its code files says so at its top. Fails, naming the files, when one has none, so a new
//! file cannot slip in without it. The script that adds the header, and checks the other code files, is
//! `scripts/license-headers.mjs`.

use std::path::{Path, PathBuf};

/// Every `.rs` file under `dir`, recursively.
fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "target") {
                continue;
            }
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn every_own_rust_file_carries_the_license_header() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
    let mut files = Vec::new();
    for dir in ["crates", "fuzz", "scripts"] {
        rust_files(&root.join(dir), &mut files);
    }
    assert!(files.len() > 50, "the walk found the sources ({} files)", files.len());
    let missing: Vec<String> = files
        .iter()
        .filter(|path| {
            let text = std::fs::read_to_string(path).unwrap_or_default();
            !text.lines().take(12).any(|l| l.contains("SPDX-License-Identifier: BUSL-1.1"))
        })
        .map(|path| path.strip_prefix(&root).unwrap_or(path).display().to_string())
        .collect();
    assert!(
        missing.is_empty(),
        "{} Rust files have no license header (run `node scripts/license-headers.mjs`): {}",
        missing.len(),
        missing.join(", ")
    );
}
