//! Optional delivery to the existing Incord engine; run evidence stays in Insi.
use std::{net::SocketAddr, path::Path};

use insi_memory::incord::tool_call;
use serde_json::{json, Value};

fn save(path: &Path, value: &Value) -> Result<(), String> {
    use std::io::Write;
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let temporary = path.with_extension(format!("{}-{stamp}.pending", std::process::id()));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|e| e.to_string())?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|e| e.to_string())?;
    drop(file);
    std::fs::rename(&temporary, path).map_err(|e| e.to_string())
}

/// Retrieve once, before discovery. Failures deliberately leave an empty context.
pub fn context(addr: SocketAddr, repo: &Path, dir: &Path) -> String {
    let args = json!({"cwd":repo,"query":"Suggest one optional improvement to working code. Relevant measured improvements, failed attempts, constraints and code references.","budget_tokens":2000});
    let result = tool_call(addr, "context", args);
    let (text, status) = match result {
        Ok(value) => {
            let text = value
                .get("pack")
                .and_then(Value::as_str)
                .or_else(|| value.as_str())
                .unwrap_or("");
            (text.chars().take(12000).collect::<String>(), "retrieved")
        }
        Err(_) => (String::new(), "unavailable"),
    };
    // Context is evidence local to this run, never part of the published patch.
    if save(
        &dir.join("memory-context.json"),
        &json!({"status":status,"cwd":repo,"text":text}),
    )
    .is_err()
    {
        eprintln!("insi: could not save memory context; continuing without it");
        return String::new();
    }
    text
}

/// Persist the exact request before attempting delivery. Never export raw logs/errors.
pub fn finish(
    addr: SocketAddr,
    repo: &Path,
    dir: &Path,
    original: &str,
    report: &Value,
) -> Result<(), String> {
    let run = dir
        .file_name()
        .and_then(|v| v.to_str())
        .ok_or("invalid run directory")?;
    let status = report["status"].as_str().unwrap_or("failed");
    let status = match status {
        "draft-pr" | "improved-local" | "kept" => status,
        _ => "failed",
    };
    let digest = insi_eval::git::git(
        repo,
        &[
            "hash-object",
            "--no-filters",
            dir.join("report.json")
                .to_str()
                .ok_or("non-UTF8 report path")?,
        ],
    )?;
    let mut summary = json!({"run":run,"baseline_commit":original,"report_git_blob":digest,
        "artifact":dir.join("report.json"),"scope":"measured workload only; no generalization claim"});
    for field in ["samples", "required_relative_margin"] {
        if report[field].is_number() {
            summary[field] = report[field].clone();
        }
    }
    if let Some(url) = report["url"]
        .as_str()
        .filter(|v| v.starts_with("https://") && v.len() <= 2048)
    {
        summary["pr_url"] = json!(url);
    }
    if let Ok(bytes) = std::fs::read(dir.join("suggestion.json")) {
        if let Ok(suggestion) = serde_json::from_slice::<Value>(&bytes) {
            for field in ["metric", "unit", "direction"] {
                if let Some(value) = suggestion[field].as_str() {
                    summary[field] = json!(value.chars().take(200).collect::<String>());
                }
            }
        }
    }
    if let Ok(commit) = insi_eval::git::git(&dir.join("candidate"), &["rev-parse", "HEAD"]) {
        summary["candidate_commit"] = json!(commit);
    }
    // The error itself may include command output. Only retain the runner's phase.
    if let Ok(bytes) = std::fs::read(dir.join("state.json")) {
        if let Ok(state) = serde_json::from_slice::<Value>(&bytes) {
            if let Some(phase) = state["phase"].as_str() {
                summary["last_phase"] = json!(phase.chars().take(80).collect::<String>());
            }
        }
    }
    if let Some(benchmark) = report.get("benchmark") {
        // Only numeric/boolean measurements, never arbitrary output strings.
        let mut metrics = serde_json::Map::new();
        if let Some(fields) = benchmark.as_object() {
            for (key, value) in fields {
                if matches!(
                    key.as_str(),
                    "mean_relative_gain"
                        | "sign_test_p"
                        | "improved"
                        | "interval_95"
                        | "baseline"
                        | "candidate"
                ) {
                    metrics.insert(key.clone(), value.clone());
                }
            }
        }
        summary["benchmark"] = Value::Object(metrics);
    }
    let args = json!({"cwd":repo,"agent_id":"insi","write_key":format!("suggest:{run}:terminal-v1"),
        "kind":"decision","record":{"decision":format!("Optional code improvement outcome: {status}"),
        "why":summary.to_string(),"tags":["insi","suggestion",format!("outcome:{status}")]}});
    let payload = serde_json::to_string(&args).map_err(|e| e.to_string())?;
    if !insi_core::secrets::scan_diff(&format!("+{payload}")).is_empty() {
        return Err("memory summary withheld by sensitive-data scan".into());
    }
    save(&dir.join("memory-pending.json"), &args)?;
    retry(addr, dir)
}

/// Replay only this run's immutable decision; never run a worker or publish a PR.
pub fn retry(addr: SocketAddr, dir: &Path) -> Result<(), String> {
    let receipt = dir.join("memory-receipt.json");
    if receipt.exists() {
        let saved: Value =
            serde_json::from_slice(&std::fs::read(&receipt).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        return if saved["stored"] == true && saved["id"].is_string() {
            Ok(())
        } else {
            Err("invalid saved memory receipt".into())
        };
    }
    let pending = dir.join("memory-pending.json");
    if std::fs::metadata(&pending)
        .map_err(|e| e.to_string())?
        .len()
        > 128 * 1024
    {
        return Err("memory request too large".into());
    }
    let bytes = std::fs::read(&pending).map_err(|e| e.to_string())?;
    if bytes.len() > 128 * 1024 {
        return Err("memory request too large".into());
    }
    let args: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if args["kind"] != "decision"
        || args["agent_id"] != "insi"
        || !args["cwd"].is_string()
        || !args["write_key"]
            .as_str()
            .is_some_and(|s| s.starts_with("suggest:") && s.len() <= 128)
        || !args["record"].is_object()
    {
        return Err("invalid pending suggestion decision".into());
    }
    if !insi_core::secrets::scan_diff(&format!("+{}", args)).is_empty() {
        return Err("pending memory request contains sensitive-data patterns".into());
    }
    let result = tool_call(addr, "lesson_write", args)?;
    if result["stored"] != true || !result["id"].is_string() {
        return Err("memory engine did not acknowledge a stored item".into());
    }
    // Keep the pending payload for exact replay after an interrupted receipt write.
    save(&receipt, &result)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::atomic::{AtomicU64, Ordering},
    };

    fn dir() -> std::path::PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.incord/temp");
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join(format!(
            "memory-test-{}-{}-{}",
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

    pub(crate) fn engine(result: Value) -> (SocketAddr, std::thread::JoinHandle<Value>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut raw = Vec::new();
            let request = loop {
                let mut buf = [0; 4096];
                let n = socket.read(&mut buf).unwrap();
                assert!(n > 0);
                raw.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&raw);
                if let Some((head, body)) = text.split_once("\r\n\r\n") {
                    let len: usize = head
                        .lines()
                        .find_map(|l| l.strip_prefix("Content-Length: "))
                        .unwrap()
                        .parse()
                        .unwrap();
                    if body.len() >= len {
                        break serde_json::from_str::<Value>(body).unwrap();
                    }
                }
            };
            let body = json!({"jsonrpc":"2.0","id":1,"result":result}).to_string();
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
            request
        });
        (addr, handle)
    }

    fn pending(dir: &Path) -> Value {
        let args = json!({"cwd":"/original/repository","agent_id":"insi","write_key":"suggest:test:terminal-v1", "kind":"decision","record":{"decision":"kept","why":"measured no gain"}});
        save(&dir.join("memory-pending.json"), &args).unwrap();
        args
    }

    #[test]
    fn context_is_bounded_saved_and_scoped_to_the_original_checkout() {
        let dir = dir();
        let (addr, handle) = engine(
            json!({"content":[{"type":"text","text":json!({"pack":"advice".repeat(3000)}).to_string()}]}),
        );
        let text = context(addr, Path::new("/original/repository"), &dir);
        assert_eq!(text.chars().count(), 12000);
        let call = handle.join().unwrap();
        assert_eq!(call["params"]["name"], "context");
        assert_eq!(call["params"]["arguments"]["cwd"], "/original/repository");
        assert!(dir.join("memory-context.json").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn refusal_keeps_exact_payload_for_retry_and_receipt_prevents_resending() {
        let dir = dir();
        let args = pending(&dir);
        let (addr, handle) = engine(json!({"isError":true,"content":[{"text":"refused"}]}));
        assert!(retry(addr, &dir).is_err());
        handle.join().unwrap();
        assert!(!dir.join("memory-receipt.json").exists());
        let (addr, handle) = engine(
            json!({"content":[{"text":json!({"stored":true,"duplicate":true,"id":"item-1"}).to_string()}]}),
        );
        retry(addr, &dir).unwrap();
        assert_eq!(handle.join().unwrap()["params"]["arguments"], args);
        // No listener remains: this must return using the stored receipt.
        retry(addr, &dir).unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn malformed_or_unacknowledged_responses_do_not_mark_delivery_complete() {
        let dir = dir();
        pending(&dir);
        let (addr, handle) = engine(json!({"structuredContent":{"stored":false}}));
        assert!(retry(addr, &dir).is_err());
        handle.join().unwrap();
        assert!(!dir.join("memory-receipt.json").exists());
        assert!(tool_call("192.0.2.1:9180".parse().unwrap(), "context", json!({})).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn export_excludes_raw_errors_and_retains_evidence_digest() {
        let dir = dir();
        let report = json!({"status":"failed","reason":"PRIVATE RAW WORKER OUTPUT"});
        save(&dir.join("report.json"), &report).unwrap();
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let (addr, handle) = engine(json!({"structuredContent":{"stored":true,"id":"item-2"}}));
        finish(addr, &repo, &dir, "abc123", &report).unwrap();
        let call = handle.join().unwrap();
        assert!(!call.to_string().contains("PRIVATE RAW"));
        assert!(call["params"]["arguments"]["record"]["why"]
            .as_str()
            .unwrap()
            .contains("report_git_blob"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
