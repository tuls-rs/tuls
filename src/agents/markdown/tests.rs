use super::*;

#[test]
fn canonical_markdown_parses_explicit_child_tool_policy() {
    let input = "---\nname: secure-agent\ndescription: x\nmodel: gpt\nmodel_provider: openai\nallow_tools: [child/*]\ndeny_tools: [child/write]\nmcp_servers:\n  child:\n    type: stdio\n    command: node\n---\nbody";
    let agent = parse_markdown(PathBuf::from("a.md"), input, MarkdownFlavor::Canonical).unwrap();
    assert!(agent.tool_policy.allows("child", "read"));
    assert!(!agent.tool_policy.allows("child", "write"));
}

#[test]
fn canonical_markdown_defaults_to_no_child_tools() {
    let input =
        "---\nname: secure-agent\ndescription: x\nmodel: gpt\nmodel_provider: openai\n---\nbody";
    let agent = parse_markdown(PathBuf::from("a.md"), input, MarkdownFlavor::Canonical).unwrap();
    assert!(!agent.tool_policy.allows("child", "read"));
}

#[test]
fn canonical_markdown_accepts_integer_temperature() {
    let agent = parse_markdown(
        PathBuf::from("agent.md"),
        "---\nname: test\ndescription: test\nmodel_provider: openai\nmodel: test\ntemperature: 1\n---\ninstructions",
        MarkdownFlavor::Canonical,
    )
    .unwrap();
    assert_eq!(agent.temperature, Some(1.0));
}

#[test]
fn claude_mcp_selectors_are_normalized_at_the_adapter_boundary() {
    let map: Mapping = noyalib::from_str(
        "tools: [mcp__child__read, mcp__other, Read]\ndisallowedTools: [mcp__child__write]",
    )
    .unwrap();
    let policy = tool_policy(&map, MarkdownFlavor::Claude).unwrap();
    assert!(policy.allows("child", "read"));
    assert!(policy.allows("other", "anything"));
    assert!(!policy.allows("child", "write"));
}

#[test]
fn claude_execution_fields_that_tuls_cannot_enforce_are_rejected() {
    for field in [
        "permissionMode: plan",
        "isolation: worktree",
        "hooks: {}",
        "memory: project",
    ] {
        let input = format!(
            "---\nname: claude-agent\ndescription: x\nmodel: claude-model\n{field}\n---\nbody"
        );
        assert!(parse_markdown(PathBuf::from("a.md"), &input, MarkdownFlavor::Claude).is_err());
    }
}

#[test]
fn canonical_schema_rejects_unknown_fields_and_wrong_types() {
    for field in ["unknown: true", "allow_tools: child/read", "max_turns: two"] {
        let input = format!(
            "---\nname: secure-agent\ndescription: x\nmodel: gpt\nmodel_provider: openai\n{field}\n---\nbody"
        );
        assert!(parse_markdown(PathBuf::from("a.md"), &input, MarkdownFlavor::Canonical).is_err());
    }
}

#[test]
fn reasoning_effort_is_validated_for_selected_wire() {
    let input = "---\nname: secure-agent\ndescription: x\nmodel: gpt\nmodel_provider: openai\nreasoning_effort: high\n---\nbody";
    let agent = parse_markdown(PathBuf::from("a.md"), input, MarkdownFlavor::Canonical).unwrap();
    assert_eq!(agent.reasoning_effort.as_deref(), Some("high"));
}

#[test]
fn openrouter_canonical_markdown_parses_with_defaults() {
    let input = "---\nname: or-agent\ndescription: x\nmodel: openai/gpt-5.6-luna\nmodel_provider: openrouter\n---\nbody";
    let agent = parse_markdown(PathBuf::from("a.md"), input, MarkdownFlavor::Canonical).unwrap();
    assert_eq!(agent.base_url.as_str(), "https://openrouter.ai/api/v1");
    assert_eq!(agent.env_key, "OPENROUTER_API_KEY");
    assert_eq!(agent.wire_api, WireApi::Responses);
}

#[test]
fn builtin_markdown_rejects_explicit_endpoint_overrides() {
    for field in [
        "base_url: https://api.openai.com/v1",
        "env_key: OPENAI_API_KEY",
        "wire_api: responses",
    ] {
        let input = format!(
            "---\nname: secure-agent\ndescription: x\nmodel: gpt\nmodel_provider: openai\n{field}\n---\nbody"
        );
        assert!(
            parse_markdown(PathBuf::from("a.md"), &input, MarkdownFlavor::Canonical).is_err(),
            "{field} was accepted"
        );
    }
}

#[test]
fn canonical_markdown_max_turns_and_skills_are_bounded() {
    let at_limit = "---\nname: secure-agent\ndescription: x\nmodel: gpt\nmodel_provider: openai\nmax_turns: 128\n---\nbody";
    assert!(parse_markdown(PathBuf::from("a.md"), at_limit, MarkdownFlavor::Canonical).is_ok());
    let over_limit = "---\nname: secure-agent\ndescription: x\nmodel: gpt\nmodel_provider: openai\nmax_turns: 129\n---\nbody";
    assert!(parse_markdown(PathBuf::from("a.md"), over_limit, MarkdownFlavor::Canonical).is_err());
    let skills = (0..MAX_SKILLS)
        .map(|index| format!("skill-{index}"))
        .collect::<Vec<_>>();
    let at_limit = format!(
        "---\nname: secure-agent\ndescription: x\nmodel: gpt\nmodel_provider: openai\nskills: [{}]\n---\nbody",
        skills.join(", ")
    );
    assert!(parse_markdown(PathBuf::from("a.md"), &at_limit, MarkdownFlavor::Canonical).is_ok());
    let over_limit = format!(
        "---\nname: secure-agent\ndescription: x\nmodel: gpt\nmodel_provider: openai\nskills: [{}, skill-over]\n---\nbody",
        skills.join(", ")
    );
    assert!(
        parse_markdown(
            PathBuf::from("a.md"),
            &over_limit,
            MarkdownFlavor::Canonical
        )
        .is_err()
    );
}
