mod drain;

use std::{future, path::PathBuf, sync::Arc, time::Duration};

use rmcp::{
    ErrorData as McpError, ServerHandler,
    model::{
        CallToolRequestMethod, CallToolRequestParams, CallToolResponse, CallToolResult,
        CancelTaskParams, ClientCapabilities, ContentBlock, CreateTaskResult, GetTaskParams,
        GetTaskResult, Implementation, ListToolsResult, ServerCapabilities, ServerInfo, Tool,
        ToolAnnotations, UpdateTaskParams,
    },
    schemars,
    task_manager::{TaskExit, TaskManager, TaskOptions},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::process::Command;

use crate::cli::DirectoryServerOptions;
use crate::policy::{Capability, ToolPolicy, ToolSpec};
use crate::support::{
    AccessControl, DEFAULT_TASK_POLL_INTERVAL_MS, MAX_TOOL_RESULT_BYTES, SPEC_VERSION,
    configure_minimal_process_environment, tool_error,
};

use self::drain::drain_limited;

pub const DEFAULT_TIMEOUT_MS: u64 = 120_000;
pub const MAX_TIMEOUT_MS: u64 = 600_000;

const STREAM_CAPTURE_LIMIT: usize = 8 * 1024;
const STREAM_DRAIN_GRACE_MS: u64 = 2_000;
const TASK_TTL_MARGIN_MS: u64 = 30_000;

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[schemars(description = "Parameters for executing a command")]
pub struct ExecuteCommandArgs {
    /// The executable to run, resolved through PATH like a direct exec. Never
    /// a shell command string.
    #[schemars(
        description = "The executable to run, resolved through PATH like a direct exec. Never a shell command string."
    )]
    pub program: String,
    /// Arguments passed to the executable, each preserved exactly as given
    /// without shell parsing, quoting, or glob expansion.
    #[serde(default)]
    #[schemars(
        description = "Arguments passed to the executable, each preserved exactly as given without shell parsing, quoting, or glob expansion."
    )]
    pub args: Vec<String>,
    /// Working directory for the command. Relative paths resolve against the
    /// first allowed directory.
    #[serde(default)]
    #[schemars(
        description = "Working directory for the command. Must resolve inside an allowed directory. Relative paths resolve against the first allowed directory."
    )]
    pub cwd: Option<String>,
    /// Maximum execution time in milliseconds.
    #[serde(default = "default_timeout_ms")]
    #[schemars(range(min = 1, max = 600000))]
    #[schemars(
        description = "Maximum execution time in milliseconds from 1 through 600000. On expiry the process is terminated and timedOut is reported."
    )]
    pub timeout_ms: u64,
}

fn default_timeout_ms() -> u64 {
    DEFAULT_TIMEOUT_MS
}

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "The result of a command execution")]
pub struct CommandOutput {
    /// Numeric exit code, or null when no normal exit code is available.
    pub exit_code: Option<i32>,
    /// Captured standard output, lossy UTF-8, bounded to 8 KiB.
    pub stdout: String,
    /// Captured standard error, lossy UTF-8, bounded to 8 KiB.
    pub stderr: String,
    /// Whether the command was terminated after exceeding `timeoutMs`.
    pub timed_out: bool,
    /// Whether stdout exceeded the capture limit.
    pub stdout_truncated: bool,
    /// Whether stderr exceeded the capture limit.
    pub stderr_truncated: bool,
}

const TOOL_SPECS: &[ToolSpec] = &[ToolSpec::new(
    "shell",
    "execute_command",
    Capability::ProcessExecute,
)];

#[derive(Clone)]
pub struct ShellServer {
    access: Arc<AccessControl>,
    tasks: TaskManager,
    tool_enabled: bool,
}

impl ShellServer {
    pub fn new(access: AccessControl, policy: ToolPolicy) -> Self {
        Self {
            access: Arc::new(access),
            tasks: TaskManager::new(),
            tool_enabled: policy.allows(TOOL_SPECS[0]),
        }
    }

    fn tools(&self) -> Vec<Tool> {
        if !self.tool_enabled {
            return Vec::new();
        }
        match self.command_tool() {
            Some(tool) => vec![tool],
            None => Vec::new(),
        }
    }

    fn command_tool(&self) -> Option<Tool> {
        let schema = match rmcp::handler::server::tool::schema_for_input::<ExecuteCommandArgs>() {
            Ok(schema) => schema,
            Err(error) => {
                tracing::error!("failed to build execute_command input schema: {error}");
                return None;
            }
        };
        Some(
            Tool::new(
                "execute_command",
                "Execute one local program directly as an MCP Task with an explicit argv. The program is resolved through PATH and spawned without a shell, so shell syntax, quoting, and glob expansion are not applied. The working directory must be inside an allowed directory. Stdout and stderr are captured separately and bounded to 8 KiB each. Observe the task with tasks/get and cancel it with tasks/cancel. On timeout the process is terminated and the completed result reports timedOut=true. The child receives a minimal inherited environment; this tool is not an operating-system sandbox and executes with the MCP process's OS permissions.",
                schema,
            )
            .with_output_schema::<CommandOutput>()
            .with_annotations(
                ToolAnnotations::new()
                    .read_only(false)
                    .destructive(true)
                    .idempotent(false)
                    .open_world(true),
            ),
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

    fn parse_arguments(
        arguments: Option<rmcp::model::JsonObject>,
    ) -> Result<ExecuteCommandArgs, McpError> {
        let arguments = arguments
            .map(Value::Object)
            .ok_or_else(|| McpError::invalid_params("missing arguments", None))?;
        serde_json::from_value(arguments)
            .map_err(|_| McpError::invalid_params("invalid arguments", None))
    }

    fn validate_arguments(args: &ExecuteCommandArgs) -> Option<CallToolResult> {
        if args.program.trim().is_empty() {
            return Some(tool_error(
                "program must be a non-empty executable name or path",
            ));
        }
        if !(1..=MAX_TIMEOUT_MS).contains(&args.timeout_ms) {
            return Some(tool_error(format!(
                "timeoutMs must be between 1 and {MAX_TIMEOUT_MS}, got {}",
                args.timeout_ms
            )));
        }
        None
    }

    fn start_task(&self, args: ExecuteCommandArgs) -> CallToolResponse {
        let access = self.access.clone();
        let ttl_ms = task_ttl_ms(args.timeout_ms);
        let task = self.tasks.spawn(
            TaskOptions::new()
                .with_ttl_ms(ttl_ms)
                .with_poll_interval_ms(DEFAULT_TASK_POLL_INTERVAL_MS)
                .with_status_message("Preparing command"),
            move |context| Box::pin(run_command(access, args, context)),
        );
        CreateTaskResult::new(task).into()
    }

    fn summary_text(output: &CommandOutput) -> String {
        let status = if output.timed_out {
            "command timed out - process terminated".to_string()
        } else {
            match output.exit_code {
                Some(code) => format!("exit code: {code}"),
                None => "exit code: (none)".to_string(),
            }
        };
        let stdout_note = if output.stdout_truncated {
            "truncated at 8 KiB"
        } else {
            "full"
        };
        let stderr_note = if output.stderr_truncated {
            "truncated at 8 KiB"
        } else {
            "full"
        };
        format!(
            "{status}\nstdout ({stdout_note}): {}\nstderr ({stderr_note}): {}",
            preview(&output.stdout),
            preview(&output.stderr)
        )
    }
}

fn task_ttl_ms(timeout_ms: u64) -> u64 {
    timeout_ms.saturating_add(TASK_TTL_MARGIN_MS)
}

async fn resolve_cwd(access: &AccessControl, cwd: Option<&str>) -> Result<PathBuf, String> {
    let requested = cwd.unwrap_or(".");
    let resolved = access.validate_path(requested).await?;
    let metadata = tokio::fs::metadata(&resolved)
        .await
        .map_err(|error| format!("Failed to access cwd {}: {error}", resolved.display()))?;
    if !metadata.is_dir() {
        return Err(format!("cwd is not a directory: {}", resolved.display()));
    }
    Ok(resolved)
}

async fn run_command(
    access: Arc<AccessControl>,
    args: ExecuteCommandArgs,
    task_context: rmcp::task_manager::TaskContext,
) -> Result<CallToolResult, TaskExit> {
    if task_context.is_cancel_requested() {
        return Err(TaskExit::Cancelled);
    }

    task_context.set_status_message("Preparing command");
    let cwd = match resolve_cwd(&access, args.cwd.as_deref()).await {
        Ok(cwd) => cwd,
        Err(error) => return Ok(tool_error(error)),
    };

    let program = args.program;
    let mut command = Command::new(&program);
    command
        .args(&args.args)
        .current_dir(&cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    configure_minimal_process_environment(&mut command);
    configure_process_group(&mut command);

    task_context.set_status_message("Starting command");
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => return Ok(tool_error(format!("Failed to spawn {program}: {error}"))),
    };
    let mut process_guard = ProcessGuard::new(&child);

    let Some(stdout) = child.stdout.take() else {
        process_guard.terminate();
        terminate_child(&mut child).await;
        process_guard.disarm();
        return Ok(tool_error("spawned process has no stdout pipe"));
    };
    let Some(stderr) = child.stderr.take() else {
        process_guard.terminate();
        terminate_child(&mut child).await;
        process_guard.disarm();
        return Ok(tool_error("spawned process has no stderr pipe"));
    };
    let stdout_task = tokio::spawn(drain_limited(stdout, STREAM_CAPTURE_LIMIT));
    let stderr_task = tokio::spawn(drain_limited(stderr, STREAM_CAPTURE_LIMIT));

    task_context.set_status_message("Running command");
    let timeout = Duration::from_millis(args.timeout_ms);
    let wait = tokio::time::sleep(timeout);
    tokio::pin!(wait);
    let mut timed_out = false;
    let mut cancelled = false;
    let mut wait_error = None;
    enum WaitOutcome {
        Exited(std::io::Result<std::process::ExitStatus>),
        TimedOut,
        Cancelled,
    }
    let wait_outcome = tokio::select! {
        result = child.wait() => WaitOutcome::Exited(result),
        _ = &mut wait => WaitOutcome::TimedOut,
        _ = task_context.cancelled() => WaitOutcome::Cancelled,
    };
    let exit_code = match wait_outcome {
        WaitOutcome::Exited(Ok(status)) => status.code(),
        WaitOutcome::Exited(Err(error)) => {
            wait_error = Some(format!("Failed to wait for {program}: {error}"));
            process_guard.terminate();
            terminate_child(&mut child).await;
            None
        }
        WaitOutcome::TimedOut => {
            timed_out = true;
            task_context.set_status_message("Terminating timed-out command");
            process_guard.terminate();
            terminate_child(&mut child).await;
            None
        }
        WaitOutcome::Cancelled => {
            cancelled = true;
            task_context.set_status_message("Cancelling command");
            process_guard.terminate();
            terminate_child(&mut child).await;
            None
        }
    };

    task_context.set_status_message("Collecting command output");
    let (stdout_result, stderr_result) = tokio::join!(
        collect_stream(stdout_task, "stdout"),
        collect_stream(stderr_task, "stderr")
    );
    if stdout_result.is_err() || stderr_result.is_err() {
        process_guard.terminate();
    }
    process_guard.disarm();

    if cancelled {
        return Err(TaskExit::Cancelled);
    }
    if let Some(error) = wait_error {
        return Ok(tool_error(error));
    }
    let (stdout, stdout_truncated) = match stdout_result {
        Ok(output) => output,
        Err(error) => return Ok(tool_error(error)),
    };
    let (stderr, stderr_truncated) = match stderr_result {
        Ok(output) => output,
        Err(error) => return Ok(tool_error(error)),
    };

    let output = CommandOutput {
        exit_code,
        stdout: sanitize_stream(&String::from_utf8_lossy(&stdout)),
        stderr: sanitize_stream(&String::from_utf8_lossy(&stderr)),
        timed_out,
        stdout_truncated,
        stderr_truncated,
    };
    render_output(output)
}

fn render_output(output: CommandOutput) -> Result<CallToolResult, TaskExit> {
    let structured = serde_json::to_value(&output).map_err(|error| {
        TaskExit::Error(McpError::internal_error(
            format!("failed to serialize command result: {error}"),
            None,
        ))
    })?;
    let structured_bytes = serde_json::to_vec(&structured).map_err(|error| {
        TaskExit::Error(McpError::internal_error(
            format!("failed to size command result: {error}"),
            None,
        ))
    })?;
    if structured_bytes.len() > MAX_TOOL_RESULT_BYTES {
        return Ok(tool_error(
            "command result exceeds the bounded tool-result size",
        ));
    }

    let mut result = CallToolResult::structured(structured);
    result.content = vec![ContentBlock::text(ShellServer::summary_text(&output))];
    Ok(result)
}

fn preview(text: &str) -> String {
    const PREVIEW_MAX: usize = 4000;
    if text.chars().count() > PREVIEW_MAX {
        let truncated: String = text.chars().take(PREVIEW_MAX).collect();
        format!("{truncated}…")
    } else {
        text.to_string()
    }
}

fn sanitize_stream(text: &str) -> String {
    text.chars()
        .map(|character| {
            if character.is_control() && !matches!(character, '\n' | '\r' | '\t') {
                ' '
            } else {
                character
            }
        })
        .collect()
}

#[cfg(unix)]
fn configure_process_group(command: &mut Command) {
    command.process_group(0);
}

#[cfg(not(unix))]
fn configure_process_group(_command: &mut Command) {}

#[cfg(unix)]
struct ProcessGuard {
    group: Option<nix::unistd::Pid>,
}

#[cfg(unix)]
impl ProcessGuard {
    fn new(child: &tokio::process::Child) -> Self {
        let group = child
            .id()
            .and_then(|id| i32::try_from(id).ok())
            .map(nix::unistd::Pid::from_raw);
        Self { group }
    }

    fn terminate(&self) {
        if let Some(group) = self.group {
            let _ = nix::sys::signal::killpg(group, nix::sys::signal::Signal::SIGKILL);
        }
    }

    fn disarm(&mut self) {
        self.group = None;
    }
}

#[cfg(unix)]
impl Drop for ProcessGuard {
    fn drop(&mut self) {
        self.terminate();
    }
}

#[cfg(not(unix))]
struct ProcessGuard;

#[cfg(not(unix))]
impl ProcessGuard {
    fn new(_child: &tokio::process::Child) -> Self {
        Self
    }

    fn terminate(&self) {}

    fn disarm(&mut self) {}
}

async fn terminate_child(child: &mut tokio::process::Child) {
    let _ = child.kill().await;
    let _ = child.wait().await;
}

async fn collect_stream(
    task: tokio::task::JoinHandle<std::io::Result<(Vec<u8>, bool)>>,
    label: &str,
) -> Result<(Vec<u8>, bool), String> {
    let mut task = task;
    let grace = Duration::from_millis(STREAM_DRAIN_GRACE_MS);
    match tokio::time::timeout(grace, &mut task).await {
        Ok(Ok(Ok(output))) => Ok(output),
        Ok(Ok(Err(error))) => Err(format!("Failed to read {label}: {error}")),
        Ok(Err(error)) => Err(format!("{label} task failed: {error}")),
        Err(_) => {
            task.abort();
            let _ = task.await;
            Err(format!(
                "{label} pipe did not close after process termination"
            ))
        }
    }
}

impl ServerHandler for ShellServer {
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
        let capabilities = if self.tool_enabled {
            ServerCapabilities::builder()
                .enable_tools()
                .enable_tasks()
                .build()
        } else {
            ServerCapabilities::builder().enable_tasks().build()
        };
        ServerInfo::new(capabilities)
            .with_server_info(Implementation::new("tuls-shell", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "Execute local programs with execute_command. The tool requires the MCP Tasks extension and returns a standard task handle. Observe status with tasks/get and cancel with tasks/cancel.",
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
        Self::require_tasks(&context)?;
        if self.get_tool(&request.name).is_none() {
            return Err(McpError::method_not_found::<CallToolRequestMethod>());
        }
        let args = Self::parse_arguments(request.arguments)?;
        if let Some(error) = Self::validate_arguments(&args) {
            return Ok(error.into());
        }
        Ok(self.start_task(args))
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

pub async fn run(options: DirectoryServerOptions) -> anyhow::Result<()> {
    use rmcp::{ServiceExt, transport::stdio};

    let access = AccessControl::from_args(&options.dirs).map_err(anyhow::Error::msg)?;
    let policy = ToolPolicy::from_selectors(&options.tools.allow, &options.tools.deny, TOOL_SPECS)
        .map_err(anyhow::Error::msg)?;
    let server = ShellServer::new(access, policy);
    let tasks = server.tasks.clone();
    let service = server
        .serve(stdio())
        .await
        .inspect_err(|error| tracing::error!("serving error: {error:?}"))?;
    tracing::info!("Shell MCP server running on stdio (MCP {SPEC_VERSION})");

    let result = service.waiting().await;
    tasks.shutdown();
    result?;
    Ok(())
}

#[cfg(test)]
mod tests;
