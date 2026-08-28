use std::process::Stdio;

use rmcp::{
    ClientLifecycleMode, ClientServiceExt, RoleClient,
    model::{CallToolRequestParams, ProtocolVersion},
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

pub struct TulsServer {
    pub service: RunningService<RoleClient, ()>,
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
        let service =
            ().serve_with_lifecycle(transport, lifecycle)
                .await
                .expect("connect to tuls server");
        Self { service }
    }

    pub async fn call(&self, name: &str, arguments: Value) -> Result<Value, ServiceError> {
        let request = CallToolRequestParams::new(name.to_string())
            .with_arguments(arguments.as_object().cloned().unwrap_or_default());
        let response = self.service.call_tool(request).await?;
        Ok(serde_json::to_value(response).expect("serialize tool response"))
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

pub fn structured_of(response: &Value) -> Value {
    response
        .get("structuredContent")
        .cloned()
        .unwrap_or(Value::Null)
}

pub fn read_file(path: &std::path::Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}
