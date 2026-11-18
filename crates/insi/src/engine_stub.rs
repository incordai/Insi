//! A stand-in memory engine for the tests of the app hub (`issue.rs` `AppHub`, `harness_pr.rs`): a local server that
//! answers each `tools/call` of `POST /v1/memory/mcp` by the tool's name, through a function the test gives, and
//! remembers every call. No network beyond this machine; stopped when dropped.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

/// What the stand-in does with a call: its result, or the refusal the engine's tool would give.
pub type Handler = dyn Fn(&str, &Value) -> Result<Value, String> + Send + Sync;

/// A running stand-in engine.
pub struct StandIn {
    pub addr: SocketAddr,
    /// Every `(tool, arguments)` it was asked, in order.
    pub calls: Arc<Mutex<Vec<(String, Value)>>>,
    stop: Arc<AtomicBool>,
}

impl StandIn {
    /// The tools asked so far, in order.
    pub fn tools(&self) -> Vec<String> {
        self.calls
            .lock()
            .expect("calls")
            .iter()
            .map(|(t, _)| t.clone())
            .collect()
    }

    /// The arguments of the last call of `tool`.
    pub fn last(&self, tool: &str) -> Value {
        self.calls
            .lock()
            .expect("calls")
            .iter()
            .rev()
            .find(|(t, _)| t == tool)
            .map(|(_, a)| a.clone())
            .unwrap_or_else(|| panic!("{tool} was never called: {:?}", self.tools()))
    }
}

impl Drop for StandIn {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

/// Start a stand-in engine whose tools are answered by `handler`.
pub fn start(
    handler: impl Fn(&str, &Value) -> Result<Value, String> + Send + Sync + 'static,
) -> StandIn {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    listener.set_nonblocking(true).expect("nonblocking");
    let addr = listener.local_addr().expect("addr");
    let calls: Arc<Mutex<Vec<(String, Value)>>> = Arc::default();
    let stop = Arc::new(AtomicBool::new(false));
    let (seen, halt) = (calls.clone(), stop.clone());
    let handler: Arc<Handler> = Arc::new(handler);
    std::thread::spawn(move || {
        while !halt.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((mut socket, _)) => {
                    let _ = socket.set_nonblocking(false);
                    serve(&mut socket, &seen, handler.as_ref());
                }
                Err(_) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
    });
    StandIn { addr, calls, stop }
}

/// One request: read it whole, answer it, close.
fn serve(socket: &mut std::net::TcpStream, seen: &Mutex<Vec<(String, Value)>>, handler: &Handler) {
    let _ = socket.set_read_timeout(Some(Duration::from_secs(10)));
    let mut raw = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = socket.read(&mut buf).unwrap_or(0);
        if n == 0 {
            break;
        }
        raw.extend_from_slice(&buf[..n]);
        let text = String::from_utf8_lossy(&raw);
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
    let text = String::from_utf8_lossy(&raw).into_owned();
    let body = text.split_once("\r\n\r\n").map_or("", |(_, b)| b);
    let request: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let tool = request["params"]["name"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let arguments = request["params"]["arguments"].clone();
    seen.lock()
        .expect("calls")
        .push((tool.clone(), arguments.clone()));
    let reply = match handler(&tool, &arguments) {
        Ok(value) => json!({"jsonrpc":"2.0","id":1,"result":{"content":[{"type":"text","text":value.to_string()}],"isError":false}}),
        Err(why) => json!({"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":why}}),
    }
    .to_string();
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
        reply.len()
    );
    let _ = socket.write_all(response.as_bytes());
}
