use super::*;
use rmcp::{
    model::{CallToolResult, TaskPayload},
    task_manager::{TaskExit, TaskManager, TaskOptions},
};
use tempfile::TempDir;

const AGENT_TOML: &str = "name='test'\ndescription='test agent'\ninstructions='answer the task'\nmodel='gpt-5'\nmodel_provider='openai'\n";

fn runtime() -> (TempDir, AgentRuntime) {
    let temp = tempfile::tempdir().unwrap();
    let agents = temp.path().join(".agents/agents");
    std::fs::create_dir_all(&agents).unwrap();
    std::fs::write(agents.join("test.toml"), AGENT_TOML).unwrap();
    let runtime = AgentRuntime::new(temp.path().to_path_buf()).unwrap();
    (temp, runtime)
}

#[tokio::test]
async fn prepared_turn_reserves_session_until_dropped() {
    let (_temp, runtime) = runtime();
    let turn = runtime.prepare_spawn("test", "first task").await.unwrap();
    let id = turn.id.clone();
    let session = runtime
        .inner
        .sessions
        .lock()
        .await
        .get(&id)
        .cloned()
        .unwrap();
    assert!(session.busy.load(Ordering::Acquire));

    let error = runtime.prepare_input(&id, "follow-up").await.err().unwrap();
    assert_eq!(error.kind, "agent_busy");

    drop(turn);
    assert!(!session.busy.load(Ordering::Acquire));
    session.data.lock().await.resumable = true;
    let follow_up = runtime.prepare_input(&id, "follow-up").await.unwrap();
    assert!(session.busy.load(Ordering::Acquire));
    assert!(!session.data.lock().await.resumable);
    drop(follow_up);
    assert!(!session.busy.load(Ordering::Acquire));
}

#[tokio::test]
async fn input_requires_a_resumable_session() {
    let (_temp, runtime) = runtime();
    let turn = runtime.prepare_spawn("test", "first task").await.unwrap();
    let id = turn.id.clone();
    drop(turn);

    let error = runtime.prepare_input(&id, "follow-up").await.err().unwrap();
    assert_eq!(error.kind, "non_resumable");
}

#[tokio::test]
async fn preparation_enforces_message_limits_before_dispatch() {
    let (_temp, runtime) = runtime();
    let empty = runtime.prepare_spawn("test", "   ").await.err().unwrap();
    assert_eq!(empty.kind, "invalid_request");

    let oversized = "x".repeat(MAX_SPAWN_TASK_BYTES + 1);
    let error = runtime
        .prepare_spawn("test", &oversized)
        .await
        .err()
        .unwrap();
    assert_eq!(error.kind, "invalid_request");

    let oversized = "x".repeat(MAX_SEND_MESSAGE_BYTES + 1);
    let error = runtime
        .prepare_input("missing", &oversized)
        .await
        .err()
        .unwrap();
    assert_eq!(error.kind, "invalid_request");
}

#[tokio::test]
async fn runtime_capacity_is_reserved_before_task_creation() {
    let (_temp, runtime) = runtime();
    let mut turns = Vec::new();
    for index in 0..RUNTIME_CAPACITY {
        turns.push(
            runtime
                .prepare_spawn("test", &format!("task {index}"))
                .await
                .unwrap(),
        );
    }
    let error = runtime
        .prepare_spawn("test", "overflow")
        .await
        .err()
        .unwrap();
    assert_eq!(error.kind, "capacity_exceeded");

    turns.pop();
    runtime.prepare_spawn("test", "available").await.unwrap();
}

#[tokio::test]
async fn idle_session_retention_is_lru_bounded() {
    let (_temp, runtime) = runtime();
    let definition = runtime.registry().get("test").unwrap();
    for index in 0..=MAX_RETAINED_IDLE_SESSIONS {
        let id = format!("agt_test_{index:03}");
        runtime.inner.sessions.lock().await.insert(
            id,
            Arc::new(Session {
                definition: definition.clone(),
                context: String::new(),
                data: Mutex::new(SessionData {
                    conversation: ConversationState::new(&definition.wire_api),
                    resumable: true,
                    last_accessed_at: Instant::now() - Duration::from_secs(index as u64),
                }),
                busy: AtomicBool::new(false),
            }),
        );
    }

    cleanup_idle_sessions(&runtime.inner).await;
    let sessions = runtime.inner.sessions.lock().await;
    assert_eq!(sessions.len(), MAX_RETAINED_IDLE_SESSIONS);
    assert!(!sessions.contains_key(&format!("agt_test_{:03}", MAX_RETAINED_IDLE_SESSIONS)));
}

#[test]
fn execution_results_preserve_agent_identity_and_resumability() {
    let completed = map_execution_result("agt_1", "test", Ok("done".into()));
    let TurnOutcome::Completed(completed) = completed else {
        panic!("expected completed turn");
    };
    assert_eq!(completed.id, "agt_1");
    assert_eq!(completed.name, "test");
    assert_eq!(completed.result, "done");

    let failed = map_execution_result(
        "agt_1",
        "test",
        Err(ExecutionFailure::new("provider_error", "failed", true)),
    );
    let TurnOutcome::Failed(failed) = failed else {
        panic!("expected failed turn");
    };
    assert_eq!(failed.kind, "provider_error");
    assert!(failed.resumable);
}

async fn settle_task(tasks: &TaskManager, task_id: &str) {
    loop {
        let state = tasks.get_task(task_id).expect("get task");
        if state.task.status.is_terminal() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// A follow-up turn cancelled before dispatch must settle cancelled and leave
/// the session resumable: no provider request or child tool execution started,
/// so the conversation boundary is unchanged. Only this pre-dispatch path may
/// restore resumability.
#[tokio::test]
async fn pre_dispatch_cancel_of_follow_up_turn_restores_resumability() {
    let (_temp, runtime) = runtime();
    let first = runtime.prepare_spawn("test", "first task").await.unwrap();
    let id = first.id.clone();
    drop(first);
    {
        let session = runtime
            .inner
            .sessions
            .lock()
            .await
            .get(&id)
            .cloned()
            .unwrap();
        session.data.lock().await.resumable = true;
    }
    let follow_up = runtime.prepare_input(&id, "follow-up").await.unwrap();

    let tasks = TaskManager::new();
    let (gate_tx, gate_rx) = tokio::sync::oneshot::channel();
    let runtime_for_task = runtime.clone();
    let task = tasks.spawn(TaskOptions::new(), move |context| {
        Box::pin(async move {
            let _ = gate_rx.await;
            match runtime_for_task.execute(follow_up, context).await {
                TurnOutcome::Cancelled => Err(TaskExit::Cancelled),
                TurnOutcome::Completed(_) | TurnOutcome::Failed(_) => {
                    Ok(CallToolResult::success(Vec::new()))
                }
            }
        })
    });
    // The cancellation is already recorded before execute() observes the turn.
    tasks.cancel_task(&task.task_id).expect("cancel task");
    gate_tx.send(()).expect("open gate");
    settle_task(&tasks, &task.task_id).await;
    assert!(
        matches!(
            tasks.get_task(&task.task_id).expect("task").payload,
            TaskPayload::Cancelled
        ),
        "the pre-dispatch cancelled turn must settle cancelled"
    );

    // The same agentId accepts another turn: resumability was restored and
    // the busy lease released.
    runtime
        .prepare_input(&id, "again")
        .await
        .expect("resumable");
}

/// Once a follow-up turn has started executing, a failure must leave the
/// session fail-closed: the pre-dispatch cancellation branch is the only path
/// that restores resumability.
#[tokio::test]
async fn post_dispatch_failure_keeps_follow_up_session_non_resumable() {
    let temp = tempfile::tempdir().unwrap();
    let agents = temp.path().join(".agents/agents");
    std::fs::create_dir_all(&agents).unwrap();
    std::fs::write(
        agents.join("credential-gated.toml"),
        "name='credential-gated'\ndescription='test agent'\ninstructions='answer'\nmodel_provider='custom'\nmodel='gpt-5'\nbase_url='http://127.0.0.1:1'\nenv_key='TULS_TEST_AGENT_CREDENTIAL'\nwire_api='responses'\n",
    )
    .unwrap();
    let runtime = AgentRuntime::new(temp.path().to_path_buf()).unwrap();

    let first = runtime
        .prepare_spawn("credential-gated", "first task")
        .await
        .unwrap();
    let id = first.id.clone();
    drop(first);
    {
        let session = runtime
            .inner
            .sessions
            .lock()
            .await
            .get(&id)
            .cloned()
            .unwrap();
        session.data.lock().await.resumable = true;
    }
    let follow_up = runtime.prepare_input(&id, "follow-up").await.unwrap();

    let tasks = TaskManager::new();
    let runtime_for_task = runtime.clone();
    let task = tasks.spawn(TaskOptions::new(), move |context| {
        Box::pin(async move {
            match runtime_for_task.execute(follow_up, context).await {
                TurnOutcome::Cancelled => Err(TaskExit::Cancelled),
                TurnOutcome::Completed(_) | TurnOutcome::Failed(_) => {
                    Ok(CallToolResult::success(Vec::new()))
                }
            }
        })
    });
    settle_task(&tasks, &task.task_id).await;

    // The turn ran to a non-resumable failure (missing credential): the
    // session must not be restored.
    let error = runtime
        .prepare_input(&id, "again")
        .await
        .err()
        .expect("session must stay non-resumable");
    assert_eq!(error.kind, "non_resumable");
}
