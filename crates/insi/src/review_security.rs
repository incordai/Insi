//! Native security scanner imports. Findings remain allegations until reproduced.
//! No scanner is installed or invoked here; trusted review plans own execution.

use serde_json::{json, Value};

/// Convert native reports to review JSON without copying secret matches or source snippets.
/// Missing report structure and partial scans are errors, never empty clean reports.
pub fn convert(format: &str, report: &Value) -> Result<Value, String> {
    let mut findings = Vec::new();
    match format {
        "semgrep" => {
            let errors = report["errors"]
                .as_array()
                .ok_or("Semgrep errors missing")?;
            if !errors.is_empty() {
                return Err("Semgrep scan is incomplete".into());
            }
            for row in report["results"]
                .as_array()
                .ok_or("Semgrep results missing")?
            {
                findings.push(json!({"file":row["path"],"line":row["start"]["line"],"rule":row["check_id"],"message":row["extra"]["message"],"severity":severity(row["extra"]["severity"].as_str().unwrap_or("WARNING"))}));
            }
        }
        "gitleaks" => {
            for row in report
                .as_array()
                .ok_or("Gitleaks output must be an array")?
            {
                // Secret and Match intentionally never enter a publishable finding.
                findings.push(json!({"file":row["File"],"line":row["StartLine"],"rule":row["RuleID"],"message":"Potential credential detected; rotate it and remove it from source/history.","severity":"error"}));
            }
        }
        "trivy" => {
            if report["SchemaVersion"].as_u64() != Some(2) {
                return Err("unsupported Trivy schema".into());
            }
            for result in report["Results"]
                .as_array()
                .ok_or("Trivy Results missing")?
            {
                let file = result["Target"].as_str().ok_or("Trivy target missing")?;
                for key in ["Vulnerabilities", "Misconfigurations", "Secrets"] {
                    let Some(rows) = result.get(key).filter(|value| !value.is_null()) else {
                        continue;
                    };
                    for row in rows.as_array().ok_or("invalid Trivy finding array")? {
                        if key == "Misconfigurations" && row["Status"] == "PASS" {
                            continue;
                        }
                        let (rule, line, message) = match key {
                            "Vulnerabilities" => (
                                row["VulnerabilityID"].clone(),
                                1,
                                format!(
                                    "Dependency {} {} has vulnerability {}; fixed version: {}",
                                    row["PkgName"].as_str().unwrap_or("unknown"),
                                    row["InstalledVersion"].as_str().unwrap_or("unknown"),
                                    row["VulnerabilityID"].as_str().unwrap_or("unknown"),
                                    row["FixedVersion"].as_str().unwrap_or("unavailable")
                                ),
                            ),
                            "Misconfigurations" => (
                                row["ID"].clone(),
                                row["CauseMetadata"]["StartLine"].as_u64().unwrap_or(1),
                                row["Message"]
                                    .as_str()
                                    .unwrap_or("Configuration policy violation")
                                    .to_string(),
                            ),
                            _ => (
                                row["RuleID"].clone(),
                                row["StartLine"].as_u64().unwrap_or(1),
                                "Potential credential detected; matched value withheld.".into(),
                            ),
                        };
                        findings.push(json!({"file":file,"line":line,"rule":rule,"message":message,"severity":severity(row["Severity"].as_str().unwrap_or("UNKNOWN"))}));
                    }
                }
            }
        }
        "cargo-audit" => {
            if report.get("error").is_some_and(|value| !value.is_null()) {
                return Err("cargo audit failed".into());
            }
            for row in report["vulnerabilities"]["list"]
                .as_array()
                .ok_or("cargo audit vulnerabilities missing")?
            {
                findings.push(json!({"file":"Cargo.lock","line":1,"rule":row["advisory"]["id"],"message":format!("{} {}: {}",row["package"]["name"].as_str().unwrap_or("unknown"),row["package"]["version"].as_str().unwrap_or("unknown"),row["advisory"]["title"].as_str().unwrap_or("Dependency advisory")),"severity":"error"}));
            }
        }
        _ => return Err("unsupported security scanner format".into()),
    }
    if findings.len() > 5000 {
        return Err("security finding limit exceeded".into());
    }
    Ok(json!({"findings":findings}))
}

/// Map scanner-specific uppercase levels onto the existing review severity vocabulary.
fn severity(level: &str) -> &'static str {
    match level.to_ascii_lowercase().as_str() {
        "critical" | "high" | "error" => "error",
        "low" | "info" | "note" => "note",
        _ => "warning",
    }
}

/// Native reports never self-certify exploitability or expose a secret's matched bytes.
#[test]
fn security_scanner_reports_preserve_locations_and_withhold_matches() {
    // Real native field shapes for code, dependency and configuration scanners.
    let semgrep = json!({"errors":[],"results":[{"path":"app.py","start":{"line":7},"check_id":"sql-injection","extra":{"message":"Unparameterized query","severity":"ERROR"}}]});
    let rows = super::review::normalize("semgrep", &semgrep, "semgrep").expect("semgrep");
    assert_eq!(
        (
            rows[0].file.as_str(),
            rows[0].line,
            rows[0].severity.as_str()
        ),
        ("app.py", 7, "error")
    );
    let audit = json!({"vulnerabilities":{"list":[{"advisory":{"id":"RUSTSEC-example","title":"unsafe dependency"},"package":{"name":"example","version":"1.0"}}]}});
    assert_eq!(
        super::review::normalize("cargo-audit", &audit, "audit").expect("audit")[0].file,
        "Cargo.lock"
    );
    let trivy = json!({"SchemaVersion":2,"Results":[{"Target":"Dockerfile","Misconfigurations":[{"ID":"root-user","Status":"FAIL","Message":"Runs as root","Severity":"HIGH","CauseMetadata":{"StartLine":4}}],"Secrets":[{"RuleID":"token","StartLine":8,"Severity":"HIGH","Match":"never-publish-me"}]}]});
    let imported = convert("trivy", &trivy).expect("trivy");
    assert!(!imported.to_string().contains("never-publish-me"));
    assert_eq!(
        super::review::normalize("trivy", &trivy, "trivy")
            .expect("trivy")
            .len(),
        2
    );
    let value = convert("gitleaks", &json!([{"File":"src/config.rs","StartLine":3,"RuleID":"token","Secret":"sensitive-match","Match":"sensitive-match"}])).expect("gitleaks");
    assert!(!value.to_string().contains("sensitive-match"));
    let rows = super::review::normalize("json", &value, "gitleaks").expect("normalize");
    assert_eq!(rows[0].line, 3);
    assert_eq!(rows[0].verification, "tool-reported");
    assert!(convert(
        "semgrep",
        &json!({"results":[],"errors":[{"message":"timeout"}]})
    )
    .is_err());
    assert!(convert("trivy", &json!({})).is_err());
    assert!(convert("cargo-audit", &json!({})).is_err());
}
