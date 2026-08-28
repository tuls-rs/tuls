mod graph;

use std::path::PathBuf;
use std::sync::Arc;

use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler,
    handler::server::router::tool::ToolRouter,
    handler::server::wrapper::Parameters,
    model::{
        CallToolResult, Implementation, ListResourcesResult, PaginatedRequestParams,
        ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult, Resource,
        ResourceContents, ResourceUpdatedNotification, ResourceUpdatedNotificationParam,
        ServerCapabilities, ServerInfo, ServerNotification, SubscriptionFilter,
    },
    schemars,
    service::{RequestContext, SubscriptionContext},
    tool, tool_handler, tool_router,
};
use serde::Deserialize;
use tokio::sync::broadcast;

use crate::cli::MemoryOptions;
use crate::policy::{Capability, ToolPolicy, ToolSpec};
use crate::support::{MAX_TOOL_RESULT_BYTES, SPEC_VERSION, tool_error};

use self::graph::{
    AddedObservation, Entity, KnowledgeGraph, KnowledgeGraphManager, ObservationInput, Relation,
};

pub const RESOURCE_URI: &str = "memory://knowledge-graph";

/// Environment variable that overrides the memory file location.
pub const MEMORY_FILE_PATH_ENV: &str = "MEMORY_FILE_PATH";

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[schemars(description = "Arguments for creating entities")]
pub struct CreateEntitiesArgs {
    #[schemars(description = "Entities to create")]
    pub entities: Vec<Entity>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[schemars(description = "Arguments for creating relations")]
pub struct CreateRelationsArgs {
    #[schemars(description = "Relations to create")]
    pub relations: Vec<Relation>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[schemars(description = "Arguments for adding observations")]
pub struct AddObservationsArgs {
    #[schemars(description = "Observations to add to existing entities")]
    pub observations: Vec<ObservationInput>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[schemars(description = "Arguments for deleting entities")]
pub struct DeleteEntitiesArgs {
    /// An array of entity names to delete
    #[schemars(description = "An array of entity names to delete")]
    pub entity_names: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[schemars(description = "An observation deletion")]
pub struct ObservationDeletion {
    /// The name of the entity containing the observations
    #[schemars(description = "The name of the entity containing the observations")]
    pub entity_name: String,
    /// An array of observations to delete
    #[schemars(description = "An array of observations to delete")]
    pub observations: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[schemars(description = "Arguments for deleting observations")]
pub struct DeleteObservationsArgs {
    #[schemars(description = "Observations to delete from entities")]
    pub deletions: Vec<ObservationDeletion>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[schemars(description = "Arguments for deleting relations")]
pub struct DeleteRelationsArgs {
    #[schemars(description = "Relations to delete")]
    pub relations: Vec<Relation>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[schemars(description = "Arguments for searching the knowledge graph")]
pub struct SearchNodesArgs {
    /// The search query to match against entity names, types, and observation content
    #[schemars(
        description = "The search query to match against entity names, types, and observation content"
    )]
    pub query: String,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[schemars(description = "Arguments for opening nodes")]
pub struct OpenNodesArgs {
    /// An array of entity names to retrieve
    #[schemars(description = "An array of entity names to retrieve")]
    pub names: Vec<String>,
}

// Server

const TOOL_SPECS: &[ToolSpec] = &[
    ToolSpec::new("memory", "create_entities", Capability::MemoryWrite),
    ToolSpec::new("memory", "create_relations", Capability::MemoryWrite),
    ToolSpec::new("memory", "add_observations", Capability::MemoryWrite),
    ToolSpec::new("memory", "delete_entities", Capability::MemoryWrite),
    ToolSpec::new("memory", "delete_observations", Capability::MemoryWrite),
    ToolSpec::new("memory", "delete_relations", Capability::MemoryWrite),
    ToolSpec::new("memory", "read_graph", Capability::MemoryRead),
    ToolSpec::new("memory", "search_nodes", Capability::MemoryRead),
    ToolSpec::new("memory", "open_nodes", Capability::MemoryRead),
];

pub struct MemoryServer {
    manager: Arc<KnowledgeGraphManager>,
    notify_tx: broadcast::Sender<()>,
    tool_router: ToolRouter<MemoryServer>,
    resources_enabled: bool,
}

impl MemoryServer {
    pub fn new(manager: KnowledgeGraphManager, policy: ToolPolicy) -> Self {
        let (notify_tx, _) = broadcast::channel(16);
        let resources_enabled = TOOL_SPECS
            .iter()
            .copied()
            .find(|spec| spec.name == "read_graph")
            .is_some_and(|spec| policy.allows_auxiliary_surface(Capability::MemoryRead, spec));
        let mut tool_router = Self::tool_router();
        for spec in TOOL_SPECS {
            if !policy.allows(*spec) {
                tool_router.disable_route(spec.name);
            }
        }
        Self {
            manager: Arc::new(manager),
            notify_tx,
            tool_router,
            resources_enabled,
        }
    }

    /// Notify subscribed clients (`subscriptions/listen` and
    /// `resources/subscribe`) that the graph changed.
    async fn notify_graph_updated(&self) {
        let _ = self.notify_tx.send(());
    }

    fn structured(value: serde_json::Value) -> CallToolResult {
        match serde_json::to_vec(&value) {
            Ok(serialized) if serialized.len() <= MAX_TOOL_RESULT_BYTES => {
                CallToolResult::structured(value)
            }
            Ok(_) => tool_error(format!(
                "memory result exceeds the {} byte tool-result limit; use search_nodes or open_nodes",
                MAX_TOOL_RESULT_BYTES
            )),
            Err(error) => tool_error(format!("Failed to serialize memory result: {error}")),
        }
    }
}

#[tool_router(router = tool_router)]
impl MemoryServer {
    #[tool(
        name = "create_entities",
        title = "Create Entities",
        description = "Create multiple new entities in the knowledge graph",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn create_entities(
        &self,
        Parameters(args): Parameters<CreateEntitiesArgs>,
    ) -> Result<CallToolResult, McpError> {
        let result = match self.manager.create_entities(args.entities).await {
            Ok(result) => result,
            Err(e) => return Ok(tool_error(e)),
        };
        if !result.is_empty() {
            self.notify_graph_updated().await;
        }
        Ok(Self::structured(serde_json::json!({ "entities": result })))
    }

    #[tool(
        name = "create_relations",
        title = "Create Relations",
        description = "Create multiple new relations between entities in the knowledge graph. Relations should be in active voice",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn create_relations(
        &self,
        Parameters(args): Parameters<CreateRelationsArgs>,
    ) -> Result<CallToolResult, McpError> {
        let result = match self.manager.create_relations(args.relations).await {
            Ok(result) => result,
            Err(e) => return Ok(tool_error(e)),
        };
        if !result.is_empty() {
            self.notify_graph_updated().await;
        }
        Ok(Self::structured(serde_json::json!({ "relations": result })))
    }

    #[tool(
        name = "add_observations",
        title = "Add Observations",
        description = "Add new observations to existing entities in the knowledge graph",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn add_observations(
        &self,
        Parameters(args): Parameters<AddObservationsArgs>,
    ) -> Result<CallToolResult, McpError> {
        let result: Vec<AddedObservation> =
            match self.manager.add_observations(args.observations).await {
                Ok(result) => result,
                Err(e) => return Ok(tool_error(e)),
            };
        if result
            .iter()
            .any(|observation| !observation.added_observations.is_empty())
        {
            self.notify_graph_updated().await;
        }
        Ok(Self::structured(serde_json::json!({ "results": result })))
    }

    #[tool(
        name = "delete_entities",
        title = "Delete Entities",
        description = "Delete multiple entities and their associated relations from the knowledge graph",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn delete_entities(
        &self,
        Parameters(args): Parameters<DeleteEntitiesArgs>,
    ) -> Result<CallToolResult, McpError> {
        let changed = match self.manager.delete_entities(args.entity_names).await {
            Ok(changed) => changed,
            Err(e) => return Ok(tool_error(e)),
        };
        if changed {
            self.notify_graph_updated().await;
        }
        let message = if changed {
            "Entities deleted successfully"
        } else {
            "No entities matched; nothing was deleted"
        };
        Ok(Self::structured(serde_json::json!({
            "success": true,
            "changed": changed,
            "message": message
        })))
    }

    #[tool(
        name = "delete_observations",
        title = "Delete Observations",
        description = "Delete specific observations from entities in the knowledge graph",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn delete_observations(
        &self,
        Parameters(args): Parameters<DeleteObservationsArgs>,
    ) -> Result<CallToolResult, McpError> {
        let changed = match self.manager.delete_observations(args.deletions).await {
            Ok(changed) => changed,
            Err(e) => return Ok(tool_error(e)),
        };
        if changed {
            self.notify_graph_updated().await;
        }
        let message = if changed {
            "Observations deleted successfully"
        } else {
            "No observations matched; nothing was deleted"
        };
        Ok(Self::structured(serde_json::json!({
            "success": true,
            "changed": changed,
            "message": message
        })))
    }

    #[tool(
        name = "delete_relations",
        title = "Delete Relations",
        description = "Delete multiple relations from the knowledge graph",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn delete_relations(
        &self,
        Parameters(args): Parameters<DeleteRelationsArgs>,
    ) -> Result<CallToolResult, McpError> {
        let changed = match self.manager.delete_relations(args.relations).await {
            Ok(changed) => changed,
            Err(e) => return Ok(tool_error(e)),
        };
        if changed {
            self.notify_graph_updated().await;
        }
        let message = if changed {
            "Relations deleted successfully"
        } else {
            "No relations matched; nothing was deleted"
        };
        Ok(Self::structured(serde_json::json!({
            "success": true,
            "changed": changed,
            "message": message
        })))
    }

    #[tool(
        name = "read_graph",
        title = "Read Graph",
        description = "Read the entire knowledge graph",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn read_graph(&self) -> Result<CallToolResult, McpError> {
        let graph = match self.manager.read_graph().await {
            Ok(graph) => graph,
            Err(e) => return Ok(tool_error(e)),
        };
        Ok(Self::structured(serde_json::to_value(&graph).map_err(
            |e| McpError::internal_error(format!("Failed to serialize graph: {e}"), None),
        )?))
    }

    #[tool(
        name = "search_nodes",
        title = "Search Nodes",
        description = "Search for nodes in the knowledge graph based on a query",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn search_nodes(
        &self,
        Parameters(args): Parameters<SearchNodesArgs>,
    ) -> Result<CallToolResult, McpError> {
        let graph = match self.manager.search_nodes(&args.query).await {
            Ok(graph) => graph,
            Err(e) => return Ok(tool_error(e)),
        };
        Ok(Self::structured(serde_json::to_value(&graph).map_err(
            |e| McpError::internal_error(format!("Failed to serialize graph: {e}"), None),
        )?))
    }

    #[tool(
        name = "open_nodes",
        title = "Open Nodes",
        description = "Open specific nodes in the knowledge graph by their names",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn open_nodes(
        &self,
        Parameters(args): Parameters<OpenNodesArgs>,
    ) -> Result<CallToolResult, McpError> {
        let graph = match self.manager.open_nodes(args.names).await {
            Ok(graph) => graph,
            Err(e) => return Ok(tool_error(e)),
        };
        Ok(Self::structured(serde_json::to_value(&graph).map_err(
            |e| McpError::internal_error(format!("Failed to serialize graph: {e}"), None),
        )?))
    }
}

fn resource_text(graph: &KnowledgeGraph) -> Result<String, String> {
    let text = serde_json::to_string_pretty(graph)
        .map_err(|error| format!("Failed to serialize graph: {error}"))?;
    if text.len() > MAX_TOOL_RESULT_BYTES {
        return Err(format!(
            "knowledge graph resource exceeds the {} byte response limit; use search_nodes or open_nodes",
            MAX_TOOL_RESULT_BYTES
        ));
    }
    Ok(text)
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for MemoryServer {
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
        let capabilities = match (
            !self.tool_router.list_all().is_empty(),
            self.resources_enabled,
        ) {
            (true, true) => ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .enable_resources_subscribe()
                .build(),
            (true, false) => ServerCapabilities::builder().enable_tools().build(),
            (false, true) => ServerCapabilities::builder()
                .enable_resources()
                .enable_resources_subscribe()
                .build(),
            (false, false) => ServerCapabilities::builder().build(),
        };
        ServerInfo::new(capabilities)
            .with_server_info(Implementation::new(
                "tuls-memory",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(
                "This server provides persistent memory as a knowledge graph of entities, \
             relations, and observations. Use create_entities, create_relations and \
             add_observations to record information, read_graph to retrieve it, and \
             search_nodes or open_nodes to find specific entries.",
            )
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        let resources = if self.resources_enabled {
            vec![
                Resource::new(RESOURCE_URI, "knowledge-graph")
                    .with_description("The full knowledge graph with all entities and relations")
                    .with_mime_type("application/json"),
            ]
        } else {
            Vec::new()
        };
        Ok(ListResourcesResult {
            resources,
            ..Default::default()
        })
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, McpError> {
        if !self.resources_enabled {
            return Err(McpError::invalid_params(
                "memory resource access is disabled",
                None,
            ));
        }
        if request.uri != RESOURCE_URI {
            return Err(McpError::invalid_params(
                format!("Unknown resource URI: {}", request.uri),
                None,
            ));
        }
        let graph = self
            .manager
            .read_graph()
            .await
            .map_err(|e| McpError::internal_error(e, None))?;
        Ok(ReadResourceResponse::Complete(ReadResourceResult::new(
            vec![ResourceContents::TextResourceContents {
                uri: RESOURCE_URI.to_string(),
                mime_type: Some("application/json".to_string()),
                text: resource_text(&graph)
                    .map_err(|error| McpError::internal_error(error, None))?,
                meta: None,
            }],
        )))
    }

    /// Modern (2026-07-28) subscription flow: accept updates for the
    /// knowledge-graph resource URI.
    fn accepted_subscription_filter(
        &self,
        requested: &SubscriptionFilter,
    ) -> Option<SubscriptionFilter> {
        if !self.resources_enabled {
            return None;
        }
        let Some(uris) = &requested.resource_subscriptions else {
            return None;
        };
        let accepted: Vec<String> = uris
            .iter()
            .filter(|uri| uri.as_str() == RESOURCE_URI)
            .cloned()
            .collect();
        if accepted.is_empty() {
            None
        } else {
            Some(
                SubscriptionFilter::builder()
                    .resource_subscriptions(accepted)
                    .build(),
            )
        }
    }

    /// Hold the subscription open and forward graph-change notifications to
    /// the client whenever a mutation tool modifies the graph.
    async fn listen(&self, context: SubscriptionContext) -> Result<(), McpError> {
        let sink = context.sink().clone();
        let mut rx = self.notify_tx.subscribe();
        loop {
            tokio::select! {
                _ = context.cancelled() => return Ok(()),
                _ = rx.recv() => {
                    if let Err(error) = sink
                        .send(ServerNotification::ResourceUpdatedNotification(
                            ResourceUpdatedNotification::new(
                                ResourceUpdatedNotificationParam::new(RESOURCE_URI),
                            ),
                        ))
                        .await
                    {
                        return Err(McpError::internal_error(error.to_string(), None));
                    }
                }
            }
        }
    }
}

/// Resolve the memory file path: `--memory-file` option wins, then the
/// `MEMORY_FILE_PATH` environment variable, otherwise `memory.jsonl` in the
/// current working directory.
pub fn resolve_memory_file_path(cli_path: Option<PathBuf>) -> Result<PathBuf, String> {
    let path = match cli_path {
        Some(path) => path,
        None => match std::env::var_os(MEMORY_FILE_PATH_ENV) {
            Some(path) => PathBuf::from(path),
            None => PathBuf::from("memory.jsonl"),
        },
    };
    if path.is_absolute() {
        Ok(path)
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .map_err(|e| format!("Failed to resolve memory file path: {e}"))
    }
}

/// Start the memory server on stdio.
pub async fn run(options: MemoryOptions) -> anyhow::Result<()> {
    use rmcp::ServiceExt;
    use rmcp::transport::stdio;

    let path = resolve_memory_file_path(options.memory_file).map_err(|e| anyhow::anyhow!(e))?;
    let manager = KnowledgeGraphManager::new(path.clone());
    let policy = ToolPolicy::from_selectors(&options.tools.allow, &options.tools.deny, TOOL_SPECS)
        .map_err(anyhow::Error::msg)?;
    let server = MemoryServer::new(manager, policy);

    let service = server
        .serve(stdio())
        .await
        .inspect_err(|e| tracing::error!("serving error: {e:?}"))?;
    tracing::info!(
        "Memory MCP server running on stdio (MCP {SPEC_VERSION}), memory file: {}",
        path.display()
    );

    service.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests;
