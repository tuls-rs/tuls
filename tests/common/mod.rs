//! Shared helpers for the integration test binaries.
//!
//! Every helper spawns the real compiled `tuls` binary (`CARGO_BIN_EXE_tuls`)
//! as a stdio MCP server and drives it with an rmcp client.

mod client;
mod client_ext;

pub use client::*;

use std::{
    io::{Read, Write},
    process::Stdio,
    time::Duration,
};

use rmcp::model::Tool;
use serde_json::{Value, json};

pub fn tool_names(tools: &[Tool]) -> Vec<String> {
    tools
        .iter()
        .map(|tool| tool.name.to_string())
        .collect::<Vec<_>>()
}

pub fn spawn_and_exit(args: &[&str]) -> std::process::Output {
    std::process::Command::new(TULS_BIN)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run tuls process")
}

/// Small thread-backed HTTP server used to stand in for a provider or a local
/// website in tests. Serves one canned response per connection and closes.
pub struct MiniHttpServer {
    pub addr: std::net::SocketAddr,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl MiniHttpServer {
    /// Serve `respond(request_line, request_body) -> (status, body)` for a
    /// bounded number of requests.
    pub fn spawn<F>(respond: F) -> Self
    where
        F: Fn(&str, &str) -> (u16, String) + Send + 'static,
    {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind test server");
        listener
            .set_nonblocking(true)
            .expect("nonblocking test server");
        let addr = listener.local_addr().expect("test server address");
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop_flag = stop.clone();
        let handle = std::thread::spawn(move || {
            let mut served = 0;
            while !stop_flag.load(std::sync::atomic::Ordering::Relaxed) && served < 64 {
                let (mut stream, _) = match listener.accept() {
                    Ok(accepted) => accepted,
                    Err(_) => {
                        std::thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                };
                served += 1;
                stream
                    .set_nonblocking(false)
                    .expect("blocking accepted stream");
                stream
                    .set_read_timeout(Some(Duration::from_secs(10)))
                    .expect("read timeout");
                let mut request = Vec::new();
                let mut buffer = [0u8; 4096];
                while let Ok(read) = stream.read(&mut buffer) {
                    if read == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..read]);
                    if request.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                let request_text = String::from_utf8_lossy(&request);
                let request_line = request_text.lines().next().unwrap_or_default();
                let body_start = request_text.find("\r\n\r\n").map(|i| i + 4).unwrap_or(0);
                let request_body = &request_text[body_start.min(request_text.len())..];
                let (status, body) = respond(request_line, request_body);
                let redirect = if (300..400).contains(&status) {
                    "Location: /redirect-target\r\n"
                } else {
                    ""
                };
                let response = format!(
                    "HTTP/1.1 {status} X\r\n{redirect}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
        });
        Self {
            addr,
            stop,
            handle: Some(handle),
        }
    }

    pub fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.addr.port())
    }
}

impl Drop for MiniHttpServer {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        let Some(handle) = self.handle.take() else {
            return;
        };
        let _ = handle.join();
    }
}

/// Serialize a value into a JSON text response body.
pub fn json_response(value: Value) -> String {
    value.to_string()
}

/// A Responses API response that completes immediately with `text`.
pub fn completed_responses(text: &str) -> String {
    json_response(json!({
        "id": "r_test",
        "object": "response",
        "status": "completed",
        "output": [{
            "type": "message",
            "id": "m_test",
            "role": "assistant",
            "content": [{ "type": "output_text", "text": text }],
            "phase": "final_answer",
        }],
        "output_text": text,
    }))
}

/// A Responses API response that requests a single function call.
pub fn function_call_responses(call_id: &str, name: &str, arguments: &str) -> String {
    json_response(json!({
        "id": "r_function",
        "object": "response",
        "status": "completed",
        "output": [{
            "type": "function_call",
            "id": call_id,
            "call_id": call_id,
            "name": name,
            "arguments": arguments,
        }],
        "output_text": "",
    }))
}
