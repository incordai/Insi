// Copyright (c) 2026-2027 Incord Ai.
// https://incord.ai
//
// SPDX-License-Identifier: BUSL-1.1

//! Require a bounded, code-grounded justification before implementation starts.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Component, Path};

/// Explanations are reviewable claims; source excerpts are independently checked.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Justification {
    project_fit: String,
    feasibility: String,
    benefit: String,
    verification: String,
    risks_and_alternatives: String,
    evidence: BTreeMap<String, String>,
}

/// Reject missing reasoning and unsafe or unbounded source references during parsing.
pub fn validate(j: &Justification) -> Result<(), String> {
    for text in [
        &j.project_fit,
        &j.feasibility,
        &j.benefit,
        &j.verification,
        &j.risks_and_alternatives,
    ] {
        if text.trim().is_empty() || text.len() > 4000 {
            return Err("justification needs project_fit, feasibility, benefit, verification and risks_and_alternatives (1..4000 bytes each)".into());
        }
    }
    if j.evidence.is_empty() || j.evidence.len() > 8 {
        return Err("justification needs 1..8 existing source or documentation references".into());
    }
    for (path, excerpt) in &j.evidence {
        if path.is_empty()
            || path.contains('\\')
            || Path::new(path)
                .components()
                .any(|c| !matches!(c, Component::Normal(_)))
            || excerpt.trim().is_empty()
            || excerpt.len() > 1000
        {
            return Err(
                "justification evidence needs relative paths and exact excerpts of 1..1000 bytes"
                    .into(),
            );
        }
    }
    Ok(())
}

/// Compare evidence against committed baseline objects, never symlinks or worker-edited files.
pub fn check(repo: &Path, j: &Justification) -> Result<(), String> {
    validate(j)?;
    for (path, excerpt) in &j.evidence {
        // Preserve committed bytes: the text-oriented Git helper strips a valid excerpt's final newline.
        let source = insi_eval::git::git_bytes(repo, &["show", &format!("HEAD:{path}")])
            .map_err(|_| format!("justification references missing committed file: {path}"))?;
        // Compare exact UTF-8 excerpt bytes without trimming or replacing non-UTF-8 source bytes.
        if !source
            .windows(excerpt.len())
            .any(|chunk| chunk == excerpt.as_bytes())
        {
            return Err(format!(
                "justification excerpt does not match baseline file: {path}"
            ));
        }
    }
    Ok(())
}
