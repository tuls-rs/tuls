use crate::agents::definition::*;
use anyhow::{Context, Result, bail};
use std::{collections::BTreeMap, path::PathBuf};
use toml::Table;
use url::Url;

pub(crate) fn parse_toml(path: PathBuf, input: &str) -> Result<AgentDefinition> {
    if input.len() > MAX_AGENT_FILE_BYTES {
        bail!("agent TOML exceeds 1 MiB")
    }

    let table: Table = toml::from_str(input).context("invalid agent TOML")?;
    reject_provider_secrets(&table)?;
    reject_unknown_fields(&table)?;

    let name = required(&table, "name")?;
    validate_name(&name)?;
    let description = required(&table, "description")?;
    validate_description(&description)?;
    let instructions = required(&table, "instructions")?;
    validate_instructions(&instructions)?;
    let model = required(&table, "model")?;
    let provider = parse_provider(&required(&table, "model_provider")?)?;

    let (base_url, env_key, wire_api) = resolve_endpoint(
        &provider,
        string(&table, "base_url")?.as_deref(),
        string(&table, "env_key")?.as_deref(),
        string(&table, "wire_api")?.as_deref(),
    )?;

    let temperature = number(&table, "temperature")?;
    if let Some(value) = temperature {
        validate_temperature(value, &wire_api)?;
    }

    let reasoning_effort = string(&table, "reasoning_effort")?
        .map(|value| normalize_reasoning_effort(&value, &wire_api))
        .transpose()?;

    let max_turns = integer(&table, "max_turns")?.unwrap_or(i64::from(DEFAULT_MAX_TURNS));
    if !(1..=i64::from(MAX_TURNS)).contains(&max_turns) {
        bail!("max_turns must be between 1 and {MAX_TURNS}")
    }

    let tool_policy = ChildToolPolicy::new(
        strings(&table, "allow_tools")?,
        strings(&table, "deny_tools")?,
    )?;
    let mcp_servers = mcp_servers(&table)?;
    tool_policy.validate_servers(mcp_servers.keys().map(String::as_str))?;
    let skills = strings(&table, "skills")?;
    validate_skills(&skills)?;

    Ok(AgentDefinition {
        name,
        description,
        instructions,
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

fn reject_unknown_fields(table: &Table) -> Result<()> {
    const SUPPORTED: &[&str] = &[
        "name",
        "description",
        "instructions",
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
    for key in table.keys() {
        if !SUPPORTED.contains(&key.as_str()) {
            bail!("unknown canonical agent field {key:?}")
        }
    }
    Ok(())
}

fn string(table: &Table, key: &str) -> Result<Option<String>> {
    match table.get(key) {
        None => Ok(None),
        Some(value) => value
            .as_str()
            .map(|value| Some(value.to_owned()))
            .with_context(|| format!("{key} must be a string")),
    }
}

fn required(table: &Table, key: &str) -> Result<String> {
    string(table, key)?
        .filter(|value| !value.trim().is_empty())
        .with_context(|| format!("{key} is required and must be nonempty"))
}

fn integer(table: &Table, key: &str) -> Result<Option<i64>> {
    match table.get(key) {
        None => Ok(None),
        Some(value) => value
            .as_integer()
            .map(Some)
            .with_context(|| format!("{key} must be an integer")),
    }
}

fn number(table: &Table, key: &str) -> Result<Option<f32>> {
    match table.get(key) {
        None => Ok(None),
        Some(value) => {
            let number = value
                .as_float()
                .or_else(|| value.as_integer().map(|integer| integer as f64))
                .with_context(|| format!("{key} must be a number"))?;
            if !number.is_finite() || number < f64::from(f32::MIN) || number > f64::from(f32::MAX) {
                bail!("{key} is outside the supported numeric range")
            }
            Ok(Some(number as f32))
        }
    }
}

fn strings(table: &Table, key: &str) -> Result<Vec<String>> {
    match table.get(key) {
        None => Ok(Vec::new()),
        Some(value) => value
            .as_array()
            .with_context(|| format!("{key} must be an array"))?
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_owned)
                    .context("array entries must be strings")
            })
            .collect(),
    }
}

fn reject_provider_secrets(table: &Table) -> Result<()> {
    for key in ["api_key", "apiKey", "token", "access_token", "accessToken"] {
        if table.contains_key(key) {
            bail!("literal secret field {key} is prohibited")
        }
    }
    if let Some(provider) = table.get("provider").and_then(|value| value.as_table()) {
        for key in ["api_key", "apiKey", "token", "access_token", "accessToken"] {
            if provider.contains_key(key) {
                bail!("literal provider secret field {key} is prohibited")
            }
        }
    }
    Ok(())
}

fn mcp_servers(table: &Table) -> Result<BTreeMap<String, McpServerDefinition>> {
    let Some(servers) = table.get("mcp_servers") else {
        return Ok(BTreeMap::new());
    };

    servers
        .as_table()
        .context("mcp_servers must be a table")?
        .iter()
        .map(|(name, value)| {
            validate_mcp_server_name(name)?;
            let config = value.as_table().context("MCP server must be a table")?;
            reject_unknown_mcp_fields(config)?;
            let server_type = string(config, "type")?;
            let server = match server_type.as_deref() {
                Some("http") => McpServerDefinition::Http {
                    url: checked_url(&required(config, "url")?)?,
                    headers: string_map(config, "headers")?,
                },
                Some("stdio") | None if config.contains_key("command") => {
                    McpServerDefinition::Stdio {
                        command: required(config, "command")?,
                        args: strings(config, "args")?,
                        env: string_map(config, "env")?,
                    }
                }
                _ => bail!("unsupported MCP server type"),
            };
            Ok((name.clone(), server))
        })
        .collect()
}

fn reject_unknown_mcp_fields(table: &Table) -> Result<()> {
    const SUPPORTED: &[&str] = &["type", "command", "args", "env", "url", "headers"];
    for key in table.keys() {
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

fn string_map(table: &Table, key: &str) -> Result<BTreeMap<String, String>> {
    match table.get(key) {
        None => Ok(BTreeMap::new()),
        Some(value) => value
            .as_table()
            .with_context(|| format!("{key} must be a table"))?
            .iter()
            .map(|(name, value)| {
                Ok((
                    name.clone(),
                    value
                        .as_str()
                        .context("map values must be strings")?
                        .to_owned(),
                ))
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests;
