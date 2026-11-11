//! Human judgments and held-out review evaluation. Durable feedback goes to Incord.
use serde::Deserialize;
use serde_json::{json, Value};
use std::{collections::BTreeSet, path::Path};

pub fn feedback(report: &Path, id: &str, verdict: &str, note: &str) -> Result<Value, String> {
    if !matches!(
        verdict,
        "accepted" | "false-positive" | "fixed" | "regression"
    ) {
        return Err("invalid feedback verdict".into());
    }
    let value: Value = serde_json::from_slice(&std::fs::read(report).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let finding = value["findings"]
        .as_array()
        .and_then(|v| v.iter().find(|f| f["id"] == id))
        .ok_or("finding not present in report")?;
    let repo = value["original_repo"]
        .as_str()
        .ok_or("report has no original project")?;
    let why=json!({"finding":id,"head":value["head"],"rule":finding["rule"],"file":finding["file"],"verdict":verdict,"note":note,"provenance":"user judgment; not independent proof of correctness"}).to_string();
    if !insi_core::secrets::scan_diff(&format!("+{why}")).is_empty() {
        return Err("feedback withheld by sensitive-data scan".into());
    }
    insi_memory::incord::tool_call(
        insi_memory::incord::address()?,
        "lesson_write",
        json!({"cwd":repo,"agent_id":"insi","kind":"decision","write_key":format!("review:{}:{id}:{verdict}",value["head"].as_str().unwrap_or("unknown")),"record":{"decision":format!("Review feedback: {verdict}"),"why":why,"tags":["insi","review-feedback"]}}),
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Corpus {
    version: u32,
    #[serde(default)]
    selection_repositories: Vec<String>,
    tasks: Vec<Task>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Task {
    id: String,
    repository: String,
    report: String,
    truth: Vec<Truth>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Truth {
    file: String,
    line: u64,
    rule: String,
}

pub fn assess(path: &Path) -> Result<Value, String> {
    let corpus: Corpus = serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    if corpus.version != 1 || corpus.tasks.is_empty() {
        return Err("quality corpus requires version 1 and tasks".into());
    }
    let mut ids = BTreeSet::new();
    let mut reports = BTreeSet::new();
    let mut repos = BTreeSet::new();
    let (mut tp, mut fp, mut missed, mut incomplete) = (0usize, 0usize, 0usize, 0usize);
    let mut seconds = 0.0;
    for task in &corpus.tasks {
        if task.id.is_empty() || task.repository.is_empty() || !ids.insert(&task.id) {
            return Err("duplicate or empty quality task".into());
        }
        if !reports.insert((&task.repository, &task.report)) {
            return Err("the same repository report cannot be evaluated twice".into());
        }
        if corpus.selection_repositories.contains(&task.repository) {
            return Err("held-out repository overlaps selection repositories".into());
        }
        repos.insert(&task.repository);
        let report_path = path.parent().unwrap_or(Path::new(".")).join(&task.report);
        let report: Value =
            serde_json::from_slice(&std::fs::read(report_path).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        if report["complete"] != true {
            incomplete += 1;
        }
        seconds += report["elapsed_seconds"]
            .as_f64()
            .filter(|n| n.is_finite() && *n >= 0.0)
            .unwrap_or(0.0);
        let mut truth = BTreeSet::new();
        for t in &task.truth {
            crate::review::safe_path(&t.file)?;
            if t.line == 0 || !truth.insert((t.file.clone(), t.line, t.rule.clone())) {
                return Err("duplicate truth label".into());
            }
        }
        let mut predicted = BTreeSet::new();
        for f in report["findings"]
            .as_array()
            .ok_or("quality report needs findings")?
        {
            let file = f["file"].as_str().ok_or("finding missing file")?;
            let rule = f["rule"].as_str().ok_or("finding missing rule")?;
            let line = f["line"]
                .as_u64()
                .filter(|n| *n > 0)
                .ok_or("finding missing positive line")?;
            predicted.insert((file.to_string(), line, rule.to_string()));
        }
        tp += truth.intersection(&predicted).count();
        fp += predicted.difference(&truth).count();
        missed += truth.difference(&predicted).count();
    }
    let ratio = |n: usize, d: usize| {
        if d == 0 {
            None
        } else {
            Some(n as f64 / d as f64)
        }
    };
    Ok(
        json!({"tasks":corpus.tasks.len(),"repositories":repos.len(),"true_positive":tp,"false_positive":fp,"missed":missed,"precision":ratio(tp,tp+fp),"recall":ratio(tp,tp+missed),"incomplete_runs":incomplete,"completion_rate":ratio(corpus.tasks.len()-incomplete,corpus.tasks.len()),"total_review_seconds":seconds,
        "generalization_supported":false,"labels":"producer-supplied judgments; exact file/line/rule matching; no independent adjudication or statistical generalization claim","selection_history_declared":!corpus.selection_repositories.is_empty()}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn incomplete_reviews_stay_in_the_denominator_and_overlap_is_rejected() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.incord/temp");
        std::fs::create_dir_all(&root).unwrap();
        let d = root.join(format!(
            "review-quality-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&d).unwrap();
        let report = json!({"complete":false,"elapsed_seconds":2,"findings":[{"file":"src/a.rs","line":1,"rule":"r"},{"file":"src/b.rs","line":2,"rule":"wrong"}]});
        std::fs::write(d.join("report.json"), report.to_string()).unwrap();
        let mut plan = json!({"version":1,"selection_repositories":["selection"],"tasks":[{"id":"one","repository":"heldout","report":"report.json","truth":[{"file":"src/a.rs","line":1,"rule":"r"},{"file":"src/c.rs","line":2,"rule":"missed"}]}]});
        let path = d.join("plan.json");
        std::fs::write(&path, plan.to_string()).unwrap();
        let metrics = assess(&path).unwrap();
        assert_eq!(metrics["precision"], 0.5);
        assert_eq!(metrics["recall"], 0.5);
        assert_eq!(metrics["incomplete_runs"], 1);
        assert_eq!(metrics["completion_rate"], 0.0);
        assert_eq!(metrics["generalization_supported"], false);
        plan["tasks"][0]["repository"] = json!("selection");
        std::fs::write(&path, plan.to_string()).unwrap();
        assert!(assess(&path).is_err());
        std::fs::remove_dir_all(d).unwrap();
    }
}
