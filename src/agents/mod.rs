mod activity;
mod child_mcp;
mod definition;
mod discovery;
mod markdown;
mod provider;
mod runtime;
mod timeouts;

use std::{future, path::PathBuf, sync::Arc};

use rmcp::{
    ErrorData as McpError, ServerHandler,
    model::{
        CallToolRequestMethod, CallToolRequestParams, CallToolResponse, CallToolResult,
        CancelTaskParams, ClientCapabilities, ContentBlock, CreateTaskResult, GetTaskParams,
        GetTaskResult, Implementation, ListToolsResult, ServerCapabilities, ServerInfo, Tool,
        ToolAnnotations, UpdateTaskParams,
    },
    task_manager::{TaskExit, TaskManager, TaskOptions},
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    cli::WorkspaceServerOptions,
    policy::{Capability, ToolPolicy, ToolSpec},
    support::{DEFAULT_TASK_POLL_INTERVAL_MS, MAX_TOOL_RESULT_BYTES},
};

use self::runtime::{
    AGENT_TASK_TTL_MS, AgentRuntime, AgentTurnError, AgentTurnResult, RuntimeError, TurnOutcome,
};

const TOOL_SPECS: &[ToolSpec] = &[
    ToolSpec::new("agents", "spawn_agent", Capability::AgentsRun),
    ToolSpec::new("agents", "send_input", Capability::AgentsRun),
];

pub(crate) struct AgentsServer {
    runtime: Arc<AgentRuntime>,
    tasks: TaskManager,
    policy: ToolPolicy,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SpawnArgs {
    name: String,
    task: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InputArgs {
    target: String,
    message: String,
}

fn json_object(value: Value) -> serde_json::Map<String, Value> {
    match value {
        Value::Object(object) => object,
        _ => serde_json::Map::new(),
    }
}

impl AgentsServer {
    pub(crate) fn new(workspace: PathBuf, policy: ToolPolicy) -> Result<Self, RuntimeError> {
        Ok(Self {
            runtime: Arc::new(AgentRuntime::new(workspace)?),
            tasks: TaskManager::new(),
            policy,
        })
    }

    fn tools(&self) -> Vec<Tool> {
        if self.runtime.registry().is_empty() {
            return Vec::new();
        }
        [
            (TOOL_SPECS[0], self.spawn_tool()),
            (TOOL_SPECS[1], self.input_tool()),
        ]
        .into_iter()
        .filter_map(|(spec, tool)| self.policy.allows(spec).then_some(tool))
        .collect()
    }

    fn spawn_tool(&self) -> Tool {
        let catalog = self
            .runtime
            .registry()
            .catalog()
            .iter()
            .map(|agent| format!("- {}: {}", agent.name, agent.description))
            .collect::<Vec<_>>()
            .join("\n");
        let names = self.runtime.registry().names();
        let schema = json_object(json!({
            "type": "object",
            "properties": {
                "name": {"type": "string", "enum": names},
                "task": {"type": "string", "minLength": 1}
            },
            "required": ["name", "task"],
            "additionalProperties": false
        }));
        Tool::new(
            "spawn_agent",
            format!(
                "Start a local workspace agent as an MCP Task. The terminal task result contains the agentId, agent name, and final response. Observe progress through standard tasks/get statusMessage updates and cancel through tasks/cancel. Use send_input with the returned agentId to continue the same conversation after the task reaches a terminal state. Available agents:\n{catalog}"
            ),
            schema,
        )
        .with_output_schema::<AgentTurnResult>()
        .with_annotations(
            ToolAnnotations::new()
                .read_only(false)
                .destructive(true)
                .idempotent(false)
                .open_world(true),
        )
    }

    fn input_tool(&self) -> Tool {
        let schema = json_object(json!({
            "type": "object",
            "properties": {
                "target": {"type": "string", "minLength": 1},
                "message": {"type": "string", "minLength": 1}
            },
            "required": ["target", "message"],
            "additionalProperties": false
        }));
        Tool::new(
            "send_input",
            "Continue an existing agent conversation as a new MCP Task. The target agent session must not already have a running task. Cancel an active task with standard tasks/cancel before starting a replacement turn.",
            schema,
        )
        .with_output_schema::<AgentTurnResult>()
        .with_annotations(
            ToolAnnotations::new()
                .read_only(false)
                .destructive(true)
                .idempotent(false)
                .open_world(true),
        )
    }

    fn require_tasks(
        context: &rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<(), McpError> {
        if context
            .client_capabilities()
            .is_some_and(|capabilities| capabilities.supports_tasks())
        {
            Ok(())
        } else {
            Err(McpError::missing_required_client_capability(
                ClientCapabilities::builder().enable_tasks().build(),
            ))
        }
    }

    fn parse_arguments<T: serde::de::DeserializeOwned>(
        arguments: Option<rmcp::model::JsonObject>,
    ) -> Result<T, McpError> {
        let arguments = arguments
            .map(Value::Object)
            .ok_or_else(|| McpError::invalid_params("missing arguments", None))?;
        serde_json::from_value(arguments)
            .map_err(|_| McpError::invalid_params("invalid arguments", None))
    }

    fn start_task(&self, turn: runtime::AgentTurn) -> CallToolResponse {
        let runtime = self.runtime.clone();
        let task = self.tasks.spawn(
            TaskOptions::new()
                .with_ttl_ms(AGENT_TASK_TTL_MS)
                .with_poll_interval_ms(DEFAULT_TASK_POLL_INTERVAL_MS)
                .with_status_message("Starting agent"),
            move |context| {
                Box::pin(async move {
                    match runtime.execute(turn, context).await {
                        TurnOutcome::Completed(result) => render_turn_result(&result),
                        TurnOutcome::Failed(error) => render_turn_error(&error),
                        TurnOutcome::Cancelled => Err(TaskExit::Cancelled),
                    }
                })
            },
        );
        CreateTaskResult::new(task).into()
    }

    async fn call(
        &self,
        request: CallToolRequestParams,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        Self::require_tasks(&context)?;
        if self.get_tool(&request.name).is_none() {
            return Err(McpError::method_not_found::<CallToolRequestMethod>());
        }

        match request.name.as_ref() {
            "spawn_agent" => {
                let args = Self::parse_arguments::<SpawnArgs>(request.arguments)?;
                match self.runtime.prepare_spawn(&args.name, &args.task).await {
                    Ok(turn) => Ok(self.start_task(turn)),
                    Err(error) => preflight_error(error),
                }
            }
            "send_input" => {
                let args = Self::parse_arguments::<InputArgs>(request.arguments)?;
                match self
                    .runtime
                    .prepare_input(&args.target, &args.message)
                    .await
                {
                    Ok(turn) => Ok(self.start_task(turn)),
                    Err(error) => preflight_error(error),
                }
            }
            _ => Err(McpError::method_not_found::<CallToolRequestMethod>()),
        }
    }
}

fn preflight_error(error: RuntimeError) -> Result<CallToolResponse, McpError> {
    Ok(runtime_error_result(&error).into())
}

fn runtime_error_result(error: &RuntimeError) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(format!(
        "{}: {}",
        error.kind, error.message
    ))])
}

fn render_turn_result(result: &AgentTurnResult) -> Result<CallToolResult, TaskExit> {
    let structured = serde_json::to_value(result).map_err(|error| {
        TaskExit::Error(McpError::internal_error(
            format!("failed to serialize agent result: {error}"),
            None,
        ))
    })?;
    let mut output = CallToolResult::structured(structured);
    output.content = vec![ContentBlock::text(format!(
        "Agent {} ({}) completed:\n{}",
        result.name, result.id, result.result
    ))];
    ensure_tool_result_size(output)
}

fn render_turn_error(error: &AgentTurnError) -> Result<CallToolResult, TaskExit> {
    let mut message = format!(
        "Agent {} ({}) failed: {}: {}",
        error.name, error.id, error.kind, error.message
    );
    if error.resumable {
        message.push_str(" The agent session can be continued with send_input.");
    }
    let structured = serde_json::to_value(error).map_err(|error| {
        TaskExit::Error(McpError::internal_error(
            format!("failed to serialize agent failure: {error}"),
            None,
        ))
    })?;
    let mut output = CallToolResult::error(vec![ContentBlock::text(message)]);
    output.structured_content = Some(structured);
    ensure_tool_result_size(output)
}

fn ensure_tool_result_size(result: CallToolResult) -> Result<CallToolResult, TaskExit> {
    let bytes = serde_json::to_vec(&result).map_err(|error| {
        TaskExit::Error(McpError::internal_error(
            format!("failed to size agent result: {error}"),
            None,
        ))
    })?;
    if bytes.len() > MAX_TOOL_RESULT_BYTES {
        return Err(TaskExit::Error(McpError::internal_error(
            "agent result exceeds the bounded tool-result size",
            None,
        )));
    }
    Ok(result)
}

impl ServerHandler for AgentsServer {
    fn initialize(
        &self,
        _request: rmcp::model::InitializeRequestParams,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> impl std::future::Future<Output = Result<rmcp::model::InitializeResult, McpError>>
    + rmcp::service::MaybeSendFuture
    + '_ {
        crate::support::reject_unsupported_initialize()
    }

    fn supported_protocol_versions(
        &self,
    ) -> std::borrow::Cow<'static, [rmcp::model::ProtocolVersion]> {
        std::borrow::Cow::Borrowed(crate::support::SUPPORTED_PROTOCOL_VERSIONS)
    }

    fn get_info(&self) -> ServerInfo {
        let capabilities = if self.tools().is_empty() {
            ServerCapabilities::builder().enable_tasks().build()
        } else {
            ServerCapabilities::builder()
                .enable_tools()
                .enable_tasks()
                .build()
        };
        ServerInfo::new(capabilities)
            .with_server_info(Implementation::new("tuls-agents", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "Start agent turns with spawn_agent or send_input. Both tools require the MCP Tasks extension and return standard task handles. Observe task status with tasks/get, cancel with tasks/cancel, and read the terminal task result for the agent response.",
            )
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.tools().into_iter().find(|tool| tool.name == name)
    }

    fn list_tools(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListToolsResult, McpError>>
    + rmcp::service::MaybeSendFuture
    + '_ {
        future::ready(Ok(ListToolsResult::with_all_items(self.tools())
            .with_ttl_ms(0)
            .with_cache_scope(rmcp::model::CacheScope::Private)))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        self.call(request, context).await
    }

    async fn get_task(
        &self,
        request: GetTaskParams,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<GetTaskResult, McpError> {
        self.tasks
            .get_task(&request.task_id)
            .map(GetTaskResult::new)
    }

    async fn update_task(
        &self,
        request: UpdateTaskParams,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<(), McpError> {
        self.tasks
            .update_task(&request.task_id, request.input_responses)
    }

    async fn cancel_task(
        &self,
        request: CancelTaskParams,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<(), McpError> {
        self.tasks.cancel_task(&request.task_id)
    }
}

pub(crate) async fn run(options: WorkspaceServerOptions) -> anyhow::Result<()> {
    use rmcp::{ServiceExt, transport::stdio};

    let policy = ToolPolicy::from_selectors(&options.tools.allow, &options.tools.deny, TOOL_SPECS)
        .map_err(anyhow::Error::msg)?;
    let server =
        AgentsServer::new(options.dir, policy).map_err(|error| anyhow::anyhow!(error.message))?;
    let tasks = server.tasks.clone();
    let service = server
        .serve(stdio())
        .await
        .inspect_err(|error| tracing::error!("serving error: {error:?}"))?;
    let result = service.waiting().await;
    tasks.shutdown();
    result?;
    Ok(())
}

#[cfg(test)]
mod tests;
