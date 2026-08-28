mod activity;
mod child_mcp;
mod definition;
mod discovery;
mod markdown;
mod provider;
mod runtime;
mod timeouts;
mod toml;

use std::{future, path::PathBuf, sync::Arc, time::Duration};

use rmcp::{
    ErrorData as McpError, ServerHandler,
    model::{
        CallToolRequestMethod, CallToolRequestParams, CallToolResponse, CallToolResult,
        ContentBlock, Implementation, ListToolsResult, NotificationMetaObject,
        ProgressNotificationParam, ServerCapabilities, ServerInfo, Tool, ToolAnnotations,
    },
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::cli::WorkspaceServerOptions;
use crate::policy::{Capability, ToolPolicy, ToolSpec};

use self::activity::bound;
use self::definition::{MAX_SEND_MESSAGE_BYTES, MAX_SPAWN_TASK_BYTES, MAX_WAIT_TARGETS};
use self::runtime::{AgentRuntime, InputAck, RuntimeError, SpawnResult, WaitResult};
use self::timeouts::MAX_WAIT_AGENT_TIMEOUT_MS;

const PROGRESS_QUEUE_CAPACITY: usize = 16;
const PROGRESS_NOTIFICATION_TIMEOUT: Duration = Duration::from_millis(100);

fn progress_notification(
    token: rmcp::model::ProgressToken,
    progress: f64,
    agent: &runtime::AgentResult,
    remaining: u64,
) -> ProgressNotificationParam {
    let summary = match agent.state {
        runtime::AgentState::Completed => "Completed".into(),
        runtime::AgentState::Failed => "Failed".into(),
        runtime::AgentState::Running => agent
            .activity
            .as_ref()
            .map(|a| a.summary.clone())
            .unwrap_or_else(|| "Working".into()),
    };
    let name = bound(agent.name.clone().unwrap_or_else(|| agent.id.clone()), 120);
    let mut meta = NotificationMetaObject::new();
    meta.insert(
        "io.tuls/agents".into(),
        json!({
            "agent": {
                "agentId": agent.id,
                "name": agent.name,
                "status": agent.state,
                "activity": agent.activity,
                "totalElapsedMs": agent.total_elapsed_ms,
            },
            "waitTimeoutRemainingMs": remaining,
        }),
    );
    let mut notification = ProgressNotificationParam::new(token, progress)
        .with_message(bound(format!("{name} · {summary}"), 256));
    notification.meta = Some(meta);
    notification
}

const TOOL_SPECS: &[ToolSpec] = &[
    ToolSpec::new("agents", "spawn_agent", Capability::AgentsRun),
    ToolSpec::new("agents", "send_input", Capability::AgentsRun),
    ToolSpec::new("agents", "wait_agent", Capability::AgentsRun),
];

pub(crate) struct AgentsServer {
    runtime: Arc<AgentRuntime>,
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
    #[serde(default)]
    interrupt: bool,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WaitArgs {
    targets: Vec<String>,
    #[serde(default = "default_timeout")]
    timeout_ms: u64,
}
fn default_timeout() -> u64 {
    30_000
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
            policy,
        })
    }
    fn tools(&self) -> Vec<Tool> {
        if self.runtime.registry().is_empty() {
            return vec![];
        }
        let candidates = [
            (TOOL_SPECS[0], self.spawn_tool()),
            (TOOL_SPECS[1], self.input_tool()),
            (TOOL_SPECS[2], self.wait_tool()),
        ];
        candidates
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
            .map(|a| format!("- {}: {}", a.name, a.description))
            .collect::<Vec<_>>()
            .join("\n");
        let names = self.runtime.registry().names();
        let schema = json_object(
            json!({"type":"object","properties":{"name":{"type":"string","enum":names},"task":{"type":"string","minLength":1}},"required":["name","task"],"additionalProperties":false}),
        );
        Tool::new(
            "spawn_agent",
            format!("Spawn a local workspace agent to run a task in the background. This returns promptly before the agent completes; save the returned agentId and use wait_agent to collect the result. Calls may run in parallel up to the runtime capacity. Available agents:\n{catalog}"),
            schema,
        )
        .with_output_schema::<SpawnResult>()
        .with_annotations(
            ToolAnnotations::new()
                .read_only(false)
                .destructive(true)
                .idempotent(false)
                .open_world(true),
        )
    }
    fn input_tool(&self) -> Tool {
        let schema = json_object(
            json!({"type":"object","properties":{"target":{"type":"string","minLength":1},"message":{"type":"string","minLength":1},"interrupt":{"type":"boolean","default":false}},"required":["target","message"],"additionalProperties":false}),
        );
        Tool::new(
            "send_input",
            "Send follow-up input to an agent session. Set interrupt to true to cancel its active run cooperatively and continue the same session with this input.",
            schema,
        )
        .with_output_schema::<InputAck>()
        .with_annotations(
            ToolAnnotations::new()
                .read_only(false)
                .destructive(true)
                .idempotent(false)
                .open_world(true),
        )
    }
    fn wait_tool(&self) -> Tool {
        let schema = json_object(
            json!({"type":"object","properties":{"targets":{"type":"array","items":{"type":"string","minLength":1},"minItems":1,"maxItems":MAX_WAIT_TARGETS,"uniqueItems":true},"timeoutMs":{"type":"integer","minimum":0,"maximum":MAX_WAIT_AGENT_TIMEOUT_MS,"default":30000}},"required":["targets"],"additionalProperties":false}),
        );
        Tool::new("wait_agent", "Wait until all requested agents reach a terminal state or the timeout expires, whichever happens first. timeoutMs is a maximum wait duration, not a sleep interval. Already-finished agents return immediately. Calling this tool pauses the parent model until the tool returns.", schema).with_output_schema::<WaitResult>().with_annotations(ToolAnnotations::new().read_only(true).destructive(false).open_world(false))
    }
    async fn call(
        &self,
        name: &str,
        arguments: Option<rmcp::model::JsonObject>,
        context: Option<rmcp::service::RequestContext<rmcp::RoleServer>>,
    ) -> CallToolResult {
        let parsed = arguments
            .map(Value::Object)
            .ok_or_else(|| RuntimeError::new("invalid_request", "missing arguments"));
        let result: Result<Value, RuntimeError> = match name {
            "spawn_agent" => match parsed.and_then(|v| {
                serde_json::from_value::<SpawnArgs>(v)
                    .map_err(|_| RuntimeError::new("invalid_request", "invalid arguments"))
            }) {
                Ok(a) if a.task.len() > MAX_SPAWN_TASK_BYTES => Err(RuntimeError::new(
                    "invalid_request",
                    format!("task exceeds the {MAX_SPAWN_TASK_BYTES}-byte limit"),
                )),
                Ok(a) => self.runtime.spawn(&a.name, &a.task).await.and_then(|v| {
                    serde_json::to_value(v).map_err(|_| {
                        RuntimeError::new("runtime_error", "unable to serialize response")
                    })
                }),
                Err(e) => Err(e),
            },
            "send_input" => match parsed.and_then(|v| {
                serde_json::from_value::<InputArgs>(v)
                    .map_err(|_| RuntimeError::new("invalid_request", "invalid arguments"))
            }) {
                Ok(a) if a.message.len() > MAX_SEND_MESSAGE_BYTES => Err(RuntimeError::new(
                    "invalid_request",
                    format!("message exceeds the {MAX_SEND_MESSAGE_BYTES}-byte limit"),
                )),
                Ok(a) => self
                    .runtime
                    .send_input(&a.target, &a.message, a.interrupt)
                    .await
                    .and_then(|v| {
                        serde_json::to_value(v).map_err(|_| {
                            RuntimeError::new("runtime_error", "unable to serialize response")
                        })
                    }),
                Err(e) => Err(e),
            },
            "wait_agent" => match parsed.and_then(|v| {
                serde_json::from_value::<WaitArgs>(v)
                    .map_err(|_| RuntimeError::new("invalid_request", "invalid arguments"))
            }) {
                Ok(a) if a.targets.len() > MAX_WAIT_TARGETS => Err(RuntimeError::new(
                    "invalid_request",
                    format!("wait_agent accepts at most {MAX_WAIT_TARGETS} targets"),
                )),
                Ok(a) => self.wait_with_progress(a, context).await.and_then(|v| {
                    serde_json::to_value(v).map_err(|_| {
                        RuntimeError::new("runtime_error", "unable to serialize response")
                    })
                }),
                Err(e) => Err(e),
            },
            _ => Err(RuntimeError::new("unknown_tool", "unknown tool")),
        };
        match result {
            Ok(value) => {
                let mut out = CallToolResult::structured(value);
                out.content
                    .push(ContentBlock::text("Agent request accepted."));
                out
            }
            Err(error) => {
                let text = serde_json::to_string(&error)
                    .unwrap_or_else(|_| format!("{}: {}", error.kind, error.message));
                CallToolResult::error(vec![ContentBlock::text(text)])
            }
        }
    }
    async fn wait_with_progress(
        &self,
        args: WaitArgs,
        context: Option<rmcp::service::RequestContext<rmcp::RoleServer>>,
    ) -> Result<WaitResult, RuntimeError> {
        let Some(context) = context else {
            return self.runtime.wait(&args.targets, args.timeout_ms).await;
        };
        let Some(token) = context.meta.get_progress_token() else {
            return self.runtime.wait(&args.targets, args.timeout_ms).await;
        };
        let (updates, mut receiver) = tokio::sync::mpsc::channel(PROGRESS_QUEUE_CAPACITY);
        let runtime = self.runtime.clone();
        let targets = args.targets.clone();
        let worker = tokio::spawn(async move {
            runtime
                .wait_observing(&targets, args.timeout_ms, updates)
                .await
        });
        let mut progress = 0.0;
        while let Some(update) = receiver.recv().await {
            for agent in &update.result.agents {
                progress += 1.0;
                let notification = progress_notification(
                    token.clone(),
                    progress,
                    agent,
                    update.wait_timeout_remaining_ms,
                );
                if !matches!(
                    tokio::time::timeout(
                        PROGRESS_NOTIFICATION_TIMEOUT,
                        context.peer.notify_progress(notification),
                    )
                    .await,
                    Ok(Ok(()))
                ) {
                    break;
                }
            }
        }
        worker
            .await
            .map_err(|_| RuntimeError::new("runtime_error", "agent wait task failed"))?
    }
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
            ServerCapabilities::builder().build()
        } else {
            ServerCapabilities::builder().enable_tools().build()
        };
        ServerInfo::new(capabilities)
            .with_server_info(Implementation::new("tuls-agents", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "Spawn local workspace agents for background tasks, save each returned agentId, send follow-up input when needed, and call wait_agent to collect terminal results.",
            )
    }
    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.tools().into_iter().find(|t| t.name == name)
    }
    fn list_tools(
        &self,
        _: Option<rmcp::model::PaginatedRequestParams>,
        _: rmcp::service::RequestContext<rmcp::RoleServer>,
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
        if self.get_tool(&request.name).is_none() {
            Err(McpError::method_not_found::<CallToolRequestMethod>())
        } else {
            Ok(self
                .call(&request.name, request.arguments, Some(context))
                .await
                .into())
        }
    }
}

pub(crate) async fn run(options: WorkspaceServerOptions) -> anyhow::Result<()> {
    use rmcp::{ServiceExt, transport::stdio};
    let policy = ToolPolicy::from_selectors(&options.tools.allow, &options.tools.deny, TOOL_SPECS)
        .map_err(anyhow::Error::msg)?;
    let server = AgentsServer::new(options.dir, policy).map_err(|e| anyhow::anyhow!(e.message))?;
    let runtime = server.runtime.clone();
    let service = server
        .serve(stdio())
        .await
        .inspect_err(|e| tracing::error!("serving error: {e:?}"))?;
    service.waiting().await?;
    runtime.shutdown().await;
    Ok(())
}

#[cfg(test)]
mod tests;
