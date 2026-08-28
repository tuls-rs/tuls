use crate::agents::definition::*;
use anyhow::{Context, Result, bail};
use noyalib::{Mapping, Value};
use std::{collections::BTreeMap, path::PathBuf};
use url::Url;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum MarkdownFlavor {
    Canonical,
    Claude,
}

pub(crate) fn parse_markdown(
    path: PathBuf,
    input: &str,
    flavor: MarkdownFlavor,
) -> Result<AgentDefinition> {
    if input.len() > MAX_AGENT_FILE_BYTES {
        bail!("agent markdown exceeds 1 MiB")
    }
    let (frontmatter, instructions) = split_frontmatter(input)?;
    let map: Mapping = noyalib::from_str(frontmatter).context("invalid agent YAML frontmatter")?;
    reject_provider_secrets(&map)?;
    match flavor {
        MarkdownFlavor::Canonical => reject_unknown_canonical_fields(&map)?,
        MarkdownFlavor::Claude => validate_claude_fields(&map)?,
    }
    validate_supported_field_types(&map, flavor)?;

    let name = required(&map, "name")?;
    match flavor {
        MarkdownFlavor::Canonical => validate_name(&name)?,
        MarkdownFlavor::Claude => validate_claude_name(&name)?,
    }
    let description = required(&map, "description")?;
    validate_description(&description)?;
    validate_instructions(instructions)?;
    if instructions.trim().is_empty() {
        bail!("agent instructions must be nonempty")
    }

    let (model, base_url, env_key, wire_api) = match flavor {
        MarkdownFlavor::Canonical => {
            let provider = parse_provider(&required(&map, "model_provider")?)?;
            let (base_url, env_key, wire_api) = resolve_endpoint(
                &provider,
                string(&map, "base_url").as_deref(),
                string(&map, "env_key").as_deref(),
                string(&map, "wire_api").as_deref(),
            )?;
            (required(&map, "model")?, base_url, env_key, wire_api)
        }
        MarkdownFlavor::Claude => {
            let provider = ModelProviderKind::Anthropic;
            let (base_url, env_key, wire_api) = defaults_for(&provider)?;
            let environment = ClaudeModelEnvironment::from_process();
            let model = resolve_claude_model(string(&map, "model").as_deref(), &environment)?;
            (model, base_url, env_key, wire_api)
        }
    };

    let temperature = match flavor {
        MarkdownFlavor::Canonical => number(&map, "temperature"),
        MarkdownFlavor::Claude => None,
    };
    if let Some(value) = temperature {
        validate_temperature(value, &wire_api)?;
    }
    let reasoning_effort = match flavor {
        MarkdownFlavor::Canonical => string(&map, "reasoning_effort"),
        MarkdownFlavor::Claude => string(&map, "effort"),
    }
    .map(|value| normalize_reasoning_effort(&value, &wire_api))
    .transpose()?;
    let max_turns = match flavor {
        MarkdownFlavor::Canonical => integer(&map, "max_turns"),
        MarkdownFlavor::Claude => integer(&map, "maxTurns"),
    }
    .unwrap_or(DEFAULT_MAX_TURNS as i64);
    if !(1..=MAX_TURNS as i64).contains(&max_turns) {
        bail!("max_turns must be between 1 and {MAX_TURNS}")
    }

    let tool_policy = tool_policy(&map, flavor)?;
    let mcp_servers = mcp_servers(&map, flavor)?;
    tool_policy.validate_servers(mcp_servers.keys().map(String::as_str))?;
    let skills = strings(&map, "skills")?;
    validate_skills(&skills)?;

    Ok(AgentDefinition {
        name,
        description,
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

fn validate_supported_field_types(map: &Mapping, flavor: MarkdownFlavor) -> Result<()> {
    let string_fields: &[&str] = match flavor {
        MarkdownFlavor::Canonical => &[
            "name",
            "description",
            "model_provider",
            "model",
            "base_url",
            "env_key",
            "wire_api",
            "reasoning_effort",
        ],
        MarkdownFlavor::Claude => &[
            "name",
            "description",
            "model",
            "effort",
            "color",
            "initialPrompt",
        ],
    };
    for key in string_fields {
        if map.contains_key(key) && map.get(key).and_then(Value::as_str).is_none() {
            bail!("{key} must be a string")
        }
    }

    let max_turns_key = match flavor {
        MarkdownFlavor::Canonical => "max_turns",
        MarkdownFlavor::Claude => "maxTurns",
    };
    if map.contains_key(max_turns_key) && map.get(max_turns_key).and_then(Value::as_i64).is_none() {
        bail!("{max_turns_key} must be an integer")
    }
    if flavor == MarkdownFlavor::Canonical && map.contains_key("temperature") {
        let value = map.get("temperature").context("temperature is missing")?;
        if value.as_f64().is_none() && value.as_i64().is_none() {
            bail!("temperature must be a number")
        }
    }
    if flavor == MarkdownFlavor::Claude
        && map.contains_key("background")
        && map.get("background").and_then(Value::as_bool).is_none()
    {
        bail!("background must be a boolean")
    }
    if let Some(color) = (flavor == MarkdownFlavor::Claude)
        .then(|| string(map, "color"))
        .flatten()
    {
        const COLORS: &[&str] = &[
            "red", "blue", "green", "yellow", "purple", "orange", "pink", "cyan",
        ];
        if !COLORS.contains(&color.as_str()) {
            bail!("invalid Claude agent color {color:?}")
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

fn claude_tool_list(map: &Mapping, key: &str) -> Result<Vec<String>> {
    match map.get(key) {
        None => Ok(Vec::new()),
        Some(value) if value.as_str().is_some() => {
            let value = value.as_str().context("tool list must be a string")?;
            let values = value
                .split(',')
                .map(str::trim)
                .filter(|entry| !entry.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>();
            if values.is_empty() && !value.trim().is_empty() {
                bail!("{key} must contain at least one nonempty entry")
            }
            Ok(values)
        }
        Some(Value::Sequence(values)) => values
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_owned)
                    .context("tool list entries must be strings")
            })
            .collect(),
        Some(_) => bail!("{key} must be a comma-separated string or an array"),
    }
}

fn claude_mcp_selectors(map: &Mapping, key: &str) -> Result<Vec<String>> {
    claude_tool_list(map, key)?
        .into_iter()
        .filter(|entry| entry.starts_with("mcp__"))
        .map(|entry| canonicalize_claude_mcp_selector(&entry))
        .collect()
}

fn canonicalize_claude_mcp_selector(value: &str) -> Result<String> {
    let rest = value
        .strip_prefix("mcp__")
        .context("Claude MCP selector must start with mcp__")?;
    if let Some((server, tool)) = rest.split_once("__") {
        if server.is_empty() || tool.is_empty() {
            bail!("invalid Claude MCP selector {value:?}")
        }
        Ok(format!("{server}/{tool}"))
    } else if rest.is_empty() {
        bail!("invalid Claude MCP selector {value:?}")
    } else {
        Ok(format!("{rest}/*"))
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

fn tool_policy(map: &Mapping, flavor: MarkdownFlavor) -> Result<ChildToolPolicy> {
    match flavor {
        MarkdownFlavor::Canonical => {
            ChildToolPolicy::new(strings(map, "allow_tools")?, strings(map, "deny_tools")?)
        }
        MarkdownFlavor::Claude => ChildToolPolicy::new(
            claude_mcp_selectors(map, "tools")?,
            claude_mcp_selectors(map, "disallowedTools")?,
        ),
    }
}

fn reject_unknown_canonical_fields(map: &Mapping) -> Result<()> {
    const SUPPORTED: &[&str] = &[
        "name",
        "description",
        "model_provider",
        "model",
        "base_url",
        "env_key",
        "wire_api",
        "temperature",
        "reasoning_effort",
        "max_turns",
        "allow_tools",
        "deny_tools",
        "skills",
        "mcp_servers",
    ];
    for key in map.keys() {
        if !SUPPORTED.contains(&key.as_str()) {
            bail!("unknown canonical agent field {key:?}")
        }
    }
    Ok(())
}

fn validate_claude_fields(map: &Mapping) -> Result<()> {
    const SUPPORTED: &[&str] = &[
        "name",
        "description",
        "tools",
        "disallowedTools",
        "model",
        "maxTurns",
        "skills",
        "mcpServers",
        "effort",
    ];
    const NON_RUNTIME_FIELDS: &[&str] = &["background", "color", "initialPrompt"];
    const UNSUPPORTED_EXECUTION_FIELDS: &[&str] =
        &["hooks", "memory", "permissionMode", "isolation"];
    for key in map.keys() {
        if SUPPORTED.contains(&key.as_str()) {
            continue;
        }
        if NON_RUNTIME_FIELDS.contains(&key.as_str()) {
            continue;
        }
        if UNSUPPORTED_EXECUTION_FIELDS.contains(&key.as_str()) {
            bail!("Claude agent field {key:?} is not executable by tuls")
        }
        bail!("unknown Claude agent field {key:?}")
    }
    Ok(())
}

#[derive(Default)]
struct ClaudeModelEnvironment {
    subagent: Option<String>,
    inherited: Option<String>,
    fable: Option<String>,
    opus: Option<String>,
    sonnet: Option<String>,
    haiku: Option<String>,
}

impl ClaudeModelEnvironment {
    fn from_process() -> Self {
        Self {
            subagent: nonempty_env("CLAUDE_CODE_SUBAGENT_MODEL"),
            inherited: nonempty_env("ANTHROPIC_MODEL"),
            fable: nonempty_env("ANTHROPIC_DEFAULT_FABLE_MODEL"),
            opus: nonempty_env("ANTHROPIC_DEFAULT_OPUS_MODEL"),
            sonnet: nonempty_env("ANTHROPIC_DEFAULT_SONNET_MODEL"),
            haiku: nonempty_env("ANTHROPIC_DEFAULT_HAIKU_MODEL"),
        }
    }
}

fn nonempty_env(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn resolve_claude_model(
    frontmatter: Option<&str>,
    environment: &ClaudeModelEnvironment,
) -> Result<String> {
    let subagent = environment
        .subagent
        .as_deref()
        .filter(|value| !value.eq_ignore_ascii_case("inherit"));
    let requested = subagent.or(frontmatter).unwrap_or("inherit");
    resolve_claude_model_name(requested, environment, 0)
}

fn resolve_claude_model_name(
    requested: &str,
    environment: &ClaudeModelEnvironment,
    depth: usize,
) -> Result<String> {
    const MAX_ALIAS_DEPTH: usize = 4;
    if depth >= MAX_ALIAS_DEPTH {
        bail!("Claude model alias resolution is cyclic")
    }
    let requested = requested.trim();
    if requested.is_empty() {
        bail!("Claude model must be nonempty")
    }
    let mapped = match requested {
        "inherit" => environment.inherited.as_deref().with_context(|| {
            "standalone tuls cannot inherit a parent Claude model; set ANTHROPIC_MODEL, \
             CLAUDE_CODE_SUBAGENT_MODEL, or an explicit model"
        })?,
        "fable" => environment
            .fable
            .as_deref()
            .context("Claude model alias 'fable' requires ANTHROPIC_DEFAULT_FABLE_MODEL")?,
        "opus" => environment
            .opus
            .as_deref()
            .context("Claude model alias 'opus' requires ANTHROPIC_DEFAULT_OPUS_MODEL")?,
        "sonnet" => environment
            .sonnet
            .as_deref()
            .context("Claude model alias 'sonnet' requires ANTHROPIC_DEFAULT_SONNET_MODEL")?,
        "haiku" => environment
            .haiku
            .as_deref()
            .context("Claude model alias 'haiku' requires ANTHROPIC_DEFAULT_HAIKU_MODEL")?,
        _ => return Ok(requested.to_owned()),
    };
    resolve_claude_model_name(mapped, environment, depth + 1)
}

fn mcp_servers(
    map: &Mapping,
    flavor: MarkdownFlavor,
) -> Result<BTreeMap<String, McpServerDefinition>> {
    match flavor {
        MarkdownFlavor::Canonical => {
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
        MarkdownFlavor::Claude => {
            let Some(Value::Sequence(entries)) = map.get("mcpServers") else {
                if map.contains_key("mcpServers") {
                    bail!("mcpServers must be an array")
                }
                return Ok(BTreeMap::new());
            };
            let mut servers = BTreeMap::new();
            for entry in entries {
                if let Some(name) = entry.as_str() {
                    bail!(
                        "Claude MCP server reference {name:?} cannot be resolved by standalone tuls; use an inline server definition"
                    )
                }
                let wrapper = entry.as_mapping().context(
                    "each Claude mcpServers entry must be a server name or one inline map",
                )?;
                if wrapper.len() != 1 {
                    bail!("each inline Claude MCP server entry must contain exactly one server")
                }
                let (name, value) = wrapper
                    .iter()
                    .next()
                    .context("inline Claude MCP server entry must not be empty")?;
                validate_mcp_server_name(name)?;
                if servers.contains_key(name) {
                    bail!("duplicate Claude MCP server name {name:?}")
                }
                let config = value.as_mapping().context("MCP server must be a map")?;
                servers.insert(name.clone(), parse_mcp_server(config)?);
            }
            Ok(servers)
        }
    }
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
