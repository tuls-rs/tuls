use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler, ServiceExt,
    model::{
        CallToolRequestMethod, CallToolResult, ContentBlock, CreateTaskResult, ElicitRequest,
        ElicitRequestParams, GetTaskResult, Implementation, InputRequest, ListToolsResult,
        ServerCapabilities, ServerInfo, Tool, UpdateTaskParams,
    },
    service::TxJsonRpcMessage,
    task_manager::{TaskExit, TaskManager, TaskOptions},
};

use super::*;

/// The behavior of the child Task materialized by the test server.
enum TaskProgram {
    /// Stays `working` until the test completes it.
    Gated(tokio::sync::oneshot::Receiver<()>),
    /// Enters `input_required` immediately and stays there.
    RequestInput,
}

/// An in-process task-capable MCP server whose single `run` tool materializes
/// a child Task with test-controlled behavior.
struct TaskScriptServer {
    tasks: TaskManager,
    options: TaskOptions,
    program: std::sync::Mutex<Option<TaskProgram>>,
    /// When `Some`, overrides the `pollIntervalMs` advertised by the seed
    /// `CreateTaskResult`; `tasks/get` keeps serving the spawned value.
    seed_poll_interval_ms: Option<u64>,
    poll_count: Arc<AtomicUsize>,
    cancel_count: Arc<AtomicUsize>,
}

impl TaskScriptServer {
    fn run_tool(&self) -> Tool {
        Tool::new(
            "run",
            "scripted task-based test tool",
            serde_json::json!({"type": "object"})
                .as_object()
                .expect("object schema")
                .clone(),
        )
    }
}

impl ServerHandler for TaskScriptServer {
    fn initialize(
        &self,
        _request: rmcp::model::InitializeRequestParams,
        _context: rmcp::service::RequestContext<RoleServer>,
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
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_tasks()
                .build(),
        )
        .with_server_info(Implementation::new(
            "tuls-child-task-test",
            env!("CARGO_PKG_VERSION"),
        ))
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        (name == "run").then(|| self.run_tool())
    }

    fn list_tools(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: rmcp::service::RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListToolsResult, McpError>>
    + rmcp::service::MaybeSendFuture
    + '_ {
        std::future::ready(Ok(ListToolsResult::with_all_items(vec![self.run_tool()])))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: rmcp::service::RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        if request.name != "run" {
            return Err(McpError::method_not_found::<CallToolRequestMethod>());
        }
        let program = self
            .program
            .lock()
            .expect("program lock")
            .take()
            .expect("program configured once");
        let task = self.tasks.spawn(self.options.clone(), move |context| {
            Box::pin(async move {
                match program {
                    TaskProgram::Gated(done) => {
                        done.await.expect("test signal");
                        Ok(CallToolResult::success(vec![ContentBlock::text(
                            "child done",
                        )]))
                    }
                    TaskProgram::RequestInput => {
                        let _ = context
                            .request_input(
                                "probe",
                                InputRequest::Elicitation(ElicitRequest::new(
                                    ElicitRequestParams::UrlElicitationParams {
                                        meta: None,
                                        message: "probe".into(),
                                        url: "https://example.test".into(),
                                        elicitation_id: "probe-1".into(),
                                    },
                                )),
                            )
                            .await;
                        Err(TaskExit::Cancelled)
                    }
                }
            })
        });
        let mut task = task;
        if let Some(poll_interval_ms) = self.seed_poll_interval_ms {
            task.poll_interval_ms = Some(poll_interval_ms);
        }
        Ok(CreateTaskResult::new(task).into())
    }

    async fn get_task(
        &self,
        request: GetTaskParams,
        _context: rmcp::service::RequestContext<RoleServer>,
    ) -> Result<GetTaskResult, McpError> {
        self.poll_count.fetch_add(1, AtomicOrdering::SeqCst);
        self.tasks
            .get_task(&request.task_id)
            .map(GetTaskResult::new)
    }

    async fn update_task(
        &self,
        request: UpdateTaskParams,
        _context: rmcp::service::RequestContext<RoleServer>,
    ) -> Result<(), McpError> {
        self.tasks
            .update_task(&request.task_id, request.input_responses)
    }

    async fn cancel_task(
        &self,
        request: CancelTaskParams,
        _context: rmcp::service::RequestContext<RoleServer>,
    ) -> Result<(), McpError> {
        self.cancel_count.fetch_add(1, AtomicOrdering::SeqCst);
        self.tasks.cancel_task(&request.task_id)
    }
}

/// Connect a task-capable client to an in-process `TaskScriptServer`.
async fn connect_to_task_server(
    server: TaskScriptServer,
) -> RunningService<RoleClient, ClientInfo> {
    let (client_to_server_tx, client_to_server_rx) =
        futures::channel::mpsc::channel::<TxJsonRpcMessage<RoleClient>>(64);
    let (server_to_client_tx, server_to_client_rx) =
        futures::channel::mpsc::channel::<TxJsonRpcMessage<RoleServer>>(64);
    tokio::spawn(async move {
        let service = server
            .serve((server_to_client_tx, client_to_server_rx))
            .await
            .expect("serve in-process task server");
        let _ = service.waiting().await;
    });
    let lifecycle = ClientLifecycleMode::Discover {
        preferred_versions: vec![ProtocolVersion::V_2026_07_28],
    };
    client_info()
        .serve_with_lifecycle((client_to_server_tx, server_to_client_rx), lifecycle)
        .await
        .expect("connect in-process task client")
}

/// Materialize the child Task through the in-process server.
async fn start_child_task(client: &RunningService<RoleClient, ClientInfo>) -> rmcp::model::Task {
    let response = client
        .call_tool_once(CallToolRequestParams::new("run"))
        .await
        .expect("call child tool");
    let CallToolResponse::Task(task) = response else {
        panic!("expected a child task handle");
    };
    task.task
}

async fn finish_child_wait(
    wait: tokio::task::JoinHandle<std::result::Result<ChildToolResult, ChildCallError>>,
) -> std::result::Result<ChildToolResult, ChildCallError> {
    for _ in 0..4 {
        tokio::time::advance(Duration::from_secs(5)).await;
        tokio::task::yield_now().await;
        if wait.is_finished() {
            return wait.await.expect("join");
        }
    }
    panic!("child wait must settle after completion");
}

/// Spawn the child-task wait, moving the client into the task.
fn spawn_child_wait(
    client: RunningService<RoleClient, ClientInfo>,
    task: rmcp::model::Task,
    cancel: CancellationToken,
) -> tokio::task::JoinHandle<std::result::Result<ChildToolResult, ChildCallError>> {
    let task_id = task.task_id.clone();
    let poll_interval_ms = task.poll_interval_ms;
    tokio::spawn(
        async move { wait_for_task_result(&client, &task_id, poll_interval_ms, &cancel).await },
    )
}

fn scripted_server(
    options: TaskOptions,
    program: TaskProgram,
    seed_poll_interval_ms: Option<u64>,
    poll_count: Arc<AtomicUsize>,
    cancel_count: Arc<AtomicUsize>,
) -> TaskScriptServer {
    TaskScriptServer {
        tasks: TaskManager::new(),
        options,
        program: std::sync::Mutex::new(Some(program)),
        seed_poll_interval_ms,
        poll_count,
        cancel_count,
    }
}

/// A child Task must remain observable past `CHILD_MCP_CALL_TIMEOUT` of total
/// lifetime; the constant bounds individual MCP RPCs only.
#[tokio::test(start_paused = true)]
async fn child_task_lifetime_is_not_bounded_by_the_call_timeout() {
    let (done_tx, done_rx) = tokio::sync::oneshot::channel();
    let server = scripted_server(
        TaskOptions::new().with_poll_interval_ms(1000),
        TaskProgram::Gated(done_rx),
        None,
        Arc::new(AtomicUsize::new(0)),
        Arc::new(AtomicUsize::new(0)),
    );
    let client = connect_to_task_server(server).await;
    let cancel = CancellationToken::new();
    let task = start_child_task(&client).await;
    let wait = spawn_child_wait(client, task, cancel);

    for _ in 0..14 {
        tokio::time::advance(Duration::from_secs(5)).await;
        tokio::task::yield_now().await;
        assert!(
            !wait.is_finished(),
            "a child task must not be abandoned merely because its lifetime \
             exceeds CHILD_MCP_CALL_TIMEOUT"
        );
    }
    done_tx.send(()).expect("complete child task");
    let result = finish_child_wait(wait).await.expect("child result");
    assert!(!result.is_error);
    assert!(result.output.contains("child done"));
}

/// The server-provided `pollIntervalMs` must be honored, never capped down.
#[tokio::test(start_paused = true)]
async fn child_polling_honors_the_server_poll_interval() {
    let (done_tx, done_rx) = tokio::sync::oneshot::channel();
    let poll_count = Arc::new(AtomicUsize::new(0));
    let server = scripted_server(
        TaskOptions::new().with_poll_interval_ms(5000),
        TaskProgram::Gated(done_rx),
        None,
        poll_count.clone(),
        Arc::new(AtomicUsize::new(0)),
    );
    let client = connect_to_task_server(server).await;
    let cancel = CancellationToken::new();
    let task = start_child_task(&client).await;
    assert_eq!(task.poll_interval_ms, Some(5000));
    let wait = spawn_child_wait(client, task, cancel);

    tokio::task::yield_now().await;
    assert_eq!(poll_count.load(AtomicOrdering::SeqCst), 0);
    tokio::time::advance(Duration::from_secs(4)).await;
    tokio::task::yield_now().await;
    assert_eq!(
        poll_count.load(AtomicOrdering::SeqCst),
        0,
        "must not poll before the first 5000 ms elapse"
    );
    tokio::time::advance(Duration::from_secs(1)).await;
    tokio::task::yield_now().await;
    assert_eq!(poll_count.load(AtomicOrdering::SeqCst), 1);
    tokio::time::advance(Duration::from_secs(4)).await;
    tokio::task::yield_now().await;
    assert_eq!(
        poll_count.load(AtomicOrdering::SeqCst),
        1,
        "a 5000 ms interval must not be capped down"
    );
    tokio::time::advance(Duration::from_secs(1)).await;
    tokio::task::yield_now().await;
    assert_eq!(poll_count.load(AtomicOrdering::SeqCst), 2);

    done_tx.send(()).expect("complete child task");
    let result = finish_child_wait(wait).await.expect("child result");
    assert!(!result.is_error);
}

/// The initial `CreateTaskResult` suggestion governs the first wait; the
/// latest `tasks/get` suggestion governs every later wait.
#[tokio::test(start_paused = true)]
async fn child_polling_follows_the_latest_server_suggestion() {
    let (done_tx, done_rx) = tokio::sync::oneshot::channel();
    let poll_count = Arc::new(AtomicUsize::new(0));
    let server = scripted_server(
        TaskOptions::new().with_poll_interval_ms(5000),
        TaskProgram::Gated(done_rx),
        Some(1000),
        poll_count.clone(),
        Arc::new(AtomicUsize::new(0)),
    );
    let client = connect_to_task_server(server).await;
    let cancel = CancellationToken::new();
    let task = start_child_task(&client).await;
    assert_eq!(task.poll_interval_ms, Some(1000));
    let wait = spawn_child_wait(client, task, cancel);

    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(1)).await;
    tokio::task::yield_now().await;
    assert_eq!(
        poll_count.load(AtomicOrdering::SeqCst),
        1,
        "the seed suggestion governs the first wait"
    );
    tokio::time::advance(Duration::from_secs(4)).await;
    tokio::task::yield_now().await;
    assert_eq!(
        poll_count.load(AtomicOrdering::SeqCst),
        1,
        "tasks/get returned 5000 ms, so the second wait must last 5000 ms"
    );
    tokio::time::advance(Duration::from_secs(1)).await;
    tokio::task::yield_now().await;
    assert_eq!(poll_count.load(AtomicOrdering::SeqCst), 2);

    done_tx.send(()).expect("complete child task");
    let result = finish_child_wait(wait).await.expect("child result");
    assert!(!result.is_error);
}

/// Parent cancellation of an already-created child Task must send a standard
/// `tasks/cancel` before the parent stops waiting.
#[tokio::test]
async fn parent_cancellation_cancels_the_child_task() {
    let (done_tx, done_rx) = tokio::sync::oneshot::channel();
    let cancel_count = Arc::new(AtomicUsize::new(0));
    let server = scripted_server(
        TaskOptions::new(),
        TaskProgram::Gated(done_rx),
        None,
        Arc::new(AtomicUsize::new(0)),
        cancel_count.clone(),
    );
    let client = connect_to_task_server(server).await;
    let cancel = CancellationToken::new();
    let task = start_child_task(&client).await;
    let wait = spawn_child_wait(client, task, cancel.clone());
    tokio::time::sleep(Duration::from_millis(50)).await;
    cancel.cancel();

    let outcome = wait.await.expect("join");
    assert!(matches!(outcome, Err(ChildCallError::Interrupted)));
    assert_eq!(
        cancel_count.load(AtomicOrdering::SeqCst),
        1,
        "tasks/cancel must be sent for the abandoned child task"
    );
    let _ = done_tx.send(());
}

/// A child Task that enters `input_required` is intentionally unsupported:
/// the parent cancels it best-effort and fails the call.
#[tokio::test(start_paused = true)]
async fn input_required_cancels_the_child_task_before_failing() {
    let cancel_count = Arc::new(AtomicUsize::new(0));
    let server = scripted_server(
        TaskOptions::new(),
        TaskProgram::RequestInput,
        None,
        Arc::new(AtomicUsize::new(0)),
        cancel_count.clone(),
    );
    let client = connect_to_task_server(server).await;
    let cancel = CancellationToken::new();
    let task = start_child_task(&client).await;
    let wait = spawn_child_wait(client, task, cancel);

    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_millis(1200)).await;
    tokio::task::yield_now().await;
    assert!(
        wait.is_finished(),
        "the input_required state must settle the wait"
    );
    let outcome = wait.await.expect("join");
    assert!(matches!(outcome, Err(ChildCallError::Failed)));
    assert_eq!(
        cancel_count.load(AtomicOrdering::SeqCst),
        1,
        "input_required must be cancelled best-effort"
    );
}

#[test]
fn interpolation_is_strict() {
    assert_eq!(
        interpolate_with("x${CHILD_MCP_TEST}/${CHILD_MCP_TEST}", |name| {
            (name == "CHILD_MCP_TEST").then(|| "one".to_owned())
        })
        .unwrap(),
        "xone/one"
    );
    assert_eq!(
        interpolate_with("plain text", |_| None).unwrap(),
        "plain text"
    );
    assert!(interpolate_with("${CHILD_MCP_ABSENT}", |_| None).is_err());
    assert!(interpolate_with("${bad-name}", |_| None).is_err());
    assert!(interpolate_with("${CHILD_MCP_TEST", |_| None).is_err());
}

#[test]
fn names_are_safe_and_unique() {
    let mut used = BTreeSet::new();
    assert_eq!(qualified_name("a b", "x/y"), "a_b__x_y");
    assert_eq!(unique_name("same".into(), &mut used).unwrap(), "same");
    assert_eq!(unique_name("same".into(), &mut used).unwrap(), "same_2");
    let base = qualified_name(&"server".repeat(20), &"tool".repeat(20));
    let first = unique_name(base.clone(), &mut used).unwrap();
    let second = unique_name(base, &mut used).unwrap();
    for name in [&first, &second] {
        assert!(name.len() <= 64);
        assert!(
            name.bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
        );
    }
    assert!(second.ends_with("_2"));
}

#[test]
fn http_configuration_disables_session_reinitialization() {
    assert!(!http_config("https://example.test/mcp").reinit_on_expired_session);
}

#[test]
fn child_tool_policy_is_default_deny_and_deny_overrides_allow() {
    let default_policy = ChildToolPolicy::default();
    assert!(!permitted(&default_policy, "server", "read"));

    let policy =
        ChildToolPolicy::new(["server/*".to_string()], ["server/write".to_string()]).unwrap();
    assert!(permitted(&policy, "server", "read"));
    assert!(!permitted(&policy, "server", "write"));
    assert!(!permitted(&policy, "other", "read"));
}

#[test]
fn output_is_bounded() {
    assert!(
        bounded_text(&"x".repeat(MAX_OUTPUT_BYTES + 10), MAX_OUTPUT_BYTES).len()
            <= MAX_OUTPUT_BYTES
    );
}

#[test]
fn rendered_output_is_valid_json_or_a_tool_error() {
    let small = render_output(
        &[rmcp::model::ContentBlock::text("ok")],
        Some(&serde_json::json!({"value": 1})),
    )
    .unwrap();
    assert!(serde_json::from_str::<serde_json::Value>(&small).is_ok());

    let oversized = map_call_result(rmcp::model::CallToolResult::success(vec![
        rmcp::model::ContentBlock::text("x".repeat(MAX_OUTPUT_BYTES)),
    ]));
    assert!(oversized.is_error);
    assert!(oversized.output.contains("exceeds"));
}

#[test]
fn child_tool_results_preserve_the_reported_is_error() {
    let success = map_call_result(rmcp::model::CallToolResult::success(vec![
        rmcp::model::ContentBlock::text("all good"),
    ]));
    assert!(!success.is_error);
    assert!(success.output.contains("all good"));
    let failure = map_call_result(rmcp::model::CallToolResult::error(vec![
        rmcp::model::ContentBlock::text("boom"),
    ]));
    assert!(failure.is_error);
    assert!(failure.output.contains("boom"));
    let absent = map_call_result(rmcp::model::CallToolResult::default());
    assert!(!absent.is_error, "an absent isError is treated as false");
    assert!(absent.output.contains("structuredContent"));
}

#[tokio::test]
async fn call_rejects_without_dispatch_when_it_cannot_route() {
    let manager = ChildMcpManager::empty();
    let cancel = CancellationToken::new();
    let rejected = manager
        .call("unknown_tool", serde_json::json!({}), &cancel)
        .await;
    assert!(matches!(rejected, Err(ChildCallError::Rejected)));
    let rejected = manager
        .call("unknown_tool", serde_json::json!([]), &cancel)
        .await;
    assert!(matches!(rejected, Err(ChildCallError::Rejected)));
}

#[test]
fn schema_and_catalog_limits_fail_closed() {
    assert!(catalog_limits_exceeded(MAX_CHILD_TOOLS + 1, 0, 1));
    assert!(catalog_limits_exceeded(1, 0, MAX_SCHEMA_BYTES + 1));
    assert!(catalog_limits_exceeded(1, MAX_TOTAL_SCHEMA_BYTES, 1));
}
