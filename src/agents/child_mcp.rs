use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    future::Future,
    path::Path,
    sync::Arc,
    time::Duration,
};

use anyhow::{Result, anyhow, bail};
use rmcp::{
    ClientLifecycleMode, ClientServiceExt, RoleClient,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, CancelTaskParams,
        ClientCapabilities, ClientInfo, GetTaskParams, JsonObject, ProtocolVersion, TaskPayload,
    },
    service::RunningService,
    transport::{
        StreamableHttpClientTransport, streamable_http_client::StreamableHttpClientTransportConfig,
    },
};
use serde_json::Value;
use tokio::{process::Command, time::timeout};
use tokio_util::sync::CancellationToken;

use super::definition::{AgentDefinition, ChildToolPolicy, McpServerDefinition};
use super::timeouts::{
    CHILD_MCP_CALL_TIMEOUT, CHILD_MCP_SHUTDOWN_TIMEOUT, CHILD_MCP_STARTUP_TIMEOUT,
    PROVIDER_CONNECT_TIMEOUT,
};
use crate::support::{DEFAULT_TASK_POLL_INTERVAL_MS, configure_minimal_process_environment};
const MAX_OUTPUT_BYTES: usize = 1024 * 1024;
const MAX_DESCRIPTION_BYTES: usize = 4096;
const INTERRUPTION_ERROR: &str = "child_mcp_interrupted";
const MAX_CHILD_SERVERS: usize = 16;
const MAX_CHILD_TOOLS: usize = 256;
const MAX_SCHEMA_BYTES: usize = 1024 * 1024;
const MAX_TOTAL_SCHEMA_BYTES: usize = 4 * 1024 * 1024;

pub(crate) struct ChildTool {
    pub provider_name: String,
    pub description: String,
    pub input_schema: Arc<JsonObject>,
}

/// The minimal outcome of a dispatched child MCP tool call. `is_error`
/// preserves the tool-level `isError` flag reported by the child so callers
/// can commit an unambiguous tool result; transport outcomes are reported as
/// `ChildCallError` instead.
pub(crate) struct ChildToolResult {
    pub output: String,
    pub is_error: bool,
}

pub(crate) struct ChildMcpManager {
    connections: Vec<RunningService<RoleClient, ClientInfo>>,
    tools: Vec<ChildTool>,
    routes: BTreeMap<String, (usize, String)>,
}

impl ChildMcpManager {
    pub(crate) async fn connect(
        definition: &AgentDefinition,
        workspace: &Path,
        cancel: &CancellationToken,
    ) -> Result<Self> {
        if definition.mcp_servers.len() > MAX_CHILD_SERVERS {
            bail!("too many configured child MCP servers");
        }
        if !workspace.is_dir() {
            bail!("child MCP workspace is not a directory");
        }
        let mut connections = Vec::new();
        let mut catalog = Vec::new();
        let mut retained_schema_bytes = 0;
        for (server_name, server) in &definition.mcp_servers {
            if !definition.tool_policy.may_allow_server(server_name) {
                continue;
            }
            let connection = match connect_one(server, workspace, cancel).await {
                Ok(connection) => connection,
                Err(error) => {
                    close_all(&mut connections).await;
                    return Err(error.context("unable to connect child MCP server"));
                }
            };
            if connection
                .peer_info()
                .is_none_or(|info| info.protocol_version != ProtocolVersion::V_2026_07_28)
            {
                let mut connection = connection;
                let _ = connection
                    .close_with_timeout(CHILD_MCP_SHUTDOWN_TIMEOUT)
                    .await;
                close_all(&mut connections).await;
                bail!("child MCP server did not discover protocol 2026-07-28");
            }
            let index = connections.len();
            let listed = match wait_cancelled(cancel, connection.list_all_tools()).await {
                Ok(tools) => tools,
                Err(error) => {
                    let mut connection = connection;
                    let _ = connection
                        .close_with_timeout(CHILD_MCP_SHUTDOWN_TIMEOUT)
                        .await;
                    close_all(&mut connections).await;
                    return Err(error.context("unable to list child MCP tools"));
                }
            };
            if listed.len() > MAX_CHILD_TOOLS {
                let mut connection = connection;
                let _ = connection
                    .close_with_timeout(CHILD_MCP_SHUTDOWN_TIMEOUT)
                    .await;
                close_all(&mut connections).await;
                bail!("child MCP tool limit exceeded");
            }
            if let Err(error) = definition
                .tool_policy
                .validate_catalog(server_name, listed.iter().map(|tool| tool.name.as_ref()))
            {
                let mut connection = connection;
                let _ = connection
                    .close_with_timeout(CHILD_MCP_SHUTDOWN_TIMEOUT)
                    .await;
                close_all(&mut connections).await;
                return Err(error.context("invalid child MCP tool policy"));
            }
            for tool in listed {
                let original = tool.name.to_string();
                let qualified = qualified_name(server_name, &original);
                if permitted(&definition.tool_policy, server_name, &original) {
                    let schema_bytes = match schema_size(tool.input_schema.as_ref()) {
                        Ok(size) => size,
                        Err(_) => {
                            let mut connection = connection;
                            let _ = connection
                                .close_with_timeout(CHILD_MCP_SHUTDOWN_TIMEOUT)
                                .await;
                            close_all(&mut connections).await;
                            bail!("invalid child MCP tool schema");
                        }
                    };
                    if catalog_limits_exceeded(
                        catalog.len() + 1,
                        retained_schema_bytes,
                        schema_bytes,
                    ) {
                        let mut connection = connection;
                        let _ = connection
                            .close_with_timeout(CHILD_MCP_SHUTDOWN_TIMEOUT)
                            .await;
                        close_all(&mut connections).await;
                        bail!("child MCP tool or schema limit exceeded");
                    }
                    retained_schema_bytes += schema_bytes;
                    catalog.push((
                        qualified,
                        index,
                        original,
                        safe_description(
                            tool.description.as_deref().unwrap_or(""),
                            MAX_DESCRIPTION_BYTES,
                        ),
                        tool.input_schema,
                    ));
                }
            }
            connections.push(connection);
        }
        catalog.sort_by(|a, b| a.0.cmp(&b.0).then(a.2.cmp(&b.2)).then(a.1.cmp(&b.1)));
        let mut used = BTreeSet::new();
        let mut tools = Vec::with_capacity(catalog.len());
        let mut routes = BTreeMap::new();
        for (base, index, original, description, input_schema) in catalog {
            let name = unique_name(base, &mut used)?;
            routes.insert(name.clone(), (index, original));
            tools.push(ChildTool {
                provider_name: name,
                description,
                input_schema,
            });
        }
        Ok(Self {
            connections,
            tools,
            routes,
        })
    }

    pub(crate) fn tools(&self) -> &[ChildTool] {
        &self.tools
    }

    pub(crate) async fn call(
        &self,
        provider_name: &str,
        arguments: Value,
        cancel: &CancellationToken,
    ) -> std::result::Result<ChildToolResult, ChildCallError> {
        let Some((index, original)) = self.routes.get(provider_name) else {
            return Err(ChildCallError::Rejected);
        };
        let Value::Object(arguments) = arguments else {
            return Err(ChildCallError::Rejected);
        };
        let request = CallToolRequestParams::new(original.clone()).with_arguments(arguments);
        let response = tokio::select! {
            _ = cancel.cancelled() => return Err(ChildCallError::Interrupted),
            result = timeout(CHILD_MCP_CALL_TIMEOUT, self.connections[*index].call_tool_once(request)) => result.map_err(|_| ChildCallError::TimedOut)?,
        }
        .map_err(|_| ChildCallError::Failed)?;
        match response {
            CallToolResponse::Complete(result) => Ok(map_call_result(result)),
            CallToolResponse::Task(task) => {
                let connection = &self.connections[*index];
                wait_for_task_result(
                    connection,
                    &task.task.task_id,
                    task.task.poll_interval_ms,
                    cancel,
                )
                .await
            }
            CallToolResponse::InputRequired(_) => Err(ChildCallError::Failed),
            _ => Err(ChildCallError::Failed),
        }
    }

    pub(crate) async fn shutdown(&mut self) {
        close_all(&mut self.connections).await;
    }

    #[cfg(test)]
    pub(crate) fn empty() -> Self {
        Self {
            connections: Vec::new(),
            tools: Vec::new(),
            routes: BTreeMap::new(),
        }
    }
}

/// Poll `tasks/get` on a child connection until the task settles, converting
/// the terminal payload into a child tool result. Any non-completed terminal
/// state is ambiguous (the tool may have executed) and surfaces as a call
/// failure.
///
/// The child Task has no local lifetime cap: the parent agent turn timeout and
/// the child's own TTL govern its lifetime, and `CHILD_MCP_CALL_TIMEOUT` bounds
/// only each individual MCP RPC.
async fn wait_for_task_result(
    connection: &RunningService<RoleClient, ClientInfo>,
    task_id: &str,
    initial_poll_interval_ms: Option<u64>,
    cancel: &CancellationToken,
) -> std::result::Result<ChildToolResult, ChildCallError> {
    // The seed value from the initial CreateTaskResult governs the first wait;
    // every later wait follows the latest suggestion from tasks/get.
    let mut poll_interval_ms = initial_poll_interval_ms.unwrap_or(DEFAULT_TASK_POLL_INTERVAL_MS);
    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                cancel_child_task(connection, task_id).await;
                return Err(ChildCallError::Interrupted);
            }
            _ = tokio::time::sleep(Duration::from_millis(poll_interval_ms)) => {}
        }
        let result = tokio::select! {
            _ = cancel.cancelled() => {
                cancel_child_task(connection, task_id).await;
                return Err(ChildCallError::Interrupted);
            }
            result = timeout(CHILD_MCP_CALL_TIMEOUT, connection.get_task(GetTaskParams::new(task_id.to_owned()))) => {
                match result {
                    Err(_) => {
                        cancel_child_task(connection, task_id).await;
                        return Err(ChildCallError::TimedOut);
                    }
                    Ok(Err(_)) => return Err(ChildCallError::Failed),
                    Ok(Ok(result)) => result,
                }
            }
        };
        match result.task.payload {
            TaskPayload::Working => {}
            TaskPayload::InputRequired { .. } => {
                cancel_child_task(connection, task_id).await;
                return Err(ChildCallError::Failed);
            }
            TaskPayload::Completed { result } => {
                let call_result =
                    serde_json::from_value::<rmcp::model::CallToolResult>(Value::Object(result))
                        .map_err(|_| ChildCallError::Failed)?;
                return Ok(map_call_result(call_result));
            }
            TaskPayload::Failed { .. } | TaskPayload::Cancelled => {
                return Err(ChildCallError::Failed);
            }
            _ => return Err(ChildCallError::Failed),
        }
        poll_interval_ms = result
            .task
            .task
            .poll_interval_ms
            .unwrap_or(DEFAULT_TASK_POLL_INTERVAL_MS);
    }
}

/// Best-effort `tasks/cancel` for a child Task that is being abandoned. The
/// RPC is bounded by the per-request timeout so cleanup cannot hang; a
/// failure to cancel must not mask the original outcome.
async fn cancel_child_task(connection: &RunningService<RoleClient, ClientInfo>, task_id: &str) {
    let _ = timeout(
        CHILD_MCP_CALL_TIMEOUT,
        connection.cancel_task(CancelTaskParams::new(task_id.to_owned())),
    )
    .await;
}

async fn connect_one(
    server: &McpServerDefinition,
    workspace: &Path,
    cancel: &CancellationToken,
) -> Result<RunningService<RoleClient, ClientInfo>> {
    let lifecycle = ClientLifecycleMode::Discover {
        preferred_versions: vec![ProtocolVersion::V_2026_07_28],
    };
    match server {
        McpServerDefinition::Stdio { command, args, env } => {
            let mut process = Command::new(command);
            process
                .args(args)
                .current_dir(workspace)
                .kill_on_drop(true)
                .stderr(std::process::Stdio::inherit());
            configure_minimal_process_environment(&mut process);
            for (key, value) in env {
                process.env(key, interpolate(value)?);
            }
            let transport = rmcp::transport::TokioChildProcess::builder(process)
                .spawn()?
                .0;
            startup(
                cancel,
                client_info().serve_with_lifecycle(transport, lifecycle),
            )
            .await
        }
        McpServerDefinition::Http { url, headers } => {
            let client = reqwest::Client::builder()
                .connect_timeout(PROVIDER_CONNECT_TIMEOUT)
                .timeout(CHILD_MCP_CALL_TIMEOUT)
                .redirect(reqwest::redirect::Policy::none())
                .build()?;
            let mut config = http_config(url.as_str());
            let mut custom_headers = HashMap::new();
            for (name, value) in headers {
                let name = reqwest::header::HeaderName::from_bytes(name.as_bytes())
                    .map_err(|_| anyhow!("invalid child MCP HTTP header name"))?;
                let value = reqwest::header::HeaderValue::from_str(&interpolate(value)?)
                    .map_err(|_| anyhow!("invalid child MCP HTTP header value"))?;
                custom_headers.insert(name, value);
            }
            config.custom_headers = custom_headers;
            let transport = StreamableHttpClientTransport::with_client(client, config);
            startup(
                cancel,
                client_info().serve_with_lifecycle(transport, lifecycle),
            )
            .await
        }
    }
}

/// A client info declaring the MCP Tasks extension capability, so task-based
/// child tools (`execute_command`, `spawn_agent`, `send_input`) are usable.
fn client_info() -> ClientInfo {
    let mut info = ClientInfo::default();
    info.protocol_version = ProtocolVersion::V_2026_07_28;
    info.capabilities = ClientCapabilities::builder().enable_tasks().build();
    info
}

async fn startup<F>(
    cancel: &CancellationToken,
    future: F,
) -> Result<RunningService<RoleClient, ClientInfo>>
where
    F: Future<
        Output = std::result::Result<
            RunningService<RoleClient, ClientInfo>,
            rmcp::service::ClientInitializeError,
        >,
    >,
{
    tokio::select! {
        _ = cancel.cancelled() => bail!(INTERRUPTION_ERROR),
        result = timeout(CHILD_MCP_STARTUP_TIMEOUT, future) => result.map_err(|_| anyhow!("child MCP startup timed out"))?.map_err(|_| anyhow!("child MCP startup failed")),
    }
}

async fn wait_cancelled<T, F>(cancel: &CancellationToken, future: F) -> Result<T>
where
    F: Future<Output = std::result::Result<T, rmcp::service::ServiceError>>,
{
    tokio::select! { _ = cancel.cancelled() => bail!(INTERRUPTION_ERROR), result = timeout(CHILD_MCP_CALL_TIMEOUT, future) => result.map_err(|_| anyhow!("child MCP request timed out"))?.map_err(|_| anyhow!("child MCP request failed")), }
}

async fn close_all(connections: &mut [RunningService<RoleClient, ClientInfo>]) {
    let _ = timeout(CHILD_MCP_SHUTDOWN_TIMEOUT, async {
        for connection in connections {
            let _ = connection
                .close_with_timeout(CHILD_MCP_SHUTDOWN_TIMEOUT)
                .await;
        }
    })
    .await;
}

fn interpolate(value: &str) -> Result<String> {
    interpolate_with(value, |name| std::env::var(name).ok())
}

fn interpolate_with<F>(value: &str, mut resolve: F) -> Result<String>
where
    F: FnMut(&str) -> Option<String>,
{
    let mut output = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(start) = rest.find("${") {
        output.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else {
            bail!("malformed environment placeholder")
        };
        let name = &after[..end];
        if !valid_env_name(name) {
            bail!("malformed environment placeholder")
        }
        let resolved =
            resolve(name).ok_or_else(|| anyhow!("missing environment placeholder {name}"))?;
        output.push_str(&resolved);
        rest = &after[end + 1..];
    }
    output.push_str(rest);
    Ok(output)
}

fn valid_env_name(name: &str) -> bool {
    let mut chars = name.bytes();
    matches!(chars.next(), Some(b) if b.is_ascii_alphabetic() || b == b'_')
        && chars.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}
fn qualified_name(server: &str, tool: &str) -> String {
    let mut server = sanitize(server);
    let mut tool = sanitize(tool);
    if server.len() + 2 + tool.len() > 64 {
        server.truncate(31);
        tool.truncate(31);
    }
    format!("{server}__{tool}")
}
fn sanitize(value: &str) -> String {
    let mut out: String = value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if out.is_empty() {
        out.push('_');
    }
    out.truncate(64);
    out
}
fn unique_name(base: String, used: &mut BTreeSet<String>) -> Result<String> {
    if used.insert(base.clone()) {
        return Ok(base);
    }
    for n in 2..=MAX_CHILD_TOOLS.saturating_add(1) {
        let suffix = format!("_{n}");
        let mut candidate = base.clone();
        candidate.truncate(64usize.saturating_sub(suffix.len()));
        candidate.push_str(&suffix);
        if used.insert(candidate.clone()) {
            return Ok(candidate);
        }
    }
    bail!("child MCP tool name space exhausted")
}
fn http_config(url: &str) -> StreamableHttpClientTransportConfig {
    let mut config = StreamableHttpClientTransportConfig::with_uri(url.to_owned());
    config.reinit_on_expired_session = false;
    config
}
fn schema_size(schema: &JsonObject) -> Result<usize> {
    serde_json::to_vec(schema)
        .map(|serialized| serialized.len())
        .map_err(|_| anyhow!("invalid child MCP tool schema"))
}
fn catalog_limits_exceeded(
    tool_count: usize,
    retained_schema_bytes: usize,
    schema_bytes: usize,
) -> bool {
    tool_count > MAX_CHILD_TOOLS
        || schema_bytes > MAX_SCHEMA_BYTES
        || retained_schema_bytes.saturating_add(schema_bytes) > MAX_TOTAL_SCHEMA_BYTES
}
fn permitted(policy: &ChildToolPolicy, server: &str, original: &str) -> bool {
    policy.allows(server, original)
}

fn bounded_text(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.to_owned();
    }
    if limit < '…'.len_utf8() {
        return String::new();
    }
    let mut end = limit;
    while !value.is_char_boundary(end - '…'.len_utf8()) {
        end -= 1;
    }
    format!("{}…", &value[..end - '…'.len_utf8()])
}
fn safe_description(value: &str, limit: usize) -> String {
    let cleaned: String = value
        .chars()
        .map(|c| {
            if c.is_control() && c != '\n' && c != '\t' {
                ' '
            } else {
                c
            }
        })
        .collect();
    bounded_text(&cleaned, limit)
}
fn render_output(
    content: &[rmcp::model::ContentBlock],
    structured: Option<&Value>,
) -> Result<String, ()> {
    let value = serde_json::json!({ "content": content, "structuredContent": structured });
    let serialized = match serde_json::to_string(&value) {
        Ok(serialized) => serialized,
        Err(_) => return Err(()),
    };
    (serialized.len() <= MAX_OUTPUT_BYTES)
        .then_some(serialized)
        .ok_or(())
}

/// Preserve the child-reported tool-level `isError` alongside the rendered
/// output. An absent `isError` is `false` per the MCP spec, so a returned
/// `ChildToolResult` always carries a definite, wire-valid error flag.
fn map_call_result(result: CallToolResult) -> ChildToolResult {
    let rendered = render_output(&result.content, result.structured_content.as_ref());
    let output_error = rendered.is_err();
    ChildToolResult {
        output: rendered.unwrap_or_else(|()| {
            "child MCP result exceeds the output size limit; narrow the request".into()
        }),
        is_error: result.is_error.unwrap_or(false) || output_error,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChildCallError {
    /// The run was cancelled before the child reported a definitive outcome.
    Interrupted,
    /// The call was dispatched but the child did not respond within the
    /// call timeout: the tool may or may not have executed.
    TimedOut,
    /// The call was dispatched but the child transport failed: the tool may
    /// or may not have executed.
    Failed,
    /// The call was never dispatched (unknown tool or malformed arguments):
    /// the outcome is unambiguous and safe to replay.
    Rejected,
}

#[cfg(test)]
mod tests;
