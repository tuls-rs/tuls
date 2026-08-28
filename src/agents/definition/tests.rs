use super::*;

#[test]
fn validates_environment_reasoning_and_adapter_ranges() {
    assert!(validate_env_key("OPENAI_API_KEY").is_ok());
    assert!(validate_env_key("9BAD").is_err());
    assert_eq!(
        normalize_reasoning_effort("HIGH", &WireApi::Responses).unwrap(),
        "high"
    );
    assert!(validate_temperature(2.0, &WireApi::Responses).is_ok());
    assert!(validate_temperature(2.1, &WireApi::Responses).is_err());
}

#[test]
fn child_tool_selectors_are_canonical_and_default_deny() {
    let policy = ChildToolPolicy::default();
    assert!(!policy.allows("server", "tool"));

    let policy =
        ChildToolPolicy::new(["server/*".to_string()], ["server/write".to_string()]).unwrap();
    assert!(policy.allows("server", "read"));
    assert!(!policy.allows("server", "write"));
    assert!(ChildToolPolicy::new(["invalid".to_string()], Vec::<String>::new()).is_err());
}

#[test]
fn endpoints_require_host_reject_credentials_and_limit_plain_http() {
    assert!(validate_endpoint(&Url::parse("https://example.test/v1").unwrap()).is_ok());
    assert!(validate_endpoint(&Url::parse("http://localhost:8080/v1").unwrap()).is_ok());
    assert!(validate_endpoint(&Url::parse("http://example.test/v1").unwrap()).is_err());
    assert!(
        validate_endpoint(&Url::parse("https://user:secret@example.test/v1").unwrap()).is_err()
    );
}

#[test]
fn reasoning_effort_is_wire_specific() {
    assert!(normalize_reasoning_effort("minimal", &WireApi::Responses).is_ok());
    assert!(normalize_reasoning_effort("minimal", &WireApi::AnthropicMessages).is_err());
    assert!(normalize_reasoning_effort("max", &WireApi::AnthropicMessages).is_ok());
}

#[test]
fn child_tool_policy_rejects_unknown_servers_and_tools() {
    let policy = ChildToolPolicy::new(["server/read".to_string()], Vec::<String>::new()).unwrap();
    assert!(policy.validate_servers(["other"]).is_err());
    assert!(policy.validate_servers(["server"]).is_ok());
    assert!(policy.validate_catalog("server", ["write"]).is_err());
    assert!(policy.validate_catalog("server", ["read"]).is_ok());
}

#[test]
fn openrouter_is_first_class_with_responses_defaults() {
    assert_eq!(
        parse_provider("openrouter").unwrap(),
        ModelProviderKind::OpenRouter
    );
    let (url, key, wire) = defaults_for(&ModelProviderKind::OpenRouter).unwrap();
    assert_eq!(url.as_str(), "https://openrouter.ai/api/v1");
    assert_eq!(key, "OPENROUTER_API_KEY");
    assert_eq!(wire, WireApi::Responses);
    assert!(validate_provider_wire(&ModelProviderKind::OpenRouter, &WireApi::Responses).is_ok());
    assert!(
        validate_provider_wire(&ModelProviderKind::OpenRouter, &WireApi::AnthropicMessages)
            .is_err()
    );
}

#[test]
fn builtin_providers_reject_all_endpoint_overrides() {
    for provider in [
        ModelProviderKind::OpenAi,
        ModelProviderKind::Anthropic,
        ModelProviderKind::OpenRouter,
    ] {
        let base = resolve_endpoint(&provider, Some("https://openrouter.ai/api/v1"), None, None);
        assert!(base.is_err(), "{provider:?} accepted base_url");
        let env = resolve_endpoint(&provider, None, Some("OPENROUTER_API_KEY"), None);
        assert!(env.is_err(), "{provider:?} accepted env_key");
        let wire = resolve_endpoint(&provider, None, None, Some("responses"));
        assert!(wire.is_err(), "{provider:?} accepted matching wire_api");
    }
    let (url, key, wire) = resolve_endpoint(&ModelProviderKind::OpenAi, None, None, None).unwrap();
    assert_eq!(url.as_str(), "https://api.openai.com/v1");
    assert_eq!(key, "OPENAI_API_KEY");
    assert_eq!(wire, WireApi::Responses);
}

#[test]
fn custom_provider_requires_all_explicit_endpoint_fields() {
    let missing_wire = resolve_endpoint(
        &ModelProviderKind::Custom,
        Some("https://example.test/v1"),
        Some("K"),
        None,
    );
    assert!(missing_wire.is_err());
    let missing_env = resolve_endpoint(
        &ModelProviderKind::Custom,
        Some("https://example.test/v1"),
        None,
        Some("responses"),
    );
    assert!(missing_env.is_err());
    let missing_base = resolve_endpoint(
        &ModelProviderKind::Custom,
        None,
        Some("K"),
        Some("responses"),
    );
    assert!(missing_base.is_err());
    let (url, key, wire) = resolve_endpoint(
        &ModelProviderKind::Custom,
        Some("https://example.test/v1"),
        Some("K"),
        Some("responses"),
    )
    .unwrap();
    assert_eq!(url.as_str(), "https://example.test/v1");
    assert_eq!(key, "K");
    assert_eq!(wire, WireApi::Responses);
}

#[test]
fn bound_constants_and_skills_are_enforced() {
    assert_eq!(DEFAULT_MAX_TURNS, 32);
    assert_eq!(MAX_TURNS, 128);
    assert_eq!(MAX_SPAWN_TASK_BYTES, 256 * 1024);
    assert_eq!(MAX_SEND_MESSAGE_BYTES, 256 * 1024);
    assert_eq!(MAX_WAIT_TARGETS, 64);
    assert_eq!(MAX_DISCOVERED_AGENTS, 256);
    assert_eq!(MAX_SKILLS, 32);
    assert_eq!(MAX_BUILT_CONTEXT_BYTES, 1024 * 1024);
    assert_eq!(MAX_AGENT_DESCRIPTION_BYTES, 4096);

    let at_limit = (0..MAX_SKILLS)
        .map(|index| format!("skill-{index}"))
        .collect::<Vec<_>>();
    assert!(validate_skills(&at_limit).is_ok());
    let over_limit = (0..=MAX_SKILLS)
        .map(|index| format!("skill-{index}"))
        .collect::<Vec<_>>();
    assert!(validate_skills(&over_limit).is_err());
    assert!(validate_skills(&[" ".into()]).is_err());
}

#[test]
fn built_context_instruction_bytes_are_bounded() {
    assert!(validate_instructions(&"x".repeat(MAX_BUILT_CONTEXT_BYTES)).is_ok());
    assert!(validate_instructions(&"x".repeat(MAX_BUILT_CONTEXT_BYTES + 1)).is_err());
}

#[test]
fn descriptions_are_nonempty_and_bounded() {
    assert!(validate_description("catalog entry").is_ok());
    assert!(validate_description(" ").is_err());
    assert!(validate_description(&"x".repeat(MAX_AGENT_DESCRIPTION_BYTES + 1)).is_err());
}
