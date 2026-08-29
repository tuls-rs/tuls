use super::*;

fn parse(input: &str) -> Result<AgentDefinition> {
    parse_markdown(PathBuf::from("agent.md"), input)
}

fn openai(extra: &str, body: &str) -> String {
    format!(
        "---\nname: reviewer\ndescription: Reviews code\nprovider: openai\nmodel: gpt-test\n{extra}---\n{body}"
    )
}

#[test]
fn minimal_openai_agent_uses_markdown_body_as_instructions() {
    let agent = parse(&openai("", "Review carefully.\n")).unwrap();
    assert_eq!(agent.name, "reviewer");
    assert!(agent.subagent);
    assert_eq!(agent.instructions, "Review carefully.\n");
    assert_eq!(agent.max_turns, DEFAULT_MAX_TURNS);
}

#[test]
fn subagent_is_a_strict_boolean_defaulting_to_true() {
    assert!(parse(&openai("", "body")).unwrap().subagent);
    assert!(parse(&openai("subagent: true\n", "body")).unwrap().subagent);
    assert!(
        !parse(&openai("subagent: false\n", "body"))
            .unwrap()
            .subagent
    );

    for value in ["\"false\"", "0", "{}", "[]"] {
        let error = parse(&openai(&format!("subagent: {value}\n"), "body")).unwrap_err();
        assert!(
            error.to_string().contains("subagent must be a boolean"),
            "unexpected error for {value}: {error}"
        );
    }
}

#[test]
fn subagent_role_aliases_remain_unknown() {
    for field in [
        "mode: primary",
        "primary: true",
        "main_agent: true",
        "mainAgent: true",
        "invocable: false",
    ] {
        let error = parse(&openai(&format!("{field}\n"), "body")).unwrap_err();
        assert!(
            error.to_string().contains("unknown agent field"),
            "unexpected error for {field}: {error}"
        );
    }
}

#[test]
fn hidden_definitions_still_require_the_complete_valid_schema() {
    let missing_provider =
        "---\nname: leader\ndescription: x\nmodel: gpt\nsubagent: false\n---\nbody";
    assert!(parse(missing_provider).is_err());

    let invalid_mcp = openai(
        "subagent: false\nmcp_servers:\n  child:\n    type: stdio\n    command: tuls\n    unsupported: true\n",
        "body",
    );
    assert!(parse(&invalid_mcp).is_err());
}

#[test]
fn body_and_required_provider_are_enforced() {
    assert!(parse(&openai("", " \n")).is_err());
    assert!(
        parse("---\nname: reviewer\ndescription: x\nmodel: gpt\n---\nbody")
            .unwrap_err()
            .to_string()
            .contains("provider is required")
    );
}

#[test]
fn legacy_and_unknown_fields_are_rejected() {
    for field in [
        "model_provider: openai",
        "allow_tools: []",
        "deny_tools: []",
        "instructions: hidden",
        "unknown: true",
    ] {
        let input = openai(&format!("{field}\n"), "body");
        assert!(parse(&input).is_err(), "{field} was accepted");
    }
}

#[test]
fn wrong_field_types_are_rejected() {
    for field in [
        "provider: [openai]",
        "tools: filesystem/read",
        "max_turns: two",
        "temperature: cold",
        "mcp_servers: []",
    ] {
        let input = openai(&format!("{field}\n"), "body");
        assert!(parse(&input).is_err(), "{field} was accepted");
    }
}

#[test]
fn tools_are_explicit_default_deny_and_disallowed_tools_win() {
    let input = openai(
        "tools: [child/*]\ndisallowed_tools: [child/write]\nmcp_servers:\n  child:\n    type: stdio\n    command: node\n",
        "body",
    );
    let agent = parse(&input).unwrap();
    assert!(agent.tool_policy.allows("child", "read"));
    assert!(!agent.tool_policy.allows("child", "write"));

    let agent = parse(&openai(
        "mcp_servers:\n  child:\n    type: stdio\n    command: node\n",
        "body",
    ))
    .unwrap();
    assert!(!agent.tool_policy.allows("child", "read"));
}

#[test]
fn integer_temperature_and_wire_specific_reasoning_are_validated() {
    let agent = parse(&openai("temperature: 1\nreasoning_effort: HIGH\n", "body")).unwrap();
    assert_eq!(agent.temperature, Some(1.0));
    assert_eq!(agent.reasoning_effort.as_deref(), Some("high"));

    let anthropic = "---\nname: reviewer\ndescription: x\nprovider: anthropic\nmodel: claude\nreasoning_effort: minimal\n---\nbody";
    assert!(parse(anthropic).is_err());
}

#[test]
fn max_turns_and_skills_are_bounded() {
    for value in [1, 128] {
        assert!(parse(&openai(&format!("max_turns: {value}\n"), "body")).is_ok());
    }
    for value in [0, 129] {
        assert!(parse(&openai(&format!("max_turns: {value}\n"), "body")).is_err());
    }
    let skills = (0..MAX_SKILLS)
        .map(|index| format!("skill-{index}"))
        .collect::<Vec<_>>();
    assert!(
        parse(&openai(
            &format!("skills: [{}]\n", skills.join(", ")),
            "body"
        ))
        .is_ok()
    );
    assert!(
        parse(&openai(
            &format!("skills: [{}, overflow]\n", skills.join(", ")),
            "body"
        ))
        .is_err()
    );
}

#[test]
fn openrouter_uses_fixed_provider_configuration() {
    let input =
        "---\nname: researcher\ndescription: x\nprovider: openrouter\nmodel: openai/gpt\n---\nbody";
    let agent = parse(input).unwrap();
    assert_eq!(agent.base_url.as_str(), "https://openrouter.ai/api/v1");
    assert_eq!(agent.env_key, "OPENROUTER_API_KEY");
    assert_eq!(agent.wire_api, WireApi::Responses);
}

#[test]
fn first_class_providers_reject_custom_configuration() {
    for field in [
        "base_url: https://api.openai.com/v1",
        "credential_env: OPENAI_API_KEY",
        "api: responses",
    ] {
        assert!(
            parse(&openai(&format!("{field}\n"), "body")).is_err(),
            "{field} was accepted"
        );
    }
}

#[test]
fn custom_provider_requires_all_fields_and_validates_credential_env() {
    let base = "---\nname: custom\ndescription: x\nprovider: custom\nmodel: model\n";
    for fields in [
        "base_url: https://example.test/v1\ncredential_env: KEY\n",
        "base_url: https://example.test/v1\napi: responses\n",
        "credential_env: KEY\napi: responses\n",
    ] {
        assert!(parse(&format!("{base}{fields}---\nbody")).is_err());
    }
    assert!(
        parse(&format!(
            "{base}base_url: https://example.test/v1\ncredential_env: 9BAD\napi: responses\n---\nbody"
        ))
        .is_err()
    );
    let agent = parse(&format!(
        "{base}base_url: https://example.test/v1\ncredential_env: GATEWAY_API_KEY\napi: responses\n---\nbody"
    ))
    .unwrap();
    assert_eq!(agent.env_key, "GATEWAY_API_KEY");
}

#[test]
fn literal_secret_fields_are_rejected() {
    for field in ["api_key: secret", "token: secret", "accessToken: secret"] {
        assert!(parse(&openai(&format!("{field}\n"), "body")).is_err());
    }
    let nested =
        "---\nname: reviewer\ndescription: x\nprovider:\n  api_key: secret\nmodel: gpt\n---\nbody";
    assert!(parse(nested).is_err());
}

#[test]
fn stdio_and_http_mcp_servers_parse() {
    let input = openai(
        "mcp_servers:\n  local:\n    type: stdio\n    command: tuls\n    args: [filesystem, .]\n    env:\n      TOKEN: ${TOKEN}\n  issues:\n    type: http\n    url: https://mcp.example.test/mcp\n    headers:\n      Authorization: Bearer ${ISSUE_TOKEN}\n",
        "body",
    );
    let agent = parse(&input).unwrap();
    assert!(matches!(
        agent.mcp_servers["local"],
        McpServerDefinition::Stdio { .. }
    ));
    assert!(matches!(
        agent.mcp_servers["issues"],
        McpServerDefinition::Http { .. }
    ));
}

#[test]
fn unknown_mcp_server_fields_are_rejected() {
    let input = openai(
        "mcp_servers:\n  child:\n    type: stdio\n    command: tuls\n    timeout: 30\n",
        "body",
    );
    assert!(parse(&input).is_err());
}
