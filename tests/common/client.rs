use std::process::Stdio;

use rmcp::{
    ClientLifecycleMode, ClientServiceExt, RoleClient,
    model::{
        CallToolRequestParams, CallToolResponse, ClientCapabilities, ClientInfo, GetTaskParams,
        ProtocolVersion,
    },
    service::{RunningService, ServiceError},
    transport::TokioChildProcess,
};
use serde_json::Value;

pub const TULS_BIN: &str = env!("CARGO_BIN_EXE_tuls");

/// The tuls binary path escaped for embedding inside a TOML basic string.
/// On Windows the path contains backslashes, which are escape characters in
/// TOML, so they must be doubled.
pub fn toml_tuls_bin() -> String {
    TULS_BIN.replace('\\', "\\\\")
}

/// A test client that declares the MCP Tasks extension capability, so task-
/// based tools (`execute_command`, `spawn_agent`, `send_input`) are usable.
fn client_info() -> ClientInfo {
    let mut info = ClientInfo::default();
    info.protocol_version = ProtocolVersion::V_2026_07_28;
    info.capabilities = ClientCapabilities::builder().enable_tasks().build();
    info
}

pub struct TulsServer {
    pub service: RunningService<RoleClient, ClientInfo>,
}

impl TulsServer {
    pub async fn connect(args: &[&str], env_pairs: &[(&str, String)]) -> Self {
        let mut command = tokio::process::Command::new(TULS_BIN);
        command
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        for (key, value) in env_pairs {
            command.env(key, value);
        }
        let transport = TokioChildProcess::builder(command)
            .spawn()
            .expect("spawn tuls server")
            .0;
        let lifecycle = ClientLifecycleMode::Discover {
            preferred_versions: vec![ProtocolVersion::V_2026_07_28],
        };
        let service = client_info()
            .serve_with_lifecycle(transport, lifecycle)
            .await
            .expect("connect to tuls server");
        Self { service }
    }

    pub async fn call(&self, name: &str, arguments: Value) -> Result<Value, ServiceError> {
        let request = CallToolRequestParams::new(name.to_string())
            .with_arguments(arguments.as_object().cloned().unwrap_or_default());
        // call_tool_once (not the MRTR-driving call_tool) so task-based tools
        // return their raw task handle for tasks/get polling.
        let response = self.service.call_tool_once(request).await?;
        Ok(match response {
            CallToolResponse::Complete(result) => {
                serde_json::to_value(result).expect("serialize tool result")
            }
            CallToolResponse::Task(result) => {
                serde_json::to_value(result).expect("serialize task handle")
            }
            CallToolResponse::InputRequired(result) => {
                serde_json::to_value(result).expect("serialize input-required result")
            }
            _ => return Err(ServiceError::UnexpectedResponse),
        })
    }

    pub async fn call_ok(&self, name: &str, arguments: Value) -> Value {
        let response = self
            .call(name, arguments)
            .await
            .unwrap_or_else(|error| panic!("tool {name:?} transport error: {error:?}"));
        if response.get("isError").and_then(Value::as_bool) == Some(true) {
            panic!("tool {name:?} returned an error: {}", text_of(&response));
        }
        response
    }

    pub async fn get_task(&self, task_id: &str) -> Result<Value, ServiceError> {
        let request = GetTaskParams::new(task_id.to_string());
        let response = self.service.get_task(request).await?;
        Ok(serde_json::to_value(response).expect("serialize get task response"))
    }

    /// Poll `tasks/get` until the task reaches a terminal status or the
    /// timeout elapses; returns the terminal task payload.
    pub async fn wait_for_task(&self, task_id: &str, timeout_ms: u64) -> Value {
        let started = std::time::Instant::now();
        loop {
            let task = self
                .get_task(task_id)
                .await
                .unwrap_or_else(|error| panic!("tasks/get {task_id:?}: {error:?}"));
            let status = task.get("status").and_then(Value::as_str).unwrap_or("?");
            if matches!(status, "completed" | "failed" | "cancelled") {
                return task;
            }
            if started.elapsed().as_millis() >= u128::from(timeout_ms) {
                panic!(
                    "task {task_id:?} did not reach a terminal state within {timeout_ms} ms: {task}"
                );
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }
}

pub fn text_of(response: &Value) -> String {
    response
        .get("content")
        .and_then(Value::as_array)
        .map(|blocks| {
            blocks
                .iter()
                .filter_map(|block| block.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

pub fn read_file(path: &std::path::Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}
