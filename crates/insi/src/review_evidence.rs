//! Run the same frozen reproduction against baseline and reviewed commit.
use insi_core::Config;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::BTreeMap, path::Path, time::Duration};

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Reproduction {
    pub id: String,
    pub file: String,
    pub line: u64,
    pub rule: String,
    pub message: String,
    pub command: Vec<String>,
    pub files: BTreeMap<String, String>,
    pub assertion_marker: String,
    #[serde(default = "failure")]
    pub failure_exit: i32,
}
fn failure() -> i32 {
    1
}

pub fn validate(r: &Reproduction) -> Result<(), String> {
    crate::review::safe_path(&r.file)?;
    if r.id.is_empty()
        || !r
            .id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        || r.line == 0
        || r.files.is_empty()
        || r.assertion_marker.is_empty()
        || !matches!(r.failure_exit, 1 | 101)
        || r.command.first().is_none_or(|c| c.is_empty())
        || r.files.values().map(String::len).sum::<usize>() > 1024 * 1024
    {
        return Err("invalid reproduction contract".into());
    }
    for path in r.files.keys() {
        crate::review::safe_path(path)?;
        if !path.starts_with(".insi-review/") {
            return Err("reproduction files must be under .insi-review/".into());
        }
    }
    Ok(())
}

fn install(repo: &Path, r: &Reproduction) -> Result<(), String> {
    use std::io::Write;
    let mut ignore = std::fs::OpenOptions::new()
        .append(true)
        .open(repo.join(".git/info/exclude"))
        .map_err(|e| e.to_string())?;
    writeln!(ignore, "\n/.insi-review/").map_err(|e| e.to_string())?;
    for (name, text) in &r.files {
        let target = repo.join(name);
        let mut parent = repo.to_path_buf();
        for component in Path::new(name)
            .parent()
            .ok_or("invalid reproduction parent")?
            .components()
        {
            parent.push(component);
            if let Ok(meta) = std::fs::symlink_metadata(&parent) {
                if !meta.is_dir() || meta.file_type().is_symlink() {
                    return Err("reproduction parent is not a real directory".into());
                }
            } else {
                std::fs::create_dir(&parent).map_err(|e| e.to_string())?;
            }
        }
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(target)
            .map_err(|e| e.to_string())?;
        file.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn run(
    records: &[Reproduction],
    base: &Path,
    head: &Path,
    config: &Config,
    output: &Path,
) -> Result<Vec<Value>, String> {
    let mut reports = Vec::new();
    for r in records {
        validate(r)?;
        install(base, r)?;
        install(head, r)?;
        let mut observed = Vec::new();
        for (label, repo) in [("baseline", base), ("head", head)] {
            let wrapped = config.sandbox.as_ref().map(|s| s.wrap(&r.command, repo));
            let result = insi_eval::process::run(
                wrapped.as_deref().unwrap_or(&r.command),
                repo,
                Duration::from_secs(300),
            )?;
            let text = format!("{}\n{}", result.stdout, result.stderr);
            std::fs::write(output.join(format!("repro-{}-{label}.log", r.id)), &text)
                .map_err(|e| e.to_string())?;
            observed.push(json!({"code":result.code,"timed_out":result.timed_out,"assertion_seen":text.contains(&r.assertion_marker)}));
        }
        let passed = |v: &Value| v["code"] == 0 && v["timed_out"] == false;
        let failed = |v: &Value| {
            v["code"] == r.failure_exit && v["timed_out"] == false && v["assertion_seen"] == true
        };
        let status = if passed(&observed[0]) && failed(&observed[1]) {
            "reproduced-regression"
        } else if failed(&observed[0]) && passed(&observed[1]) {
            "verified-fix"
        } else if passed(&observed[0]) && passed(&observed[1]) {
            "not-reproduced"
        } else {
            "inconclusive"
        };
        reports.push(json!({"id":r.id,"file":r.file,"line":r.line,"rule":r.rule,"message":r.message,"status":status,"baseline":observed[0],"head":observed[1],"limits":"Observed command exits and assertion marker; representativeness and causal diagnosis still require review."}));
    }
    Ok(reports)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    fn fixture() -> std::path::PathBuf {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.incord/temp");
        std::fs::create_dir_all(&root).unwrap();
        let d = root.join(format!(
            "repro-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&d).unwrap();
        d
    }
    #[test]
    fn frozen_reproduction_distinguishes_regression_from_environment_failure() {
        let d = fixture();
        let base = d.join("base");
        let head = d.join("head");
        for repo in [&base, &head] {
            std::fs::create_dir(repo).unwrap();
            insi_eval::git::git(repo, &["init", "--quiet"]).unwrap();
        }
        std::fs::write(base.join("value"), "ok").unwrap();
        std::fs::write(head.join("value"), "bad").unwrap();
        let r:Reproduction=serde_json::from_value(json!({"id":"invariant","file":"value","line":1,"rule":"value-contract","message":"value changed","command":["sh",".insi-review/invariant.sh"],"files":{".insi-review/invariant.sh":"if [ \"$(cat value)\" = ok ]; then exit 0; fi; echo ASSERT_VALUE; exit 1"},"assertion_marker":"ASSERT_VALUE"})).unwrap();
        let result = run(&[r], &base, &head, &Config::default(), &d).unwrap();
        assert_eq!(result[0]["status"], "reproduced-regression");
        let r:Reproduction=serde_json::from_value(json!({"id":"environment","file":"value","line":1,"rule":"value-contract","message":"value changed","command":["sh",".insi-review/environment.sh"],"files":{".insi-review/environment.sh":"exit 1"},"assertion_marker":"ASSERT_VALUE"})).unwrap();
        let result = run(&[r], &base, &head, &Config::default(), &d).unwrap();
        assert_eq!(result[0]["status"], "inconclusive");
        std::fs::remove_dir_all(d).unwrap();
    }
}
