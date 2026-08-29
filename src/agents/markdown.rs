use crate::agents::definition::*;
use anyhow::{Context, Result, bail};
use noyalib::{Mapping, Value};
use std::{collections::BTreeMap, path::PathBuf};
use url::Url;

const SUPPORTED_FIELDS: &[&str] = &[
    "name",
    "description",
    "subagent",
    "provider",
    "model",
    "base_url",
    "credential_env",
    "api",
    "temperature",
    "reasoning_effort",
    "max_turns",
    "tools",
    "disallowed_tools",
    "skills",
    "mcp_servers",
];

pub(crate) fn parse_markdown(path: PathBuf, input: &str) -> Result<AgentDefinition> {
    if input.len() > MAX_AGENT_FILE_BYTES {
        bail!("agent markdown exceeds 1 MiB")
    }
    let (frontmatter, instructions) = split_frontmatter(input)?;
    let map: Mapping = noyalib::from_str(frontmatter).context("invalid agent YAML frontmatter")?;
    reject_provider_secrets(&map)?;
    reject_unknown_fields(&map)?;
    validate_supported_field_types(&map)?;

    let name = required(&map, "name")?;
    validate_name(&name)?;
    let description = required(&map, "description")?;
    validate_description(&description)?;
    let subagent = boolean(&map, "subagent")?.unwrap_or(true);
    validate_instructions(instructions)?;
    if instructions.trim().is_empty() {
        bail!("agent instructions body must be nonempty")
    }

    let model = required(&map, "model")?;
    let provider = parse_provider(&required(&map, "provider")?)?;
    let (base_url, env_key, wire_api) = resolve_endpoint(
        &provider,
        string(&map, "base_url").as_deref(),
        string(&map, "credential_env").as_deref(),
        string(&map, "api").as_deref(),
    )?;

    let temperature = number(&map, "temperature");
    if let Some(value) = temperature {
        validate_temperature(value, &wire_api)?;
    }
    let reasoning_effort = string(&map, "reasoning_effort")
        .map(|value| normalize_reasoning_effort(&value, &wire_api))
        .transpose()?;
    let max_turns = integer(&map, "max_turns").unwrap_or(DEFAULT_MAX_TURNS as i64);
    if !(1..=MAX_TURNS as i64).contains(&max_turns) {
        bail!("max_turns must be between 1 and {MAX_TURNS}")
    }

    let tool_policy =
        ChildToolPolicy::new(strings(&map, "tools")?, strings(&map, "disallowed_tools")?)?;
    let mcp_servers = mcp_servers(&map)?;
    tool_policy.validate_servers(mcp_servers.keys().map(String::as_str))?;
    let skills = strings(&map, "skills")?;
    validate_skills(&skills)?;

    Ok(AgentDefinition {
        name,
        description,
        subagent,
        instructions: instructions.into(),
        model,
        base_url,
        env_key,
        wire_api,
        reasoning_effort,
        temperature,
        max_turns: max_turns as u32,
        tool_policy,
        skills,
        mcp_servers,
        source_path: path,
    })
}

fn reject_unknown_fields(map: &Mapping) -> Result<()> {
    for key in map.keys() {
        if !SUPPORTED_FIELDS.contains(&key.as_str()) {
            bail!("unknown agent field {key:?}")
        }
    }
    Ok(())
}

fn validate_supported_field_types(map: &Mapping) -> Result<()> {
    for key in [
        "name",
        "description",
        "provider",
        "model",
        "base_url",
        "credential_env",
        "api",
        "reasoning_effort",
    ] {
        if map.contains_key(key) && map.get(key).and_then(Value::as_str).is_none() {
            bail!("{key} must be a string")
        }
    }
    if map.contains_key("max_turns") && map.get("max_turns").and_then(Value::as_i64).is_none() {
        bail!("max_turns must be an integer")
    }
    if map.contains_key("temperature") {
        let value = map.get("temperature").context("temperature is missing")?;
        if value.as_f64().is_none() && value.as_i64().is_none() {
            bail!("temperature must be a number")
        }
    }
    Ok(())
}

fn split_frontmatter(input: &str) -> Result<(&str, &str)> {
    let rest = input
        .strip_prefix("---\n")
        .or_else(|| input.strip_prefix("---\r\n"))
        .context("agent markdown must begin with YAML frontmatter")?;
    for marker in ["\n---\n", "\n---\r\n"] {
        if let Some(pair) = rest.split_once(marker) {
            return Ok(pair);
        }
    }
    bail!("agent YAML frontmatter is not terminated")
}

fn string(map: &Mapping, key: &str) -> Option<String> {
    map.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn required(map: &Mapping, key: &str) -> Result<String> {
    string(map, key)
        .filter(|value| !value.trim().is_empty())
        .with_context(|| format!("{key} is required"))
}

fn integer(map: &Mapping, key: &str) -> Option<i64> {
    map.get(key).and_then(Value::as_i64)
}

fn number(map: &Mapping, key: &str) -> Option<f32> {
    map.get(key)
        .and_then(|value| {
            value
                .as_f64()
                .or_else(|| value.as_i64().map(|value| value as f64))
        })
        .map(|value| value as f32)
}

fn boolean(map: &Mapping, key: &str) -> Result<Option<bool>> {
    match map.get(key) {
        None => Ok(None),
        Some(value) => value
            .as_bool()
            .map(Some)
            .with_context(|| format!("{key} must be a boolean")),
    }
}

fn strings(map: &Mapping, key: &str) -> Result<Vec<String>> {
    match map.get(key) {
        None => Ok(Vec::new()),
        Some(Value::Sequence(values)) => values
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_owned)
                    .context("array entries must be strings")
            })
            .collect(),
        Some(_) => bail!("{key} must be an array"),
    }
}

fn reject_provider_secrets(map: &Mapping) -> Result<()> {
    for key in ["api_key", "apiKey", "token", "access_token", "accessToken"] {
        if map.contains_key(key) {
            bail!("literal secret field {key} is prohibited")
        }
    }
    if let Some(Value::Mapping(provider)) = map.get("provider") {
        for key in ["api_key", "apiKey", "token", "access_token", "accessToken"] {
            if provider.contains_key(key) {
                bail!("literal provider secret field {key} is prohibited")
            }
        }
    }
    Ok(())
}

fn mcp_servers(map: &Mapping) -> Result<BTreeMap<String, McpServerDefinition>> {
    let Some(Value::Mapping(servers)) = map.get("mcp_servers") else {
        if map.contains_key("mcp_servers") {
            bail!("mcp_servers must be a map")
        }
        return Ok(BTreeMap::new());
    };
    servers
        .iter()
        .map(|(name, value)| {
            validate_mcp_server_name(name)?;
            let config = value.as_mapping().context("MCP server must be a map")?;
            Ok((name.clone(), parse_mcp_server(config)?))
        })
        .collect()
}

fn parse_mcp_server(config: &Mapping) -> Result<McpServerDefinition> {
    reject_unknown_mcp_fields(config)?;
    match string(config, "type").as_deref() {
        Some("http") => Ok(McpServerDefinition::Http {
            url: checked_url(&required(config, "url")?)?,
            headers: string_map(config, "headers")?,
        }),
        Some("stdio") | None if config.contains_key("command") => Ok(McpServerDefinition::Stdio {
            command: required(config, "command")?,
            args: strings(config, "args")?,
            env: string_map(config, "env")?,
        }),
        _ => bail!("unsupported MCP server type"),
    }
}

fn reject_unknown_mcp_fields(map: &Mapping) -> Result<()> {
    const SUPPORTED: &[&str] = &["type", "command", "args", "env", "url", "headers"];
    for key in map.keys() {
        if !SUPPORTED.contains(&key.as_str()) {
            bail!("unknown MCP server field {key:?}")
        }
    }
    Ok(())
}

fn checked_url(value: &str) -> Result<Url> {
    let url = Url::parse(value)?;
    validate_endpoint(&url)?;
    Ok(url)
}

fn string_map(map: &Mapping, key: &str) -> Result<BTreeMap<String, String>> {
    match map.get(key) {
        None => Ok(BTreeMap::new()),
        Some(Value::Mapping(values)) => values
            .iter()
            .map(|(name, value)| {
                Ok((
                    name.clone(),
                    value.as_str().context("map value must be a string")?.into(),
                ))
            })
            .collect(),
        Some(_) => bail!("{key} must be a map"),
    }
}

#[cfg(test)]
mod tests;
