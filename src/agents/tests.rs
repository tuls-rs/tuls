use super::*;
use rmcp::model::{NumberOrString, ProgressToken};
use std::fs;
use tempfile::tempdir;

fn server() -> AgentsServer {
    let root = tempdir().unwrap().keep();
    let agents = root.join(".agents/agents");
    fs::create_dir_all(&agents).unwrap();
    fs::write(
        agents.join("a.md"),
        "---\nname: alpha\ndescription: First\nmodel: g\nmodel_provider: openai\n---\nWork.",
    )
    .unwrap();
    fs::write(
        agents.join("b.md"),
        "---\nname: beta\ndescription: Second\nmodel: g\nmodel_provider: openai\n---\nWork.",
    )
    .unwrap();
    AgentsServer::new(root, ToolPolicy::default()).unwrap()
}

#[test]
fn tools_have_dynamic_catalog_schema_order_and_identity() {
    let server = server();
    assert_eq!(
        server
            .tools()
            .iter()
            .map(|tool| tool.name.as_ref())
            .collect::<Vec<_>>(),
        ["spawn_agent", "send_input", "wait_agent"]
    );
    let spawn = server.spawn_tool();
    let schema = serde_json::to_value(&spawn).unwrap();
    assert_eq!(schema["inputSchema"]["additionalProperties"], false);
    assert_eq!(
        schema["inputSchema"]["properties"]["name"]["enum"],
        json!(["alpha", "beta"])
    );
    assert!(
        spawn
            .description
            .as_deref()
            .unwrap()
            .contains("- alpha: First\n- beta: Second")
    );
    assert_eq!(
        serde_json::to_value(server.get_info()).unwrap()["serverInfo"]["name"],
        "tuls-agents"
    );
    assert_eq!(
        spawn.annotations.as_ref().unwrap().destructive_hint,
        Some(true)
    );
    assert_eq!(
        server
            .input_tool()
            .annotations
            .as_ref()
            .unwrap()
            .destructive_hint,
        Some(true)
    );
}

#[tokio::test]
async fn oversized_spawn_task_is_rejected() {
    let server = server();
    let arguments = json!({"name":"alpha","task":"x".repeat(MAX_SPAWN_TASK_BYTES + 1)});
    let result = server
        .call(
            "spawn_agent",
            Some(arguments.as_object().unwrap().clone()),
            None,
        )
        .await;
    assert_eq!(result.is_error, Some(true));
    let rendered = serde_json::to_string(&result).unwrap();
    assert!(rendered.contains("invalid_request"));
    assert!(rendered.contains(&MAX_SPAWN_TASK_BYTES.to_string()));
}

#[tokio::test]
async fn oversized_send_input_message_is_rejected() {
    let server = server();
    let arguments = json!({"target":"alpha","message":"x".repeat(MAX_SEND_MESSAGE_BYTES + 1)});
    let result = server
        .call(
            "send_input",
            Some(arguments.as_object().unwrap().clone()),
            None,
        )
        .await;
    assert_eq!(result.is_error, Some(true));
    let rendered = serde_json::to_string(&result).unwrap();
    assert!(rendered.contains("invalid_request"));
    assert!(rendered.contains(&MAX_SEND_MESSAGE_BYTES.to_string()));
}

#[tokio::test]
async fn wait_agent_rejects_more_than_64_targets() {
    let server = server();
    let targets = (0..=MAX_WAIT_TARGETS)
        .map(|index| format!("agt_{index}"))
        .collect::<Vec<_>>();
    let arguments = json!({"targets": targets});
    let result = server
        .call(
            "wait_agent",
            Some(arguments.as_object().unwrap().clone()),
            None,
        )
        .await;
    assert_eq!(result.is_error, Some(true));
    let rendered = serde_json::to_string(&result).unwrap();
    assert!(rendered.contains("invalid_request"));
    assert!(rendered.contains(&MAX_WAIT_TARGETS.to_string()));
}

#[test]
fn wait_schema_bounds_targets_to_64() {
    let schema = serde_json::to_value(server().wait_tool()).unwrap();
    assert_eq!(
        schema["inputSchema"]["properties"]["targets"]["maxItems"],
        json!(MAX_WAIT_TARGETS)
    );
}

#[test]
fn progress_metadata_is_namespaced_bounded_and_monotonic() {
    let running = runtime::AgentResult {
        id: "agt_1".into(),
        name: Some("agent\n🦀".repeat(100)),
        state: runtime::AgentState::Running,
        result: None,
        error: None,
        total_elapsed_ms: 12,
        activity: Some(
            activity::AgentActivity::new(activity::AgentActivityEvent::new(
                activity::ActivityPhase::Model,
                "Working\n".repeat(100),
            ))
            .snapshot(std::time::Instant::now()),
        ),
    };
    let token = ProgressToken(NumberOrString::String("wait-token".into()));
    let first = progress_notification(token.clone(), 1.0, &running, 999);
    let second = progress_notification(token.clone(), 2.0, &running, 998);
    assert_eq!(first.progress_token, token);
    assert!(first.progress < second.progress);
    assert!(first.message.as_deref().unwrap().len() <= 256);
    let metadata = first.meta.unwrap();
    assert_eq!(metadata["io.tuls/agents"]["waitTimeoutRemainingMs"], 999);
    assert_eq!(
        metadata["io.tuls/agents"]["agent"]["activity"]["phase"],
        "model"
    );
    let terminal = runtime::AgentResult {
        state: runtime::AgentState::Completed,
        activity: None,
        result: Some(format!("large-result-marker{}", "x".repeat(1024 * 1024))),
        error: Some(runtime::RuntimeError::new(
            "provider_error",
            "full-error-marker",
        )),
        ..running
    };
    let terminal = progress_notification(token, 3.0, &terminal, 0);
    assert!(terminal.message.unwrap().contains("Completed"));
    let metadata = terminal.meta.unwrap();
    let agent = &metadata["io.tuls/agents"]["agent"];
    assert!(agent.get("result").is_none());
    assert!(agent.get("error").is_none());
    let rendered = serde_json::to_string(&metadata).unwrap();
    assert!(!rendered.contains("large-result-marker"));
    assert!(!rendered.contains("full-error-marker"));
    assert!(rendered.len() < 2048);
}
