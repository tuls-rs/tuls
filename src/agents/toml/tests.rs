use super::*;

const BASE: &str =
    "name='a'\ndescription='x'\ninstructions='body'\nmodel='gpt'\nmodel_provider='openai'\n";

#[test]
fn reasoning_temperature_and_mcp_validation() {
    let valid = format!(
        "{BASE}reasoning_effort='medium'\ntemperature=2.0\n[mcp_servers.local]\ntype='stdio'\ncommand='node'"
    );
    let agent = parse_toml(PathBuf::from("a.toml"), &valid).unwrap();
    assert_eq!(agent.reasoning_effort.as_deref(), Some("medium"));

    let invalid = format!("{BASE}[mcp_servers.remote]\ntype='unknown'\nurl='https://example.test'");
    assert!(parse_toml(PathBuf::from("a.toml"), &invalid).is_err());
}

#[test]
fn provider_is_required_and_wire_conflicts_are_rejected() {
    let missing_provider = "name='a'\ndescription='x'\ninstructions='body'\nmodel='gpt'";
    assert!(parse_toml(PathBuf::from("a.toml"), missing_provider).is_err());
    let input = format!("{BASE}wire_api='anthropic-messages'");
    assert!(parse_toml(PathBuf::from("a.toml"), &input).is_err());
}

#[test]
fn canonical_schema_is_strict() {
    for field in [
        "unknown_field='x'",
        "allow_tools='child/read'",
        "max_turns='2'",
    ] {
        assert!(
            parse_toml(PathBuf::from("a.toml"), &format!("{BASE}{field}")).is_err(),
            "{field} was accepted"
        );
    }
}

#[test]
fn child_tool_policy_is_explicit_and_default_deny() {
    let default = parse_toml(PathBuf::from("a.toml"), BASE).unwrap();
    assert!(!default.tool_policy.allows("child", "read"));

    let input = format!(
        "{BASE}allow_tools=['child/*']\ndeny_tools=['child/write']\n[mcp_servers.child]\ntype='stdio'\ncommand='node'"
    );
    let agent = parse_toml(PathBuf::from("a.toml"), &input).unwrap();
    assert!(agent.tool_policy.allows("child", "read"));
    assert!(!agent.tool_policy.allows("child", "write"));
}

#[test]
fn integer_temperature_is_accepted_as_a_number() {
    let agent = parse_toml(PathBuf::from("a.toml"), &format!("{BASE}temperature=1")).unwrap();
    assert_eq!(agent.temperature, Some(1.0));
}

#[test]
fn openrouter_is_first_class_with_defaults() {
    let input = "name='or'\ndescription='x'\ninstructions='body'\nmodel='openai/gpt-5.6-luna'\nmodel_provider='openrouter'\n";
    let agent = parse_toml(PathBuf::from("a.toml"), input).unwrap();
    assert_eq!(agent.base_url.as_str(), "https://openrouter.ai/api/v1");
    assert_eq!(agent.env_key, "OPENROUTER_API_KEY");
    assert_eq!(agent.wire_api, WireApi::Responses);
}

#[test]
fn builtin_providers_reject_explicit_endpoint_overrides() {
    for field in [
        "base_url='https://api.openai.com/v1'",
        "env_key='OPENAI_API_KEY'",
        "wire_api='responses'",
        "wire_api='anthropic-messages'",
    ] {
        let input = format!("{BASE}{field}");
        assert!(
            parse_toml(PathBuf::from("a.toml"), &input).is_err(),
            "{field} was accepted"
        );
    }
}

#[test]
fn custom_provider_requires_all_endpoint_fields() {
    let partial = "name='a'\ndescription='x'\ninstructions='body'\nmodel='gpt'\nmodel_provider='custom'\nbase_url='https://example.test/v1'\nenv_key='K'\n";
    assert!(parse_toml(PathBuf::from("a.toml"), partial).is_err());
    let complete = "name='a'\ndescription='x'\ninstructions='body'\nmodel='gpt'\nmodel_provider='custom'\nbase_url='https://example.test/v1'\nenv_key='K'\nwire_api='responses'\n";
    let agent = parse_toml(PathBuf::from("a.toml"), complete).unwrap();
    assert_eq!(agent.base_url.as_str(), "https://example.test/v1");
    assert_eq!(agent.env_key, "K");
}

#[test]
fn max_turns_is_bounded_to_128() {
    assert!(parse_toml(PathBuf::from("a.toml"), &format!("{BASE}max_turns=128")).is_ok());
    assert!(parse_toml(PathBuf::from("a.toml"), &format!("{BASE}max_turns=129")).is_err());
}

#[test]
fn skills_are_bounded_to_32() {
    let skills = (0..MAX_SKILLS)
        .map(|index| format!("skill-{index}"))
        .collect::<Vec<_>>();
    let at_limit = format!("{BASE}skills={}", serde_json::to_string(&skills).unwrap());
    assert!(parse_toml(PathBuf::from("a.toml"), &at_limit).is_ok());
    let over_limit = format!(
        "{BASE}skills={}",
        serde_json::to_string(
            &skills
                .into_iter()
                .chain(std::iter::once("skill-over".into()))
                .collect::<Vec<_>>()
        )
        .unwrap()
    );
    assert!(parse_toml(PathBuf::from("a.toml"), &over_limit).is_err());
}
