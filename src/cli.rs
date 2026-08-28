use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

/// Filesystem, fetch, memory, process, skills, and agent MCP servers in one binary.
#[derive(Debug, Parser)]
#[command(name = "tuls", version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Start the filesystem MCP server.
    Filesystem(DirectoryServerOptions),
    /// Start the fetch MCP server.
    Fetch(FetchOptions),
    /// Start the persistent memory MCP server.
    Memory(MemoryOptions),
    /// Start the local process execution MCP server.
    Shell(DirectoryServerOptions),
    /// Start the Agent Skills MCP server for one workspace.
    Skills(WorkspaceServerOptions),
    /// Start the local subagents MCP server for one workspace.
    Agents(WorkspaceServerOptions),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Args)]
pub struct ToolPolicyOptions {
    /// Allow a capability or exact tool id. Repeating this option creates an allowlist.
    #[arg(long, value_name = "SELECTOR")]
    pub allow: Vec<String>,
    /// Deny a capability or exact tool id. Deny always takes precedence.
    #[arg(long, value_name = "SELECTOR")]
    pub deny: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Args)]
pub struct DirectoryServerOptions {
    /// Directories available to the server. The first entry is the base for relative paths.
    #[arg(value_name = "DIR", num_args = 0.., default_value = ".")]
    pub dirs: Vec<PathBuf>,
    #[command(flatten)]
    pub tools: ToolPolicyOptions,
}

#[derive(Debug, Clone, PartialEq, Args)]
pub struct WorkspaceServerOptions {
    /// Workspace root.
    #[arg(value_name = "DIR", default_value = ".")]
    pub dir: PathBuf,
    #[command(flatten)]
    pub tools: ToolPolicyOptions,
}

#[derive(Debug, Clone, PartialEq, Args)]
pub struct MemoryOptions {
    /// Location of the memory JSONL file.
    #[arg(long, value_name = "PATH")]
    pub memory_file: Option<PathBuf>,
    #[command(flatten)]
    pub tools: ToolPolicyOptions,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum RobotsPolicy {
    #[default]
    Respect,
    Ignore,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum NetworkPolicy {
    #[default]
    Public,
    Unrestricted,
}

#[derive(Debug, Clone, Default, PartialEq, Args)]
pub struct FetchOptions {
    /// robots.txt policy for autonomous tool fetches.
    #[arg(long, value_enum, default_value = "respect")]
    pub robots: RobotsPolicy,
    /// Outbound network policy. Public blocks local and non-public destinations.
    #[arg(long, value_enum, default_value = "public")]
    pub network: NetworkPolicy,
    /// User-Agent header used for outbound requests.
    #[arg(long, value_name = "USER_AGENT")]
    pub user_agent: Option<String>,
    /// Route outbound requests through this HTTP(S) proxy. Requires unrestricted network mode.
    #[arg(long, value_name = "URL")]
    pub proxy_url: Option<String>,
    #[command(flatten)]
    pub tools: ToolPolicyOptions,
}

#[cfg(test)]
mod tests;
