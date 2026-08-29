use std::{fmt, io::Write, path::Path};

use reqwest::{Client, StatusCode, redirect::Policy};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use url::Url;

use super::{
    activity::{ActivityPhase, ActivityReporter, AgentActivityEvent, bound},
    child_mcp::{ChildCallError, ChildMcpManager, ChildTool, ChildToolResult},
    definition::{AgentDefinition, WireApi},
    timeouts::{PROVIDER_CONNECT_TIMEOUT, PROVIDER_REQUEST_TIMEOUT},
};

const DEFAULT_ANTHROPIC_MAX_TOKENS: u32 = 8_192;
const ANTHROPIC_API_VERSION: &str = "2023-06-01";
const ERROR_MESSAGE_LIMIT: usize = 256;
const MAX_PROVIDER_BODY_BYTES: usize = 8 * 1024 * 1024;

/// An API credential deliberately does not implement `Display` and only emits a
/// redacted representation when logged.
pub(crate) struct ProviderCredential(String);

impl fmt::Debug for ProviderCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ProviderCredential([REDACTED])")
    }
}

impl ProviderCredential {
    pub(crate) fn resolve(definition: &AgentDefinition) -> Result<Self, ProviderError> {
        Self::resolve_with(&definition.env_key, |key| std::env::var(key).ok())
    }

    fn resolve_with<F>(env_key: &str, resolve: F) -> Result<Self, ProviderError>
    where
        F: FnOnce(&str) -> Option<String>,
    {
        match resolve(env_key) {
            Some(value) if !value.is_empty() => Ok(Self(value)),
            _ => Err(ProviderError::missing_environment_variable(env_key)),
        }
    }
}

#[derive(Clone)]
pub(crate) enum ConversationState {
    Responses(Vec<Value>),
    AnthropicMessages(Vec<Value>),
}

impl ConversationState {
    pub(crate) fn new(wire_api: &WireApi) -> Self {
        match wire_api {
            WireApi::Responses => Self::Responses(Vec::new()),
            WireApi::AnthropicMessages => Self::AnthropicMessages(Vec::new()),
        }
    }
}

#[derive(Debug)]
pub(crate) struct ProviderError {
    pub kind: &'static str,
    pub message: String,
    pub resumable: bool,
}

impl ProviderError {
    fn missing_environment_variable(env_key: &str) -> Self {
        Self {
            kind: "missing_environment_variable",
            message: format!("Required environment variable {env_key} is not available."),
            resumable: false,
        }
    }

    fn provider(message: impl Into<String>) -> Self {
        Self {
            kind: "provider_error",
            message: bounded_message(&message.into()),
            resumable: true,
        }
    }

    fn interrupted() -> Self {
        Self {
            kind: "run_interrupted",
            message: "agent run was interrupted".into(),
            resumable: true,
        }
    }

    fn ambiguous_tool_execution() -> Self {
        Self {
            kind: "ambiguous_tool_execution",
            message: "a child MCP tool call was dispatched but its outcome is unknown; \
                      the tool may have executed, so the session cannot be resumed"
                .into(),
            resumable: false,
        }
    }

    fn context_limit() -> Self {
        Self {
            kind: "context_limit",
            message: "The retained agent conversation no longer fits the selected model context."
                .into(),
            resumable: false,
        }
    }

    fn terminal_provider(message: impl Into<String>) -> Self {
        Self {
            kind: "provider_error",
            message: bounded_message(&message.into()),
            resumable: false,
        }
    }
}

impl fmt::Display for ProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ProviderError {}

pub(crate) struct ProviderClient {
    client: Client,
}

#[derive(Clone, Copy)]
pub(crate) struct ProviderRun<'a> {
    pub(crate) definition: &'a AgentDefinition,
    pub(crate) credential: &'a ProviderCredential,
    pub(crate) system_context: &'a str,
    pub(crate) child: &'a ChildMcpManager,
    pub(crate) cancel: &'a CancellationToken,
    pub(crate) reporter: &'a ActivityReporter,
    pub(crate) workspace: &'a Path,
}

impl ProviderClient {
    pub(crate) fn new() -> Result<Self, ProviderError> {
        let client = Client::builder()
            .connect_timeout(PROVIDER_CONNECT_TIMEOUT)
            .timeout(PROVIDER_REQUEST_TIMEOUT)
            .redirect(Policy::none())
            .build()
            .map_err(|_| ProviderError::provider("unable to create provider HTTP client"))?;
        Ok(Self { client })
    }

    pub(crate) async fn run(
        &self,
        context: ProviderRun<'_>,
        user_message: &str,
        state: &mut ConversationState,
    ) -> Result<String, ProviderError> {
        if user_message.trim().is_empty() {
            return Err(ProviderError::provider("user message must not be empty"));
        }
        match (&context.definition.wire_api, state) {
            (WireApi::Responses, ConversationState::Responses(history)) => {
                history.push(responses_user_message(user_message));
                self.run_responses(context, history).await
            }
            (WireApi::AnthropicMessages, ConversationState::AnthropicMessages(history)) => {
                append_anthropic_user_message(history, user_message);
                self.run_anthropic(context, history).await
            }
            _ => Err(ProviderError::provider(
                "conversation state does not match provider wire API",
            )),
        }
    }

    async fn run_responses(
        &self,
        context: ProviderRun<'_>,
        history: &mut Vec<Value>,
    ) -> Result<String, ProviderError> {
        let endpoint = endpoint(&context.definition.base_url, "responses")?;
        for _ in 0..context.definition.max_turns {
            context
                .reporter
                .report(AgentActivityEvent {
                    phase: ActivityPhase::Model,
                    summary: "Waiting for model response".into(),
                    target: None,
                    tool: None,
                    kind: "model_started",
                })
                .await;
            let request = responses_request(
                context.definition,
                context.system_context,
                history,
                context.child.tools(),
            );
            let response = self
                .post_json(
                    endpoint.clone(),
                    request,
                    context.credential,
                    WireApi::Responses,
                    context.cancel,
                )
                .await?;
            let parsed = parse_responses_response(response)?;
            if parsed.calls.is_empty() {
                let empty_message = empty_responses_message(&parsed.items);
                commit_responses(history, parsed.items, Vec::new());
                return parsed
                    .text
                    .filter(|text| !text.trim().is_empty())
                    .ok_or_else(|| ProviderError::provider(empty_message));
            }
            let outputs = execute_calls(
                context.child,
                parsed.calls,
                context.cancel,
                context.reporter,
                context.workspace,
            )
            .await?;
            commit_responses(history, parsed.items, responses_tool_outputs(outputs));
        }
        Err(ProviderError::provider(
            "provider exceeded the configured turn limit",
        ))
    }

    async fn run_anthropic(
        &self,
        context: ProviderRun<'_>,
        history: &mut Vec<Value>,
    ) -> Result<String, ProviderError> {
        let endpoint = endpoint(&context.definition.base_url, "v1/messages")?;
        for _ in 0..context.definition.max_turns {
            context
                .reporter
                .report(AgentActivityEvent {
                    phase: ActivityPhase::Model,
                    summary: "Waiting for model response".into(),
                    target: None,
                    tool: None,
                    kind: "model_started",
                })
                .await;
            let request = anthropic_request(
                context.definition,
                context.system_context,
                history,
                context.child.tools(),
            );
            let response = self
                .post_json(
                    endpoint.clone(),
                    request,
                    context.credential,
                    WireApi::AnthropicMessages,
                    context.cancel,
                )
                .await?;
            let parsed = parse_anthropic_response(response)?;
            if parsed.calls.is_empty() {
                commit_anthropic(history, parsed.content, Vec::new());
                if parsed.stop_reason.as_deref() == Some("model_context_window_exceeded") {
                    return Err(ProviderError::context_limit());
                }
                if matches!(
                    parsed.stop_reason.as_deref(),
                    Some("max_tokens" | "refusal" | "error")
                ) {
                    return Err(ProviderError::provider(
                        "provider stopped without a usable response",
                    ));
                }
                if parsed.stop_reason.as_deref() == Some("pause_turn") {
                    continue;
                }
                return parsed
                    .text
                    .filter(|text| !text.trim().is_empty())
                    .ok_or_else(|| ProviderError::provider("provider returned an empty response"));
            }
            let outputs = execute_calls(
                context.child,
                parsed.calls,
                context.cancel,
                context.reporter,
                context.workspace,
            )
            .await?;
            commit_anthropic(history, parsed.content, anthropic_tool_results(outputs));
        }
        Err(ProviderError::provider(
            "provider exceeded the configured turn limit",
        ))
    }

    async fn post_json(
        &self,
        endpoint: Url,
        request: Value,
        credential: &ProviderCredential,
        wire_api: WireApi,
        cancel: &CancellationToken,
    ) -> Result<Value, ProviderError> {
        let body = serialize_provider_body(&request)?;
        let request = match wire_api {
            WireApi::Responses => self.client.post(endpoint).bearer_auth(&credential.0),
            WireApi::AnthropicMessages => self
                .client
                .post(endpoint)
                .header("x-api-key", &credential.0)
                .header("anthropic-version", ANTHROPIC_API_VERSION),
        }
        .header("content-type", "application/json")
        .body(body);
        let response = tokio::select! {
            _ = cancel.cancelled() => return Err(ProviderError::interrupted()),
            result = request.send() => result.map_err(request_error)?,
        };
        let status = response.status();
        if status != StatusCode::PAYLOAD_TOO_LARGE
            && response
                .content_length()
                .is_some_and(|length| length > MAX_PROVIDER_BODY_BYTES as u64)
        {
            return Err(ProviderError::provider(
                "provider response body exceeds the size limit",
            ));
        }
        let mut response = response;
        let mut body = Vec::new();
        loop {
            let chunk = tokio::select! {
                _ = cancel.cancelled() => return Err(ProviderError::interrupted()),
                result = response.chunk() => result.map_err(request_error)?,
            };
            let Some(chunk) = chunk else {
                break;
            };
            append_provider_bytes(&mut body, &chunk)?;
        }
        if !status.is_success() {
            return Err(status_error(status, &body));
        }
        serde_json::from_slice(&body)
            .map_err(|_| ProviderError::provider("provider returned malformed JSON"))
    }
}

struct ToolCall {
    id: String,
    name: String,
    arguments: Value,
}

#[derive(Debug)]
struct ToolResult {
    id: String,
    output: String,
    is_error: bool,
}

struct ResponsesOutput {
    items: Vec<Value>,
    calls: Vec<ToolCall>,
    text: Option<String>,
}

struct AnthropicOutput {
    content: Vec<Value>,
    calls: Vec<ToolCall>,
    text: Option<String>,
    stop_reason: Option<String>,
}

async fn execute_calls(
    child: &ChildMcpManager,
    calls: Vec<ToolCall>,
    cancel: &CancellationToken,
    reporter: &ActivityReporter,
    workspace: &Path,
) -> Result<Vec<ToolResult>, ProviderError> {
    let mut outputs = Vec::with_capacity(calls.len());
    for call in calls {
        if cancel.is_cancelled() {
            return if outputs.is_empty() {
                Err(ProviderError::interrupted())
            } else {
                Err(ProviderError::ambiguous_tool_execution())
            };
        }
        let (summary, tool, target) = safe_tool_activity(&call.name, &call.arguments, workspace);
        reporter
            .report(AgentActivityEvent::tool(summary, tool, target))
            .await;
        let result = child.call(&call.name, call.arguments, cancel).await;
        match result.as_ref().err() {
            Some(ChildCallError::TimedOut) => {
                reporter.report(AgentActivityEvent::tool_timed_out()).await
            }
            Some(ChildCallError::Failed) => {
                reporter.report(AgentActivityEvent::tool_failed()).await
            }
            _ => reporter.report(AgentActivityEvent::tool_completed()).await,
        }
        outputs.push(tool_result_from_call(call.id, result)?);
    }
    Ok(outputs)
}

fn tool_result_from_call(
    id: String,
    result: Result<ChildToolResult, ChildCallError>,
) -> Result<ToolResult, ProviderError> {
    match result {
        Ok(result) => Ok(ToolResult {
            id,
            output: result.output,
            is_error: result.is_error,
        }),
        Err(ChildCallError::Interrupted) => Err(ProviderError::ambiguous_tool_execution()),
        Err(ChildCallError::Rejected) => Ok(ToolResult {
            id,
            output: "child MCP tool call was rejected before execution".into(),
            is_error: true,
        }),
        Err(ChildCallError::TimedOut | ChildCallError::Failed) => {
            Err(ProviderError::ambiguous_tool_execution())
        }
    }
}

fn safe_tool_activity(
    name: &str,
    arguments: &Value,
    workspace: &Path,
) -> (String, String, Option<String>) {
    let name = safe_name(name);
    let object = arguments.as_object();
    if object.is_some_and(|value| value.contains_key("command") || value.contains_key("program")) {
        return ("Running shell command".into(), name, None);
    }
    let path = object
        .and_then(|value| value.get("path").or_else(|| value.get("file_path")))
        .and_then(Value::as_str)
        .and_then(|value| safe_workspace_path(value, workspace));
    let lower = name.to_ascii_lowercase();
    let operation = lower
        .rsplit_once("__")
        .map_or(lower.as_str(), |(_, operation)| operation);
    let operation = operation
        .rsplit_once('.')
        .map_or(operation, |(_, operation)| operation);
    let operation = operation.rsplit('/').next().unwrap_or(operation);
    let action = if operation == "read"
        || operation.starts_with("read_")
        || operation.starts_with("get_file")
    {
        Some("Reading")
    } else if operation == "write"
        || operation.starts_with("write_")
        || operation.starts_with("edit_")
        || operation.starts_with("create_file")
        || operation.starts_with("move_file")
    {
        Some("Writing")
    } else if operation == "search"
        || operation.starts_with("search_")
        || operation.starts_with("grep")
        || operation.starts_with("find")
        || operation.starts_with("list_")
        || operation.starts_with("directory_tree")
    {
        Some("Searching")
    } else {
        None
    };
    match (action, path) {
        (Some(action), Some(path)) => (format!("{action} {path}"), name, Some(path)),
        _ => (format!("Calling {name}"), name, None),
    }
}
fn safe_name(value: &str) -> String {
    let cleaned: String = value
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/'))
        .collect();
    bound(
        if cleaned.is_empty() {
            "tool".into()
        } else {
            cleaned
        },
        80,
    )
}
fn safe_workspace_path(value: &str, workspace: &Path) -> Option<String> {
    let path = Path::new(value);
    let relative = if path.is_absolute() {
        path.strip_prefix(workspace).ok()?
    } else {
        path
    };
    (!relative.as_os_str().is_empty()
        && !relative
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir)))
    .then(|| bound(relative.to_string_lossy().replace('\\', "/"), 120))
}

fn responses_tool_outputs(results: Vec<ToolResult>) -> Vec<Value> {
    results
        .into_iter()
        .map(|result| {
            let output = if result.is_error {
                json!({ "isError": true, "output": result.output }).to_string()
            } else {
                result.output
            };
            json!({
                "type": "function_call_output",
                "call_id": result.id,
                "output": output,
            })
        })
        .collect()
}

fn anthropic_tool_results(results: Vec<ToolResult>) -> Vec<Value> {
    results
        .into_iter()
        .map(|result| {
            json!({
                "type": "tool_result",
                "tool_use_id": result.id,
                "content": result.output,
                "is_error": result.is_error,
            })
        })
        .collect()
}

/// Commits a Responses turn only after every function call has a matching
/// output, keeping replay history valid if a child call fails or is cancelled.
fn commit_responses(history: &mut Vec<Value>, items: Vec<Value>, outputs: Vec<Value>) {
    history.extend(items);
    history.extend(outputs);
}

/// Commits the assistant turn and, when applicable, its complete batch of
/// tool results as the single user turn required by the Messages wire format.
fn commit_anthropic(history: &mut Vec<Value>, content: Vec<Value>, results: Vec<Value>) {
    history.push(json!({ "role": "assistant", "content": content }));
    if !results.is_empty() {
        history.push(json!({ "role": "user", "content": results }));
    }
}

fn responses_request(
    definition: &AgentDefinition,
    instructions: &str,
    input: &[Value],
    tools: &[ChildTool],
) -> Value {
    let mut items = Vec::with_capacity(input.len() + 1);
    items.push(responses_developer_item(instructions));
    items.extend(input.iter().cloned());
    let mut request = json!({
        "model": definition.model,
        "input": items,
        "tools": responses_tools(tools),
        "tool_choice": "auto",
        "parallel_tool_calls": true,
        "store": false,
    });
    if let Some(effort) = &definition.reasoning_effort {
        request["reasoning"] = json!({ "effort": effort });
    }
    if let Some(temperature) = definition.temperature {
        request["temperature"] = json!(temperature);
    }
    request
}

fn anthropic_request(
    definition: &AgentDefinition,
    system: &str,
    messages: &[Value],
    tools: &[ChildTool],
) -> Value {
    let mut request = json!({
        "model": definition.model,
        "max_tokens": DEFAULT_ANTHROPIC_MAX_TOKENS,
        "system": system,
        "messages": messages,
        "tools": anthropic_tools(tools),
    });
    if let Some(temperature) = definition.temperature {
        request["temperature"] = json!(temperature);
    }
    if let Some(effort) = &definition.reasoning_effort {
        request["output_config"] = json!({ "effort": effort });
    }
    request
}

fn responses_tools(tools: &[ChildTool]) -> Vec<Value> {
    tools
        .iter()
        .map(|tool| {
            json!({ "type": "function", "name": tool.provider_name, "description": tool.description, "parameters": (*tool.input_schema).clone() })
        })
        .collect()
}

fn anthropic_tools(tools: &[ChildTool]) -> Vec<Value> {
    tools
        .iter()
        .map(|tool| {
            json!({ "name": tool.provider_name, "description": tool.description, "input_schema": (*tool.input_schema).clone() })
        })
        .collect()
}

fn responses_user_message(message: &str) -> Value {
    json!({ "role": "user", "content": [{ "type": "input_text", "text": message }] })
}

fn responses_developer_item(instructions: &str) -> Value {
    json!({ "role": "developer", "content": [{ "type": "input_text", "text": instructions }] })
}

fn anthropic_user_message(message: &str) -> Value {
    json!({ "role": "user", "content": message })
}

fn append_anthropic_user_message(history: &mut Vec<Value>, message: &str) {
    let Some(last) = history.last_mut().filter(|item| item["role"] == "user") else {
        history.push(anthropic_user_message(message));
        return;
    };
    let content = &mut last["content"];
    if let Some(blocks) = content.as_array_mut() {
        blocks.push(json!({"type": "text", "text": message}));
    } else if let Some(previous) = content.as_str().map(str::to_owned) {
        *content = json!([
            {"type": "text", "text": previous},
            {"type": "text", "text": message}
        ]);
    } else {
        *content = json!([{"type": "text", "text": message}]);
    }
}

fn parse_responses_response(value: Value) -> Result<ResponsesOutput, ProviderError> {
    let object = value
        .as_object()
        .ok_or_else(|| ProviderError::provider("provider returned an invalid response"))?;
    if let Some(status) = object.get("status").and_then(Value::as_str) {
        match status {
            "completed" => {}
            "incomplete" => {
                return Err(ProviderError::provider("provider response was incomplete"));
            }
            "failed" | "cancelled" => {
                return Err(ProviderError::provider(
                    "provider did not complete the response",
                ));
            }
            _ => {
                return Err(ProviderError::provider(
                    "provider returned a nonterminal response",
                ));
            }
        }
    }
    let items = object
        .get("output")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| ProviderError::provider("provider response omitted output"))?;
    let mut calls = Vec::new();
    let mut block_text = String::new();
    for item in &items {
        if item.get("type").and_then(Value::as_str) == Some("function_call") {
            let id = required_string(item, "call_id")?;
            let name = required_string(item, "name")?;
            let raw_arguments = required_string(item, "arguments")?;
            let arguments: Value = serde_json::from_str(&raw_arguments)
                .map_err(|_| ProviderError::provider("provider returned invalid tool arguments"))?;
            calls.push(ToolCall {
                id,
                name,
                arguments,
            });
        }
        collect_response_text(item, &mut block_text);
    }
    let text = object
        .get("output_text")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| (!block_text.is_empty()).then_some(block_text));
    Ok(ResponsesOutput { items, calls, text })
}

fn parse_anthropic_response(value: Value) -> Result<AnthropicOutput, ProviderError> {
    let object = value
        .as_object()
        .ok_or_else(|| ProviderError::provider("provider returned an invalid response"))?;
    let content = object
        .get("content")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| ProviderError::provider("provider response omitted content"))?;
    let mut calls = Vec::new();
    let mut text = String::new();
    for block in &content {
        match block.get("type").and_then(Value::as_str) {
            Some("tool_use") => calls.push(ToolCall {
                id: required_string(block, "id")?,
                name: required_string(block, "name")?,
                arguments: block
                    .get("input")
                    .cloned()
                    .ok_or_else(|| ProviderError::provider("provider tool call omitted input"))?,
            }),
            Some("text") => {
                if let Some(value) = block.get("text").and_then(Value::as_str) {
                    text.push_str(value);
                }
            }
            _ => {}
        }
    }
    Ok(AnthropicOutput {
        content,
        calls,
        text: (!text.is_empty()).then_some(text),
        stop_reason: object
            .get("stop_reason")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

fn collect_response_text(item: &Value, output: &mut String) {
    if item.get("type").and_then(Value::as_str) == Some("message")
        && let Some(content) = item.get("content").and_then(Value::as_array)
    {
        for block in content {
            if block.get("type").and_then(Value::as_str) == Some("output_text")
                && let Some(text) = block.get("text").and_then(Value::as_str)
            {
                output.push_str(text);
            }
        }
    }
}

fn empty_responses_message(items: &[Value]) -> String {
    let types = items
        .iter()
        .filter_map(|item| item.get("type").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join(",");
    if types.is_empty() {
        "provider returned an empty response with no output items".into()
    } else {
        format!("provider returned an empty response with output item types: {types}")
    }
}

fn required_string(value: &Value, key: &str) -> Result<String, ProviderError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ProviderError::provider("provider returned a malformed tool call"))
}

fn endpoint(base_url: &Url, suffix: &str) -> Result<Url, ProviderError> {
    let mut base = base_url.clone();
    if !base.path().ends_with('/') {
        base.set_path(&format!("{}/", base.path()));
    }
    base.join(suffix)
        .map_err(|_| ProviderError::provider("invalid provider endpoint"))
}

fn request_error(error: reqwest::Error) -> ProviderError {
    if error.is_timeout() {
        ProviderError {
            kind: "provider_timeout",
            message: "provider request timed out".into(),
            resumable: true,
        }
    } else {
        ProviderError::provider("provider request failed")
    }
}

fn status_error(status: StatusCode, body: &[u8]) -> ProviderError {
    if status == StatusCode::PAYLOAD_TOO_LARGE || provider_reports_context_limit(body) {
        return ProviderError::context_limit();
    }
    let message = format!("provider returned HTTP status {}", status.as_u16());
    if status == StatusCode::REQUEST_TIMEOUT
        || status == StatusCode::TOO_MANY_REQUESTS
        || status.is_server_error()
    {
        ProviderError::provider(message)
    } else {
        ProviderError::terminal_provider(message)
    }
}

fn provider_reports_context_limit(body: &[u8]) -> bool {
    let Ok(value) = serde_json::from_slice::<Value>(body) else {
        return false;
    };
    let Some(error) = value.get("error") else {
        return false;
    };
    let code = error
        .get("code")
        .or_else(|| error.get("type"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    code.contains("context_length")
        || code.contains("context_window")
        || message.contains("maximum context length")
        || message.contains("context window")
        || message.contains("too many tokens")
}

fn bounded_message(value: &str) -> String {
    let cleaned: String = value
        .chars()
        .filter(|character| !character.is_control())
        .collect();
    if cleaned.len() <= ERROR_MESSAGE_LIMIT {
        return cleaned;
    }
    let mut end = ERROR_MESSAGE_LIMIT - '…'.len_utf8();
    while !cleaned.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &cleaned[..end])
}

fn serialize_provider_body(value: &Value) -> Result<String, ProviderError> {
    struct BoundedBody {
        bytes: Vec<u8>,
        exceeded: bool,
    }
    impl Write for BoundedBody {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            let remaining = MAX_PROVIDER_BODY_BYTES.saturating_sub(self.bytes.len());
            if bytes.len() > remaining {
                self.exceeded = true;
                return Err(std::io::Error::other(
                    "provider request body limit exceeded",
                ));
            }
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut body = BoundedBody {
        bytes: Vec::new(),
        exceeded: false,
    };
    if serde_json::to_writer(&mut body, value).is_err() {
        if body.exceeded {
            return Err(ProviderError::context_limit());
        }
        return Err(ProviderError::provider(
            "unable to serialize provider request",
        ));
    }
    String::from_utf8(body.bytes)
        .map_err(|_| ProviderError::provider("unable to serialize provider request"))
}

fn append_provider_bytes(body: &mut Vec<u8>, chunk: &[u8]) -> Result<(), ProviderError> {
    let remaining = MAX_PROVIDER_BODY_BYTES.saturating_sub(body.len());
    if chunk.len() > remaining {
        return Err(ProviderError::provider(
            "provider response body exceeds the size limit",
        ));
    }
    body.extend_from_slice(chunk);
    Ok(())
}

#[cfg(test)]
mod tests;
