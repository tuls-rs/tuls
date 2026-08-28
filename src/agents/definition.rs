use anyhow::{Context, Result, bail};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};
use url::{Host, Url};

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) enum ModelProviderKind {
    OpenAi,
    Anthropic,
    OpenRouter,
    Custom,
}
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) enum WireApi {
    Responses,
    AnthropicMessages,
}
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub(crate) struct ChildToolPolicy {
    /// Explicitly granted child MCP tools. Empty means no child tools are exposed.
    pub(crate) allow: BTreeSet<String>,
    /// Explicit denials. A denial always overrides an allow rule.
    pub(crate) deny: BTreeSet<String>,
}

impl ChildToolPolicy {
    pub(crate) fn new(
        allow: impl IntoIterator<Item = String>,
        deny: impl IntoIterator<Item = String>,
    ) -> Result<Self> {
        let allow = allow.into_iter().collect::<BTreeSet<_>>();
        let deny = deny.into_iter().collect::<BTreeSet<_>>();
        for selector in allow.iter().chain(deny.iter()) {
            validate_child_tool_selector(selector)?;
        }
        let policy = Self { allow, deny };
        for selector in &policy.deny {
            let Some((server, _)) = selector.split_once('/') else {
                continue;
            };
            if !policy.may_allow_server(server) {
                bail!("deny selector {selector:?} has no matching allow grant")
            }
        }
        Ok(policy)
    }

    pub(crate) fn validate_servers<'a>(
        &self,
        servers: impl IntoIterator<Item = &'a str>,
    ) -> Result<()> {
        let configured = servers.into_iter().collect::<BTreeSet<_>>();
        for selector in self.allow.iter().chain(self.deny.iter()) {
            let Some((server, _)) = selector.split_once('/') else {
                continue;
            };
            if !configured.contains(server) {
                bail!("child tool selector references unknown MCP server {server:?}")
            }
        }
        Ok(())
    }

    pub(crate) fn validate_catalog<'a>(
        &self,
        server: &str,
        tools: impl IntoIterator<Item = &'a str>,
    ) -> Result<()> {
        let tools = tools.into_iter().collect::<BTreeSet<_>>();
        let prefix = format!("{server}/");
        for selector in self
            .allow
            .iter()
            .chain(self.deny.iter())
            .filter(|selector| selector.starts_with(&prefix))
        {
            let Some((_, tool)) = selector.split_once('/') else {
                continue;
            };
            if tool != "*" && !tools.contains(tool) {
                bail!("child tool selector references unavailable tool {selector:?}")
            }
        }
        Ok(())
    }

    pub(crate) fn allows(&self, server: &str, tool: &str) -> bool {
        let exact = format!("{server}/{tool}");
        let server_all = format!("{server}/*");
        if self.deny.contains(&exact) || self.deny.contains(&server_all) {
            return false;
        }
        self.allow.contains(&exact) || self.allow.contains(&server_all)
    }

    pub(crate) fn may_allow_server(&self, server: &str) -> bool {
        let prefix = format!("{server}/");
        self.allow
            .iter()
            .any(|selector| selector.starts_with(&prefix))
    }
}

pub(crate) fn validate_mcp_server_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 64 {
        bail!("MCP server name must be 1–64 characters")
    }
    if name.bytes().any(|byte| {
        !(byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'-')
    }) {
        bail!("invalid MCP server name {name:?}")
    }
    Ok(())
}

fn validate_child_tool_selector(selector: &str) -> Result<()> {
    let Some((server, tool)) = selector.split_once('/') else {
        bail!("child tool selector must use server/tool syntax")
    };
    validate_mcp_server_name(server)?;
    if tool.is_empty() {
        bail!("invalid child tool selector {selector:?}")
    }
    if tool == "*" || !tool.chars().any(char::is_whitespace) {
        Ok(())
    } else {
        bail!("invalid child tool selector {selector:?}")
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) enum McpServerDefinition {
    Stdio {
        command: String,
        args: Vec<String>,
        env: BTreeMap<String, String>,
    },
    Http {
        url: Url,
        headers: BTreeMap<String, String>,
    },
}
#[derive(Clone, PartialEq, Debug)]
pub(crate) struct AgentDefinition {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) instructions: String,
    pub(crate) model: String,
    pub(crate) base_url: Url,
    pub(crate) env_key: String,
    pub(crate) wire_api: WireApi,
    pub(crate) reasoning_effort: Option<String>,
    pub(crate) temperature: Option<f32>,
    pub(crate) max_turns: u32,
    pub(crate) tool_policy: ChildToolPolicy,
    pub(crate) skills: Vec<String>,
    pub(crate) mcp_servers: BTreeMap<String, McpServerDefinition>,
    pub(crate) source_path: PathBuf,
}
pub(crate) const DEFAULT_MAX_TURNS: u32 = 32;
pub(crate) const MAX_TURNS: u32 = 128;
pub(crate) const MAX_SPAWN_TASK_BYTES: usize = 256 * 1024;
pub(crate) const MAX_SEND_MESSAGE_BYTES: usize = 256 * 1024;
pub(crate) const MAX_WAIT_TARGETS: usize = 64;
pub(crate) const MAX_DISCOVERED_AGENTS: usize = 256;
pub(crate) const MAX_AGENT_FILE_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_AGENT_CATALOG_BYTES: usize = 64 * 1024;
pub(crate) const MAX_SKILLS: usize = 32;
pub(crate) const MAX_BUILT_CONTEXT_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_AGENT_DESCRIPTION_BYTES: usize = 4096;
pub(crate) fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 64 {
        bail!("agent name must be 1–64 characters")
    }
    let mut separator = false;
    for (i, byte) in name.bytes().enumerate() {
        if byte.is_ascii_lowercase() || byte.is_ascii_digit() {
            separator = false;
        } else if (byte == b'_' || byte == b'-') && i != 0 && !separator {
            separator = true;
        } else {
            bail!("invalid agent name {name:?}")
        }
    }
    if separator {
        bail!("invalid agent name {name:?}")
    }
    Ok(())
}

pub(crate) fn validate_claude_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 64 {
        bail!("Claude agent name must be 1–64 characters")
    }
    if name.starts_with('-')
        || name
            .bytes()
            .any(|byte| !(byte.is_ascii_lowercase() || byte == b'-'))
    {
        bail!("invalid Claude agent name {name:?}")
    }
    Ok(())
}

pub(crate) fn validate_endpoint(url: &Url) -> Result<()> {
    if url.cannot_be_a_base() || url.host().is_none() {
        bail!("endpoint must be an absolute URL with a host")
    }
    if !url.username().is_empty() || url.password().is_some() {
        bail!("endpoint credentials must not be embedded in the URL")
    }
    if url.scheme() == "https" {
        return Ok(());
    }
    let loopback = matches!(url.host(), Some(Host::Domain(h)) if h.eq_ignore_ascii_case("localhost"))
        || matches!(url.host(), Some(Host::Ipv4(ip)) if ip.is_loopback())
        || matches!(url.host(), Some(Host::Ipv6(ip)) if ip.is_loopback());
    if url.scheme() == "http" && loopback {
        Ok(())
    } else {
        bail!("endpoint must use HTTPS (HTTP is allowed only for loopback)")
    }
}
pub(crate) fn parse_provider(value: &str) -> Result<ModelProviderKind> {
    match value {
        "openai" => Ok(ModelProviderKind::OpenAi),
        "anthropic" => Ok(ModelProviderKind::Anthropic),
        "openrouter" => Ok(ModelProviderKind::OpenRouter),
        "custom" => Ok(ModelProviderKind::Custom),
        _ => bail!("unsupported model provider {value:?}"),
    }
}
pub(crate) fn parse_wire_api(value: &str) -> Result<WireApi> {
    match value {
        "responses" => Ok(WireApi::Responses),
        "anthropic-messages" => Ok(WireApi::AnthropicMessages),
        _ => bail!("unsupported wire API {value:?}"),
    }
}
pub(crate) fn defaults_for(provider: &ModelProviderKind) -> Result<(Url, String, WireApi)> {
    match provider {
        ModelProviderKind::OpenAi => Ok((
            Url::parse("https://api.openai.com/v1")?,
            "OPENAI_API_KEY".into(),
            WireApi::Responses,
        )),
        ModelProviderKind::Anthropic => Ok((
            Url::parse("https://api.anthropic.com")?,
            "ANTHROPIC_API_KEY".into(),
            WireApi::AnthropicMessages,
        )),
        ModelProviderKind::OpenRouter => Ok((
            Url::parse("https://openrouter.ai/api/v1")?,
            "OPENROUTER_API_KEY".into(),
            WireApi::Responses,
        )),
        ModelProviderKind::Custom => {
            bail!("custom provider requires explicit base_url, env_key, and wire_api")
        }
    }
}
pub(crate) fn validate_provider_wire(provider: &ModelProviderKind, wire: &WireApi) -> Result<()> {
    match (provider, wire) {
        (ModelProviderKind::OpenAi, WireApi::Responses)
        | (ModelProviderKind::Anthropic, WireApi::AnthropicMessages)
        | (ModelProviderKind::OpenRouter, WireApi::Responses)
        | (ModelProviderKind::Custom, _) => Ok(()),
        _ => bail!("provider and wire_api are inconsistent"),
    }
}
pub(crate) fn resolve_endpoint(
    provider: &ModelProviderKind,
    base: Option<&str>,
    env: Option<&str>,
    wire: Option<&str>,
) -> Result<(Url, String, WireApi)> {
    if matches!(provider, ModelProviderKind::Custom) {
        let url = Url::parse(base.context("custom provider requires base_url")?)?;
        validate_endpoint(&url)?;
        let key = env.context("custom provider requires env_key")?.to_owned();
        validate_env_key(&key)?;
        return Ok((
            url,
            key,
            parse_wire_api(wire.context("custom provider requires wire_api")?)?,
        ));
    }
    if base.is_some() || env.is_some() || wire.is_some() {
        bail!(
            "{provider:?} is a first-class provider and does not support base_url, env_key, or \
             wire_api overrides; configure model_provider = \"custom\" for custom endpoints"
        );
    }
    let (url, key, api) = defaults_for(provider)?;
    validate_provider_wire(provider, &api)?;
    Ok((url, key, api))
}
pub(crate) fn validate_skills(skills: &[String]) -> Result<()> {
    if skills.len() > MAX_SKILLS {
        bail!("an agent may reference at most {MAX_SKILLS} skills")
    }
    if let Some(empty) = skills.iter().find(|skill| skill.trim().is_empty()) {
        bail!("skill reference {empty:?} must be nonempty")
    }
    Ok(())
}
pub(crate) fn validate_instructions(instructions: &str) -> Result<()> {
    if instructions.len() > MAX_BUILT_CONTEXT_BYTES {
        bail!("agent instructions exceed the {MAX_BUILT_CONTEXT_BYTES}-byte context limit")
    }
    Ok(())
}
pub(crate) fn validate_description(description: &str) -> Result<()> {
    if description.trim().is_empty() || description.len() > MAX_AGENT_DESCRIPTION_BYTES {
        bail!("agent description must contain 1 to {MAX_AGENT_DESCRIPTION_BYTES} bytes")
    }
    Ok(())
}
pub(crate) fn validate_env_key(value: &str) -> Result<()> {
    let mut bytes = value.bytes();
    let Some(first) = bytes.next() else {
        bail!("env_key must be a nonempty environment variable identifier")
    };
    if !(first.is_ascii_alphabetic() || first == b'_')
        || !bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        bail!("env_key must be a nonempty environment variable identifier")
    }
    Ok(())
}
pub(crate) fn normalize_reasoning_effort(value: &str, wire: &WireApi) -> Result<String> {
    let value = value.trim().to_ascii_lowercase();
    let supported = match wire {
        WireApi::Responses => matches!(
            value.as_str(),
            "none" | "minimal" | "low" | "medium" | "high" | "xhigh"
        ),
        WireApi::AnthropicMessages => {
            matches!(value.as_str(), "low" | "medium" | "high" | "xhigh" | "max")
        }
    };
    if supported {
        Ok(value)
    } else {
        bail!("reasoning effort is not supported by the selected wire API")
    }
}
pub(crate) fn validate_temperature(value: f32, wire: &WireApi) -> Result<()> {
    if !value.is_finite() {
        bail!("temperature must be finite")
    }
    let max = if matches!(wire, WireApi::Responses) {
        2.0
    } else {
        1.0
    };
    if !(0.0..=max).contains(&value) {
        bail!("temperature is outside the range supported by the selected wire API")
    }
    Ok(())
}

#[cfg(test)]
mod tests;
