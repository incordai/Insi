//! Project-defined workloads with reusable measurement and comparison rules.
use insi_core::Config;
use insi_eval::process;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    path::{Component, Path},
    time::Duration,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub version: u32,
    #[serde(default = "samples")]
    pub samples: u64,
    #[serde(default)]
    pub warmup: u64,
    pub workloads: Vec<Workload>,
}
fn samples() -> u64 {
    10
}
fn timeout() -> u64 {
    300
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Workload {
    pub id: String,
    /// Human-readable measurement scope: e.g. database, memory or frontend.
    pub category: String,
    pub kind: Kind,
    #[serde(default)]
    pub command: Vec<String>,
    #[serde(default)]
    pub verify: Vec<String>,
    #[serde(default)]
    pub prepare: Vec<String>,
    #[serde(default)]
    pub artifacts: Vec<String>,
    #[serde(default)]
    pub operations: Option<u64>,
    pub unit: String,
    pub direction: Direction,
    #[serde(default)]
    pub primary: bool,
    #[serde(default)]
    pub min_gain: f64,
    #[serde(default)]
    pub max_regression: f64,
    #[serde(default = "timeout")]
    pub timeout_secs: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    WallTime,
    Throughput,
    ArtifactBytes,
    Json,
    Check,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Lower,
    Higher,
}

pub fn read(path: &Path) -> Result<Plan, String> {
    if std::fs::metadata(path).map_err(|e| e.to_string())?.len() > 1024 * 1024 {
        return Err("benchmark plan exceeds 1MiB".into());
    }
    let plan: Plan = serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    validate(&plan)?;
    Ok(plan)
}

pub fn validate(plan: &Plan) -> Result<(), String> {
    if plan.version != 1
        || !(5..=1000).contains(&plan.samples)
        || plan.warmup > 20
        || plan.workloads.is_empty()
        || plan.workloads.len() > 32
    {
        return Err(
            "benchmark plan needs version 1, 5..1000 samples, <=20 warmups and 1..32 workloads"
                .into(),
        );
    }
    if plan.workloads.iter().filter(|w| w.primary).count() > 1 {
        return Err(
            "declare at most one primary benchmark; all others are regression guardrails".into(),
        );
    }
    let mut ids = BTreeSet::new();
    for w in &plan.workloads {
        let canonical_unit = match w.kind {
            Kind::WallTime => Some("seconds"),
            Kind::Throughput => Some("operations/second"),
            Kind::ArtifactBytes => Some("bytes"),
            Kind::Check => Some("pass"),
            Kind::Json => None,
        };
        if canonical_unit.is_some_and(|unit| w.unit != unit) {
            return Err(format!(
                "{} has the wrong unit for its measurement method",
                w.id
            ));
        }
        if [
            (Kind::WallTime, Direction::Lower),
            (Kind::ArtifactBytes, Direction::Lower),
            (Kind::Throughput, Direction::Higher),
        ]
        .iter()
        .any(|(kind, direction)| w.kind == *kind && w.direction != *direction)
            || [&w.command, &w.prepare, &w.verify]
                .iter()
                .any(|args| !args.is_empty() && args[0].trim().is_empty())
        {
            return Err("invalid benchmark command or measurement direction".into());
        }
        if w.id.is_empty()
            || !w
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            || !ids.insert(&w.id)
            || w.category.trim().is_empty()
            || w.unit.trim().is_empty()
            || !(1..=3600).contains(&w.timeout_secs)
            || !w.min_gain.is_finite()
            || !(0.0..1.0).contains(&w.min_gain)
            || !w.max_regression.is_finite()
            || !(0.0..1.0).contains(&w.max_regression)
            || (w.kind != Kind::ArtifactBytes && w.command.first().is_none_or(|v| v.is_empty()))
            || (w.kind == Kind::Throughput && w.operations.is_none_or(|n| n == 0))
            || (w.kind == Kind::ArtifactBytes && w.artifacts.is_empty())
            || (w.kind == Kind::Check && w.primary)
            || (w.kind != Kind::Check && w.verify.is_empty())
        {
            return Err(format!("invalid benchmark {}: measurement, correctness verification, units or limits missing",w.id));
        }
        for path in &w.artifacts {
            if path.is_empty()
                || Path::new(path).is_absolute()
                || path.contains('\\')
                || Path::new(path)
                    .components()
                    .any(|c| !matches!(c, Component::Normal(_)))
            {
                return Err("artifact paths must be relative paths without traversal".into());
            }
        }
    }
    Ok(())
}

fn command(
    config: &Config,
    repo: &Path,
    argv: &[String],
    seconds: u64,
    log: &Path,
) -> Result<(String, Duration), String> {
    if argv.is_empty() {
        return Ok((String::new(), Duration::ZERO));
    }
    let wrapped = config.sandbox.as_ref().map(|s| s.wrap(argv, repo));
    let out = process::run(
        wrapped.as_deref().unwrap_or(argv),
        repo,
        Duration::from_secs(seconds),
    )?;
    std::fs::write(log, format!("{}\n{}", out.stdout, out.stderr)).map_err(|e| e.to_string())?;
    if !out.success() {
        return Err(format!(
            "benchmark command failed or timed out; see {}",
            log.display()
        ));
    }
    Ok((out.stdout, out.wall))
}

fn artifact_bytes(repo: &Path, paths: &[String]) -> Result<f64, String> {
    fn size(path: &Path) -> Result<u64, String> {
        let meta = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        if meta.file_type().is_symlink() {
            return Err("symlink artifacts are refused".into());
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 {
                return Err("reparse-point artifacts are refused".into());
            }
        }
        if meta.is_file() {
            return Ok(meta.len());
        }
        if !meta.is_dir() {
            return Err("artifact must be a regular file or directory".into());
        }
        std::fs::read_dir(path)
            .map_err(|e| e.to_string())?
            .try_fold(0u64, |n, entry| {
                let next = size(&entry.map_err(|e| e.to_string())?.path())?;
                n.checked_add(next).ok_or("artifact size overflow".into())
            })
    }
    let mut total = 0u64;
    let mut counted = BTreeSet::new();
    for rel in paths {
        // Reject symlink parents, including a directory that escapes the checkout.
        let mut path = repo.to_path_buf();
        for part in Path::new(rel).components() {
            path.push(part);
            let m = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
            if m.file_type().is_symlink() {
                return Err("symlink artifact parents are refused".into());
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if m.file_attributes() & 0x400 != 0 {
                    return Err("reparse-point parents are refused".into());
                }
            }
        }
        let real = std::fs::canonicalize(&path).map_err(|e| e.to_string())?;
        if counted
            .iter()
            .any(|p: &std::path::PathBuf| real.starts_with(p) || p.starts_with(&real))
        {
            return Err("overlapping artifact paths would count bytes twice".into());
        }
        counted.insert(real);
        total = total
            .checked_add(size(&path)?)
            .ok_or("artifact size overflow")?;
    }
    Ok(total as f64)
}

fn measure(
    w: &Workload,
    repo: &Path,
    config: &Config,
    logs: &Path,
    label: &str,
) -> Result<f64, String> {
    command(
        config,
        repo,
        &w.prepare,
        w.timeout_secs,
        &logs.join(format!("{}-{label}-prepare.log", w.id)),
    )?;
    let (stdout, wall) = command(
        config,
        repo,
        &w.command,
        w.timeout_secs,
        &logs.join(format!("{}-{label}.log", w.id)),
    )?;
    let elapsed = wall.as_secs_f64();
    let value = match w.kind {
        Kind::WallTime => elapsed,
        Kind::Throughput => w.operations.unwrap_or(0) as f64 / elapsed,
        Kind::ArtifactBytes => artifact_bytes(repo, &w.artifacts)?,
        Kind::Check => 1.0,
        Kind::Json => {
            let line = stdout
                .lines()
                .rfind(|l| !l.trim().is_empty())
                .ok_or("missing benchmark JSON")?;
            let v: Value = serde_json::from_str(line).map_err(|e| e.to_string())?;
            v["value"]
                .as_f64()
                .ok_or("benchmark must end with JSON containing numeric value")?
        }
    };
    command(
        config,
        repo,
        &w.verify,
        w.timeout_secs,
        &logs.join(format!("{}-{label}-verify.log", w.id)),
    )?;
    if !value.is_finite() || value < 0.0 {
        return Err("benchmark metric must be finite and nonnegative".into());
    }
    Ok(value)
}

/// Establish executable workloads and equivalent-behavior checks before editing.
pub fn baseline(plan: &Plan, repo: &Path, config: &Config, logs: &Path) -> Result<Value, String> {
    validate(plan)?;
    let mut results = Vec::new();
    for w in &plan.workloads {
        let value = measure(w, repo, config, logs, "initial")?;
        if value <= 0.0 {
            return Err(format!(
                "{} baseline must be positive for relative comparison",
                w.id
            ));
        }
        results.push(json!({"id":w.id,"value":value}));
    }
    Ok(json!(results))
}

/// Every configured workload is required. Unsupported/failing workloads are errors.
pub fn compare(
    plan: &Plan,
    before: &Path,
    after: &Path,
    config: &Config,
    logs: &Path,
) -> Result<Value, String> {
    validate(plan)?;
    let mut results = Vec::new();
    let mut qualified = plan.workloads.iter().any(|w| w.primary);
    for w in &plan.workloads {
        for n in 0..plan.warmup {
            measure(w, before, config, logs, &format!("baseline-warmup-{n}"))?;
            measure(w, after, config, logs, &format!("candidate-warmup-{n}"))?;
        }
        let mut b = Vec::new();
        let mut c = Vec::new();
        for n in 0..plan.samples {
            let bl = format!("baseline-{n}");
            let cl = format!("candidate-{n}");
            let (bv, cv) = if n % 2 == 0 {
                (
                    measure(w, before, config, logs, &bl)?,
                    measure(w, after, config, logs, &cl)?,
                )
            } else {
                let cv = measure(w, after, config, logs, &cl)?;
                (measure(w, before, config, logs, &bl)?, cv)
            };
            b.push(bv);
            c.push(cv);
            std::fs::write(
                logs.join(format!("suite-samples-{}.json", w.id)),
                serde_json::to_vec_pretty(&json!({"baseline":b,"candidate":c}))
                    .map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
        }
        if b.iter().any(|v| *v <= 0.0) {
            return Err(format!(
                "{} has a zero baseline; relative comparison is undefined",
                w.id
            ));
        }
        let direction = match w.direction {
            Direction::Lower => crate::suggest::Direction::Lower,
            Direction::Higher => crate::suggest::Direction::Higher,
        };
        let evidence = crate::suggest::compare(b, c, direction, w.min_gain);
        let passed = if w.kind == Kind::Check {
            true
        } else if w.primary {
            evidence.improved
        } else {
            evidence.interval_95.0 >= -w.max_regression
        };
        qualified &= passed;
        results.push(json!({"id":w.id,"category":w.category,"kind":w.kind,"unit":w.unit,"primary":w.primary,"passed":passed,"evidence":evidence}));
        std::fs::write(
            logs.join("suite-progress.json"),
            serde_json::to_vec_pretty(&results).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(
        json!({"qualified":qualified,"samples":plan.samples,"workloads":results,"host":{"os":std::env::consts::OS,"arch":std::env::consts::ARCH},
        "limits":"One predeclared primary; secondary guardrails require the descriptive bootstrap lower bound to clear the allowed regression. This is not proof of equivalence. Repeated samples do not establish generalization."}),
    )
}

/// Detection suggests commands, never runs or installs anything.
pub fn detect(repo: &Path) -> Value {
    let markers = [
        (
            "Cargo.toml",
            "rust",
            vec!["cargo", "test", "--locked"],
            vec!["cargo", "build", "--release", "--locked"],
        ),
        (
            "package.json",
            "javascript",
            vec!["npm", "test"],
            vec!["npm", "run", "build"],
        ),
        (
            "pyproject.toml",
            "python",
            vec!["python3", "-m", "pytest"],
            vec!["python3", "-m", "build"],
        ),
        (
            "go.mod",
            "go",
            vec!["go", "test", "./..."],
            vec!["go", "build", "./..."],
        ),
        (
            "pom.xml",
            "java",
            vec!["mvn", "test"],
            vec!["mvn", "package"],
        ),
        (
            "build.gradle",
            "gradle",
            vec!["./gradlew", "test"],
            vec!["./gradlew", "build"],
        ),
        (
            "CMakeLists.txt",
            "cmake",
            vec!["ctest", "--test-dir", "build"],
            vec!["cmake", "--build", "build"],
        ),
    ];
    let projects:Vec<Value>=markers.iter().filter(|(file,_,_,_)|repo.join(file).is_file()).map(|(marker,stack,check,build)|
        json!({"stack":stack,"marker":marker,"status":"needs_configuration","suggested_check":check,"suggested_build":build})).collect();
    json!({"projects":projects,"measurement_methods":["wall_time","throughput","artifact_bytes","json","check"],
        "custom_metric_categories":["latency","startup","memory","cpu","io","network","database","frontend","coverage","complexity","dependencies","energy","quality"],
        "status":"needs_configuration","note":"Detection is a hint, not proof tools or scripts exist. Supply representative inputs, correctness checks and cache preparation. JSON adapters support project-specific tools; no paid services or installations are required by Insi."})
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn dir() -> std::path::PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.incord/temp");
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join(format!(
            "suite-test-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        path
    }
    fn plan() -> Plan {
        serde_json::from_value(json!({"version":1,"samples":5,"workloads":[{
            "id":"latency","category":"database","kind":"json","command":["sh","-c","cat metric.json"],
            "verify":["sh","-c","test -f metric.json"],"unit":"milliseconds","direction":"lower","primary":true,"min_gain":0.05
        }]})).unwrap()
    }

    #[test]
    fn rejects_ambiguous_targets_unverifiable_workloads_and_invalid_limits() {
        let mut p = plan();
        validate(&p).unwrap();
        p.workloads.push(p.workloads[0].clone());
        assert!(validate(&p).is_err());
        let mut p = plan();
        p.workloads[0].verify.clear();
        assert!(validate(&p).is_err());
        let mut p = plan();
        p.samples = 0;
        assert!(validate(&p).is_err());
        let mut p = plan();
        p.workloads[0].kind = Kind::Check;
        assert!(validate(&p).is_err());
        let mut p = plan();
        p.workloads[0].artifacts = vec!["../outside".into()];
        assert!(validate(&p).is_err());
        let mut p = plan();
        p.workloads[0].kind = Kind::Throughput;
        assert!(validate(&p).is_err());
    }

    #[test]
    fn detects_multiple_stacks_without_claiming_they_are_configured() {
        let d = dir();
        std::fs::write(d.join("Cargo.toml"), "").unwrap();
        std::fs::write(d.join("package.json"), "{}").unwrap();
        let value = detect(&d);
        assert_eq!(value["projects"].as_array().unwrap().len(), 2);
        assert_eq!(value["status"], "needs_configuration");
        std::fs::remove_dir_all(d).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn check_only_suites_run_diagnostics_without_claiming_improvement() {
        let d = dir();
        let mut p = plan();
        let w = &mut p.workloads[0];
        w.primary = false;
        w.kind = Kind::Check;
        w.unit = "pass".into();
        w.command = vec!["sh".into(), "-c".into(), "true".into()];
        w.verify.clear();
        assert!(baseline(&p, &d, &Config::default(), &d).is_ok());
        assert_eq!(
            compare(&p, &d, &d, &Config::default(), &d).unwrap()["qualified"],
            false
        );
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn artifact_measurement_refuses_missing_and_overlapping_paths() {
        let d = dir();
        std::fs::create_dir(d.join("dist")).unwrap();
        std::fs::write(d.join("dist/app"), b"12345").unwrap();
        assert_eq!(artifact_bytes(&d, &["dist".into()]).unwrap(), 5.0);
        assert!(artifact_bytes(&d, &["missing".into()]).is_err());
        assert!(artifact_bytes(&d, &["dist".into(), "dist/app".into()]).is_err());
        std::fs::remove_dir_all(d).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_primary_gain_cannot_hide_a_secondary_regression_or_failed_correctness_check() {
        let d = dir();
        let before = d.join("before");
        let after = d.join("after");
        let logs = d.join("logs");
        for path in [&before, &after, &logs] {
            std::fs::create_dir(path).unwrap();
        }
        std::fs::write(before.join("metric.json"), "{\"value\":100}").unwrap();
        std::fs::write(after.join("metric.json"), "{\"value\":70}").unwrap();
        let mut p = plan();
        let result = compare(&p, &before, &after, &Config::default(), &logs).unwrap();
        assert_eq!(result["qualified"], true);
        let mut guard = p.workloads[0].clone();
        guard.id = "bytes".into();
        guard.primary = false;
        guard.kind = Kind::ArtifactBytes;
        guard.command.clear();
        guard.artifacts = vec!["output".into()];
        guard.unit = "bytes".into();
        p.workloads.push(guard);
        std::fs::write(before.join("output"), [0; 10]).unwrap();
        std::fs::write(after.join("output"), [0; 20]).unwrap();
        assert_eq!(
            compare(&p, &before, &after, &Config::default(), &logs).unwrap()["qualified"],
            false
        );
        p.workloads[0].verify = vec!["sh".into(), "-c".into(), "exit 1".into()];
        assert!(baseline(&p, &before, &Config::default(), &logs).is_err());
        std::fs::remove_dir_all(d).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn measures_time_throughput_and_rejects_bad_metrics_and_symlink_artifacts() {
        let d = dir();
        let mut w = plan().workloads.remove(0);
        std::fs::write(d.join("metric.json"), "{\"value\":0}").unwrap();
        assert_eq!(
            measure(&w, &d, &Config::default(), &d, "zero").unwrap(),
            0.0
        );
        let mut p = plan();
        assert!(compare(&p, &d, &d, &Config::default(), &d).is_err());
        w.kind = Kind::WallTime;
        w.command = vec!["sh".into(), "-c".into(), "true".into()];
        assert!(measure(&w, &d, &Config::default(), &d, "time").unwrap() > 0.0);
        w.kind = Kind::Throughput;
        w.operations = Some(100);
        assert!(measure(&w, &d, &Config::default(), &d, "throughput").unwrap() > 0.0);
        std::fs::write(d.join("metric.json"), "not JSON").unwrap();
        p.workloads[0].timeout_secs = 1;
        assert!(baseline(&p, &d, &Config::default(), &d).is_err());
        std::os::unix::fs::symlink(d.join("metric.json"), d.join("linked")).unwrap();
        assert!(artifact_bytes(&d, &["linked".into()]).is_err());
        std::fs::remove_dir_all(d).unwrap();
    }
}
