use super::*;
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use super::super::definition::ChildToolPolicy;

fn definition(wire_api: WireApi) -> AgentDefinition {
    AgentDefinition {
        name: "test".into(),
        description: String::new(),
        instructions: String::new(),
        model: "model".into(),
        base_url: Url::parse("https://example.test/v1").unwrap(),
        env_key: "TEST_KEY".into(),
        wire_api,
        reasoning_effort: Some("high".into()),
        temperature: Some(0.5),
        max_turns: 2,
        tool_policy: ChildToolPolicy::default(),
        skills: Vec::new(),
        mcp_servers: BTreeMap::new(),
        source_path: PathBuf::new(),
    }
}

#[test]
fn credentials_are_redacted_and_resolved_without_environment_mutation() {
    let credential =
        ProviderCredential::resolve_with("TEST_KEY", |_| Some("secret-value".into())).unwrap();
    assert!(!format!("{credential:?}").contains("secret-value"));
    let error = ProviderCredential::resolve_with("TEST_KEY", |_| Some(String::new())).unwrap_err();
    assert_eq!(error.kind, "missing_environment_variable");
    assert_eq!(
        error.message,
        "Required environment variable TEST_KEY is not available."
    );
}

#[test]
fn parses_responses_items_and_preserves_reasoning() {
    let parsed = parse_responses_response(json!({"output":[
        {"type":"reasoning","summary":[],"encrypted_content":"opaque"},
        {"type":"function_call","call_id":"call_1","name":"tool","arguments":"{\"x\":1}"}
    ]}))
    .unwrap();
    assert_eq!(parsed.items.len(), 2);
    assert_eq!(parsed.calls[0].arguments, json!({"x": 1}));
}

#[test]
fn request_builders_are_stateless_and_wire_specific() {
    let history = [responses_user_message("hello")];
    let responses = responses_request(&definition(WireApi::Responses), "system", &history, &[]);
    assert_eq!(responses["store"], false);
    assert_eq!(responses["reasoning"]["effort"], "high");
    assert!(responses.get("instructions").is_none());
    assert!(responses.get("previous_response_id").is_none());
    assert_eq!(responses["input"][0]["role"], "developer");
    assert_eq!(responses["input"][0]["content"][0]["text"], "system");
    // Every turn is the same complete, stateless replay, so a follow-up
    // request is structurally identical to the first.
    let follow_up = responses_request(&definition(WireApi::Responses), "system", &history, &[]);
    assert_eq!(follow_up, responses);
    let anthropic = anthropic_request(
        &definition(WireApi::AnthropicMessages),
        "system",
        &[anthropic_user_message("hello")],
        &[],
    );
    assert_eq!(anthropic["max_tokens"], DEFAULT_ANTHROPIC_MAX_TOKENS);
    assert_eq!(anthropic["output_config"]["effort"], "high");
    assert_eq!(anthropic["temperature"], 0.5);
}

#[test]
fn responses_replay_has_exactly_one_developer_item_and_never_duplicates() {
    let mut history = vec![responses_user_message("hello")];
    commit_responses(
        &mut history,
        vec![
            json!({"type":"message","role":"assistant","content":[{"type":"output_text","text":"calling"}]}),
            json!({"type":"function_call","call_id":"call_1","name":"tool","arguments":"{}"}),
        ],
        responses_tool_outputs(vec![ToolResult {
            id: "call_1".into(),
            output: "done".into(),
            is_error: true,
        }]),
    );
    let request = responses_request(&definition(WireApi::Responses), "system", &history, &[]);
    let input = request["input"].as_array().unwrap();
    let developer_items = input
        .iter()
        .filter(|item| item["role"] == "developer")
        .count();
    assert_eq!(developer_items, 1);
    assert_eq!(input[0]["role"], "developer");
    assert_eq!(input[0]["content"][0]["type"], "input_text");
    assert_eq!(input[0]["content"][0]["text"], "system");
    assert!(request.get("instructions").is_none());
    assert!(request.get("previous_response_id").is_none());
    assert_eq!(request["store"], false);
    assert_eq!(input.len(), 5);
    assert!(!history.iter().any(|item| item["role"] == "developer"));
}

#[test]
fn responses_tool_outputs_mark_errors_unambiguously() {
    let outputs = responses_tool_outputs(vec![
        ToolResult {
            id: "call_ok".into(),
            output: "fine".into(),
            is_error: false,
        },
        ToolResult {
            id: "call_bad".into(),
            output: "failed".into(),
            is_error: true,
        },
    ]);
    assert_eq!(outputs[0]["type"], "function_call_output");
    assert_eq!(outputs[0]["call_id"], "call_ok");
    assert_eq!(outputs[0]["output"], "fine");
    assert!(outputs[0].get("is_error").is_none());
    assert_eq!(outputs[1]["call_id"], "call_bad");
    assert!(outputs[1].get("is_error").is_none());
    let error: Value = serde_json::from_str(outputs[1]["output"].as_str().unwrap()).unwrap();
    assert_eq!(error["isError"], true);
    assert_eq!(error["output"], "failed");
}

#[test]
fn child_call_outcomes_are_committed_or_ambiguous() {
    let committed = tool_result_from_call(
        "call_1".into(),
        Ok(ChildToolResult {
            output: "done".into(),
            is_error: true,
        }),
    )
    .unwrap();
    assert_eq!(committed.id, "call_1");
    assert_eq!(committed.output, "done");
    assert!(committed.is_error);
    let rejected = tool_result_from_call("call_2".into(), Err(ChildCallError::Rejected)).unwrap();
    assert!(rejected.is_error);
    assert!(rejected.output.contains("rejected"));
    for outcome in [ChildCallError::TimedOut, ChildCallError::Failed] {
        let error = tool_result_from_call("call_3".into(), Err(outcome)).unwrap_err();
        assert_eq!(error.kind, "ambiguous_tool_execution");
        assert!(!error.resumable);
        assert!(error.message.len() <= ERROR_MESSAGE_LIMIT);
    }
    let interrupted =
        tool_result_from_call("call_4".into(), Err(ChildCallError::Interrupted)).unwrap_err();
    assert_eq!(interrupted.kind, "ambiguous_tool_execution");
    assert!(!interrupted.resumable);
}

#[test]
fn provider_request_serialization_enforces_the_body_limit() {
    assert_eq!(
        serialize_provider_body(&json!({"ok": true})).unwrap(),
        "{\"ok\":true}"
    );
    let oversized = json!("x".repeat(MAX_PROVIDER_BODY_BYTES));
    let error = serialize_provider_body(&oversized).unwrap_err();
    assert_eq!(error.kind, "context_limit");
    assert!(error.message.len() <= ERROR_MESSAGE_LIMIT);
    assert!(!error.message.contains("xxxxxxxx"));
}

#[test]
fn provider_context_limit_errors_are_safe_and_portable() {
    for (status, body) in [
        (StatusCode::PAYLOAD_TOO_LARGE, br#"secret payload"#.as_slice()),
        (
            StatusCode::BAD_REQUEST,
            br#"{"error":{"code":"context_length_exceeded","message":"secret conversation maximum context length"}}"#
                .as_slice(),
        ),
    ] {
        let error = status_error(status, body);
        assert_eq!(error.kind, "context_limit");
        assert!(!error.resumable);
        assert!(!error.message.contains("secret"));
        assert!(error.message.len() <= ERROR_MESSAGE_LIMIT);
    }
    assert_eq!(
        status_error(StatusCode::BAD_REQUEST, br#"{"error":{"code":"invalid"}}"#).kind,
        "provider_error"
    );
    assert!(!status_error(StatusCode::BAD_REQUEST, b"").resumable);
    assert!(!status_error(StatusCode::UNAUTHORIZED, b"").resumable);
    assert!(status_error(StatusCode::REQUEST_TIMEOUT, b"").resumable);
    assert!(status_error(StatusCode::TOO_MANY_REQUESTS, b"").resumable);
    assert!(status_error(StatusCode::BAD_GATEWAY, b"").resumable);
}

#[test]
fn provider_response_accumulation_is_chunk_independent_and_bounded() {
    let mut body = Vec::new();
    append_provider_bytes(&mut body, b"one").unwrap();
    append_provider_bytes(&mut body, b"two").unwrap();
    assert_eq!(body, b"onetwo");

    let mut at_limit = vec![0; MAX_PROVIDER_BODY_BYTES - 1];
    append_provider_bytes(&mut at_limit, b"x").unwrap();
    assert_eq!(at_limit.len(), MAX_PROVIDER_BODY_BYTES);
    assert!(append_provider_bytes(&mut at_limit, b"x").is_err());
}

#[test]
fn parses_anthropic_tool_use_and_text() {
    let parsed = parse_anthropic_response(json!({"stop_reason":"tool_use","content":[
        {"type":"thinking","thinking":"opaque","signature":"sig"},
        {"type":"tool_use","id":"tu_1","name":"tool","input":{"x":true}},
        {"type":"text","text":"working"}
    ]}))
    .unwrap();
    assert_eq!(parsed.content.len(), 3);
    assert_eq!(parsed.calls[0].id, "tu_1");
    assert_eq!(parsed.text.as_deref(), Some("working"));
}

#[test]
fn anthropic_commits_all_tool_results_in_one_user_message() {
    let mut history = Vec::new();
    let results = anthropic_tool_results(vec![
        ToolResult {
            id: "one".into(),
            output: "first".into(),
            is_error: false,
        },
        ToolResult {
            id: "two".into(),
            output: "second".into(),
            is_error: true,
        },
    ]);
    commit_anthropic(
        &mut history,
        vec![json!({"type":"thinking", "signature":"opaque"})],
        results,
    );
    assert_eq!(history.len(), 2);
    assert_eq!(history[0]["role"], "assistant");
    assert_eq!(history[1]["role"], "user");
    assert_eq!(history[1]["content"].as_array().unwrap().len(), 2);
    assert_eq!(history[1]["content"][0]["tool_use_id"], "one");
    assert_eq!(history[1]["content"][1]["tool_use_id"], "two");
    assert_eq!(history[1]["content"][0]["is_error"], false);
    assert_eq!(history[1]["content"][1]["is_error"], true);
}

#[test]
fn anthropic_resume_merges_with_a_trailing_user_turn() {
    let mut history = vec![
        anthropic_user_message("initial"),
        json!({"role":"assistant","content":[{"type":"tool_use","id":"one"}]}),
        json!({"role":"user","content":[{"type":"tool_result","tool_use_id":"one","content":"done"}]}),
    ];
    append_anthropic_user_message(&mut history, "continue");
    assert_eq!(history.len(), 3);
    assert_eq!(history[2]["role"], "user");
    assert_eq!(history[2]["content"].as_array().unwrap().len(), 2);
    assert_eq!(history[2]["content"][1]["type"], "text");
    assert_eq!(history[2]["content"][1]["text"], "continue");
    assert!(
        history
            .windows(2)
            .all(|pair| pair[0]["role"] != pair[1]["role"])
    );
}

#[test]
fn state_and_error_messages_are_safe() {
    let mut state = ConversationState::new(&WireApi::Responses);
    if let ConversationState::Responses(history) = &mut state {
        history.push(responses_user_message("hello"));
    }
    assert!(matches!(state, ConversationState::Responses(ref history) if history.len() == 1));
    assert_eq!(
        endpoint(&Url::parse("https://example.test/v1").unwrap(), "responses")
            .unwrap()
            .as_str(),
        "https://example.test/v1/responses"
    );
    assert!(bounded_message(&("x\n".to_owned() + &"y".repeat(500))).len() <= ERROR_MESSAGE_LIMIT);
}

#[test]
fn activity_summaries_bound_untrusted_arguments() {
    let workspace = Path::new("/workspace");
    let (summary, _, target) = safe_tool_activity(
        "shell\nSECRET",
        &json!({"command":"curl https://secret.invalid --token=secret"}),
        workspace,
    );
    assert_eq!(summary, "Running shell command");
    assert!(target.is_none());
    let (summary, _, target) = safe_tool_activity(
        "read_text_file",
        &json!({"path":"src/main.rs", "token":"secret", "body":"x".repeat(10_000)}),
        workspace,
    );
    assert_eq!(summary, "Reading src/main.rs");
    assert_eq!(target.as_deref(), Some("src/main.rs"));
    let (summary, _, target) =
        safe_tool_activity("shell", &json!({"command":"cargo test -p app"}), workspace);
    assert_eq!(summary, "Running shell command");
    assert!(target.is_none());
    let (summary, _, _) = safe_tool_activity(
        "shell__execute_command",
        &json!({"command":["cargo", "clippy", "--token", "secret"]}),
        workspace,
    );
    assert_eq!(summary, "Running shell command");

    // Absolute in-workspace paths are summarized relative to the workspace
    // root. `/workspace` (POSIX) is not an absolute path on Windows, so the
    // workspace root is built with the platform's separators.
    let workspace_root = if cfg!(windows) {
        r"C:\workspace".to_string()
    } else {
        "/workspace".to_string()
    };
    let workspace = Path::new(&workspace_root);
    let inside = |relative: &str| {
        if cfg!(windows) {
            format!(r"C:\workspace\{}", relative.replace('/', "\\"))
        } else {
            format!("/workspace/{relative}")
        }
    };
    for (tool, expected) in [
        ("filesystem__read_text_file", "Reading src/main.rs"),
        ("filesystem.write_file", "Writing src/main.rs"),
        ("filesystem__search_files", "Searching src/main.rs"),
    ] {
        let (summary, _, target) =
            safe_tool_activity(tool, &json!({"path": inside("src/main.rs")}), workspace);
        assert_eq!(summary, expected);
        assert_eq!(target.as_deref(), Some("src/main.rs"));
    }
    let outside = if cfg!(windows) {
        r"C:\outside\secret"
    } else {
        "/outside/secret"
    };
    for path in [outside, "../secret", &format!("{workspace_root}/../secret")] {
        let (summary, _, target) =
            safe_tool_activity("read_text_file", &json!({"path":path}), workspace);
        assert_eq!(summary, "Calling read_text_file");
        assert!(target.is_none());
    }
    let (summary, _, _) = safe_tool_activity(
        "shell",
        &json!({"command":"cargo test -- Authorization: Bearer token API_KEY=secret"}),
        workspace,
    );
    assert_eq!(summary, "Running shell command");
    assert!(!summary.contains("token"));
    let (summary, _, target) = safe_tool_activity(
        "child/mcp",
        &json!({"arguments":"x".repeat(1_000_000), "token":"secret"}),
        workspace,
    );
    assert_eq!(summary, "Calling child/mcp");
    assert!(target.is_none());
    assert!(summary.len() < 120);
}

#[test]
fn anthropic_context_limit_stop_reason_is_preserved() {
    let parsed = parse_anthropic_response(json!({
        "content": [],
        "stop_reason": "model_context_window_exceeded"
    }))
    .unwrap();
    assert_eq!(
        parsed.stop_reason.as_deref(),
        Some("model_context_window_exceeded")
    );
}

#[test]
fn responses_reject_nonterminal_and_incomplete_statuses() {
    for status in ["incomplete", "failed", "cancelled", "queued", "in_progress"] {
        assert!(
            parse_responses_response(json!({"status": status, "output": []})).is_err(),
            "{status} was accepted"
        );
    }
    assert!(parse_responses_response(json!({"status": "completed", "output": []})).is_ok());
}

async fn read_http_request(stream: &mut TcpStream) -> (String, Vec<u8>) {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 4096];
    while !buffer.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = stream.read(&mut chunk).await.unwrap();
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
    let header_end = buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .unwrap()
        + 4;
    let headers = String::from_utf8_lossy(&buffer[..header_end]).into_owned();
    let content_length = headers
        .lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length:")
                .and_then(|value| value.trim().parse::<usize>().ok())
        })
        .unwrap_or(0);
    while buffer.len().saturating_sub(header_end) < content_length {
        let read = stream.read(&mut chunk).await.unwrap();
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
    let body = buffer[header_end..header_end + content_length].to_vec();
    (headers, body)
}

fn http_response(status: u16, body: &Value) -> String {
    let body = serde_json::to_string(body).unwrap();
    let reason = if status == 200 { "OK" } else { "Error" };
    format!(
        "HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    )
}

#[tokio::test]
async fn later_provider_failure_preserves_the_completed_round_and_replay_is_stateless() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let bodies = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let server_bodies = bodies.clone();
    let server = tokio::spawn(async move {
        for turn in 0..2 {
            let (mut stream, _) = listener.accept().await.unwrap();
            let (_, body) = read_http_request(&mut stream).await;
            server_bodies
                .lock()
                .unwrap()
                .push(String::from_utf8(body).unwrap());
            let response = if turn == 0 {
                http_response(
                    200,
                    &json!({
                        "status": "completed",
                        "output": [
                            {"type":"message","role":"assistant","content":[{"type":"output_text","text":"calling"}]},
                            {"type":"function_call","call_id":"call_1","name":"absent_tool","arguments":"{\"x\":1}"}
                        ]
                    }),
                )
            } else {
                "HTTP/1.1 500 Error\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".into()
            };
            stream.write_all(response.as_bytes()).await.unwrap();
        }
    });

    let mut definition = definition(WireApi::Responses);
    definition.base_url = Url::parse(&format!("http://127.0.0.1:{port}/v1")).unwrap();
    definition.max_turns = 2;
    let client = ProviderClient::new().unwrap();
    let credential = ProviderCredential("test-secret".into());
    let child = ChildMcpManager::empty();
    let cancel = CancellationToken::new();
    let reporter = ActivityReporter::new(|_event| Box::pin(async {}));
    let mut state = ConversationState::new(&WireApi::Responses);
    let outcome = client
        .run(
            ProviderRun {
                definition: &definition,
                credential: &credential,
                system_context: "system instructions",
                child: &child,
                cancel: &cancel,
                reporter: &reporter,
                workspace: Path::new("/workspace"),
            },
            "hello",
            &mut state,
        )
        .await;
    server.await.unwrap();
    let error = outcome.unwrap_err();
    assert_eq!(error.kind, "provider_error");
    assert!(error.resumable);
    let ConversationState::Responses(history) = &state else {
        panic!("unexpected wire API");
    };
    assert_eq!(history.len(), 4);
    assert_eq!(history[0]["role"], "user");
    assert_eq!(history[1]["type"], "message");
    assert_eq!(history[2]["type"], "function_call");
    assert_eq!(history[2]["call_id"], "call_1");
    assert_eq!(history[3]["type"], "function_call_output");
    assert_eq!(history[3]["call_id"], "call_1");
    assert!(history[3].get("is_error").is_none());
    let error: Value = serde_json::from_str(history[3]["output"].as_str().unwrap()).unwrap();
    assert_eq!(error["isError"], true);

    let captured = bodies.lock().unwrap().clone();
    assert_eq!(captured.len(), 2);
    for body in &captured {
        let request: Value = serde_json::from_str(body).unwrap();
        let input = request["input"].as_array().unwrap();
        let developer_items = input
            .iter()
            .filter(|item| item["role"] == "developer")
            .count();
        assert_eq!(developer_items, 1);
        assert_eq!(input[0]["role"], "developer");
        assert_eq!(input[0]["content"][0]["text"], "system instructions");
        assert!(request.get("instructions").is_none());
        assert!(request.get("previous_response_id").is_none());
        assert_eq!(request["store"], false);
    }
    let replay: Value = serde_json::from_str(&captured[1]).unwrap();
    let input = replay["input"].as_array().unwrap();
    assert_eq!(input.len(), 5);
    assert_eq!(input[2]["type"], "message");
    assert_eq!(input[3]["type"], "function_call");
    assert_eq!(input[4]["type"], "function_call_output");
}
