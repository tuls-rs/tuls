//! README conformance tests.
//!
//! Each test verifies a behavior documented in the README against the real
//! compiled `tuls` binary. These tests are deterministic and offline: the
//! LLM-driven parts use a loopback mock provider instead of a real model.

mod common;

use std::{
    fs,
    path::Path,
    sync::{Arc, Mutex},
};

use common::{
    MiniHttpServer, TulsServer, completed_responses, function_call_responses, read_file,
    spawn_and_exit, structured_of, text_of, tool_names,
};
use serde_json::{Value, json};
use tempfile::TempDir;

const READ_ONLY_TOOLS: &[&str] = &[
    "directory_tree",
    "get_file_info",
    "list_allowed_directories",
    "list_directory",
    "list_directory_with_sizes",
    "read_media_file",
    "read_multiple_files",
    "read_text_file",
    "search_files",
];
const ALL_FS_TOOLS: &[&str] = &[
    "create_directory",
    "directory_tree",
    "edit_file",
    "get_file_info",
    "list_allowed_directories",
    "list_directory",
    "list_directory_with_sizes",
    "move_file",
    "read_media_file",
    "read_multiple_files",
    "read_text_file",
    "search_files",
    "write_file",
];

// ---------------------------------------------------------------------------
// Capability policy (README: "Capability policy")
// ---------------------------------------------------------------------------

#[tokio::test]
async fn readme_policy_no_allow_exposes_all_tools() {
    let workspace = TempDir::new().expect("tempdir");
    let server =
        TulsServer::connect(&["filesystem", workspace.path().to_str().unwrap()], &[]).await;
    assert_eq!(tool_names(&server.tools().await), ALL_FS_TOOLS);
}

#[tokio::test]
async fn readme_policy_allowlist_read_only() {
    let workspace = TempDir::new().expect("tempdir");
    let server = TulsServer::connect(
        &[
            "filesystem",
            workspace.path().to_str().unwrap(),
            "--allow",
            "filesystem.read",
        ],
        &[],
    )
    .await;
    let names = tool_names(&server.tools().await);
    assert_eq!(names, READ_ONLY_TOOLS);

    // A denied tool is rejected at call time, not merely hidden.
    let result = server
        .call("write_file", json!({"path": "x.txt", "content": "x"}))
        .await;
    assert!(
        result.is_err(),
        "write_file must be rejected when only filesystem.read is allowed"
    );
}

#[tokio::test]
async fn readme_policy_deny_exact_tool_wins() {
    let workspace = TempDir::new().expect("tempdir");
    let server = TulsServer::connect(
        &[
            "filesystem",
            workspace.path().to_str().unwrap(),
            "--allow",
            "filesystem.read",
            "--allow",
            "filesystem.write",
            "--deny",
            "filesystem/move_file",
        ],
        &[],
    )
    .await;
    let names = tool_names(&server.tools().await);
    assert!(names.contains(&"write_file".to_string()));
    assert!(!names.contains(&"move_file".to_string()));
}

#[tokio::test]
async fn readme_policy_exact_tool_grant() {
    let workspace = TempDir::new().expect("tempdir");
    let server = TulsServer::connect(
        &[
            "filesystem",
            workspace.path().to_str().unwrap(),
            "--allow",
            "filesystem/read_text_file",
        ],
        &[],
    )
    .await;
    assert_eq!(
        tool_names(&server.tools().await),
        vec!["read_text_file".to_string()]
    );
}

#[test]
fn readme_policy_invalid_selectors_fail_startup() {
    let workspace = TempDir::new().expect("tempdir");
    let dir = workspace.path().to_str().unwrap();

    let unknown_capability = spawn_and_exit(&["filesystem", dir, "--allow", "filesystem.reed"]);
    assert!(
        !unknown_capability.status.success(),
        "unknown capability must fail startup: {}",
        String::from_utf8_lossy(&unknown_capability.stderr)
    );

    let wrong_server = spawn_and_exit(&["filesystem", dir, "--allow", "network.fetch"]);
    assert!(
        !wrong_server.status.success(),
        "capability from another server must fail startup: {}",
        String::from_utf8_lossy(&wrong_server.stderr)
    );

    let bad_tool_id = spawn_and_exit(&["filesystem", dir, "--allow", "filesystem/read-file"]);
    assert!(
        !bad_tool_id.status.success(),
        "misspelled tool id must fail startup: {}",
        String::from_utf8_lossy(&bad_tool_id.stderr)
    );
}

// ---------------------------------------------------------------------------
// Policy surface matrix (README: "Capability policy")
//
// The capability policy gates two surfaces independently: the tool routes and
// the auxiliary capability-scoped surface (the fetch prompt; the memory
// knowledge-graph resource).
//
// | allow selectors                    | tool  | prompt / resource |
// |------------------------------------|-------|-------------------|
// | none (default)                     | yes   | yes               |
// | none + deny <server>/<tool>        | no    | no                |
// | <server>/<tool> (exact grant)      | yes   | no                |
// | <capability> (capability grant)    | yes   | yes               |
// | <capability> + deny <server>/<tool>| no    | no                |
//
// The plain tool-level rows (default, capability allowlist, exact grants,
// deny precedence) are covered by the readme_policy_* tests above; the tests
// in this section pin the auxiliary surface for fetch and memory.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn readme_policy_matrix_fetch_default_exposes_tool_and_prompt() {
    let site = fetch_site();
    let server = TulsServer::connect(&["fetch", "--network", "unrestricted"], &[]).await;
    assert!(tool_names(&server.tools().await).contains(&"fetch".to_string()));
    let prompt = server
        .get_prompt("fetch", json!({"url": site.url("/")}))
        .await
        .expect("default policy must keep the fetch prompt");
    assert!(
        prompt.to_string().contains("Hello from local test server"),
        "default prompt fetch: {prompt}"
    );
}

#[tokio::test]
async fn readme_policy_matrix_fetch_default_with_exact_deny_exposes_nothing() {
    let site = fetch_site();
    let server = TulsServer::connect(
        &[
            "fetch",
            "--network",
            "unrestricted",
            "--deny",
            "fetch/fetch",
        ],
        &[],
    )
    .await;
    assert!(
        !tool_names(&server.tools().await).contains(&"fetch".to_string()),
        "an exact deny must remove the fetch tool"
    );
    assert!(
        server
            .get_prompt("fetch", json!({"url": site.url("/")}))
            .await
            .is_err(),
        "an exact deny of the fetch tool must also hide the fetch prompt"
    );
}

#[tokio::test]
async fn readme_policy_matrix_fetch_exact_allow_keeps_tool_drops_prompt() {
    let site = fetch_site();
    let server = TulsServer::connect(
        &[
            "fetch",
            "--network",
            "unrestricted",
            "--allow",
            "fetch/fetch",
        ],
        &[],
    )
    .await;
    assert!(tool_names(&server.tools().await).contains(&"fetch".to_string()));
    assert!(
        server
            .get_prompt("fetch", json!({"url": site.url("/")}))
            .await
            .is_err(),
        "an exact tool grant must not expose the fetch prompt"
    );
    let fetched = server
        .call_ok("fetch", json!({"url": site.url("/"), "maxLength": 2000}))
        .await;
    assert!(
        text_of(&fetched).contains("Hello from local test server"),
        "exact-grant fetch: {}",
        text_of(&fetched)
    );
}

#[tokio::test]
async fn readme_policy_matrix_fetch_capability_allow_keeps_tool_and_prompt() {
    let site = fetch_site();
    let server = TulsServer::connect(
        &[
            "fetch",
            "--network",
            "unrestricted",
            "--allow",
            "network.fetch",
        ],
        &[],
    )
    .await;
    assert!(tool_names(&server.tools().await).contains(&"fetch".to_string()));
    let prompt = server
        .get_prompt("fetch", json!({"url": site.url("/")}))
        .await
        .expect("capability grant must keep the fetch prompt");
    assert!(
        prompt.to_string().contains("Hello from local test server"),
        "capability prompt fetch: {prompt}"
    );
}

#[tokio::test]
async fn readme_policy_matrix_fetch_capability_with_exact_deny_exposes_nothing() {
    let site = fetch_site();
    let server = TulsServer::connect(
        &[
            "fetch",
            "--network",
            "unrestricted",
            "--allow",
            "network.fetch",
            "--deny",
            "fetch/fetch",
        ],
        &[],
    )
    .await;
    assert!(
        !tool_names(&server.tools().await).contains(&"fetch".to_string()),
        "an exact deny must beat the capability grant for the tool"
    );
    assert!(
        server
            .get_prompt("fetch", json!({"url": site.url("/")}))
            .await
            .is_err(),
        "an exact deny of the fetch tool must also hide the fetch prompt"
    );
}

#[tokio::test]
async fn readme_policy_matrix_memory_default_exposes_tool_and_resource() {
    let workspace = TempDir::new().expect("tempdir");
    let memory_file = workspace.path().join("memory.jsonl");
    let server = TulsServer::connect(
        &["memory", "--memory-file", memory_file.to_str().unwrap()],
        &[],
    )
    .await;
    assert!(tool_names(&server.tools().await).contains(&"read_graph".to_string()));
    let resources = server.list_resources().await.expect("list resources");
    assert!(
        resources
            .iter()
            .any(|resource| resource.get("uri").and_then(Value::as_str)
                == Some("memory://knowledge-graph")),
        "default policy must expose the knowledge-graph resource: {resources:?}"
    );
    server
        .read_resource("memory://knowledge-graph")
        .await
        .expect("default read_resource");
}

#[tokio::test]
async fn readme_policy_matrix_memory_default_with_exact_deny_exposes_nothing() {
    let workspace = TempDir::new().expect("tempdir");
    let memory_file = workspace.path().join("memory.jsonl");
    let server = TulsServer::connect(
        &[
            "memory",
            "--memory-file",
            memory_file.to_str().unwrap(),
            "--deny",
            "memory/read_graph",
        ],
        &[],
    )
    .await;
    assert!(
        !tool_names(&server.tools().await).contains(&"read_graph".to_string()),
        "an exact deny must remove the read_graph tool"
    );
    assert!(
        server
            .list_resources()
            .await
            .expect("list resources")
            .is_empty(),
        "an exact deny of read_graph must also hide the knowledge-graph resource"
    );
}

#[tokio::test]
async fn readme_policy_matrix_memory_exact_allow_keeps_tool_drops_resource() {
    let workspace = TempDir::new().expect("tempdir");
    let memory_file = workspace.path().join("memory.jsonl");
    let server = TulsServer::connect(
        &[
            "memory",
            "--memory-file",
            memory_file.to_str().unwrap(),
            "--allow",
            "memory/read_graph",
        ],
        &[],
    )
    .await;
    assert!(tool_names(&server.tools().await).contains(&"read_graph".to_string()));
    assert!(
        server
            .list_resources()
            .await
            .expect("list resources")
            .is_empty(),
        "an exact tool grant must not expose the knowledge-graph resource"
    );
    assert!(
        server
            .read_resource("memory://knowledge-graph")
            .await
            .is_err(),
        "reading a hidden resource must fail"
    );
}

#[tokio::test]
async fn readme_policy_matrix_memory_capability_allow_keeps_tool_and_resource() {
    let workspace = TempDir::new().expect("tempdir");
    let memory_file = workspace.path().join("memory.jsonl");
    let server = TulsServer::connect(
        &[
            "memory",
            "--memory-file",
            memory_file.to_str().unwrap(),
            "--allow",
            "memory.read",
        ],
        &[],
    )
    .await;
    assert!(tool_names(&server.tools().await).contains(&"read_graph".to_string()));
    let resources = server.list_resources().await.expect("list resources");
    assert!(
        resources
            .iter()
            .any(|resource| resource.get("uri").and_then(Value::as_str)
                == Some("memory://knowledge-graph")),
        "capability grant must expose the knowledge-graph resource: {resources:?}"
    );
    server
        .read_resource("memory://knowledge-graph")
        .await
        .expect("capability read_resource");
}

#[tokio::test]
async fn readme_policy_matrix_memory_capability_with_exact_deny_exposes_nothing() {
    let workspace = TempDir::new().expect("tempdir");
    let memory_file = workspace.path().join("memory.jsonl");
    let server = TulsServer::connect(
        &[
            "memory",
            "--memory-file",
            memory_file.to_str().unwrap(),
            "--allow",
            "memory.read",
            "--deny",
            "memory/read_graph",
        ],
        &[],
    )
    .await;
    assert!(
        !tool_names(&server.tools().await).contains(&"read_graph".to_string()),
        "an exact deny must beat the capability grant for read_graph"
    );
    assert!(
        server
            .list_resources()
            .await
            .expect("list resources")
            .is_empty(),
        "an exact deny of read_graph must also hide the knowledge-graph resource"
    );
}

// ---------------------------------------------------------------------------
// Fetch server (README: "Fetch server", "Default network posture")
// ---------------------------------------------------------------------------

/// Local website: robots.txt allows everything, `/` is a normal page, `/r`
/// redirects to `/`, and `/robots-redirect` answers 302 for robots.txt when
/// requested at the site root.
fn fetch_site() -> MiniHttpServer {
    MiniHttpServer::spawn(|request_line, _body| {
        let path = request_line.split_whitespace().nth(1).unwrap_or("/");
        match path {
            "/robots.txt" => (200, "User-agent: *\nAllow: /\n".to_string()),
            "/" => (
                200,
                "<html><body><h1>Hello from local test server</h1></body></html>".to_string(),
            ),
            "/r" => (302, String::new()),
            _ => (404, "not found".to_string()),
        }
    })
}

#[tokio::test]
async fn readme_fetch_public_blocks_loopback_and_unrestricted_allows() {
    let site = fetch_site();

    let public = TulsServer::connect(&["fetch", "--network", "public"], &[]).await;
    let blocked = public
        .call("fetch", json!({"url": site.url("/"), "maxLength": 2000}))
        .await
        .expect("fetch transport ok");
    assert_eq!(
        blocked.get("isError").and_then(Value::as_bool),
        Some(true),
        "public policy must block loopback: {}",
        text_of(&blocked)
    );
    assert!(
        text_of(&blocked).contains("network policy"),
        "block message: {}",
        text_of(&blocked)
    );

    let unrestricted = TulsServer::connect(&["fetch", "--network", "unrestricted"], &[]).await;
    let fetched = unrestricted
        .call_ok("fetch", json!({"url": site.url("/"), "maxLength": 2000}))
        .await;
    assert!(
        text_of(&fetched).contains("Hello from local test server"),
        "unrestricted fetch: {}",
        text_of(&fetched)
    );
}

#[tokio::test]
async fn readme_fetch_redirects_are_not_followed() {
    let site = fetch_site();
    let server = TulsServer::connect(
        &["fetch", "--robots", "ignore", "--network", "unrestricted"],
        &[],
    )
    .await;
    let result = server
        .call("fetch", json!({"url": site.url("/r"), "maxLength": 2000}))
        .await
        .expect("fetch transport ok");
    assert_eq!(
        result.get("isError").and_then(Value::as_bool),
        Some(true),
        "redirect must not be followed: {}",
        text_of(&result)
    );
    assert!(
        text_of(&result).contains("302"),
        "redirect error message: {}",
        text_of(&result)
    );
}

#[tokio::test]
async fn readme_fetch_max_length_matches_the_documented_limit() {
    let site = fetch_site();
    let server = TulsServer::connect(
        &["fetch", "--robots", "ignore", "--network", "unrestricted"],
        &[],
    )
    .await;
    let result = server
        .call("fetch", json!({"url": site.url("/"), "maxLength": 50001}))
        .await
        .expect("fetch transport ok");
    assert_eq!(result.get("isError").and_then(Value::as_bool), Some(true));
    assert!(text_of(&result).contains("50000"));
}

#[tokio::test]
async fn readme_fetch_robots_redirect_fails_closed() {
    let site = MiniHttpServer::spawn(|request_line, _body| {
        let path = request_line.split_whitespace().nth(1).unwrap_or("/");
        if path.ends_with("/robots.txt") {
            (302, String::new())
        } else {
            (200, "<html><body>ok</body></html>".to_string())
        }
    });
    let server = TulsServer::connect(
        &["fetch", "--robots", "respect", "--network", "unrestricted"],
        &[],
    )
    .await;
    let result = server
        .call("fetch", json!({"url": site.url("/"), "maxLength": 2000}))
        .await
        .expect("fetch transport ok");
    assert_eq!(
        result.get("isError").and_then(Value::as_bool),
        Some(true),
        "redirected robots.txt must fail closed: {}",
        text_of(&result)
    );
    assert!(
        text_of(&result).contains("redirected"),
        "robots failure message: {}",
        text_of(&result)
    );
}

#[test]
fn readme_fetch_proxy_requires_unrestricted_mode() {
    // Public + proxy: the server must refuse to start.
    let refused = spawn_and_exit(&[
        "fetch",
        "--network",
        "public",
        "--proxy-url",
        "http://127.0.0.1:1",
    ]);
    assert!(
        !refused.status.success(),
        "proxy with public policy must fail startup: {}",
        String::from_utf8_lossy(&refused.stderr)
    );

    // Unrestricted + proxy: the server starts and the tool is available.
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    let server = runtime.block_on(async {
        TulsServer::connect(
            &[
                "fetch",
                "--network",
                "unrestricted",
                "--proxy-url",
                "http://127.0.0.1:1",
            ],
            &[],
        )
        .await
    });
    let names = runtime.block_on(async { tool_names(&server.tools().await) });
    assert!(names.contains(&"fetch".to_string()));
}

// ---------------------------------------------------------------------------
// Memory server (README: "Memory server")
// ---------------------------------------------------------------------------

#[tokio::test]
async fn readme_memory_resource_exposed_as_documented_uri() {
    let workspace = TempDir::new().expect("tempdir");
    let memory_file = workspace.path().join("memory.jsonl");
    let server = TulsServer::connect(
        &["memory", "--memory-file", memory_file.to_str().unwrap()],
        &[],
    )
    .await;
    server
        .call_ok(
            "create_entities",
            json!({"entities": [{"name": "acme", "entityType": "company", "observations": ["founded 2024"]}]}),
        )
        .await;

    let resources = server.list_resources().await.expect("list resources");
    let uris = resources
        .iter()
        .filter_map(|resource| resource.get("uri").and_then(Value::as_str))
        .collect::<Vec<_>>();
    assert!(
        uris.contains(&"memory://knowledge-graph"),
        "resources: {uris:?}"
    );

    let graph = server
        .read_resource("memory://knowledge-graph")
        .await
        .expect("read memory resource");
    assert!(
        graph.to_string().contains("acme"),
        "memory resource contents: {graph}"
    );
}

#[tokio::test]
async fn readme_memory_jsonl_persistence() {
    let workspace = TempDir::new().expect("tempdir");
    let memory_file = workspace.path().join("memory.jsonl");
    {
        let server = TulsServer::connect(
            &["memory", "--memory-file", memory_file.to_str().unwrap()],
            &[],
        )
        .await;
        server
            .call_ok(
                "create_entities",
                json!({"entities": [{"name": "alice", "entityType": "person", "observations": []}]}),
            )
            .await;
    }
    assert!(memory_file.is_file(), "memory file must persist to disk");
    let persisted = read_file(&memory_file);
    assert!(
        persisted.contains("\"name\":\"alice\"") || persisted.contains("\"name\": \"alice\""),
        "persisted graph: {persisted}"
    );
}

// ---------------------------------------------------------------------------
// Filesystem integration boundaries (README: "Filesystem server")
//
// edit_file semantics are unit-tested in src/fs/edit, but dry_run only exists
// at the tool surface, so every behavior is pinned here against the real
// binary: unique match, no match, ambiguous match, sequential edits, CRLF
// preservation, and dry-run preview without writing.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn readme_edit_file_unique_match_applies_and_renders_diff() {
    let workspace = TempDir::new().expect("tempdir");
    let file = workspace.path().join("doc.txt");
    fs::write(&file, "line one\nline two\nline three\n").expect("write");
    let server =
        TulsServer::connect(&["filesystem", workspace.path().to_str().unwrap()], &[]).await;

    let result = server
        .call_ok(
            "edit_file",
            json!({"path": "doc.txt", "edits": [{"oldText": "line two", "newText": "changed"}]}),
        )
        .await;
    let diff = text_of(&result);
    assert!(diff.contains("```diff"), "diff fence: {diff}");
    assert!(
        diff.contains("+changed"),
        "diff must show the replacement: {diff}"
    );
    assert_eq!(read_file(&file), "line one\nchanged\nline three\n");
}

#[tokio::test]
async fn readme_edit_file_no_match_errors_and_leaves_file_unchanged() {
    let workspace = TempDir::new().expect("tempdir");
    let file = workspace.path().join("doc.txt");
    fs::write(&file, "hello world\n").expect("write");
    let server =
        TulsServer::connect(&["filesystem", workspace.path().to_str().unwrap()], &[]).await;

    let result = server
        .call(
            "edit_file",
            json!({"path": "doc.txt", "edits": [{"oldText": "no such text", "newText": "x"}]}),
        )
        .await
        .expect("transport ok");
    assert_eq!(
        result.get("isError").and_then(Value::as_bool),
        Some(true),
        "a missing oldText must be an error: {}",
        text_of(&result)
    );
    assert!(
        text_of(&result).contains("oldText not found"),
        "error message: {}",
        text_of(&result)
    );
    assert_eq!(
        read_file(&file),
        "hello world\n",
        "failed edit must not write"
    );
}

#[tokio::test]
async fn readme_edit_file_ambiguous_match_errors_and_leaves_file_unchanged() {
    let workspace = TempDir::new().expect("tempdir");
    let file = workspace.path().join("doc.txt");
    fs::write(&file, "hello world hello\n").expect("write");
    let server =
        TulsServer::connect(&["filesystem", workspace.path().to_str().unwrap()], &[]).await;

    let result = server
        .call(
            "edit_file",
            json!({"path": "doc.txt", "edits": [{"oldText": "hello", "newText": "hi"}]}),
        )
        .await
        .expect("transport ok");
    assert_eq!(
        result.get("isError").and_then(Value::as_bool),
        Some(true),
        "an ambiguous oldText must be an error: {}",
        text_of(&result)
    );
    assert!(
        text_of(&result).contains("ambiguous match"),
        "error message: {}",
        text_of(&result)
    );
    assert_eq!(
        read_file(&file),
        "hello world hello\n",
        "ambiguous edit must not write"
    );
}

#[tokio::test]
async fn readme_edit_file_sequential_edits_apply_in_order() {
    let workspace = TempDir::new().expect("tempdir");
    let file = workspace.path().join("doc.txt");
    fs::write(&file, "a\nb\nc\n").expect("write");
    let server =
        TulsServer::connect(&["filesystem", workspace.path().to_str().unwrap()], &[]).await;

    server
        .call_ok(
            "edit_file",
            json!({"path": "doc.txt", "edits": [{"oldText": "a", "newText": "x"}, {"oldText": "c", "newText": "z"}]}),
        )
        .await;
    assert_eq!(read_file(&file), "x\nb\nz\n");
}

#[tokio::test]
async fn readme_edit_file_crlf_line_endings_are_preserved() {
    let workspace = TempDir::new().expect("tempdir");
    let file = workspace.path().join("doc.txt");
    fs::write(&file, "a\r\nb\r\n").expect("write");
    let server =
        TulsServer::connect(&["filesystem", workspace.path().to_str().unwrap()], &[]).await;

    server
        .call_ok(
            "edit_file",
            json!({"path": "doc.txt", "edits": [{"oldText": "b", "newText": "c"}]}),
        )
        .await;
    assert_eq!(
        read_file(&file),
        "a\r\nc\r\n",
        "CRLF endings must survive the edit"
    );
}

#[tokio::test]
async fn readme_edit_file_dry_run_previews_without_writing() {
    let workspace = TempDir::new().expect("tempdir");
    let file = workspace.path().join("doc.txt");
    fs::write(&file, "hello world\n").expect("write");
    let server =
        TulsServer::connect(&["filesystem", workspace.path().to_str().unwrap()], &[]).await;

    let preview = server
        .call_ok(
            "edit_file",
            json!({"path": "doc.txt", "edits": [{"oldText": "hello world", "newText": "goodbye world"}], "dryRun": true}),
        )
        .await;
    assert!(
        text_of(&preview).contains("+goodbye world"),
        "dry run must render the diff: {}",
        text_of(&preview)
    );
    assert_eq!(
        read_file(&file),
        "hello world\n",
        "dry run must not modify the file"
    );

    server
        .call_ok(
            "edit_file",
            json!({"path": "doc.txt", "edits": [{"oldText": "hello world", "newText": "goodbye world"}]}),
        )
        .await;
    assert_eq!(
        read_file(&file),
        "goodbye world\n",
        "real run must apply the edit"
    );
}

#[tokio::test]
async fn readme_edit_file_rejects_excessive_batches_before_reading() {
    let workspace = TempDir::new().expect("tempdir");
    let server =
        TulsServer::connect(&["filesystem", workspace.path().to_str().unwrap()], &[]).await;
    let edits = (0..1025)
        .map(|index| json!({"oldText": format!("old-{index}"), "newText": "new"}))
        .collect::<Vec<_>>();
    let result = server
        .call("edit_file", json!({"path": "missing.txt", "edits": edits}))
        .await
        .expect("edit transport ok");
    assert_eq!(result.get("isError").and_then(Value::as_bool), Some(true));
    assert!(text_of(&result).contains("1024"));
}

#[tokio::test]
async fn readme_directory_tree_truncation_remains_valid_json() {
    let workspace = TempDir::new().expect("tempdir");
    for index in 0..1030 {
        fs::write(workspace.path().join(format!("f{index:04}.txt")), "x").expect("write entry");
    }
    let server =
        TulsServer::connect(&["filesystem", workspace.path().to_str().unwrap()], &[]).await;

    let result = text_of(&server.call_ok("directory_tree", json!({"path": "."})).await);
    let parsed: Value = serde_json::from_str(&result).unwrap_or_else(|error| {
        panic!("truncated tree must remain valid JSON ({error}): {result}")
    });
    let entries = parsed
        .as_array()
        .expect("tree must serialize as a JSON array");
    assert!(
        entries.len() >= 1024,
        "expected the tree to hit its entry budget"
    );
    let marker = entries.last().expect("truncation marker entry");
    assert_eq!(
        marker.get("type").and_then(Value::as_str),
        Some("truncated"),
        "marker entry: {marker}"
    );
    assert!(
        marker
            .get("name")
            .and_then(Value::as_str)
            .is_some_and(|name| name.contains("truncated at 1024 entries")),
        "marker name: {marker}"
    );
}

// ---------------------------------------------------------------------------
// Shell server (README: "Shell server")
// ---------------------------------------------------------------------------

#[cfg(unix)]
#[tokio::test]
async fn readme_shell_no_shell_parsing_and_minimal_env() {
    let workspace = TempDir::new().expect("tempdir");
    let server = TulsServer::connect(&["shell", workspace.path().to_str().unwrap()], &[]).await;

    // program is an executable, not a shell command string.
    let result = server
        .call(
            "execute_command",
            json!({"program": "echo hello && pwd", "timeoutMs": 10000}),
        )
        .await
        .expect("transport ok");
    assert_eq!(
        result.get("isError").and_then(Value::as_bool),
        Some(true),
        "shell command string must not be parsed: {}",
        text_of(&result)
    );

    // The documented explicit-shell pattern works.
    let combined = structured_of(
        &server
            .call_ok(
                "execute_command",
                json!({"program": "bash", "args": ["-lc", "echo a && echo b"], "timeoutMs": 10000}),
            )
            .await,
    );
    let stdout = combined.get("stdout").and_then(Value::as_str).unwrap_or("");
    assert!(
        stdout.contains('a') && stdout.contains('b'),
        "bash -lc: {stdout}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn readme_shell_minimal_environment_drops_unrelated_vars() {
    let workspace = TempDir::new().expect("tempdir");
    let marker = "TULS_SHOULD_NOT_LEAK=secret-value-12345";
    let server = TulsServer::connect(
        &["shell", workspace.path().to_str().unwrap()],
        &[("TULS_SHOULD_NOT_LEAK", "secret-value-12345".to_string())],
    )
    .await;
    let env_output = structured_of(
        &server
            .call_ok(
                "execute_command",
                json!({"program": "env", "timeoutMs": 10000}),
            )
            .await,
    );
    let stdout = env_output
        .get("stdout")
        .and_then(Value::as_str)
        .unwrap_or("");
    assert!(
        !stdout.contains(marker),
        "unrelated parent environment leaked into the child: {stdout}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn readme_shell_child_stdin_is_disconnected_from_mcp_transport() {
    let workspace = TempDir::new().expect("tempdir");
    let server = TulsServer::connect(&["shell", workspace.path().to_str().unwrap()], &[]).await;
    let output = structured_of(
        &server
            .call_ok(
                "execute_command",
                json!({"program": "cat", "timeoutMs": 1000}),
            )
            .await,
    );
    assert_eq!(output.get("timedOut").and_then(Value::as_bool), Some(false));
    assert_eq!(output.get("stdout").and_then(Value::as_str), Some(""));
}

// ---------------------------------------------------------------------------
// Skills server (README: "Skills server")
// ---------------------------------------------------------------------------

#[tokio::test]
async fn readme_skills_discovery_roots() {
    let workspace = TempDir::new().expect("tempdir");
    let root = workspace.path();
    fs::create_dir_all(root.join(".agents/skills/rust-review")).expect("skill dir");
    fs::write(
        root.join(".agents/skills/rust-review/SKILL.md"),
        "---\nname: rust-review\ndescription: Reviews Rust code for correctness.\n---\nReview the code and report issues.\n",
    )
    .expect("SKILL.md");
    fs::write(
        root.join(".agents/skills/rust-review/checklist.md"),
        "## Checklist\n- safety\n",
    )
    .expect("checklist");

    let server = TulsServer::connect(&["skills", root.to_str().unwrap()], &[]).await;
    let activated = structured_of(
        &server
            .call_ok("activate_skill", json!({"name": "rust-review"}))
            .await,
    );
    let text = activated.to_string();
    assert!(text.contains("rust-review"), "activation: {text}");
    assert!(activated.get("skillDir").is_some(), "activation: {text}");
    assert!(activated.get("skill_dir").is_none(), "activation: {text}");
    assert!(
        text.contains("checklist.md"),
        "resource manifest should include supporting files: {text}"
    );
}

// ---------------------------------------------------------------------------
// Agents server (README: "Subagent configuration", "Child MCP servers")
// ---------------------------------------------------------------------------

fn write_agent(workspace: &Path, name: &str, body: &str) {
    let dir = workspace.join(".agents/agents");
    fs::create_dir_all(&dir).expect("agents dir");
    fs::write(dir.join(format!("{name}.toml")), body).expect("write agent");
}

/// A provider-independent agent definition that points at the loopback mock
/// provider, so agent runs work offline.
fn mock_provider_agent(name: &str, instructions: &str, extras: &str, port: u16) -> String {
    format!(
        "name = \"{name}\"\n\
         description = \"Conformance agent\"\n\
         instructions = \"{instructions}\"\n\
         model_provider = \"custom\"\n\
         model = \"mock/model\"\n\
         base_url = \"http://127.0.0.1:{port}\"\n\
         env_key = \"MOCK_API_KEY\"\n\
         wire_api = \"responses\"\n\
         max_turns = 5\n\
         {extras}"
    )
}

/// The minimal OpenAI agent from the README, with the binary path made
/// absolute so it resolves when the agents server spawns the child.
fn readme_reviewer_toml() -> String {
    format!(
        "name = \"reviewer\"\n\
         description = \"Reviews workspace code without modifying files\"\n\
         instructions = \"Review the requested code and report concrete correctness, security, and maintainability issues.\"\n\
         \n\
         model_provider = \"openai\"\n\
         model = \"YOUR_OPENAI_MODEL\"\n\
         \n\
         allow_tools = [\"filesystem/*\"]\n\
         \n\
         [mcp_servers.filesystem]\n\
         type = \"stdio\"\n\
         command = \"{}\"\n\
         args = [\"filesystem\", \".\", \"--allow\", \"filesystem.read\"]",
        common::toml_tuls_bin()
    )
}

#[tokio::test]
async fn readme_agents_minimal_example_is_discovered() {
    let workspace = TempDir::new().expect("tempdir");
    write_agent(workspace.path(), "reviewer", &readme_reviewer_toml());

    let server = TulsServer::connect(&["agents", workspace.path().to_str().unwrap()], &[]).await;
    let names = tool_names(&server.tools().await);
    assert_eq!(names, vec!["spawn_agent", "send_input", "wait_agent"]);

    let spawn = server
        .tools()
        .await
        .into_iter()
        .find(|tool| tool.name == "spawn_agent")
        .expect("spawn_agent tool");
    let catalog = spawn.description.as_deref().unwrap_or("");
    assert!(
        catalog.contains("reviewer"),
        "spawn_agent catalog must list the README example agent"
    );
}

#[tokio::test]
async fn readme_openrouter_example_parses_without_endpoint_overrides() {
    let workspace = TempDir::new().expect("tempdir");
    write_agent(
        workspace.path(),
        "openrouter-researcher",
        &format!(
            "name = \"openrouter-researcher\"\n\
             description = \"Researches public web sources through OpenRouter\"\n\
             instructions = \"Research the requested topic.\"\n\
             model_provider = \"openrouter\"\n\
             model = \"openai/gpt-5.6-luna\"\n\
             reasoning_effort = \"high\"\n\
             allow_tools = [\"fetch/*\"]\n\
             [mcp_servers.fetch]\n\
             type = \"stdio\"\n\
             command = \"{}\"\n\
             args = [\"fetch\", \"--allow\", \"network.fetch\"]",
            common::toml_tuls_bin()
        ),
    );
    let server = TulsServer::connect(&["agents", workspace.path().to_str().unwrap()], &[]).await;
    let spawn = server
        .tools()
        .await
        .into_iter()
        .find(|tool| tool.name == "spawn_agent")
        .expect("spawn_agent tool");
    assert!(
        spawn
            .description
            .as_deref()
            .is_some_and(|description| description.contains("openrouter-researcher"))
    );
}

#[tokio::test]
async fn readme_agent_message_and_wait_limits_are_enforced_before_provider_work() {
    let workspace = TempDir::new().expect("tempdir");
    write_agent(workspace.path(), "reviewer", &readme_reviewer_toml());
    let server = TulsServer::connect(&["agents", workspace.path().to_str().unwrap()], &[]).await;

    let spawn = server
        .call(
            "spawn_agent",
            json!({"name": "reviewer", "task": "x".repeat(256 * 1024 + 1)}),
        )
        .await
        .expect("spawn transport ok");
    assert_eq!(spawn.get("isError").and_then(Value::as_bool), Some(true));
    assert!(text_of(&spawn).contains("262144"));

    let send = server
        .call(
            "send_input",
            json!({"target": "missing", "message": "x".repeat(256 * 1024 + 1)}),
        )
        .await
        .expect("send transport ok");
    assert_eq!(send.get("isError").and_then(Value::as_bool), Some(true));
    assert!(text_of(&send).contains("262144"));

    let targets = (0..65)
        .map(|index| format!("agent-{index}"))
        .collect::<Vec<_>>();
    let wait = server
        .call("wait_agent", json!({"targets": targets}))
        .await
        .expect("wait transport ok");
    assert_eq!(wait.get("isError").and_then(Value::as_bool), Some(true));
    assert!(text_of(&wait).contains("64"));
}

#[tokio::test]
async fn readme_agents_default_deny_gives_no_child_tools() {
    let workspace = TempDir::new().expect("tempdir");
    let root = workspace.path();
    let provider =
        MiniHttpServer::spawn(move |_request_line, _body| (200, completed_responses("OK")));
    write_agent(
        root,
        "no-tools",
        &mock_provider_agent(
            "no-tools",
            "Reply with the single word OK.",
            "[mcp_servers.filesystem]\n\
             type = \"stdio\"\n\
             command = \"/definitely/missing/tuls\"\n\
             args = [\"filesystem\", \".\"]",
            provider.addr.port(),
        ),
    );

    let server = TulsServer::connect(
        &["agents", root.to_str().unwrap()],
        &[("MOCK_API_KEY", "mock-secret".into())],
    )
    .await;
    let spawned = structured_of(
        &server
            .call_ok(
                "spawn_agent",
                json!({"name": "no-tools", "task": "Reply OK"}),
            )
            .await,
    );
    let id = spawned
        .get("agentId")
        .and_then(Value::as_str)
        .expect("agentId")
        .to_string();

    // With no allow_tools the child server is not even started; the agent
    // runs with zero tools and completes through the mock provider.
    let wait = server
        .call_ok("wait_agent", json!({"targets": [id], "timeoutMs": 30000}))
        .await;
    let results = wait
        .get("structuredContent")
        .and_then(|value| value.get("agents"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert_eq!(results.len(), 1, "wait: {wait}");
    assert_eq!(
        results[0].get("status").and_then(Value::as_str),
        Some("completed"),
        "default-deny agent must complete without child tools: {wait}"
    );
}

#[tokio::test]
async fn readme_agents_unknown_server_selector_fails() {
    let workspace = TempDir::new().expect("tempdir");
    let root = workspace.path();
    write_agent(
        root,
        "valid",
        &mock_provider_agent("valid", "Reply OK.", "", 1),
    );
    write_agent(
        root,
        "broken",
        &mock_provider_agent(
            "broken",
            "Reply OK.",
            "allow_tools = [\"nope/*\"]\n\
             \n\
             [mcp_servers.filesystem]\n\
             type = \"stdio\"\n\
             command = \"/definitely/missing/tuls\"",
            1,
        ),
    );

    let server = TulsServer::connect(&["agents", root.to_str().unwrap()], &[]).await;
    let spawn = server
        .tools()
        .await
        .into_iter()
        .find(|tool| tool.name == "spawn_agent")
        .expect("spawn_agent");
    let catalog = spawn.description.as_deref().unwrap_or("");
    assert!(
        !catalog.contains("broken"),
        "a definition with an unknown server selector must be rejected: {catalog}"
    );
    let spawned = server
        .call("spawn_agent", json!({"name": "broken", "task": "Reply OK"}))
        .await
        .expect("transport ok");
    assert_eq!(
        spawned.get("isError").and_then(Value::as_bool),
        Some(true),
        "spawning a rejected definition must fail: {}",
        text_of(&spawned)
    );
}

#[tokio::test]
async fn readme_agents_unavailable_child_tool_fails_at_connect() {
    let workspace = TempDir::new().expect("tempdir");
    let root = workspace.path();
    let provider =
        MiniHttpServer::spawn(move |_request_line, _body| (200, completed_responses("OK")));
    write_agent(
        root,
        "stale",
        &mock_provider_agent(
            "stale",
            "Reply OK.",
            &format!(
                "allow_tools = [\"filesystem/nonexistent_tool\"]\n\
                 \n\
                 [mcp_servers.filesystem]\n\
                 type = \"stdio\"\n\
                 command = \"{}\"\n\
                 args = [\"filesystem\", \".\", \"--allow\", \"filesystem.read\"]",
                common::toml_tuls_bin()
            ),
            provider.addr.port(),
        ),
    );

    let server = TulsServer::connect(
        &["agents", root.to_str().unwrap()],
        &[("MOCK_API_KEY", "mock-secret".into())],
    )
    .await;
    let spawned = structured_of(
        &server
            .call_ok("spawn_agent", json!({"name": "stale", "task": "Reply OK"}))
            .await,
    );
    let id = spawned
        .get("agentId")
        .and_then(Value::as_str)
        .expect("agentId")
        .to_string();

    // The child connects fine, but the catalog check rejects the selector,
    // so the run fails with child_mcp_startup_error instead of calling the
    // provider.
    let wait = server
        .call_ok("wait_agent", json!({"targets": [id], "timeoutMs": 30000}))
        .await;
    let results = wait
        .get("structuredContent")
        .and_then(|value| value.get("agents"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert_eq!(results.len(), 1, "wait: {wait}");
    let error_kind = results[0]
        .get("error")
        .and_then(|error| error.get("kind"))
        .and_then(Value::as_str);
    assert_eq!(
        error_kind,
        Some("child_mcp_startup_error"),
        "unavailable child tool selector must fail the run: {wait}"
    );
}

// ---------------------------------------------------------------------------
// Deterministic agent runs against a scripted loopback provider
// ---------------------------------------------------------------------------

enum MockResponse {
    Completed(&'static str),
    FunctionCall {
        call_id: &'static str,
        name: &'static str,
        arguments: &'static str,
    },
    HttpStatus(u16),
}

#[derive(Default)]
struct CapturedProvider {
    served: usize,
    bodies: Vec<Value>,
}

/// Serve one canned response per request, capturing every request body as
/// JSON. Requests beyond the scripted list receive HTTP 500.
fn scripted_provider(
    responses: Vec<MockResponse>,
) -> (MiniHttpServer, Arc<Mutex<CapturedProvider>>) {
    let captured = Arc::new(Mutex::new(CapturedProvider::default()));
    let inner = captured.clone();
    let server = MiniHttpServer::spawn(move |_request_line, request_body| {
        let mut captured = inner.lock().expect("provider capture lock");
        let index = captured.served;
        captured.served += 1;
        if let Ok(body) = serde_json::from_str::<Value>(request_body) {
            captured.bodies.push(body);
        }
        match responses
            .get(index)
            .unwrap_or(&MockResponse::HttpStatus(500))
        {
            MockResponse::Completed(text) => (200, completed_responses(text)),
            MockResponse::FunctionCall {
                call_id,
                name,
                arguments,
            } => (200, function_call_responses(call_id, name, arguments)),
            MockResponse::HttpStatus(status) => (
                *status,
                r#"{"error":{"message":"scripted provider failure","type":"server_error"}}"#
                    .to_string(),
            ),
        }
    });
    (server, captured)
}

fn agent_results(wait: &Value) -> Vec<Value> {
    wait.get("structuredContent")
        .and_then(|value| value.get("agents"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// A provider failure (HTTP 500) after a completed tool round must fail the
/// run resumably; an explicit send_input resumes the same session, the
/// completed round is replayed to the provider, and the child tool is never
/// executed a second time.
#[tokio::test]
async fn readme_agents_resume_retains_completed_round_without_repeating_tools() {
    let workspace = TempDir::new().expect("tempdir");
    let root = workspace.path();
    let counter = root.join("counter.txt");
    fs::write(&counter, "seed\n").expect("counter file");

    let (provider, captured) = scripted_provider(vec![
        MockResponse::FunctionCall {
            call_id: "call_append_once",
            name: "shell__execute_command",
            arguments: r#"{"program":"sh","args":["-c","echo appended >> counter.txt"],"timeoutMs":10000}"#,
        },
        MockResponse::HttpStatus(500),
        MockResponse::Completed("resumed after the provider failure"),
    ]);
    write_agent(
        root,
        "counter",
        &mock_provider_agent(
            "counter",
            "Append the word appended to counter.txt, then stop.",
            &format!(
                "allow_tools = [\"shell/execute_command\"]\n\
                 \n\
                 [mcp_servers.shell]\n\
                 type = \"stdio\"\n\
                 command = \"{}\"\n\
                 args = [\"shell\", \".\"]",
                common::toml_tuls_bin()
            ),
            provider.addr.port(),
        ),
    );

    let server = TulsServer::connect(
        &["agents", root.to_str().unwrap()],
        &[("MOCK_API_KEY", "mock-secret".into())],
    )
    .await;
    let spawned = structured_of(
        &server
            .call_ok(
                "spawn_agent",
                json!({"name": "counter", "task": "Append one line to counter.txt"}),
            )
            .await,
    );
    let id = spawned
        .get("agentId")
        .and_then(Value::as_str)
        .expect("agentId")
        .to_string();

    let failed = server
        .call_ok("wait_agent", json!({"targets": [id], "timeoutMs": 30000}))
        .await;
    let results = agent_results(&failed);
    assert_eq!(results.len(), 1, "failed wait: {failed}");
    assert_eq!(
        results[0].get("status").and_then(Value::as_str),
        Some("failed"),
        "the 500 must fail the run: {failed}"
    );
    assert_eq!(
        results[0]
            .get("error")
            .and_then(|error| error.get("kind"))
            .and_then(Value::as_str),
        Some("provider_error"),
        "a provider 500 is resumable: {failed}"
    );

    let ack = structured_of(
        &server
            .call_ok(
                "send_input",
                json!({"target": id, "message": "Continue and finish."}),
            )
            .await,
    );
    assert_eq!(
        ack.get("accepted").and_then(Value::as_bool),
        Some(true),
        "resume must be accepted: {ack}"
    );

    let wait = server
        .call_ok("wait_agent", json!({"targets": [id], "timeoutMs": 30000}))
        .await;
    let results = agent_results(&wait);
    assert_eq!(results.len(), 1, "resumed wait: {wait}");
    assert_eq!(
        results[0].get("status").and_then(Value::as_str),
        Some("completed"),
        "resumed run must complete: {wait}"
    );
    assert!(
        results[0]
            .get("result")
            .and_then(Value::as_str)
            .is_some_and(|result| !result.trim().is_empty()),
        "resumed run must produce a result: {wait}"
    );

    let text = read_file(&counter);
    assert_eq!(
        text.matches("appended").count(),
        1,
        "the completed round must not re-execute on resume: {text:?}"
    );

    let captured = captured.lock().expect("captured provider");
    assert_eq!(captured.served, 3, "provider request count");
    let replay = captured
        .bodies
        .get(2)
        .expect("resumed provider request body");
    let input = replay
        .get("input")
        .and_then(Value::as_array)
        .expect("replay input");
    let outputs = input
        .iter()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("function_call_output"))
        .collect::<Vec<_>>();
    assert_eq!(
        outputs.len(),
        1,
        "replay must retain the completed round exactly once: {replay}"
    );
    assert_eq!(
        outputs[0].get("call_id").and_then(Value::as_str),
        Some("call_append_once"),
        "replayed round call id: {replay}"
    );
    assert!(
        outputs[0]
            .get("output")
            .and_then(Value::as_str)
            .is_some_and(|output| output.contains("exitCode")),
        "replayed round must carry the executed shell result: {replay}"
    );
}

/// A child MCP tool-level `isError` must be preserved as a tool result (not
/// a run failure) and encoded unambiguously into the function_call_output the
/// provider sees on the follow-up request.
#[tokio::test]
async fn readme_agents_child_mcp_is_error_reaches_the_provider() {
    let workspace = TempDir::new().expect("tempdir");
    let root = workspace.path();
    let notes = root.join("notes.txt");
    fs::write(&notes, "hello world\n").expect("notes");

    let (provider, captured) = scripted_provider(vec![
        MockResponse::FunctionCall {
            call_id: "call_edit_no_match",
            name: "filesystem__edit_file",
            arguments: r#"{"path":"notes.txt","edits":[{"oldText":"no such text","newText":"replacement"}]}"#,
        },
        MockResponse::Completed("The edit failed; the tool reported an error."),
    ]);
    write_agent(
        root,
        "edit-fail",
        &mock_provider_agent(
            "edit-fail",
            "Replace 'no such text' with 'replacement' in notes.txt using edit_file. Report the tool output.",
            &format!(
                "allow_tools = [\"filesystem/edit_file\"]\n\
                 \n\
                 [mcp_servers.filesystem]\n\
                 type = \"stdio\"\n\
                 command = \"{}\"\n\
                 args = [\"filesystem\", \".\", \"--allow\", \"filesystem.write\"]",
                common::toml_tuls_bin()
            ),
            provider.addr.port(),
        ),
    );

    let server = TulsServer::connect(
        &["agents", root.to_str().unwrap()],
        &[("MOCK_API_KEY", "mock-secret".into())],
    )
    .await;
    let spawned = structured_of(
        &server
            .call_ok(
                "spawn_agent",
                json!({"name": "edit-fail", "task": "Edit notes.txt as instructed."}),
            )
            .await,
    );
    let id = spawned
        .get("agentId")
        .and_then(Value::as_str)
        .expect("agentId")
        .to_string();

    let wait = server
        .call_ok("wait_agent", json!({"targets": [id], "timeoutMs": 30000}))
        .await;
    let results = agent_results(&wait);
    assert_eq!(results.len(), 1, "wait: {wait}");
    assert_eq!(
        results[0].get("status").and_then(Value::as_str),
        Some("completed"),
        "a child isError is a tool result, not a run failure: {wait}"
    );

    let captured = captured.lock().expect("captured provider");
    assert_eq!(captured.served, 2, "provider request count");
    let follow_up = captured.bodies.get(1).expect("follow-up request body");
    let input = follow_up
        .get("input")
        .and_then(Value::as_array)
        .expect("follow-up input");
    let output_item = input
        .iter()
        .find(|item| {
            item.get("type").and_then(Value::as_str) == Some("function_call_output")
                && item.get("call_id").and_then(Value::as_str) == Some("call_edit_no_match")
        })
        .unwrap_or_else(|| panic!("missing function_call_output: {follow_up}"));
    let encoded = output_item
        .get("output")
        .and_then(Value::as_str)
        .expect("encoded output");
    let decoded: Value = serde_json::from_str(encoded).expect("output must be JSON");
    assert_eq!(
        decoded.get("isError").and_then(Value::as_bool),
        Some(true),
        "child isError must be preserved: {encoded}"
    );
    assert!(
        decoded
            .get("output")
            .and_then(Value::as_str)
            .is_some_and(|output| output.contains("oldText not found")),
        "child error text must be preserved: {encoded}"
    );

    assert_eq!(
        read_file(&notes),
        "hello world\n",
        "the failed edit must not modify the file"
    );
}
