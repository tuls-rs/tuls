use super::*;

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
    assert!(text.contains('…'), "long output is previewed");
    assert!(!text.contains("exit code"), "no exit code on timeout");
}

#[test]
fn preview_keeps_short_text_intact() {
    assert_eq!(preview("short"), "short");
    let long = "a".repeat(5000);
    let p = preview(&long);
    assert_eq!(p.chars().count(), 4001, "4000 chars plus ellipsis");
    assert!(p.ends_with('…'));
}

#[test]
fn sanitize_stream_replaces_binary_controls() {
    assert_eq!(sanitize_stream("a\0b\n"), "a b\n");
}
