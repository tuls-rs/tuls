use super::*;

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
