use super::*;
use std::fs;
use tempfile::tempdir;

#[test]
fn empty_registry_advertises_no_tools() {
    let temp = tempdir().unwrap();
    let server = SkillsServer::new(temp.path().to_path_buf(), ToolPolicy::default()).unwrap();
    assert!(server.get_tool("activate_skill").is_none());
    assert!(server.get_info().capabilities.tools.is_none());
}

#[test]
fn activation_reparses_and_builds_manifest() {
    let temp = tempdir().unwrap();
    let dir = temp.path().join(".agents/skills/demo");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("SKILL.md"),
        "---\nname: demo\ndescription: Demo\n---\nDo it.\n",
    )
    .unwrap();
    fs::write(dir.join("guide.txt"), "guide").unwrap();
    let activated = SkillsServer::new(temp.path().to_path_buf(), ToolPolicy::default())
        .unwrap()
        .activate("demo")
        .unwrap();
    assert_eq!(activated.instructions, "Do it.\n");
    assert_eq!(activated.resources, ["guide.txt"]);
    let value = serde_json::to_value(&activated).unwrap();
    assert!(value.get("skillDir").is_some());
    assert!(value.get("skill_dir").is_none());
    let mut schema = schemars::schema_for!(ActivatedSkill);
    let properties = schema
        .ensure_object()
        .get("properties")
        .and_then(Value::as_object)
        .unwrap();
    assert!(properties.contains_key("skillDir"));
    assert!(!properties.contains_key("skill_dir"));
}

#[test]
fn tool_schema_matches_registry_and_catalog_excludes_bodies() {
    let temp = tempdir().unwrap();
    let dir = temp.path().join(".agents/skills/demo");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("SKILL.md"),
        "---\nname: demo\ndescription: Demo\n---\nprivate body\n",
    )
    .unwrap();
    let server = SkillsServer::new(temp.path().to_path_buf(), ToolPolicy::default()).unwrap();
    let tool = server.get_tool("activate_skill").unwrap();
    let schema = tool.schema_as_json_value();
    assert_eq!(schema["properties"]["name"]["enum"], json!(["demo"]));
    assert_eq!(schema["additionalProperties"], false);
    let description = tool.description.unwrap();
    assert!(description.contains("- demo: Demo"));
    assert!(!description.contains("private body"));
}

#[test]
fn activation_rejects_changed_skill_name() {
    let temp = tempdir().unwrap();
    let dir = temp.path().join(".agents/skills/demo");
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("SKILL.md");
    fs::write(&file, "---\nname: demo\ndescription: Demo\n---\nbody\n").unwrap();
    let server = SkillsServer::new(temp.path().to_path_buf(), ToolPolicy::default()).unwrap();
    fs::write(file, "---\nname: changed\ndescription: Demo\n---\nbody\n").unwrap();
    assert!(
        server
            .activate("demo")
            .unwrap_err()
            .to_string()
            .contains("name changed")
    );
}

#[test]
fn activation_rejects_oversized_structured_output() {
    let temp = tempdir().unwrap();
    let dir = temp.path().join(".agents/skills/demo");
    fs::create_dir_all(&dir).unwrap();
    let prefix = "---\nname: demo\ndescription: Demo\n---\n";
    let body = format!(
        "{prefix}{}",
        "x".repeat(super::parser::MAX_SKILL_BYTES as usize - prefix.len())
    );
    fs::write(dir.join("SKILL.md"), body).unwrap();
    let server = SkillsServer::new(temp.path().to_path_buf(), ToolPolicy::default()).unwrap();
    let result = server.call_activate(Some(json!({"name": "demo"}).as_object().unwrap().clone()));
    assert_eq!(result.is_error, Some(true));
}
