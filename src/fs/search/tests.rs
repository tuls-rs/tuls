use super::*;

#[test]
fn glob_star_does_not_cross_separator() {
    let local = build_glob_set(["*.rs".to_string()]).unwrap();
    let recursive = build_glob_set(["**/*.rs".to_string()]).unwrap();
    assert!(local.is_match("file.rs"));
    assert!(!local.is_match("a/b.rs"));
    assert!(recursive.is_match("a/b.rs"));
}

#[test]
fn glob_matches_dotfiles() {
    assert!(
        build_glob_set([".*".to_string()])
            .unwrap()
            .is_match(".hidden")
    );
    assert!(
        build_glob_set(["**/.*".to_string()])
            .unwrap()
            .is_match("a/.hidden")
    );
}

#[test]
fn excludes_expand_plain_directory_names() {
    let excludes = build_excludes(&["node_modules".into()]).unwrap().unwrap();
    assert!(excludes.is_match("node_modules"));
    assert!(excludes.is_match("a/node_modules"));
    assert!(excludes.is_match("a/node_modules/pkg"));
    assert!(!excludes.is_match("a/nodemodules"));
}

#[test]
fn invalid_excludes_fail_closed() {
    assert!(build_excludes(&["[".into()]).is_err());
}

#[test]
fn serialize_tree_round_trips_valid_json() {
    let tree = vec![
        TreeEntry {
            name: "src".into(),
            kind: "directory",
            children: Some(vec![TreeEntry {
                name: "main.rs".into(),
                kind: "file",
                children: None,
            }]),
        },
        TreeEntry {
            name: "README.md".into(),
            kind: "file",
            children: None,
        },
    ];
    let json = serialize_tree(&tree).expect("fits within the limit");
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
    let entries = parsed.as_array().expect("array");
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["name"], "src");
    assert_eq!(entries[0]["children"][0]["name"], "main.rs");
    assert_eq!(entries[1]["type"], "file");
}

/// An oversized tree must produce an error, never a truncated JSON blob
/// that a client cannot parse.
#[test]
fn serialize_tree_errors_instead_of_truncating_json() {
    let wide: Vec<TreeEntry> = (0..2_000)
        .map(|i| TreeEntry {
            name: format!("entry-{i:04}-with-a-rather-long-name"),
            kind: "file",
            children: None,
        })
        .collect();
    let error = serialize_tree(&wide).expect_err("exceeds the tool-result limit");
    assert!(
        error.contains("exceeds") && error.contains("excludePatterns"),
        "{error}"
    );
}

/// Walking an oversized tree stays valid JSON: the entry budget is enforced
/// inside the tree and marked with a structured `truncated` entry instead of
/// cutting the output.
#[tokio::test]
async fn directory_tree_truncation_marker_is_structured_and_valid() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root");
    tokio::fs::create_dir_all(&root).await.unwrap();
    let access = AccessControl::from_args(std::slice::from_ref(&root)).unwrap();

    // More entries than the budget so the walk is cut off mid-tree.
    for i in 0..(MAX_TREE_ENTRIES + 8) {
        tokio::fs::write(root.join(format!("file-{i:04}")), b"x")
            .await
            .unwrap();
    }

    let tree = directory_tree(&root, &[], &access).await.unwrap();
    let json = serde_json::to_string(&tree).expect("tree serializes");
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
    let names: Vec<&str> = parsed
        .as_array()
        .expect("array")
        .iter()
        .filter_map(|entry| entry.get("name").and_then(serde_json::Value::as_str))
        .collect();
    assert!(
        names.iter().any(|name| name.contains("truncated")),
        "truncation marker present: {names:?}"
    );
    assert!(
        names.iter().any(|name| name.contains("file-0000")),
        "walked entries present: {names:?}"
    );
}
