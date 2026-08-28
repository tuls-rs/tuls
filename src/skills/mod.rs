mod discovery;
mod manifest;
mod parser;

use std::future;

use rmcp::{
    ErrorData as McpError, ServerHandler,
    model::{
        CallToolRequestMethod, CallToolRequestParams, CallToolResponse, CallToolResult,
        ContentBlock, Implementation, ListToolsResult, ServerCapabilities, ServerInfo, Tool,
        ToolAnnotations,
    },
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::cli::WorkspaceServerOptions;
use crate::policy::{Capability, ToolPolicy, ToolSpec};

pub(crate) use self::discovery::SkillRegistry;
use self::manifest::resource_manifest;

const TOOL_SPECS: &[ToolSpec] = &[ToolSpec::new(
    "skills",
    "activate_skill",
    Capability::SkillsRead,
)];
const MAX_ACTIVATED_SKILL_BYTES: usize = 1024 * 1024;

#[derive(Debug)]
pub(crate) struct SkillsServer {
    registry: SkillRegistry,
    tool_enabled: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ActivateArgs {
    name: String,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
#[schemars(rename_all = "camelCase")]
pub(crate) struct ActivatedSkill {
    name: String,
    description: String,
    skill_dir: String,
    instructions: String,
    resources: Vec<String>,
}

impl SkillsServer {
    pub(crate) fn new(workspace: std::path::PathBuf, policy: ToolPolicy) -> anyhow::Result<Self> {
        Ok(Self {
            registry: SkillRegistry::discover(workspace)?,
            tool_enabled: policy.allows(TOOL_SPECS[0]),
        })
    }

    fn activate(&self, name: &str) -> anyhow::Result<ActivatedSkill> {
        let skill = self
            .registry
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("unknown skill: {name}"))?;
        let parsed = self.registry.load(name)?;
        Ok(ActivatedSkill {
            name: parsed.name,
            description: parsed.description,
            skill_dir: skill.skill_dir.display().to_string(),
            instructions: parsed.instructions,
            resources: resource_manifest(&skill.skill_dir)?,
        })
    }

    fn activate_tool(&self) -> Tool {
        let names = self.registry.names().map(str::to_owned).collect::<Vec<_>>();
        let mut name_schema = Map::new();
        name_schema.insert("type".into(), Value::String("string".into()));
        name_schema.insert("enum".into(), json!(names));
        let mut properties = Map::new();
        properties.insert("name".into(), Value::Object(name_schema));
        let mut schema = Map::new();
        schema.insert("type".into(), Value::String("object".into()));
        schema.insert("properties".into(), Value::Object(properties));
        schema.insert("required".into(), json!(["name"]));
        schema.insert("additionalProperties".into(), Value::Bool(false));
        Tool::new("activate_skill", format!("Activate one available skill and load its full instructions. Supporting files are not loaded automatically; read only referenced files when needed.\n\nAvailable skills:\n{}", self.registry.catalog()), schema)
            .with_output_schema::<ActivatedSkill>()
            .with_annotations(ToolAnnotations::new().read_only(true).destructive(false).idempotent(true).open_world(false))
    }

    fn call_activate(&self, arguments: Option<rmcp::model::JsonObject>) -> CallToolResult {
        let args = arguments
            .map(Value::Object)
            .ok_or_else(|| anyhow::anyhow!("missing arguments"))
            .and_then(|value| serde_json::from_value::<ActivateArgs>(value).map_err(Into::into));
        match args.and_then(|args| self.activate(&args.name)) {
            Ok(activated) => match serde_json::to_vec(&activated) {
                Ok(bytes) if bytes.len() <= MAX_ACTIVATED_SKILL_BYTES => {
                    let value = match serde_json::from_slice(&bytes) {
                        Ok(value) => value,
                        Err(error) => {
                            return CallToolResult::error(vec![ContentBlock::text(format!(
                                "failed to serialize activated skill: {error}"
                            ))]);
                        }
                    };
                    let mut result = CallToolResult::structured(value);
                    result.content.push(ContentBlock::text(format!("Activated {}. Instructions and resource paths are available in structured content.", activated.name)));
                    result
                }
                Ok(_) => CallToolResult::error(vec![ContentBlock::text(format!(
                    "activated skill exceeds the {MAX_ACTIVATED_SKILL_BYTES}-byte output limit"
                ))]),
                Err(error) => CallToolResult::error(vec![ContentBlock::text(format!(
                    "failed to serialize activated skill: {error}"
                ))]),
            },
            Err(error) => CallToolResult::error(vec![ContentBlock::text(error.to_string())]),
        }
    }
}

impl ServerHandler for SkillsServer {
    fn initialize(
        &self,
        _request: rmcp::model::InitializeRequestParams,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> impl std::future::Future<Output = Result<rmcp::model::InitializeResult, McpError>>
    + rmcp::service::MaybeSendFuture
    + '_ {
        crate::support::reject_unsupported_initialize()
    }

    fn supported_protocol_versions(
        &self,
    ) -> std::borrow::Cow<'static, [rmcp::model::ProtocolVersion]> {
        std::borrow::Cow::Borrowed(crate::support::SUPPORTED_PROTOCOL_VERSIONS)
    }

    fn get_info(&self) -> ServerInfo {
        let capabilities = if !self.tool_enabled || self.registry.is_empty() {
            ServerCapabilities::builder().build()
        } else {
            ServerCapabilities::builder().enable_tools().build()
        };
        ServerInfo::new(capabilities)
            .with_server_info(Implementation::new("tuls-skills", env!("CARGO_PKG_VERSION")))
            .with_instructions("Activate an available skill to load its full instructions. Supporting files are not loaded automatically; read only referenced files when needed.")
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        (self.tool_enabled && !self.registry.is_empty() && name == "activate_skill")
            .then(|| self.activate_tool())
    }

    fn list_tools(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListToolsResult, McpError>>
    + rmcp::service::MaybeSendFuture
    + '_ {
        future::ready(Ok(ListToolsResult::with_all_items(
            self.get_tool("activate_skill").into_iter().collect(),
        )
        .with_ttl_ms(0)
        .with_cache_scope(rmcp::model::CacheScope::Private)))
    }

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> impl std::future::Future<Output = Result<CallToolResponse, McpError>>
    + rmcp::service::MaybeSendFuture
    + '_ {
        if self.tool_enabled && request.name == "activate_skill" && !self.registry.is_empty() {
            future::ready(Ok(self.call_activate(request.arguments).into()))
        } else {
            future::ready(Err(McpError::method_not_found::<CallToolRequestMethod>()))
        }
    }
}

/// Start the skills server for a workspace on stdio.
pub(crate) async fn run(options: WorkspaceServerOptions) -> anyhow::Result<()> {
    use rmcp::{ServiceExt, transport::stdio};
    let policy = ToolPolicy::from_selectors(&options.tools.allow, &options.tools.deny, TOOL_SPECS)
        .map_err(anyhow::Error::msg)?;
    let server = SkillsServer::new(options.dir, policy)?;
    let service = server
        .serve(stdio())
        .await
        .inspect_err(|e| tracing::error!("serving error: {e:?}"))?;
    service.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests;
