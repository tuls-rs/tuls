use super::*;
use crate::agents::definition::{ChildToolPolicy, WireApi};
use std::{collections::BTreeMap, time::Instant};
use tempfile::tempdir;
use url::Url;

fn definition() -> Arc<AgentDefinition> {
    Arc::new(AgentDefinition {
        name: "test".into(),
        description: "test".into(),
        instructions: "test".into(),
        model: "test".into(),
        base_url: Url::parse("https://example.test").unwrap(),
        env_key: "TEST_KEY".into(),
        wire_api: WireApi::Responses,
        reasoning_effort: None,
        temperature: None,
        max_turns: 1,
        tool_policy: ChildToolPolicy::default(),
        skills: vec![],
        mcp_servers: BTreeMap::new(),
        source_path: PathBuf::new(),
    })
}
async fn insert(runtime: &AgentRuntime, id: &str, state: AgentState, result: Option<&str>) {
    insert_at(runtime, id, state, result, Instant::now()).await;
}
async fn insert_at(
    runtime: &AgentRuntime,
    id: &str,
    state: AgentState,
    result: Option<&str>,
    created_at: Instant,
) {
    runtime.inner.sessions.lock().await.insert(
        id.into(),
        Arc::new(Session {
            definition: definition(),
            context: String::new(),
            data: Mutex::new(SessionData {
                conversation: None,
                queue: VecDeque::new(),
                cancel: None,
                interrupt_pending: false,
                accepting_input: true,
                state: state.clone(),
                result: result.map(str::to_owned),
                error: None,
                resumable: true,
                activity: (state == AgentState::Running).then(|| {
                    AgentActivity::new(AgentActivityEvent::new(
                        ActivityPhase::Starting,
                        "Starting agent",
                    ))
                }),
                terminal_at: (state != AgentState::Running).then(Instant::now),
                last_accessed_at: Instant::now(),
                revision: 1,
            }),
            created_at,
        }),
    );
}
async fn runtime() -> AgentRuntime {
    AgentRuntime::new(tempdir().unwrap().keep()).unwrap()
}

#[tokio::test]
async fn wait_is_immediate_non_consuming_and_zero_timeout_is_a_snapshot() {
    let runtime = runtime().await;
    insert(&runtime, "done", AgentState::Completed, Some("answer")).await;
    insert(&runtime, "running", AgentState::Running, None).await;
    let done = vec!["done".into()];
    assert!(!runtime.wait(&done, 10).await.unwrap().timed_out);
    assert_eq!(
        runtime.wait(&done, 0).await.unwrap().agents[0]
            .result
            .as_deref(),
        Some("answer")
    );
    assert!(
        runtime
            .wait(&["running".into()], 0)
            .await
            .unwrap()
            .timed_out
    );
}

#[tokio::test]
async fn wait_observes_event_driven_and_post_subscription_transition() {
    let runtime = runtime().await;
    insert_at(
        &runtime,
        "one",
        AgentState::Running,
        None,
        Instant::now() - Duration::from_secs(9),
    )
    .await;
    let waiter = {
        let runtime = runtime.clone();
        tokio::spawn(async move { runtime.wait(&["one".into()], 2_000).await.unwrap() })
    };
    tokio::time::sleep(Duration::from_millis(5)).await;
    let started = Instant::now();
    let session = runtime.session("one").await.unwrap();
    session.data.lock().await.state = AgentState::Completed;
    runtime
        .inner
        .version
        .send_modify(|v| *v = v.wrapping_add(1));
    assert!(!waiter.await.unwrap().timed_out);
    assert!(started.elapsed() < Duration::from_millis(200));
}

#[tokio::test]
async fn snapshots_keep_activity_and_terminal_timing_stable() {
    let runtime = runtime().await;
    insert_at(
        &runtime,
        "one",
        AgentState::Running,
        None,
        Instant::now() - Duration::from_secs(9),
    )
    .await;
    let session = runtime.session("one").await.unwrap();
    {
        let mut data = session.data.lock().await;
        data.activity.as_mut().unwrap().started_at = Instant::now() - Duration::from_secs(2);
    }
    let initial = runtime
        .wait(&["one".into()], 0)
        .await
        .unwrap()
        .agents
        .remove(0);
    assert!((8_900..=9_100).contains(&initial.total_elapsed_ms));
    assert!((1_900..=2_100).contains(&initial.activity.unwrap().activity_elapsed_ms));
    {
        let mut data = session.data.lock().await;
        data.activity.as_mut().unwrap().started_at = Instant::now() - Duration::from_secs(1);
    }
    let replaced = runtime
        .wait(&["one".into()], 0)
        .await
        .unwrap()
        .agents
        .remove(0);
    assert!((8_900..=9_100).contains(&replaced.total_elapsed_ms));
    assert!((900..=1_100).contains(&replaced.activity.unwrap().activity_elapsed_ms));
    {
        let mut data = session.data.lock().await;
        data.state = AgentState::Completed;
        data.terminal_at = Some(Instant::now() - Duration::from_secs(3));
        data.revision += 1;
    }
    runtime.inner.version.send_modify(|v| *v += 1);
    let first = runtime
        .wait(&["one".into()], 0)
        .await
        .unwrap()
        .agents
        .remove(0);
    let second = runtime
        .wait(&["one".into()], 0)
        .await
        .unwrap()
        .agents
        .remove(0);
    assert!(first.activity.is_none());
    assert!((5_900..=6_100).contains(&first.total_elapsed_ms));
    assert_eq!(first.total_elapsed_ms, second.total_elapsed_ms);
}

#[tokio::test]
async fn observations_are_initial_then_only_revised_targets() {
    let runtime = runtime().await;
    insert(&runtime, "one", AgentState::Running, None).await;
    insert(&runtime, "other", AgentState::Running, None).await;
    let (tx, mut rx) = mpsc::channel(16);
    let task = {
        let runtime = runtime.clone();
        tokio::spawn(async move {
            runtime
                .wait_observing(&["one".into()], 100, tx)
                .await
                .unwrap()
        })
    };
    assert_eq!(rx.recv().await.unwrap().result.agents.len(), 1);
    // A global version change and another session's revision must not emit.
    {
        let other = runtime.session("other").await.unwrap();
        other.data.lock().await.revision += 1;
    }
    runtime.inner.version.send_modify(|v| *v += 1);
    assert!(
        tokio::time::timeout(Duration::from_millis(10), rx.recv())
            .await
            .is_err()
    );
    let session = runtime.session("one").await.unwrap();
    {
        let mut data = session.data.lock().await;
        data.activity = Some(AgentActivity::new(AgentActivityEvent::tool(
            "Reading src/lib.rs".into(),
            "read_text_file".into(),
            Some("src/lib.rs".into()),
        )));
        data.revision += 1;
    }
    runtime.inner.version.send_modify(|v| *v += 1);
    let update = rx.recv().await.unwrap();
    assert_eq!(update.result.agents.len(), 1);
    assert_eq!(
        update.result.agents[0]
            .activity
            .as_ref()
            .unwrap()
            .tool
            .as_deref(),
        Some("read_text_file")
    );
    task.await.unwrap();
}

#[tokio::test]
async fn activity_updates_observe_without_completing_the_wait() {
    let runtime = runtime().await;
    insert(&runtime, "one", AgentState::Running, None).await;
    let (tx, mut rx) = mpsc::channel(16);
    let task = {
        let runtime = runtime.clone();
        tokio::spawn(async move {
            runtime
                .wait_observing(&["one".into()], 2_000, tx)
                .await
                .unwrap()
        })
    };
    rx.recv().await.unwrap();
    let session = runtime.session("one").await.unwrap();
    {
        let mut data = session.data.lock().await;
        data.activity = Some(AgentActivity::new(AgentActivityEvent::new(
            ActivityPhase::Model,
            "Waiting for model response",
        )));
        data.revision += 1;
    }
    runtime.inner.version.send_modify(|v| *v += 1);
    let observed = tokio::time::timeout(Duration::from_millis(100), rx.recv())
        .await
        .expect("activity update is event-driven")
        .unwrap();
    assert_eq!(observed.result.agents[0].state, AgentState::Running);
    assert!(!task.is_finished(), "activity alone must not end the wait");
    {
        let mut data = session.data.lock().await;
        data.state = AgentState::Completed;
        data.activity = Some(AgentActivity::new(AgentActivityEvent::new(
            ActivityPhase::Completed,
            "Completed",
        )));
        data.terminal_at = Some(Instant::now());
        data.revision += 1;
    }
    runtime.inner.version.send_modify(|v| *v += 1);
    assert!(
        tokio::time::timeout(Duration::from_millis(100), task)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn progress_backpressure_is_lossy_and_never_blocks_completion() {
    let runtime = runtime().await;
    insert(&runtime, "one", AgentState::Running, None).await;
    let (tx, _rx) = mpsc::channel(1);
    let waiter = {
        let runtime = runtime.clone();
        tokio::spawn(async move {
            runtime
                .wait_observing(&["one".into()], 2_000, tx)
                .await
                .unwrap()
        })
    };
    tokio::task::yield_now().await;
    let session = runtime.session("one").await.unwrap();
    for index in 0..100 {
        let mut data = session.data.lock().await;
        data.activity = Some(AgentActivity::new(AgentActivityEvent::new(
            ActivityPhase::Model,
            format!("Update {index}"),
        )));
        data.revision += 1;
        drop(data);
        runtime.inner.version.send_modify(|value| *value += 1);
    }
    {
        let mut data = session.data.lock().await;
        data.state = AgentState::Completed;
        data.result = Some("final".into());
        data.terminal_at = Some(Instant::now());
        data.revision += 1;
    }
    runtime.inner.version.send_modify(|value| *value += 1);
    let result = tokio::time::timeout(Duration::from_millis(200), waiter)
        .await
        .expect("a full progress channel cannot block the wait")
        .unwrap();
    assert_eq!(result.agents[0].result.as_deref(), Some("final"));
}

#[tokio::test]
async fn terminal_retention_is_lru_bounded_and_never_evicts_running_sessions() {
    let runtime = runtime().await;
    let tied = Instant::now() - Duration::from_secs(10);
    for index in 0..=MAX_RETAINED_TERMINAL_SESSIONS {
        let id = format!("terminal-{index:03}");
        insert(&runtime, &id, AgentState::Completed, Some("answer")).await;
        runtime
            .session(&id)
            .await
            .unwrap()
            .data
            .lock()
            .await
            .last_accessed_at = tied;
    }
    insert(&runtime, "running", AgentState::Running, None).await;
    cleanup_terminal_sessions(&runtime.inner).await;
    assert!(runtime.session("running").await.is_ok());
    let error = match runtime.session("terminal-000").await {
        Ok(_) => panic!("least-recent terminal session was retained"),
        Err(error) => error,
    };
    assert_eq!(error.kind, "unknown_agent");
    assert!(runtime.session("terminal-001").await.is_ok());
    assert_eq!(
        runtime.inner.sessions.lock().await.len(),
        MAX_RETAINED_TERMINAL_SESSIONS + 1
    );
}

#[tokio::test]
async fn wait_and_send_input_refresh_session_recency() {
    let runtime = runtime().await;
    insert(&runtime, "done", AgentState::Completed, Some("answer")).await;
    insert(&runtime, "running", AgentState::Running, None).await;
    let old = Instant::now() - Duration::from_secs(10);
    runtime
        .session("done")
        .await
        .unwrap()
        .data
        .lock()
        .await
        .last_accessed_at = old;
    runtime
        .session("running")
        .await
        .unwrap()
        .data
        .lock()
        .await
        .last_accessed_at = old;
    runtime.wait(&["done".into()], 0).await.unwrap();
    assert!(
        runtime
            .session("done")
            .await
            .unwrap()
            .data
            .lock()
            .await
            .last_accessed_at
            > old
    );
    runtime
        .send_input("running", "queued", false)
        .await
        .unwrap();
    assert!(
        runtime
            .session("running")
            .await
            .unwrap()
            .data
            .lock()
            .await
            .last_accessed_at
            > old
    );
}

#[tokio::test]
async fn interrupt_intent_is_preserved_before_a_run_installs_its_token() {
    let runtime = runtime().await;
    insert(&runtime, "starting", AgentState::Running, None).await;
    runtime
        .send_input("starting", "replacement", true)
        .await
        .unwrap();
    let session = runtime.session("starting").await.unwrap();
    let data = session.data.lock().await;
    assert!(data.interrupt_pending);
    assert_eq!(data.queue.front().map(String::as_str), Some("replacement"));
}

#[tokio::test]
async fn interrupt_is_transferred_across_the_startup_to_turn_token_handoff() {
    let runtime = runtime().await;
    insert(&runtime, "handoff", AgentState::Running, None).await;
    let session = runtime.session("handoff").await.unwrap();
    let startup_cancel = CancellationToken::new();
    let mut data = session.data.lock().await;
    data.cancel = Some(startup_cancel.clone());

    // Reproduce the exact race: the worker has passed its startup check,
    // then send_input cancels the published startup token before handoff.
    assert!(!startup_cancel.is_cancelled());
    queue_input(&mut data, "replacement", true).unwrap();
    assert!(startup_cancel.is_cancelled());
    let turn_cancel = install_turn_cancel(&mut data, Some(&startup_cancel));

    assert!(turn_cancel.is_cancelled());
    assert!(data.cancel.as_ref().unwrap().is_cancelled());
    assert_eq!(data.queue.front().map(String::as_str), Some("replacement"));
}

#[tokio::test]
async fn interrupt_pending_is_applied_to_every_turn_handoff() {
    let runtime = runtime().await;
    insert(&runtime, "between-turns", AgentState::Running, None).await;
    let session = runtime.session("between-turns").await.unwrap();
    let mut data = session.data.lock().await;
    data.cancel = None;
    queue_input(&mut data, "replacement", true).unwrap();

    let turn_cancel = install_turn_cancel(&mut data, None);

    assert!(turn_cancel.is_cancelled());
    assert!(!data.interrupt_pending);
    assert_eq!(data.queue.front().map(String::as_str), Some("replacement"));
}

#[tokio::test]
async fn queued_interrupt_is_not_applied_to_the_message_that_carried_it() {
    let runtime = runtime().await;
    insert(&runtime, "between-turns", AgentState::Running, None).await;
    let session = runtime.session("between-turns").await.unwrap();
    let mut data = session.data.lock().await;
    data.cancel = None;
    queue_input(&mut data, "replacement", true).unwrap();

    assert_eq!(
        pop_input_for_execution(&mut data).as_deref(),
        Some("replacement")
    );
    let turn_cancel = install_turn_cancel(&mut data, None);
    assert!(!turn_cancel.is_cancelled());
    assert!(!data.interrupt_pending);
}

#[tokio::test]
async fn accepted_inputs_remain_fifo_after_a_resumable_failure() {
    let runtime = runtime().await;
    insert(&runtime, "one", AgentState::Running, None).await;
    let session = runtime.session("one").await.unwrap();
    let mut data = session.data.lock().await;
    queue_input(&mut data, "second", false).unwrap();
    queue_input(&mut data, "third", false).unwrap();
    set_terminal(
        &mut data,
        Err(super::super::provider::ProviderError {
            kind: "provider_error",
            message: "transient".into(),
            resumable: true,
        }),
    );
    assert_eq!(
        pop_input_for_execution(&mut data).as_deref(),
        Some("second")
    );
    assert_eq!(pop_input_for_execution(&mut data).as_deref(), Some("third"));
}

#[tokio::test]
async fn finishing_run_rejects_input_before_acknowledging_it() {
    let runtime = runtime().await;
    insert(&runtime, "one", AgentState::Running, None).await;
    let session = runtime.session("one").await.unwrap();
    session.data.lock().await.accepting_input = false;
    let error = runtime
        .send_input("one", "too late", false)
        .await
        .unwrap_err();
    assert_eq!(error.kind, "not_accepting_input");
    assert!(session.data.lock().await.queue.is_empty());
}

#[tokio::test]
async fn ambiguous_tool_execution_is_terminal_and_non_resumable() {
    let runtime = runtime().await;
    insert(&runtime, "one", AgentState::Running, None).await;
    let session = runtime.session("one").await.unwrap();
    let mut data = session.data.lock().await;
    set_terminal(
        &mut data,
        Err(super::super::provider::ProviderError {
            kind: "ambiguous_tool_execution",
            message: "dispatched but outcome unknown".into(),
            resumable: false,
        }),
    );
    assert_eq!(data.state, AgentState::Failed);
    assert!(!data.resumable);
    assert_eq!(
        data.error.as_ref().unwrap().kind,
        "ambiguous_tool_execution"
    );
    assert!(ensure_resumable(&data).is_err());
}

#[tokio::test]
async fn resumable_provider_failures_stay_resumable() {
    let runtime = runtime().await;
    insert(&runtime, "one", AgentState::Running, None).await;
    let session = runtime.session("one").await.unwrap();
    let mut data = session.data.lock().await;
    set_terminal(
        &mut data,
        Err(super::super::provider::ProviderError {
            kind: "provider_error",
            message: "transient".into(),
            resumable: true,
        }),
    );
    assert_eq!(data.state, AgentState::Failed);
    assert!(data.resumable);
    assert!(ensure_resumable(&data).is_ok());
}
