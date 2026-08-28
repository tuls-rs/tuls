use super::*;

#[test]
fn activity_events_cover_lifecycle_and_sanitize_unicode() {
    let starting = AgentActivity::new(AgentActivityEvent::new(ActivityPhase::Starting, "Start"));
    let model = AgentActivity::new(AgentActivityEvent::new(ActivityPhase::Model, "Model"));
    let tool_event = AgentActivityEvent::tool(
        "Reading src/🦀.rs".into(),
        "read_text_file".into(),
        Some("src/🦀.rs".into()),
    );
    assert_eq!(tool_event.phase, ActivityPhase::Tool);
    assert!(tool_event.deadline.is_some());
    let tool = AgentActivity::new(tool_event);
    let model_after_tool = AgentActivity::new(AgentActivityEvent::tool_completed());
    let completed = AgentActivity::new(AgentActivityEvent::new(ActivityPhase::Completed, "Done"));
    let failed = AgentActivity::new(AgentActivityEvent::new(ActivityPhase::Failed, "Failed"));
    assert_eq!(
        starting.snapshot(Instant::now()).phase,
        ActivityPhase::Starting
    );
    assert_eq!(model.snapshot(Instant::now()).phase, ActivityPhase::Model);
    let snapshot = tool.snapshot(Instant::now());
    assert_eq!(snapshot.phase, ActivityPhase::Tool);
    assert_eq!(snapshot.tool.as_deref(), Some("read_text_file"));
    assert_eq!(snapshot.target.as_deref(), Some("src/🦀.rs"));
    assert!(snapshot.operation_timeout_remaining_ms.is_some());
    assert_eq!(
        model_after_tool.snapshot(Instant::now()).phase,
        ActivityPhase::Model
    );
    assert_eq!(
        completed.snapshot(Instant::now()).phase,
        ActivityPhase::Completed
    );
    assert_eq!(failed.snapshot(Instant::now()).phase, ActivityPhase::Failed);
    let bounded = bound("🦀\n".repeat(100), 17);
    assert!(!bounded.chars().any(char::is_control));
    assert!(bounded.len() <= 17);
}

#[test]
fn activity_deadlines_use_execution_timeouts() {
    let before = Instant::now();
    let tool = AgentActivityEvent::tool("Calling tool".into(), "tool".into(), None);
    let tool_deadline = tool.deadline.unwrap();
    assert!(tool_deadline >= before + CHILD_MCP_CALL_TIMEOUT);
    assert!(tool_deadline <= Instant::now() + CHILD_MCP_CALL_TIMEOUT);

    let before = Instant::now();
    let model = AgentActivityEvent::tool_completed();
    let model_deadline = model.deadline.unwrap();
    assert!(model_deadline >= before + PROVIDER_REQUEST_TIMEOUT);
    assert!(model_deadline <= Instant::now() + PROVIDER_REQUEST_TIMEOUT);
}
