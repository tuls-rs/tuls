mod http;

use std::sync::Arc;

use rmcp::{
    ErrorData as McpError, ServerHandler,
    handler::server::router::prompt::PromptRouter,
    handler::server::router::tool::ToolRouter,
    handler::server::wrapper::Parameters,
    model::{
        CallToolResult, ContentBlock, GetPromptResult, Implementation, PromptMessage, Role,
        ServerCapabilities, ServerInfo,
    },
    prompt, prompt_handler, prompt_router, schemars, tool, tool_handler, tool_router,
};
use serde::Deserialize;

use crate::cli::{FetchOptions, RobotsPolicy};
use crate::policy::{Capability, ToolPolicy, ToolSpec};
use crate::support::{MAX_TOOL_RESULT_BYTES, SPEC_VERSION, tool_error, truncate_text};

use self::http::{
    DEFAULT_USER_AGENT, FetchClient, MAX_URL_CHARS, check_may_fetch, fetch_url, validate_http_url,
    validate_user_agent,
};

pub const MIN_FETCH_MAX_LENGTH: i64 = 1;
pub const MAX_FETCH_MAX_LENGTH: i64 = 50_000;

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[schemars(description = "Parameters for fetching a URL")]
pub struct FetchArgs {
    /// URL to fetch
    #[schemars(length(max = MAX_URL_CHARS))]
    pub url: String,
    /// Maximum number of characters to return
    #[serde(default = "default_max_length")]
    #[schemars(range(min = MIN_FETCH_MAX_LENGTH, max = MAX_FETCH_MAX_LENGTH))]
    pub max_length: i64,
    /// Start content from this character index, useful for continuing a
    /// truncated response and more context is required
    #[serde(default)]
    #[schemars(range(min = 0))]
    pub start_index: i64,
    /// Get the actual content of the requested page without simplification
    #[serde(default)]
    pub raw: bool,
}

fn default_max_length() -> i64 {
    5000
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[schemars(description = "Arguments for the fetch prompt")]
pub struct FetchPromptArgs {
    /// URL to fetch
    #[schemars(length(max = MAX_URL_CHARS))]
    pub url: String,
}

const TOOL_SPECS: &[ToolSpec] = &[ToolSpec::new("fetch", "fetch", Capability::NetworkFetch)];

pub struct FetchServer {
    client: Arc<FetchClient>,
    user_agent: String,
    respect_robots_txt: bool,
    tool_router: ToolRouter<FetchServer>,
    prompt_router: PromptRouter<FetchServer>,
}

impl FetchServer {
    pub fn new(
        client: FetchClient,
        user_agent: String,
        respect_robots_txt: bool,
        policy: ToolPolicy,
    ) -> Self {
        let mut tool_router = Self::tool_router();
        for spec in TOOL_SPECS {
            if !policy.allows(*spec) {
                tool_router.disable_route(spec.name);
            }
        }
        let mut prompt_router = Self::prompt_router();
        if !policy.allows_auxiliary_surface(Capability::NetworkFetch, TOOL_SPECS[0]) {
            prompt_router.remove_route("fetch");
        }
        Self {
            client: Arc::new(client),
            user_agent,
            respect_robots_txt,
            tool_router,
            prompt_router,
        }
    }
}

#[tool_router(router = tool_router)]
impl FetchServer {
    #[tool(
        name = "fetch",
        title = "Fetch",
        description = "Fetch an HTTP or HTTPS URL and optionally simplify HTML to markdown. The response body is bounded and redirects are not followed. When robots.txt enforcement is enabled, autonomous fetches fail closed on unavailable or redirected robots policies. For web search, fetch DuckDuckGo Lite results: https://lite.duckduckgo.com/lite/?q=<query>&kl=en-us&kp=0 (kl is the region, kp is the safe-search policy; kp=0 disables safe search).",
        annotations(open_world_hint = true)
    )]
    async fn fetch(
        &self,
        Parameters(args): Parameters<FetchArgs>,
    ) -> Result<CallToolResult, McpError> {
        let url = args.url;

        if let Err(error) = validate_http_url(&url) {
            return Ok(tool_error(error));
        }
        if !(MIN_FETCH_MAX_LENGTH..=MAX_FETCH_MAX_LENGTH).contains(&args.max_length) {
            return Ok(tool_error(format!(
                "maxLength must be between {MIN_FETCH_MAX_LENGTH} and {MAX_FETCH_MAX_LENGTH}, got {}",
                args.max_length
            )));
        }
        if args.start_index < 0 {
            return Ok(tool_error(format!(
                "startIndex must be non-negative, got {}",
                args.start_index
            )));
        }

        if self.respect_robots_txt
            && let Err(e) = check_may_fetch(&self.client, &url, &self.user_agent).await
        {
            return Ok(tool_error(e));
        }

        let (content, prefix) =
            match fetch_url(&self.client, &url, &self.user_agent, args.raw).await {
                Ok(result) => result,
                Err(e) => return Ok(tool_error(e)),
            };

        let start_index = match usize::try_from(args.start_index) {
            Ok(index) => index,
            Err(_) => return Ok(tool_error("startIndex is too large for this platform")),
        };
        let max_length = usize::try_from(args.max_length).map_err(|_| {
            McpError::invalid_params("maxLength is too large for this platform", None)
        })?;
        let rendered = render_fetch_result(&url, &prefix, &content, start_index, max_length);
        Ok(CallToolResult::success(vec![ContentBlock::text(rendered)]))
    }
}

fn render_fetch_result(
    url: &str,
    prefix: &str,
    content: &str,
    start_index: usize,
    max_length: usize,
) -> String {
    let header = format!("{prefix}Contents of {url}:\n");
    if header.len() >= MAX_TOOL_RESULT_BYTES {
        return truncate_text(
            &header,
            MAX_TOOL_RESULT_BYTES,
            "\n[truncated: fetch result header exceeded the tool output limit]",
        );
    }
    let page = content_page(
        content,
        start_index,
        max_length,
        MAX_TOOL_RESULT_BYTES - header.len(),
    );
    let mut rendered = header;
    rendered.push_str(&page.text);
    if let Some(next) = page.next_start_index {
        rendered.push_str(&continuation_hint(next));
    }
    rendered
}

struct ContentPage {
    text: String,
    next_start_index: Option<usize>,
}

fn continuation_hint(next: usize) -> String {
    format!(
        "\n\n<error>Content truncated. Call the fetch tool with a startIndex of {next} to get more content.</error>"
    )
}

fn content_page(
    content: &str,
    start_index: usize,
    max_length: usize,
    byte_budget: usize,
) -> ContentPage {
    let mut remaining = content.chars().skip(start_index).peekable();
    if remaining.peek().is_none() {
        return ContentPage {
            text: if start_index > 0 {
                "<error>No more content available.</error>".into()
            } else {
                String::new()
            },
            next_start_index: None,
        };
    }
    let mut text = String::new();
    let mut returned = 0usize;
    while returned < max_length {
        let Some(&character) = remaining.peek() else {
            break;
        };
        let next = start_index.saturating_add(returned + 1);
        let has_more = remaining.clone().nth(1).is_some();
        let required = text
            .len()
            .saturating_add(character.len_utf8())
            .saturating_add(if has_more {
                continuation_hint(next).len()
            } else {
                0
            });
        if required > byte_budget {
            break;
        }
        remaining.next();
        text.push(character);
        returned += 1;
    }
    let has_more = remaining.peek().is_some();
    let next_start_index = has_more
        .then(|| start_index.saturating_add(returned))
        .filter(|next| text.len().saturating_add(continuation_hint(*next).len()) <= byte_budget);
    ContentPage {
        text,
        next_start_index,
    }
}

#[prompt_router]
impl FetchServer {
    #[prompt(
        name = "fetch",
        description = "Fetch a URL and extract its contents as markdown"
    )]
    async fn fetch_prompt(
        &self,
        Parameters(args): Parameters<FetchPromptArgs>,
    ) -> Result<GetPromptResult, McpError> {
        let url = args.url;
        if let Err(error) = validate_http_url(&url) {
            return Err(McpError::invalid_params(error, None));
        }
        // User-initiated fetches always skip the robots.txt check.
        match fetch_url(&self.client, &url, &self.user_agent, false).await {
            Ok((content, prefix)) => Ok(GetPromptResult::new(vec![PromptMessage::new_text(
                Role::User,
                truncate_text(
                    &format!("{prefix}{content}"),
                    MAX_TOOL_RESULT_BYTES,
                    "\n[truncated: fetched prompt content exceeded the output limit]",
                ),
            )])
            .with_description(format!("Contents of {url}"))),
            Err(e) => Ok(
                GetPromptResult::new(vec![PromptMessage::new_text(Role::User, e)])
                    .with_description(format!("Failed to fetch {url}")),
            ),
        }
    }
}

#[tool_handler(router = self.tool_router)]
#[prompt_handler(router = self.prompt_router)]
impl ServerHandler for FetchServer {
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
            !self.prompt_router.list_all().is_empty(),
            !self.tool_router.list_all().is_empty(),
        ) {
            (true, true) => ServerCapabilities::builder()
                .enable_prompts()
                .enable_tools()
                .build(),
            (true, false) => ServerCapabilities::builder().enable_prompts().build(),
            (false, true) => ServerCapabilities::builder().enable_tools().build(),
            (false, false) => ServerCapabilities::builder().build(),
        };
        ServerInfo::new(capabilities)
            .with_server_info(Implementation::new("tuls-fetch", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "This server fetches bounded HTTP(S) content and converts HTML pages to markdown. \
                 Autonomous tool fetches follow the configured robots.txt and network policies. \
                 The fetch prompt is user-initiated and does not apply robots.txt.",
            )
    }
}

/// Start the fetch server on stdio.
pub async fn run(options: FetchOptions) -> anyhow::Result<()> {
    use rmcp::ServiceExt;
    use rmcp::transport::stdio;

    let user_agent = options
        .user_agent
        .unwrap_or_else(|| DEFAULT_USER_AGENT.to_string());
    validate_user_agent(&user_agent).map_err(anyhow::Error::msg)?;

    let policy = ToolPolicy::from_selectors(&options.tools.allow, &options.tools.deny, TOOL_SPECS)
        .map_err(anyhow::Error::msg)?;
    let client = FetchClient::new(options.proxy_url.as_deref(), options.network)
        .map_err(anyhow::Error::msg)?;
    let server = FetchServer::new(
        client,
        user_agent,
        options.robots == RobotsPolicy::Respect,
        policy,
    );

    let service = server
        .serve(stdio())
        .await
        .inspect_err(|e| tracing::error!("serving error: {e:?}"))?;
    tracing::info!("Fetch MCP server running on stdio (MCP {SPEC_VERSION})");

    service.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests;
