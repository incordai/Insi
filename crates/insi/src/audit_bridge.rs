//! Delegate candidate edits to Incord's existing approval/coder path. No local fallback.
use insi_memory::incord::{http, http_wait};
use serde_json::{json, Value};
pub struct Engine {
    pub addr: SocketAddr,
    pub seconds: u64,
}
use std::{
    net::SocketAddr,
    path::Path,
    time::{Duration, Instant},
};

fn endpoint(id: &str, action: &str) -> Result<String, String> {
    if id.is_empty()
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    {
        return Err("invalid audit finding ID".into());
    }
    Ok(format!("/v1/memory/audit/findings/{id}{action}"))
}

pub fn finding(addr: SocketAddr, id: &str, repo: &Path) -> Result<Value, String> {
    let response = http(addr, "GET", &endpoint(id, "")?, &json!({}))?;
    let f = &response["finding"];
    if f["id"] != id
        || f["category"] != "improvement"
        || f["status"] != "approved"
        || !f["updated_at"].is_i64()
    {
        return Err("an approved improvement finding is required".into());
    }
    let file = Path::new(f["file"].as_str().ok_or("finding has no file")?);
    let file = std::fs::canonicalize(file).map_err(|e| e.to_string())?;
    if !file.starts_with(std::fs::canonicalize(repo).map_err(|e| e.to_string())?) {
        return Err("finding belongs to another project".into());
    }
    Ok(f.clone())
}

pub fn implement(
    engine: Engine,
    id: &str,
    finding: &Value,
    repo: &Path,
    candidate: &Path,
    baseline: &str,
    dir: &Path,
) -> Result<(), String> {
    let Engine { addr, seconds } = engine;
    if !(1..=3600).contains(&seconds) {
        return Err("audit candidate execution needs a 1..3600 second budget".into());
    }
    let request = json!({"project":repo,"candidate":candidate,"baseline":baseline,"finding_updated_at":finding["updated_at"]});
    let prepared = http(addr, "POST", &endpoint(id, "/candidate/prepare")?, &request)?;
    let token = prepared["token"]
        .as_str()
        .filter(|t| !t.is_empty())
        .ok_or("engine did not return an approval token")?;
    let mut request = request;
    request["approval_token"] = json!(token);
    std::fs::write(
        dir.join("engine-candidate.json"),
        serde_json::to_vec_pretty(&json!({"finding":id,"request":request}))
            .map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(seconds.min(3600));
    loop {
        let status = http(addr, "POST", &endpoint(id, "/candidate/status")?, &request)?;
        match status["status"].as_str() {
            Some("approved") => break,
            Some("pending") => {
                if Instant::now() >= deadline {
                    return Err("candidate approval timed out; no fallback edit was run".into());
                }
                std::thread::sleep(Duration::from_millis(500));
            }
            _ => {
                return Err(
                    "candidate approval denied, revised, expired or unavailable; no edit was run"
                        .into(),
                )
            }
        }
    }
    let result = http_wait(
        addr,
        "POST",
        &endpoint(id, "/candidate/execute")?,
        &request,
        deadline
            .saturating_duration_since(Instant::now())
            .as_secs()
            .clamp(1, 3600),
    )?;
    std::fs::write(
        dir.join("engine-execution.json"),
        serde_json::to_vec_pretty(&result).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    if result["status"] != "tested"
        || result["candidate"] != json!(candidate)
        || result["baseline"] != baseline
    {
        return Err("engine candidate was not successfully tested; nothing published".into());
    }
    Ok(())
}

pub fn report(addr: SocketAddr, dir: &Path, outcome: &Value) -> Result<(), String> {
    let request: Value = serde_json::from_slice(
        &std::fs::read(dir.join("engine-candidate.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let id = request["finding"].as_str().ok_or("missing finding")?;
    let body = json!({"approval_token":request["request"]["approval_token"],"status":outcome["status"],"evidence":outcome.get("url").cloned().unwrap_or_else(||json!(dir.join("report.json")))});
    http(addr, "POST", &endpoint(id, "/candidate/report")?, &body)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    fn engine(replies: Vec<Value>) -> (SocketAddr, std::thread::JoinHandle<Vec<String>>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let thread = std::thread::spawn(move || {
            let mut calls = Vec::new();
            let deadline = Instant::now() + Duration::from_secs(10);
            for reply in replies {
                let mut socket = loop {
                    match listener.accept() {
                        Ok((s, _)) => break s,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(Instant::now() < deadline, "missing expected engine call");
                            std::thread::sleep(Duration::from_millis(10));
                        }
                        Err(e) => panic!("{e}"),
                    }
                };
                socket.set_nonblocking(false).unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut bytes = Vec::new();
                loop {
                    let mut buf = [0; 4096];
                    let n = socket.read(&mut buf).unwrap();
                    assert!(n > 0);
                    bytes.extend_from_slice(&buf[..n]);
                    let text = String::from_utf8_lossy(&bytes);
                    if let Some((head, body)) = text.split_once("\r\n\r\n") {
                        let len: usize = head
                            .lines()
                            .find_map(|l| l.strip_prefix("Content-Length: "))
                            .unwrap()
                            .parse()
                            .unwrap();
                        if body.len() >= len {
                            calls.push(text.to_string());
                            break;
                        }
                    }
                }
                let body = reply.to_string();
                write!(
                    socket,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
            calls
        });
        (addr, thread)
    }
    #[test]
    fn denial_stops_before_execution_and_never_uses_a_direct_worker() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.incord/temp");
        std::fs::create_dir_all(&root).unwrap();
        let dir = root.join(format!(
            "bridge-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&dir).unwrap();
        let (addr, thread) = engine(vec![
            json!({"token":"approval-token"}),
            json!({"status":"denied-or-revised"}),
        ]);
        assert!(implement(
            Engine { addr, seconds: 10 },
            "finding-1",
            &json!({"updated_at":1}),
            &dir,
            &dir.join("candidate"),
            "base",
            &dir
        )
        .is_err());
        let calls = thread.join().unwrap();
        assert_eq!(calls.len(), 2);
        assert!(calls[0].starts_with("POST /v1/memory/audit/findings/finding-1/candidate/prepare"));
        assert!(!calls.iter().any(|c| c.contains("/execute")));
        assert!(dir.join("engine-candidate.json").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn invalid_ids_cannot_inject_an_endpoint() {
        assert!(endpoint("../other", "/execute").is_err());
    }
}
