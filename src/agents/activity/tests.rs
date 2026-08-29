use super::*;

#[test]
fn activity_events_are_bounded_and_sanitized() {
    let tool = AgentActivityEvent::tool(
        "Reading src/🦀.rs\n".repeat(100),
        "read_text_file".repeat(100),
        Some("src/🦀.rs\n".repeat(100)),
    );
    assert_eq!(tool.phase, ActivityPhase::Tool);
    assert!(tool.summary.len() <= SUMMARY_LIMIT);
    assert!(tool.tool.as_deref().unwrap().len() <= TARGET_LIMIT);
    assert!(tool.target.as_deref().unwrap().len() <= TARGET_LIMIT);
    assert!(!tool.summary.chars().any(char::is_control));
    assert!(
        !tool
            .target
            .as_deref()
            .unwrap()
            .chars()
            .any(char::is_control)
    );
}

#[test]
fn lifecycle_helpers_emit_human_readable_status() {
    let starting = AgentActivityEvent::new(ActivityPhase::Starting, "Starting agent");
    let completed = AgentActivityEvent::tool_completed();
    let failed = AgentActivityEvent::tool_failed();
    let timed_out = AgentActivityEvent::tool_timed_out();

    assert_eq!(starting.summary, "Starting agent");
    assert_eq!(completed.summary, "Waiting for model response");
    assert_eq!(failed.summary, "Tool call failed");
    assert_eq!(timed_out.summary, "Tool call timed out");
}
