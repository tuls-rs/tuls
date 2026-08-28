use super::*;

use rmcp::handler::server::wrapper::Parameters;

use crate::cli::NetworkPolicy;

/// The advertised JSON schema for `maxLength` must describe exactly the
/// range the runtime validates, so a client can never be promised a value
/// that the server would reject.
#[test]
fn fetch_max_length_schema_advertises_exactly_the_runtime_range() {
    let mut schema = schemars::schema_for!(FetchArgs);
    let root = schema.ensure_object();
    let properties = root
        .get("properties")
        .expect("properties")
        .as_object()
        .expect("properties object");
    let max_length = properties.get("maxLength").expect("maxLength property");
    let number = max_length
        .as_object()
        .expect("maxLength schema must be an object");
    assert_eq!(
        number.get("minimum").and_then(serde_json::Value::as_f64),
        Some(MIN_FETCH_MAX_LENGTH as f64),
        "schema minimum must match the runtime constant"
    );
    assert_eq!(
        number.get("maximum").and_then(serde_json::Value::as_f64),
        Some(MAX_FETCH_MAX_LENGTH as f64),
        "schema maximum must match the runtime constant"
    );
    assert!(number.get("exclusiveMinimum").is_none());
    assert!(number.get("exclusiveMaximum").is_none());
}

#[test]
fn fetch_result_render_is_bounded_under_the_tool_result_limit() {
    let body = "x".repeat(4 * 1024 * 1024);
    let rendered = render_fetch_result(
        "https://example.com/",
        "",
        &body,
        0,
        MAX_FETCH_MAX_LENGTH as usize,
    );
    assert!(
        rendered.len() <= MAX_TOOL_RESULT_BYTES,
        "rendered {} bytes exceeds the {} byte limit",
        rendered.len(),
        MAX_TOOL_RESULT_BYTES
    );
    assert!(rendered.contains("startIndex of 50000"), "{rendered}");
}

#[test]
fn fetch_result_render_respects_start_index() {
    let rendered = render_fetch_result("https://example.com/", "", "hello world", 6, 5);
    assert!(rendered.contains("world"), "{rendered}");
}

#[test]
fn fetch_result_render_reports_truncation_within_the_limit() {
    let body = "y".repeat(1_000_000);
    let rendered = render_fetch_result("https://example.com/", "", &body, 0, 5_000);
    assert!(rendered.len() <= MAX_TOOL_RESULT_BYTES);
    assert!(rendered.contains("startIndex of 5000"), "{rendered}");

    let rendered = render_fetch_result("https://example.com/", "", &body, 5_000, 5_000);
    assert!(rendered.len() <= MAX_TOOL_RESULT_BYTES);
    assert!(rendered.contains("startIndex of 10000"), "{rendered}");
}

#[test]
fn multibyte_pagination_never_skips_or_duplicates_content() {
    let content = "界😀".repeat(30_000);
    let mut start = 0usize;
    let mut reconstructed = String::new();
    loop {
        let page = content_page(
            &content,
            start,
            MAX_FETCH_MAX_LENGTH as usize,
            MAX_TOOL_RESULT_BYTES - 64,
        );
        assert!(
            !page.text.is_empty(),
            "pagination must always make progress"
        );
        reconstructed.push_str(&page.text);
        let Some(next) = page.next_start_index else {
            break;
        };
        assert_eq!(next, start + page.text.chars().count());
        start = next;
    }
    assert_eq!(reconstructed, content);
}

#[test]
fn multibyte_render_hint_matches_the_last_returned_character() {
    let content = "😀".repeat(30_000);
    let rendered = render_fetch_result(
        "https://example.com/",
        "",
        &content,
        0,
        MAX_FETCH_MAX_LENGTH as usize,
    );
    assert!(rendered.len() <= MAX_TOOL_RESULT_BYTES);
    let useful = rendered
        .strip_prefix("Contents of https://example.com/:\n")
        .unwrap()
        .split("\n\n<error>")
        .next()
        .unwrap();
    let returned = useful.chars().count();
    assert!(returned < MAX_FETCH_MAX_LENGTH as usize);
    assert!(rendered.contains(&format!("startIndex of {returned}")));
}

#[tokio::test]
async fn fetch_tool_rejects_out_of_range_max_length_at_call_time() {
    let client = FetchClient::new(None, NetworkPolicy::Unrestricted).unwrap();
    let policy = ToolPolicy::from_selectors(&[], &[], TOOL_SPECS).unwrap();
    let server = FetchServer::new(client, DEFAULT_USER_AGENT.to_string(), false, policy);

    for bad in [0, MAX_FETCH_MAX_LENGTH + 1, i64::MAX] {
        let result = server
            .fetch(Parameters(FetchArgs {
                url: "https://example.com/".into(),
                max_length: bad,
                start_index: 0,
                raw: false,
            }))
            .await
            .expect("handler returns a tool result");
        assert_eq!(result.is_error, Some(true), "maxLength {bad} rejected");
        let text = result
            .content
            .iter()
            .filter_map(|block| block.as_text())
            .map(|text| text.text.as_str())
            .collect::<String>();
        assert!(text.contains("maxLength"), "maxLength {bad}: {text}");
    }

    for bad in [-1, i64::MIN] {
        let result = server
            .fetch(Parameters(FetchArgs {
                url: "https://example.com/".into(),
                max_length: 100,
                start_index: bad,
                raw: false,
            }))
            .await
            .expect("handler returns a tool result");
        assert_eq!(result.is_error, Some(true), "startIndex {bad} rejected");
        let text = result
            .content
            .iter()
            .filter_map(|block| block.as_text())
            .map(|text| text.text.as_str())
            .collect::<String>();
        assert!(text.contains("startIndex"), "startIndex {bad}: {text}");
    }
}
