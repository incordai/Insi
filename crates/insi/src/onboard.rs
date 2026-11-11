//! Inspect and verify setup using existing detection, doctor and review execution.
use serde_json::{json, Value};
use std::path::Path;

pub fn run(repo: &Path, output: &Path, verify: bool) -> Result<(), String> {
    std::fs::create_dir(output).map_err(|e| format!("onboarding output must be new: {e}"))?;
    let profile = crate::benchmark::detect(repo);
    let commands: Vec<Value> = profile["projects"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| p.get("suggested_check").cloned())
        .collect();
    let plan = json!({"version":1,"tools":[],"full_checks":commands,"tests":[],"graph":[]});
    let path = output.join("review.json");
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&plan).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    std::fs::write(
        output.join("profile.json"),
        serde_json::to_vec_pretty(&profile).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let mut status = "needs_configuration";
    if verify {
        if commands.is_empty() {
            return Err(
                "no detected check command; configure review.json before verification".into(),
            );
        }
        crate::review::run(
            repo,
            &path,
            &output.join("verification"),
            Some("HEAD"),
            None,
            false,
            false,
        )?;
        status = "checks-verified";
    }
    println!("{}",serde_json::to_string_pretty(&json!({"status":status,"profile":profile,"review_plan":path,"next":"Review commands and add local-tool adapters, a production sandbox and project-specific benchmarks. No existing configuration was overwritten; nothing was installed."})).map_err(|e|e.to_string())?);
    Ok(())
}
