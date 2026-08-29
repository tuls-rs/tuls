use super::*;
use tempfile::tempdir;

fn server() -> ShellServer {
    let temp = tempdir().unwrap();
    let access = AccessControl::from_args(&[temp.keep()]).unwrap();
    ShellServer::new(
        access,
        ToolPolicy::from_selectors(&[], &[], TOOL_SPECS).unwrap(),
    )
}

#[test]
fn server_advertises_tasks_and_only_the_command_tool() {
    let server = server();
    let info = server.get_info();
    assert!(info.capabilities.supports_tasks());
    assert!(info.capabilities.tools.is_some());
    let tools = server.tools();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "execute_command");
    assert!(
        tools[0]
            .description
            .as_deref()
            .unwrap()
            .contains("MCP Task")
    );
}

#[test]
fn semantic_argument_errors_are_tool_errors() {
    let empty = ExecuteCommandArgs {
        program: "   ".into(),
        args: Vec::new(),
        cwd: None,
        timeout_ms: DEFAULT_TIMEOUT_MS,
    };
    assert_eq!(
        ShellServer::validate_arguments(&empty).unwrap().is_error,
        Some(true)
    );

    let out_of_range = ExecuteCommandArgs {
        program: "echo".into(),
        args: Vec::new(),
        cwd: None,
        timeout_ms: MAX_TIMEOUT_MS + 1,
    };
    assert_eq!(
        ShellServer::validate_arguments(&out_of_range)
            .unwrap()
            .is_error,
        Some(true)
    );
}

#[tokio::test]
async fn command_tasks_advertise_the_shared_poll_interval() {
    let server = server();
    let args = ExecuteCommandArgs {
        program: "true".into(),
        args: Vec::new(),
        cwd: None,
        timeout_ms: 1000,
    };
    let CallToolResponse::Task(task) = server.start_task(args) else {
        panic!("expected a task handle");
    };
    assert_eq!(
        task.task.poll_interval_ms,
        Some(DEFAULT_TASK_POLL_INTERVAL_MS)
    );
}

#[test]
fn task_ttl_exceeds_command_execution_limit() {
    assert_eq!(task_ttl_ms(1), TASK_TTL_MARGIN_MS + 1);
    assert_eq!(
        task_ttl_ms(MAX_TIMEOUT_MS),
        MAX_TIMEOUT_MS + TASK_TTL_MARGIN_MS
    );
}

#[test]
fn summary_text_reports_exit_code() {
    let output = CommandOutput {
        exit_code: Some(0),
        stdout: "out".to_string(),
        stderr: String::new(),
        timed_out: false,
        stdout_truncated: false,
        stderr_truncated: false,
    };
    let text = ShellServer::summary_text(&output);
    assert!(text.contains("exit code: 0"));
    assert!(text.contains("stdout (full): out"));
}

#[test]
fn summary_text_reports_timeout_and_truncation() {
    let output = CommandOutput {
        exit_code: None,
        stdout: "x".repeat(5000),
        stderr: String::new(),
        timed_out: true,
        stdout_truncated: true,
        stderr_truncated: false,
    };
    let text = ShellServer::summary_text(&output);
    assert!(text.contains("timed out"));
    assert!(text.contains("stdout (truncated at 8 KiB)"));
    assert!(text.contains('…'));
    assert!(!text.contains("exit code"));
}

#[test]
fn preview_keeps_short_text_intact() {
    assert_eq!(preview("short"), "short");
    let long = "a".repeat(5000);
    let preview = preview(&long);
    assert_eq!(preview.chars().count(), 4001);
    assert!(preview.ends_with('…'));
}

#[test]
fn sanitize_stream_replaces_binary_controls() {
    assert_eq!(sanitize_stream("a\0b\n"), "a b\n");
}

#[test]
fn command_output_is_structured_and_bounded() {
    let output = CommandOutput {
        exit_code: Some(0),
        stdout: "out".into(),
        stderr: String::new(),
        timed_out: false,
        stdout_truncated: false,
        stderr_truncated: false,
    };
    let result = render_output(output).unwrap();
    assert_eq!(result.structured_content.as_ref().unwrap()["exitCode"], 0);
    assert!(!result.is_error.unwrap_or(false));
}
