use super::*;
use serde_json::json;
use tempfile::tempdir;

fn server() -> AgentsServer {
    let temp = tempdir().unwrap();
    let agents = temp.path().join(".agents/agents");
    std::fs::create_dir_all(&agents).unwrap();
    std::fs::write(
        agents.join("test.toml"),
        "name='test'\ndescription='test agent'\ninstructions='answer'\nmodel='gpt-5'\nmodel_provider='openai'\n",
    )
    .unwrap();
    AgentsServer::new(
        temp.keep(),
        ToolPolicy::from_selectors(&[], &[], TOOL_SPECS).unwrap(),
    )
    .unwrap()
}

#[test]
fn tools_are_task_native_and_minimal() {
    let server = server();
    let tools = server.tools();
    assert_eq!(
        tools
            .iter()
            .map(|tool| tool.name.as_ref())
            .collect::<Vec<_>>(),
        ["spawn_agent", "send_input"]
    );

    let spawn = tools
        .iter()
        .find(|tool| tool.name == "spawn_agent")
        .unwrap();
    assert_eq!(spawn.input_schema["required"], json!(["name", "task"]));
    assert!(spawn.description.as_deref().unwrap().contains("MCP Task"));

    let input = tools.iter().find(|tool| tool.name == "send_input").unwrap();
    assert_eq!(input.input_schema["required"], json!(["target", "message"]));
    let properties = input.input_schema["properties"].as_object().unwrap();
    assert_eq!(
        properties.keys().map(String::as_str).collect::<Vec<_>>(),
        ["message", "target"]
    );
}

#[test]
fn server_advertises_standard_tasks_extension() {
    let info = server().get_info();
    assert!(info.capabilities.supports_tasks());
    assert!(info.capabilities.tools.is_some());
}

#[test]
fn policy_can_grant_each_agent_tool_independently() {
    let temp = tempdir().unwrap();
    let agents = temp.path().join(".agents/agents");
    std::fs::create_dir_all(&agents).unwrap();
    std::fs::write(
        agents.join("test.toml"),
        "name='test'\ndescription='test agent'\ninstructions='answer'\nmodel='gpt-5'\nmodel_provider='openai'\n",
    )
    .unwrap();
    let server = AgentsServer::new(
        temp.keep(),
        ToolPolicy::from_selectors(&["agents/spawn_agent".into()], &[], TOOL_SPECS).unwrap(),
    )
    .unwrap();
    assert!(server.get_tool("spawn_agent").is_some());
    assert!(server.get_tool("send_input").is_none());
}

#[tokio::test]
async fn agent_tasks_advertise_the_shared_poll_interval() {
    let server = server();
    let turn = server
        .runtime
        .prepare_spawn("test", "first task")
        .await
        .unwrap();
    let CallToolResponse::Task(task) = server.start_task(turn) else {
        panic!("expected a task handle");
    };
    assert_eq!(
        task.task.poll_interval_ms,
        Some(DEFAULT_TASK_POLL_INTERVAL_MS)
    );
}

#[test]
fn completed_task_result_contains_full_text_and_structured_output() {
    let result = runtime::AgentTurnResult {
        id: "agt_1".into(),
        name: "test".into(),
        result: "final answer".into(),
    };
    let rendered = render_turn_result(&result).unwrap();
    assert!(!rendered.is_error.unwrap_or(false));
    assert_eq!(
        rendered.structured_content.as_ref().unwrap()["agentId"],
        "agt_1"
    );
    assert_eq!(
        rendered.structured_content.as_ref().unwrap()["result"],
        "final answer"
    );
    let text = rendered.content[0].as_text().unwrap().text.as_str();
    assert!(text.contains("final answer"));
}

#[test]
fn agent_failures_complete_as_tool_errors_not_task_failures() {
    let error = runtime::AgentTurnError {
        id: "agt_1".into(),
        name: "test".into(),
        kind: "provider_error".into(),
        message: "provider unavailable".into(),
        resumable: true,
    };
    let rendered = render_turn_error(&error).unwrap();
    assert_eq!(rendered.is_error, Some(true));
    let structured = rendered.structured_content.as_ref().expect("structured");
    assert_eq!(structured["agentId"], "agt_1");
    assert_eq!(structured["kind"], "provider_error");
    assert_eq!(structured["resumable"], true);
    let text = rendered.content[0].as_text().unwrap().text.as_str();
    assert!(text.contains("provider_error"));
    assert!(text.contains("send_input"), "resumable error hints: {text}");
}
