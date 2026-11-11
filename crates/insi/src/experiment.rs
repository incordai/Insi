//! Offline, conservative assessment of recorded component experiments.
//! No agent, shell command, or memory store is started here.
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use insi_core::EvalResult;
use insi_eval::store::read_results;
use insi_loop::accept;
// The one shared one-sided sign-test tail (also used by `suggest::compare`).
use crate::suggest::sign_tail;
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Plan {
    version: u32,
    /// Selection repositories must never appear in the final set.
    selection_repositories: Vec<String>,
    tasks: BTreeMap<String, Task>,
    runs: BTreeMap<String, Run>,
    comparisons: Vec<Contrast>,
    /// Total comparisons planned, including ones evaluated elsewhere.
    planned_comparisons: usize,
    min_tasks: usize,
    min_seeds: usize,
    margin: f64,
    /// Caller attestation; the report never represents this as verified history.
    final_set_untouched: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Task {
    repository: String,
    commit: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Run {
    results: PathBuf,
    /// Immutable identifiers supplied by the producer, retained in the report.
    model: String,
    worker_version: String,
    environment: String,
    tool_versions: BTreeMap<String, String>,
    budget_secs: u64,
    /// Component name -> content snapshot identifier, including unchanged components.
    components: BTreeMap<String, String>,
    /// Seed numbers are scheduling labels unless the producer attests otherwise.
    model_seed_supported: bool,
    #[serde(default)]
    labels: Vec<Label>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Label {
    task: String,
    seed: u32,
    category: Category,
    /// Existing graph node or artifact reference; no new knowledge store.
    evidence: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "snake_case")]
enum Category {
    Reasoning,
    Retrieval,
    Tooling,
    Environment,
    Orchestration,
    Unknown,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Contrast {
    baseline: String,
    candidate: String,
    component: String,
}

pub fn run(path: &Path) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let plan: Plan = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let report = assess(&plan, path.parent().unwrap_or(Path::new(".")))?;
    println!(
        "{}",
        serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
    );
    Ok(())
}

fn nonempty(s: &str) -> bool {
    !s.trim().is_empty()
}

fn assess(p: &Plan, root: &Path) -> Result<Value, String> {
    if p.version != 1
        || p.min_tasks < 2
        || p.min_seeds < 2
        || p.comparisons.is_empty()
        || p.planned_comparisons < p.comparisons.len()
        || !p.margin.is_finite()
        || !(0.0..=1.0).contains(&p.margin)
        || p.tasks.is_empty()
    {
        return Err("invalid plan: version=1, >=2 tasks/seeds, nonempty tasks/comparisons, sufficient planned_comparisons and margin in [0,1] required".into());
    }
    let selection: BTreeSet<_> = p.selection_repositories.iter().collect();
    if p.tasks.values().any(|t| {
        !nonempty(&t.repository) || !nonempty(&t.commit) || selection.contains(&t.repository)
    }) {
        return Err("final repositories must have commits and be disjoint from selection repositories (use canonical repository identifiers)".into());
    }
    let mut data = BTreeMap::new();
    let mut runs = BTreeMap::new();
    for (name, r) in &p.runs {
        if !nonempty(name)
            || !nonempty(&r.model)
            || !nonempty(&r.worker_version)
            || !nonempty(&r.environment)
            || r.budget_secs == 0
            || r.components.is_empty()
            || r.tool_versions.is_empty()
            || r.components
                .iter()
                .chain(r.tool_versions.iter())
                .any(|(k, v)| !nonempty(k) || !nonempty(v))
        {
            return Err(format!("{name}: incomplete provenance"));
        }
        let path = root.join(&r.results);
        let rows = read_results(&path)?;
        validate_rows(name, r, &rows, &p.tasks)?;
        runs.insert(name, metrics(r, &rows));
        data.insert(name, rows);
    }
    let mut contrasts = Vec::new();
    for c in &p.comparisons {
        let baseline = p.runs.get(&c.baseline).ok_or("unknown baseline")?;
        let candidate = p.runs.get(&c.candidate).ok_or("unknown candidate")?;
        if baseline.model != candidate.model
            || baseline.worker_version != candidate.worker_version
            || baseline.environment != candidate.environment
            || baseline.tool_versions != candidate.tool_versions
            || baseline.budget_secs != candidate.budget_secs
            || baseline.model_seed_supported != candidate.model_seed_supported
        {
            return Err("component comparisons require identical model, worker, environment, tools, seed support and budget".into());
        }
        let keys: BTreeSet<_> = baseline
            .components
            .keys()
            .chain(candidate.components.keys())
            .collect();
        let changed: Vec<_> = keys
            .into_iter()
            .filter(|k| baseline.components.get(*k) != candidate.components.get(*k))
            .collect();
        if changed.len() != 1 || changed[0] != &c.component {
            return Err("each contrast must change exactly its declared component".into());
        }
        let a = &data[&c.baseline];
        let b = &data[&c.candidate];
        let am = workers(a);
        let bm = workers(b);
        if am.keys().collect::<Vec<_>>() != bm.keys().collect::<Vec<_>>() {
            return Err(
                "unmatched task/seed pairs; record failed attempts rather than dropping them"
                    .into(),
            );
        }
        let mut issues = Vec::new();
        if p.selection_repositories.is_empty() {
            issues.push(
                "no selection repository history supplied; diagnostic assessment only".to_string(),
            );
        }
        if !p.final_set_untouched {
            issues.push("final set has not been attested untouched".to_string());
        }
        let mut per_task: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
        for ((task, seed), ar) in &am {
            let br = bm[&(task.clone(), *seed)];
            if ar.error.is_none() && br.error.is_none() {
                per_task
                    .entry(task)
                    .or_default()
                    .push(f64::from(br.passed) - f64::from(ar.passed));
            }
        }
        if per_task.len() < p.min_tasks
            || per_task.len() != p.tasks.len()
            || per_task.values().any(|v| v.len() < p.min_seeds)
        {
            issues.push("insufficient fully paired tasks/seeds".into());
        }
        for (name, rows) in [(&c.baseline, a), (&c.candidate, b)] {
            integrity(name, rows, &p.tasks, &mut issues);
        }
        let comparison = accept::compare(b, a, 0);
        let decision = accept::decide(&comparison, p.margin);
        let diffs: Vec<f64> = per_task
            .values()
            .map(|v| v.iter().sum::<f64>() / v.len() as f64)
            .collect();
        let wins = diffs.iter().filter(|d| **d > 0.0).count();
        let losses = diffs.iter().filter(|d| **d < 0.0).count();
        let probability = sign_tail(wins, losses);
        let threshold = 0.05 / p.planned_comparisons as f64;
        // Exact sign test over tasks, with Bonferroni across the declared family.
        // Bootstrap intervals remain descriptive, not selection-adjusted intervals.
        let supported = issues.is_empty()
            && decision.accepted()
            && probability <= threshold
            && comparison.diff > 0.0;
        contrasts.push(json!({
            "baseline": c.baseline, "candidate": c.candidate, "component": c.component,
            "supported": supported, "blocking_issues": issues, "existing_decision": decision.why(),
            "tasks": comparison.tasks, "pairs": comparison.candidate.judged,
            "mean_task_difference": comparison.diff, "descriptive_95_percent_interval": comparison.interval,
            "task_sign_test_p": probability, "family_adjusted_threshold": threshold,
            "improved_tasks": comparison.better, "regressed_tasks": comparison.worse,
            "broken_tasks": comparison.broke,
        }));
    }
    Ok(
        json!({"schema_version":1, "insi_version":env!("CARGO_PKG_VERSION"),
        "provenance_status":"producer-declared; historical isolation and snapshot identifiers are not independently verified",
        "tasks":p.tasks.iter().map(|(id,t)| (id,json!({"repository":t.repository,"commit":t.commit}))).collect::<BTreeMap<_,_>>(),
        "selection_repositories":p.selection_repositories,
        "runs":runs, "comparisons":contrasts,
        "interpretation":"Component effects are conditional on this harness. Use reversed contrasts and additional component combinations to test necessity and interactions. Final assessment must not feed candidate selection."}),
    )
}

fn workers(rows: &[EvalResult]) -> BTreeMap<(String, u32), &EvalResult> {
    rows.iter()
        .filter(|r| r.kind == "worker")
        .map(|r| ((r.task.clone(), r.seed), r))
        .collect()
}

fn validate_rows(
    name: &str,
    run: &Run,
    rows: &[EvalResult],
    tasks: &BTreeMap<String, Task>,
) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    let mut condition = None;
    for r in rows {
        if !tasks.contains_key(&r.task)
            || !["worker", "base", "reference"].contains(&r.kind.as_str())
            || !r.wall_secs.is_finite()
            || r.wall_secs < 0.0
            || r.cost_usd.is_some_and(|v| !v.is_finite() || v < 0.0)
            || r.phases.values().any(|v| !v.is_finite() || *v < 0.0)
            || (r.passed && (r.error.is_some() || r.timed_out))
        {
            return Err(format!(
                "{name}: invalid or contradictory evaluation record"
            ));
        }
        if r.kind == "worker" {
            if !seen.insert((&r.task, r.seed)) {
                return Err(format!(
                    "{name}: duplicate task/seed; separate reruns into different files"
                ));
            }
            let rc = r.condition();
            if condition.as_ref().is_some_and(|c| c != &rc) {
                return Err(format!("{name}: mixed conditions"));
            }
            condition = Some(rc);
            if r.model.as_ref().is_some_and(|m| m != &run.model) {
                return Err(format!("{name}: model differs from manifest"));
            }
        }
    }
    let attempts = workers(rows);
    for l in &run.labels {
        if !nonempty(&l.evidence)
            || !attempts
                .get(&(l.task.clone(), l.seed))
                .is_some_and(|r| !r.passed)
        {
            return Err(format!(
                "{name}: failure labels require a failed attempt and evidence reference"
            ));
        }
    }
    Ok(())
}

fn integrity(
    name: &str,
    rows: &[EvalResult],
    tasks: &BTreeMap<String, Task>,
    issues: &mut Vec<String>,
) {
    if rows.iter().filter(|r| r.kind == "worker").any(|r| {
        !r.isolation
            .as_ref()
            .is_some_and(|i| i.sandboxed && i.history == "base-only")
    }) {
        issues.push(format!("{name}: worker isolation missing"));
    }
    for task in tasks.keys() {
        for (kind, expected) in [("base", false), ("reference", true)] {
            let controls: Vec<_> = rows
                .iter()
                .filter(|r| &r.task == task && r.kind == kind)
                .collect();
            if controls.len() < 2
                || controls
                    .iter()
                    .any(|r| r.error.is_some() || r.timed_out || r.passed != expected)
            {
                issues.push(format!(
                    "{name}/{task}: require >=2 stable {kind} controls with expected outcome"
                ));
            }
        }
    }
}

fn metrics(run: &Run, rows: &[EvalResult]) -> Value {
    let rs: Vec<_> = rows.iter().filter(|r| r.kind == "worker").collect();
    let passed = rs.iter().filter(|r| r.passed && r.error.is_none()).count();
    let errors = rs.iter().filter(|r| r.error.is_some()).count();
    let seconds: f64 = rs.iter().map(|r| r.wall_secs).sum();
    let failures: Vec<_> = rs.iter().filter(|r| !r.passed).map(|r| {
        let labels: Vec<_> = run.labels.iter().filter(|l| l.task == r.task && l.seed == r.seed)
            .map(|l| json!({"category":format!("{:?}",l.category).to_lowercase(),"evidence":l.evidence,"status":"hypothesis"})).collect();
        json!({"task":r.task,"seed":r.seed,"observed_error":r.error,"timed_out":r.timed_out,
            "artifacts":r.artifacts,"classification":if labels.is_empty() {vec![json!({"category":"unknown","status":"unclassified"})]} else {labels}})
    }).collect();
    json!({"attempts":rs.len(), "passed":passed,"errors":errors,
        "completion_rate":ratio(passed,rs.len()),"judged_pass_rate":ratio(passed,rs.len()-errors),
        "timeouts":rs.iter().filter(|r| r.timed_out).count(), "wall_secs":seconds,
        "successes_per_hour":if seconds>0.0 {Some(passed as f64*3600.0/seconds)} else {None},
        "tokens":rs.iter().filter_map(|r| r.tokens).map(u128::from).sum::<u128>().to_string(),
        "token_coverage":rs.iter().filter(|r| r.tokens.is_some()).count(),
        "public_hidden_disagreements":rs.iter().filter(|r| r.public_passed==Some(true)&&!r.passed).count(),
        "model":run.model,"worker_version":run.worker_version,"environment":run.environment,
        "tools":run.tool_versions,"budget_secs":run.budget_secs,"components":run.components,
        "model_seed_supported":run.model_seed_supported,"failures":failures})
}

fn ratio(a: usize, b: usize) -> Option<f64> {
    (b > 0).then(|| a as f64 / b as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(task: &str, seed: u32, kind: &str, passed: bool) -> EvalResult {
        serde_json::from_value(json!({"task":task,"seed":seed,"kind":kind,
            "passed":passed,"wall_secs":1.0}))
        .unwrap()
    }

    fn run_spec() -> Run {
        serde_json::from_value(json!({"results":"results.jsonl", "model":"fixed",
            "worker_version":"v1","environment":"image:abc", "tool_versions":{"git":"2"},
            "budget_secs":60,"components":{"prompt":"abc"},"model_seed_supported":false}))
        .unwrap()
    }

    fn tasks() -> BTreeMap<String, Task> {
        BTreeMap::from([(
            "a".into(),
            Task {
                repository: "org/unseen".into(),
                commit: "abc".into(),
            },
        )])
    }

    #[test]
    fn exact_sign_probabilities_and_multiplicity() {
        assert_eq!(sign_tail(0, 0), 1.0);
        assert_eq!(sign_tail(3, 4), 1.0);
        assert!((sign_tail(5, 0) - 0.03125).abs() < 1e-12);
        assert!((sign_tail(4, 1) - 0.1875).abs() < 1e-12);
        assert!(sign_tail(5, 0) > 0.05 / 2.0);
        assert!(sign_tail(10, 0) < 0.05 / 10.0);
        assert!(sign_tail(600, 400).is_finite());
    }

    #[test]
    fn refuses_duplicates_mixed_conditions_and_contradictions() {
        let a = row("a", 0, "worker", true);
        assert!(validate_rows("r", &run_spec(), &[a.clone(), a.clone()], &tasks()).is_err());
        let mut b = row("a", 1, "worker", false);
        b.tags.push("different=yes".into());
        assert!(validate_rows("r", &run_spec(), &[a.clone(), b], &tasks()).is_err());
        let mut b = a;
        b.timed_out = true;
        assert!(validate_rows("r", &run_spec(), &[b], &tasks()).is_err());
    }

    #[test]
    fn classification_needs_evidence_and_cannot_label_success() {
        let mut spec = run_spec();
        spec.labels.push(Label {
            task: "a".into(),
            seed: 0,
            category: Category::Reasoning,
            evidence: "graph:node".into(),
        });
        assert!(validate_rows("r", &spec, &[row("a", 0, "worker", true)], &tasks()).is_err());
        assert!(validate_rows("r", &spec, &[row("a", 0, "worker", false)], &tasks()).is_ok());
        spec.labels[0].evidence.clear();
        assert!(validate_rows("r", &spec, &[row("a", 0, "worker", false)], &tasks()).is_err());
    }

    #[test]
    fn flaky_or_missing_controls_block_evidence() {
        let mut rows = vec![
            row("a", 0, "base", false),
            row("a", 1, "base", false),
            row("a", 0, "reference", true),
            row("a", 1, "reference", true),
        ];
        let mut issues = Vec::new();
        integrity("r", &rows, &tasks(), &mut issues);
        assert!(issues.is_empty());
        rows[3].passed = false;
        integrity("r", &rows, &tasks(), &mut issues);
        assert_eq!(issues.len(), 1);
    }

    #[test]
    fn infrastructure_errors_stay_in_completion_denominator() {
        let mut failure = row("a", 1, "worker", false);
        failure.error = Some("setup failed".into());
        let m = metrics(&run_spec(), &[row("a", 0, "worker", true), failure]);
        assert_eq!(m["completion_rate"], 0.5);
        assert_eq!(m["judged_pass_rate"], 1.0);
        assert_eq!(m["failures"][0]["classification"][0]["category"], "unknown");
    }

    #[test]
    fn repository_overlap_is_rejected_before_reading_results() {
        let p:Plan=serde_json::from_value(json!({"version":1,"selection_repositories":["org/repo"],
            "tasks":{"a":{"repository":"org/repo","commit":"abc"}},"runs":{},
            "comparisons":[{"baseline":"a","candidate":"b","component":"prompt"}],
            "planned_comparisons":1,"min_tasks":2,"min_seeds":2,"margin":0.0,"final_set_untouched":true})).unwrap();
        assert!(assess(&p, Path::new(".")).unwrap_err().contains("disjoint"));
    }

    #[test]
    fn recorded_experiment_supports_only_a_controlled_reproducible_gain() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.incord/temp")
            .join(format!(
                "experiment-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        std::fs::create_dir_all(&root).unwrap();
        let mut task_spec = serde_json::Map::new();
        for n in 0..10 {
            task_spec.insert(
                format!("t{n}"),
                json!({"repository":"org/final","commit":"abc"}),
            );
        }
        for (file, pass) in [("base.jsonl", false), ("candidate.jsonl", true)] {
            let mut text = String::new();
            for n in 0..10 {
                for seed in 0..3 {
                    let mut r = row(&format!("t{n}"), seed, "worker", pass);
                    r.isolation = Some(insi_core::Isolation {
                        history: "base-only".into(),
                        sandboxed: true,
                    });
                    text.push_str(&serde_json::to_string(&r).unwrap());
                    text.push('\n');
                }
                for seed in 0..2 {
                    for (kind, passed) in [("base", false), ("reference", true)] {
                        text.push_str(
                            &serde_json::to_string(&row(&format!("t{n}"), seed, kind, passed))
                                .unwrap(),
                        );
                        text.push('\n');
                    }
                }
            }
            std::fs::write(root.join(file), text).unwrap();
        }
        let spec = |file: &str, hash: &str| {
            json!({"results":file,"model":"fixed","worker_version":"v1",
            "environment":"image:abc","tool_versions":{"git":"2"},"budget_secs":60,
            "components":{"prompt":hash},"model_seed_supported":false})
        };
        let mut p:Plan=serde_json::from_value(json!({"version":1,"selection_repositories":["org/selection"],
            "tasks":task_spec,"runs":{"base":spec("base.jsonl","old"),"candidate":spec("candidate.jsonl","new")},
            "comparisons":[{"baseline":"base","candidate":"candidate","component":"prompt"}],
            "planned_comparisons":1,"min_tasks":10,"min_seeds":3,"margin":0.05,"final_set_untouched":true})).unwrap();
        assert_eq!(
            assess(&p, &root).unwrap()["comparisons"][0]["supported"],
            true
        );
        p.selection_repositories.clear();
        let diagnostic = assess(&p, &root).unwrap();
        assert_eq!(diagnostic["comparisons"][0]["supported"], false);
        assert!(diagnostic["comparisons"][0]["blocking_issues"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v.as_str().unwrap().contains("diagnostic")));
        p.selection_repositories.push("org/selection".into());
        p.final_set_untouched = false;
        assert_eq!(
            assess(&p, &root).unwrap()["comparisons"][0]["supported"],
            false
        );
        p.final_set_untouched = true;
        p.planned_comparisons = 100;
        assert_eq!(
            assess(&p, &root).unwrap()["comparisons"][0]["supported"],
            false
        );
        p.runs
            .get_mut("candidate")
            .unwrap()
            .components
            .insert("tool".into(), "extra".into());
        assert!(assess(&p, &root).unwrap_err().contains("exactly"));
        p.runs
            .get_mut("candidate")
            .unwrap()
            .components
            .remove("tool");
        p.runs.get_mut("candidate").unwrap().environment = "different".into();
        assert!(assess(&p, &root).unwrap_err().contains("identical"));
        std::fs::remove_dir_all(root).unwrap();
    }
}
