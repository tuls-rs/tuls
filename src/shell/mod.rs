mod drain;

use std::path::PathBuf;
use std::sync::Arc;

use rmcp::{
    ErrorData as McpError, ServerHandler,
    handler::server::router::tool::ToolRouter,
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router,
};
use serde::{Deserialize, Serialize};
use tokio::process::Command;

use crate::cli::DirectoryServerOptions;
use crate::policy::{Capability, ToolPolicy, ToolSpec};
use crate::support::{
    AccessControl, MAX_TOOL_RESULT_BYTES, SPEC_VERSION, configure_minimal_process_environment,
    tool_error,
};

use self::drain::drain_limited;

/// Default and maximum execution time in milliseconds.
pub const DEFAULT_TIMEOUT_MS: u64 = 120_000;
pub const MAX_TIMEOUT_MS: u64 = 600_000;

/// Maximum number of bytes retained per captured stream. After this limit the
/// pipe keeps being drained, but additional bytes are discarded.
const STREAM_CAPTURE_LIMIT: usize = 8 * 1024;
const STREAM_DRAIN_GRACE_MS: u64 = 2_000;

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[schemars(description = "Arguments for executing a command")]
pub struct ExecuteCommandArgs {
    /// The executable to run, resolved through PATH like a direct exec. Never
    /// a shell command string.
    #[schemars(
        description = "The executable to run, resolved through PATH like a direct exec. Never a shell command string."
    )]
    pub program: String,
    /// Arguments passed to the executable, each preserved exactly as given
    /// (no shell parsing, quoting, or glob expansion)
    #[serde(default)]
    #[schemars(
        description = "Arguments passed to the executable, each preserved exactly as given (no shell parsing, quoting, or glob expansion)"
    )]
    pub args: Vec<String>,
    /// Working directory for the command. Must resolve inside one of the
    /// allowed directories. Relative paths resolve against the first allowed
    /// directory. Defaults to the first allowed directory.
    #[serde(default)]
    #[schemars(
        description = "Working directory for the command. Must resolve inside one of the allowed directories. Relative paths resolve against the first allowed directory. Defaults to the first allowed directory."
    )]
    pub cwd: Option<String>,
    /// Maximum execution time in milliseconds (1 to 600000, default 120000).
    /// On expiry the process is terminated and `timedOut` is reported.
    #[serde(default = "default_timeout_ms")]
    #[schemars(range(min = 1, max = 600000))]
    #[schemars(
        description = "Maximum execution time in milliseconds (1 to 600000, default 120000). On expiry the process is terminated and `timedOut` is reported."
    )]
    pub timeout_ms: u64,
}

fn default_timeout_ms() -> u64 {
    DEFAULT_TIMEOUT_MS
}

/// Structured result of a completed (or timed out) command execution.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(description = "The result of a command execution")]
pub struct CommandOutput {
    /// The child's numeric exit code, or null when no normal exit code is
    /// available (e.g. the process was terminated on timeout)
    #[schemars(
        description = "The child's numeric exit code, or null when no normal exit code is available (e.g. the process was terminated on timeout)"
    )]
    pub exit_code: Option<i32>,
    /// Captured standard output, lossy UTF-8, bounded to 8 KiB
    #[schemars(description = "Captured standard output, lossy UTF-8, bounded to 8 KiB")]
    pub stdout: String,
    /// Captured standard error, lossy UTF-8, bounded to 8 KiB
    #[schemars(description = "Captured standard error, lossy UTF-8, bounded to 8 KiB")]
    pub stderr: String,
    /// True when the command was terminated because it exceeded `timeoutMs`
    #[schemars(
        description = "True when the command was terminated because it exceeded `timeoutMs`"
    )]
    pub timed_out: bool,
    /// True when stdout exceeded the 8 KiB capture limit and was truncated
    #[schemars(
        description = "True when stdout exceeded the 8 KiB capture limit and was truncated"
    )]
    pub stdout_truncated: bool,
    /// True when stderr exceeded the 8 KiB capture limit and was truncated
    #[schemars(
        description = "True when stderr exceeded the 8 KiB capture limit and was truncated"
    )]
    pub stderr_truncated: bool,
}

const TOOL_SPECS: &[ToolSpec] = &[ToolSpec::new(
    "shell",
    "execute_command",
    Capability::ProcessExecute,
)];

#[derive(Debug, Clone)]
pub struct ShellServer {
    access: Arc<AccessControl>,
    tool_router: ToolRouter<ShellServer>,
}

impl ShellServer {
    pub fn new(access: AccessControl, policy: ToolPolicy) -> Self {
        let mut tool_router = Self::tool_router();
        for spec in TOOL_SPECS {
            if !policy.allows(*spec) {
                tool_router.disable_route(spec.name);
            }
        }
        Self {
            access: Arc::new(access),
            tool_router,
        }
    }

    /// Resolve and verify `cwd` against the allowed directories. `None` falls
    /// back to the first allowed directory.
    async fn resolve_cwd(&self, cwd: Option<&str>) -> Result<PathBuf, String> {
        let requested = cwd.unwrap_or(".");
        let resolved = self.access.validate_path(requested).await?;
        let meta = tokio::fs::metadata(&resolved)
            .await
            .map_err(|e| format!("Failed to access cwd {}: {e}", resolved.display()))?;
        if !meta.is_dir() {
            return Err(format!("cwd is not a directory: {}", resolved.display()));
        }
        Ok(resolved)
    }

    /// Concise text representation for clients that primarily consume MCP
    /// text content; the full payload is in `structuredContent`.
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

fn preview(text: &str) -> String {
    const PREVIEW_MAX: usize = 4000;
    let count = text.chars().count();
    if count > PREVIEW_MAX {
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
fn process_group_id(child: &tokio::process::Child) -> Option<nix::unistd::Pid> {
    child
        .id()
        .and_then(|id| i32::try_from(id).ok())
        .map(nix::unistd::Pid::from_raw)
}

#[cfg(not(unix))]
fn process_group_id(_child: &tokio::process::Child) -> Option<()> {
    None
}

#[cfg(unix)]
fn terminate_process_group(group: Option<nix::unistd::Pid>) {
    if let Some(group) = group {
        let _ = nix::sys::signal::killpg(group, nix::sys::signal::Signal::SIGKILL);
    }
}

#[cfg(not(unix))]
fn terminate_process_group(_group: Option<()>) {}

async fn terminate_child(child: &mut tokio::process::Child) {
    let _ = child.kill().await;
    let _ = child.wait().await;
}

async fn collect_stream(
    task: tokio::task::JoinHandle<std::io::Result<(Vec<u8>, bool)>>,
    label: &str,
) -> Result<(Vec<u8>, bool), String> {
    let mut task = task;
    let grace = std::time::Duration::from_millis(STREAM_DRAIN_GRACE_MS);
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

#[tool_router(router = tool_router)]
impl ShellServer {
    #[tool(
        name = "execute_command",
        title = "Execute Command",
        description = "Execute one local program directly with an explicit argv and wait for it to finish or time out. The program is resolved through PATH and spawned without a shell: no shell syntax, quoting, or glob expansion is applied, and the argument list is passed exactly as given. If shell features are required, run an installed shell explicitly, e.g. program=\"bash\" with args=[\"-lc\", \"cargo test && git status\"]. The working directory must be inside one of the server's allowed directories. Stdout and stderr are captured separately, each bounded to 8 KiB. On timeout the process is terminated and timedOut is reported. The child receives a minimal inherited environment; this tool is not an operating-system sandbox and executes with the MCP process's OS permissions.",
        output_schema = rmcp::handler::server::tool::schema_for_output::<CommandOutput>(),
        annotations(read_only_hint = false, destructive_hint = true, idempotent_hint = false, open_world_hint = true)
    )]
    async fn execute_command(
        &self,
        Parameters(args): Parameters<ExecuteCommandArgs>,
    ) -> Result<CallToolResult, McpError> {
        if args.program.trim().is_empty() {
            return Ok(tool_error(
                "program must be a non-empty executable name or path",
            ));
        }
        if !(1..=MAX_TIMEOUT_MS).contains(&args.timeout_ms) {
            return Ok(tool_error(format!(
                "timeoutMs must be between 1 and {MAX_TIMEOUT_MS}, got {}",
                args.timeout_ms
            )));
        }
        let cwd = match self.resolve_cwd(args.cwd.as_deref()).await {
            Ok(cwd) => cwd,
            Err(e) => return Ok(tool_error(e)),
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
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(e) => {
                return Ok(tool_error(format!("Failed to spawn {program}: {e}")));
            }
        };

        let group = process_group_id(&child);
        let Some(stdout) = child.stdout.take() else {
            terminate_process_group(group);
            terminate_child(&mut child).await;
            return Ok(tool_error("spawned process has no stdout pipe"));
        };
        let Some(stderr) = child.stderr.take() else {
            terminate_process_group(group);
            terminate_child(&mut child).await;
            return Ok(tool_error("spawned process has no stderr pipe"));
        };
        let stdout_task = tokio::spawn(drain_limited(stdout, STREAM_CAPTURE_LIMIT));
        let stderr_task = tokio::spawn(drain_limited(stderr, STREAM_CAPTURE_LIMIT));

        let timeout = std::time::Duration::from_millis(args.timeout_ms);
        let mut timed_out = false;
        let exit_code = match tokio::time::timeout(timeout, child.wait()).await {
            Ok(Ok(status)) => status.code(),
            Ok(Err(e)) => {
                terminate_process_group(group);
                terminate_child(&mut child).await;
                return Ok(tool_error(format!("Failed to wait for {program}: {e}")));
            }
            Err(_) => {
                timed_out = true;
                terminate_process_group(group);
                terminate_child(&mut child).await;
                None
            }
        };

        let (stdout_result, stderr_result) = tokio::join!(
            collect_stream(stdout_task, "stdout"),
            collect_stream(stderr_task, "stderr")
        );
        if stdout_result.is_err() || stderr_result.is_err() {
            terminate_process_group(group);
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

        let structured = serde_json::to_value(&output).map_err(|e| {
            McpError::internal_error(format!("Failed to serialize result: {e}"), None)
        })?;
        let structured_bytes = serde_json::to_vec(&structured)
            .map_err(|e| McpError::internal_error(format!("Failed to size result: {e}"), None))?;
        if structured_bytes.len() > MAX_TOOL_RESULT_BYTES {
            return Ok(tool_error(
                "command result exceeds the bounded tool-result size",
            ));
        }

        // Structured result with the declared output schema, plus a concise
        // text representation for text-only clients.
        let mut result = CallToolResult::structured(structured);
        result.content = vec![ContentBlock::text(Self::summary_text(&output))];
        Ok(result)
    }
}

#[tool_handler(router = self.tool_router)]
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
        ServerInfo::new(if self.tool_router.list_all().is_empty() {
            ServerCapabilities::builder().build()
        } else {
            ServerCapabilities::builder().enable_tools().build()
        })
        .with_server_info(Implementation::new("tuls-shell", env!("CARGO_PKG_VERSION")))
        .with_instructions(
            "This server executes local programs directly (no shell) with the OS \
                 permissions of the MCP server process. The working directory is \
                 restricted to the allowed directories passed on the command line.",
        )
    }
}

/// Start the shell server on stdio.
pub async fn run(options: DirectoryServerOptions) -> anyhow::Result<()> {
    use rmcp::ServiceExt;
    use rmcp::transport::stdio;

    let access = AccessControl::from_args(&options.dirs).map_err(anyhow::Error::msg)?;
    let policy = ToolPolicy::from_selectors(&options.tools.allow, &options.tools.deny, TOOL_SPECS)
        .map_err(anyhow::Error::msg)?;

    let server = ShellServer::new(access, policy);
    let service = server
        .serve(stdio())
        .await
        .inspect_err(|e| tracing::error!("serving error: {e:?}"))?;
    tracing::info!("Shell MCP server running on stdio (MCP {SPEC_VERSION})");

    service.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests;
