//! Verified lessons into Incord Memory, where every agent on the machine
//! reads them.
//!
//! The benchmark keeps its lessons in a file of its own ([`crate::store`]) so
//! an experiment can be reset and repeated. This is the production path: a
//! lesson learned from real work is written to the memory engine as an
//! `issue` item through its `lesson_write` tool, filed under the project of
//! the repository it came from (the `cwd` the engine resolves), and tagged
//! so it can be told apart from what a person wrote.
//!
//! Local only: the engine listens on `127.0.0.1`, and an address anywhere
//! else is refused rather than sent a lesson. No HTTP library: one POST over
//! a plain socket, with a deadline on every read and write.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use insi_core::Lesson;
use serde_json::{json, Value};

/// Where the engine listens unless `INCORD_MEMORY_ADDR` says otherwise.
pub const DEFAULT_ADDR: &str = "127.0.0.1:9180";
/// The environment variable naming the engine's address.
pub const ADDR_ENV: &str = "INCORD_MEMORY_ADDR";
/// The engine's JSON-RPC route, where every tool is reachable.
pub const RPC_PATH: &str = "/v1/memory/mcp";
/// How long one request may take.
pub const DEADLINE: Duration = Duration::from_secs(15);

/// The engine's address: [`ADDR_ENV`], else [`DEFAULT_ADDR`]. Refused unless
/// it is on this machine.
pub fn address() -> Result<SocketAddr, String> {
    let raw = std::env::var(ADDR_ENV).unwrap_or_else(|_| DEFAULT_ADDR.to_string());
    let addr: SocketAddr = raw
        .trim()
        .parse()
        .map_err(|_| format!("{ADDR_ENV}=\"{raw}\" is not an address like {DEFAULT_ADDR}"))?;
    if !addr.ip().is_loopback() {
        return Err(format!(
            "{ADDR_ENV}={addr} is not on this machine; lessons are only sent to a local engine"
        ));
    }
    Ok(addr)
}

/// The `lesson_write` call for one lesson, filed under the repository at
/// `repo` (the engine resolves the project from it).
pub fn lesson_write_request(lesson: &Lesson, repo: &str) -> Value {
    let mut fix = lesson.use_.trim().to_string();
    if let Some(avoid) = &lesson.avoid {
        fix.push_str(&format!(" Not: {}", avoid.trim()));
    }
    json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "lesson_write",
            "arguments": {
                "kind": "issue",
                "cwd": repo,
                "task_id": lesson.evidence,
                "record": {
                    "issue": lesson.issue,
                    "root_cause": lesson.root_cause,
                    "fix": fix,
                    "tags": [
                        "insi",
                        format!("verified:{}", verification_name(lesson)),
                        format!("topic:{}", lesson.topic),
                    ],
                },
            },
        },
    })
}

fn verification_name(lesson: &Lesson) -> &'static str {
    match lesson.verified_by {
        insi_core::Verification::Human => "human",
        insi_core::Verification::Check => "check",
        insi_core::Verification::AgentClaim => "agent-claim",
    }
}

/// Write one lesson to the engine at `addr`. An error names what went wrong:
/// the engine unreachable, an HTTP failure, or the tool refusing the item.
pub fn publish(addr: SocketAddr, lesson: &Lesson, repo: &str) -> Result<(), String> {
    let body = lesson_write_request(lesson, repo).to_string();
    let reply = post(addr, RPC_PATH, &body)?;
    let value: Value = serde_json::from_str(&reply)
        .map_err(|e| format!("the memory engine's reply is not JSON: {e}"))?;
    if let Some(error) = value.get("error") {
        return Err(format!("the memory engine refused the lesson: {error}"));
    }
    if value["result"]["isError"] == true {
        let why = value["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or("no reason given");
        return Err(format!("the memory engine refused the lesson: {why}"));
    }
    Ok(())
}

/// One HTTP/1.1 POST of a JSON body; the response body on a 2xx.
fn post(addr: SocketAddr, path: &str, body: &str) -> Result<String, String> {
    let unreachable =
        |e: std::io::Error| format!("the memory engine at {addr} is unreachable: {e}");
    let mut stream = TcpStream::connect_timeout(&addr, DEADLINE).map_err(unreachable)?;
    stream
        .set_read_timeout(Some(DEADLINE))
        .map_err(unreachable)?;
    stream
        .set_write_timeout(Some(DEADLINE))
        .map_err(unreachable)?;
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|e| format!("sending to the memory engine: {e}"))?;
    let mut raw = Vec::new();
    stream
        .take(4 * 1024 * 1024)
        .read_to_end(&mut raw)
        .map_err(|e| format!("reading the memory engine's reply: {e}"))?;
    let text = String::from_utf8_lossy(&raw);
    let (head, rest) = text
        .split_once("\r\n\r\n")
        .ok_or("the memory engine's reply has no HTTP header")?;
    let status: u16 = head
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or("the memory engine's reply has no status")?;
    if !(200..300).contains(&status) {
        return Err(format!(
            "the memory engine answered {status}: {}",
            rest.chars().take(300).collect::<String>()
        ));
    }
    let chunked = head
        .lines()
        .any(|l| l.to_ascii_lowercase().replace(' ', "") == "transfer-encoding:chunked");
    Ok(if chunked {
        dechunk(rest)
    } else {
        rest.to_string()
    })
}

/// A chunked body, joined.
fn dechunk(mut rest: &str) -> String {
    let mut out = String::new();
    while let Some((size, after)) = rest.split_once("\r\n") {
        let Ok(n) = usize::from_str_radix(size.trim(), 16) else {
            break;
        };
        if n == 0 || after.len() < n {
            break;
        }
        out.push_str(&after[..n]);
        rest = after[n..].trim_start_matches("\r\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use insi_core::Verification;
    use std::net::TcpListener;

    fn lesson() -> Lesson {
        Lesson {
            id: "watchman-1-worker-s0".into(),
            topic: "watchman".into(),
            issue: "The decoder was not exported.".into(),
            root_cause: "Tests import decodePtyChunk from src/pty.ts.".into(),
            use_: "Export helpers the tests import.".into(),
            avoid: Some("Keeping them module-private".into()),
            signature: String::new(),
            evidence: "watchman-1".into(),
            learned_at: None,
            verified_by: Verification::Check,
        }
    }

    #[test]
    fn a_lesson_becomes_an_issue_filed_under_its_repository() {
        let call = lesson_write_request(&lesson(), "D:/Projects/watchman");
        let args = &call["params"]["arguments"];
        assert_eq!(call["params"]["name"], "lesson_write");
        assert_eq!(args["kind"], "issue");
        assert_eq!(args["cwd"], "D:/Projects/watchman");
        assert_eq!(args["task_id"], "watchman-1");
        assert_eq!(
            args["record"]["fix"],
            "Export helpers the tests import. Not: Keeping them module-private"
        );
        let tags: Vec<&str> = args["record"]["tags"]
            .as_array()
            .expect("tags")
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert_eq!(tags, ["insi", "verified:check", "topic:watchman"]);
    }

    /// One request against a stand-in engine on a free local port: the
    /// lesson arrives as the JSON-RPC call, and the engine's verdict — taken,
    /// or refused with a reason — comes back as Ok or the reason.
    fn engine_replying(reply: &'static str) -> (SocketAddr, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let handle = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().expect("accept");
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("timeout");
            let mut seen = Vec::new();
            let mut buf = [0u8; 4096];
            // Read until the whole body has arrived.
            loop {
                let n = socket.read(&mut buf).unwrap_or(0);
                if n == 0 {
                    break;
                }
                seen.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&seen);
                if let Some((head, body)) = text.split_once("\r\n\r\n") {
                    let len: usize = head
                        .lines()
                        .find_map(|l| l.strip_prefix("Content-Length: "))
                        .and_then(|v| v.trim().parse().ok())
                        .unwrap_or(0);
                    if body.len() >= len {
                        break;
                    }
                }
            }
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{reply}",
                reply.len()
            );
            socket.write_all(response.as_bytes()).expect("reply");
            String::from_utf8_lossy(&seen).into_owned()
        });
        (addr, handle)
    }

    #[test]
    fn a_lesson_is_posted_and_the_engines_verdict_comes_back() {
        let (addr, engine) = engine_replying(
            r#"{"jsonrpc":"2.0","id":1,"result":{"content":[{"type":"text","text":"stored"}],"isError":false}}"#,
        );
        publish(addr, &lesson(), "D:/Projects/watchman").expect("taken");
        let request = engine.join().expect("engine");
        assert!(
            request.starts_with("POST /v1/memory/mcp HTTP/1.1"),
            "{request}"
        );
        assert!(request.contains("\"lesson_write\""), "{request}");

        let (addr, engine) = engine_replying(
            r#"{"jsonrpc":"2.0","id":1,"result":{"content":[{"type":"text","text":"a code block is not allowed"}],"isError":true}}"#,
        );
        let why = publish(addr, &lesson(), "D:/x").expect_err("refused");
        assert!(why.contains("a code block is not allowed"), "{why}");
        engine.join().expect("engine");
    }

    #[test]
    fn only_a_local_engine_is_sent_lessons() {
        std::env::set_var(ADDR_ENV, "10.0.0.5:9180");
        assert!(address()
            .expect_err("remote")
            .contains("not on this machine"));
        std::env::set_var(ADDR_ENV, "not an address");
        assert!(address().is_err());
        std::env::remove_var(ADDR_ENV);
        assert_eq!(address().expect("default").to_string(), DEFAULT_ADDR);
    }

    #[test]
    fn a_chunked_body_is_joined() {
        assert_eq!(dechunk("4\r\nWiki\r\n5\r\npedia\r\n0\r\n\r\n"), "Wikipedia");
    }
}
