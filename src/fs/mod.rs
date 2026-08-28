mod edit;
mod format;
mod search;

use std::path::PathBuf;
use std::sync::Arc;

use rmcp::{
    ErrorData as McpError, ServerHandler,
    handler::server::router::tool::ToolRouter,
    handler::server::wrapper::Parameters,
    model::{
        CallToolResult, ContentBlock, Implementation, ResourceContents, ServerCapabilities,
        ServerInfo, Tool,
    },
    schemars, tool, tool_handler, tool_router,
};
use serde::Deserialize;
use tokio::io::AsyncReadExt;

use crate::cli::DirectoryServerOptions;
use crate::policy::{Capability, ToolPolicy, ToolSpec};
use crate::support::{
    AccessControl, MAX_TOOL_RESULT_BYTES, SPEC_VERSION, atomic_write, text_result, tool_error,
    truncate_text,
};

use self::edit::{EditOperation, apply_edits, render_diff};
use self::format::{format_size, head_lines, tail_lines};
use self::search::{TreeEntry, directory_tree, search_files, serialize_tree};

/// Maximum size of a media file (`read_media_file`) that will be base64
/// encoded into a tool result, protecting the client's context window.
const MAX_MEDIA_FILE_BYTES: usize = 1024 * 1024;
const MAX_TEXT_FILE_BYTES: usize = 8 * 1024 * 1024;
const MAX_BATCH_FILES: usize = 32;
const MAX_EDIT_OPERATIONS: usize = 1024;

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[schemars(description = "Arguments for reading a text file")]
pub struct ReadTextFileArgs {
    /// File path to read, must be within an allowed directory
    pub path: String,
    /// If provided, returns only the first N lines of the file
    #[serde(default)]
    pub head: Option<u32>,
    /// If provided, returns only the last N lines of the file
    #[serde(default)]
    pub tail: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[schemars(description = "Arguments for reading a media file")]
pub struct ReadMediaFileArgs {
    /// File path to read, must be within an allowed directory
    pub path: String,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[schemars(description = "Arguments for reading multiple files")]
pub struct ReadMultipleFilesArgs {
    /// Array of file paths to read. Each path must point to a valid file within allowed directories.
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[schemars(description = "Arguments for writing a file")]
pub struct WriteFileArgs {
    /// File location
    pub path: String,
    /// File content
    pub content: String,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[schemars(description = "Arguments for editing a file")]
pub struct EditFileArgs {
    /// File to edit
    pub path: String,
    /// List of edit operations
    pub edits: Vec<EditOperation>,
    /// Preview changes using git-style diff format without applying them
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[schemars(description = "Arguments for creating a directory")]
pub struct CreateDirectoryArgs {
    /// Directory path to create, must be within an allowed directory
    pub path: String,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[schemars(description = "Arguments for listing a directory")]
pub struct ListDirectoryArgs {
    /// Directory path to list, must be within an allowed directory
    pub path: String,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[schemars(description = "Arguments for listing a directory with sizes")]
pub struct ListDirectoryWithSizesArgs {
    /// Directory path to list, must be within an allowed directory
    pub path: String,
    /// Sort entries by name or size.
    #[serde(default)]
    pub sort_by: DirectorySort,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum DirectorySort {
    #[default]
    Name,
    Size,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[schemars(description = "Arguments for building a directory tree")]
pub struct DirectoryTreeArgs {
    /// Starting directory
    pub path: String,
    /// Exclude any paths matching these patterns
    #[serde(default)]
    pub exclude_patterns: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[schemars(description = "Arguments for moving a file or directory")]
pub struct MoveFileArgs {
    /// Source file or directory
    pub source: String,
    /// Destination file or directory
    pub destination: String,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[schemars(description = "Arguments for searching files")]
pub struct SearchFilesArgs {
    /// Starting directory for the search
    pub path: String,
    /// Glob-style pattern to match, e.g. `*.rs` or `**/*.rs`
    pub pattern: String,
    /// Exclude any paths matching these patterns
    #[serde(default)]
    pub exclude_patterns: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[schemars(description = "Arguments for getting file information")]
pub struct GetFileInfoArgs {
    /// File or directory path, must be within an allowed directory
    pub path: String,
}

const TOOL_SPECS: &[ToolSpec] = &[
    ToolSpec::new("filesystem", "read_text_file", Capability::FilesystemRead),
    ToolSpec::new("filesystem", "read_media_file", Capability::FilesystemRead),
    ToolSpec::new(
        "filesystem",
        "read_multiple_files",
        Capability::FilesystemRead,
    ),
    ToolSpec::new("filesystem", "write_file", Capability::FilesystemWrite),
    ToolSpec::new("filesystem", "edit_file", Capability::FilesystemWrite),
    ToolSpec::new(
        "filesystem",
        "create_directory",
        Capability::FilesystemWrite,
    ),
    ToolSpec::new("filesystem", "list_directory", Capability::FilesystemRead),
    ToolSpec::new(
        "filesystem",
        "list_directory_with_sizes",
        Capability::FilesystemRead,
    ),
    ToolSpec::new("filesystem", "directory_tree", Capability::FilesystemRead),
    ToolSpec::new("filesystem", "move_file", Capability::FilesystemWrite),
    ToolSpec::new("filesystem", "search_files", Capability::FilesystemRead),
    ToolSpec::new("filesystem", "get_file_info", Capability::FilesystemRead),
    ToolSpec::new(
        "filesystem",
        "list_allowed_directories",
        Capability::FilesystemRead,
    ),
];

#[derive(Debug, Clone)]
pub struct FilesystemServer {
    access: Arc<AccessControl>,
    tool_router: ToolRouter<FilesystemServer>,
}

impl FilesystemServer {
    pub fn new(access: AccessControl, policy: ToolPolicy) -> Self {
        let mut tool_router = Self::tool_router();
        for spec in TOOL_SPECS {
            if !policy.allows(*spec) {
                tool_router.disable_route(spec.name);
            }
        }
        Self {
            access: Arc::new(access),
            tool_router,
        }
    }
}

#[tool_router(router = tool_router)]
impl FilesystemServer {
    #[tool(
        name = "read_text_file",
        title = "Read Text File",
        description = "Read a bounded UTF-8 text file inside the allowed filesystem scope. Set head or tail to return only the first or last N lines.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn read_text_file(
        &self,
        Parameters(args): Parameters<ReadTextFileArgs>,
    ) -> Result<CallToolResult, McpError> {
        self.read_text_file_impl(args).await
    }

    #[tool(
        name = "read_media_file",
        title = "Read Media File",
        description = "Read a bounded media file inside the allowed filesystem scope and return a typed MCP content block.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn read_media_file(
        &self,
        Parameters(args): Parameters<ReadMediaFileArgs>,
    ) -> Result<CallToolResult, McpError> {
        let valid_path = match self.resolve(&args.path).await {
            Ok(p) => p,
            Err(_) => return Ok(tool_error(access_denied(&args.path))),
        };
        let bytes = match read_bounded_file(&valid_path, MAX_MEDIA_FILE_BYTES).await {
            Ok(bytes) => bytes,
            Err(error) => return Ok(tool_error(error)),
        };
        let mime = mime_guess::from_path(&valid_path)
            .first_raw()
            .unwrap_or("application/octet-stream")
            .to_string();
        let encoded = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes);

        let block = if mime.starts_with("image/") {
            ContentBlock::image(encoded, mime)
        } else if mime.starts_with("audio/") {
            ContentBlock::audio(encoded, mime)
        } else {
            let uri = match url::Url::from_file_path(&valid_path) {
                Ok(uri) => uri.to_string(),
                Err(()) => {
                    return Ok(tool_error(format!(
                        "Failed to convert {} to a file URI",
                        valid_path.display()
                    )));
                }
            };
            ContentBlock::resource(ResourceContents::BlobResourceContents {
                uri,
                mime_type: Some(mime),
                blob: encoded,
                meta: None,
            })
        };
        Ok(CallToolResult::success(vec![block]))
    }

    #[tool(
        name = "read_multiple_files",
        title = "Read Multiple Files",
        description = "Read multiple UTF-8 files inside the allowed filesystem scope in one bounded batch. Individual read failures are returned alongside successful results.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn read_multiple_files(
        &self,
        Parameters(args): Parameters<ReadMultipleFilesArgs>,
    ) -> Result<CallToolResult, McpError> {
        if args.paths.is_empty() {
            return Ok(tool_error("At least one file path must be provided"));
        }
        if args.paths.len() > MAX_BATCH_FILES {
            return Ok(tool_error(format!(
                "At most {MAX_BATCH_FILES} files may be read in one call"
            )));
        }
        let mut results = Vec::with_capacity(args.paths.len());
        for file_path in &args.paths {
            match self.resolve(file_path).await {
                Ok(valid) => match read_text_file_limited(&valid, MAX_TOOL_RESULT_BYTES).await {
                    Ok(content) => results.push(format!("{file_path}:\n{content}\n")),
                    Err(error) => results.push(format!("{file_path}: Error - {error}")),
                },
                Err(e) => results.push(format!("{file_path}: Error - {e}")),
            }
        }
        let combined = truncate_text(
            &results.join("\n---\n"),
            MAX_TOOL_RESULT_BYTES,
            &format!(
                "\n\n[truncated: combined output exceeds {MAX_TOOL_RESULT_BYTES} bytes; \
                 read the files individually]"
            ),
        );
        Ok(text_result(combined))
    }

    #[tool(
        name = "write_file",
        title = "Write File",
        description = "Atomically create or replace a UTF-8 file inside the allowed filesystem scope.",
        annotations(
            read_only_hint = false,
            idempotent_hint = true,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn write_file(
        &self,
        Parameters(args): Parameters<WriteFileArgs>,
    ) -> Result<CallToolResult, McpError> {
        if args.content.len() > MAX_TEXT_FILE_BYTES {
            return Ok(tool_error(format!(
                "content exceeds the {} byte text file limit",
                MAX_TEXT_FILE_BYTES
            )));
        }
        let valid_path = match self.resolve(&args.path).await {
            Ok(p) => p,
            Err(e) => return Ok(tool_error(e)),
        };
        // Atomic write via temp file + rename: replaces the target without
        // following a symlink that appears between validation and write.
        match atomic_write(&valid_path, args.content.as_bytes()).await {
            Ok(()) => Ok(text_result(format!("Successfully wrote to {}", args.path))),
            Err(e) => Ok(tool_error(format!(
                "Failed to write to {}: {e}",
                valid_path.display()
            ))),
        }
    }

    #[tool(
        name = "edit_file",
        title = "Edit File",
        description = "Apply ordered exact text replacements inside the allowed filesystem scope and return a bounded diff. Each nonempty oldText must occur exactly once in the current content. Set dryRun to use identical matching without writing.",
        annotations(
            read_only_hint = false,
            idempotent_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn edit_file(
        &self,
        Parameters(args): Parameters<EditFileArgs>,
    ) -> Result<CallToolResult, McpError> {
        let edit_bytes = args.edits.iter().fold(0usize, |total, edit| {
            total
                .saturating_add(edit.old_text.len())
                .saturating_add(edit.new_text.len())
        });
        if args.edits.len() > MAX_EDIT_OPERATIONS || edit_bytes > MAX_TEXT_FILE_BYTES {
            return Ok(tool_error(format!(
                "edit batch must contain at most {MAX_EDIT_OPERATIONS} operations and {MAX_TEXT_FILE_BYTES} total text bytes"
            )));
        }
        let valid_path = match self.resolve(&args.path).await {
            Ok(p) => p,
            Err(e) => return Ok(tool_error(e)),
        };
        let original = match read_text_file(&valid_path).await {
            Ok(content) => content,
            Err(error) => return Ok(tool_error(error)),
        };
        let modified = match apply_edits(&original, &args.edits) {
            Ok(modified) => modified,
            Err(e) => return Ok(tool_error(e)),
        };
        if modified.len() > MAX_TEXT_FILE_BYTES {
            return Ok(tool_error(format!(
                "edited file exceeds the {} byte text file limit",
                MAX_TEXT_FILE_BYTES
            )));
        }
        let diff = render_diff(&original, &modified);

        if !args.dry_run
            && modified != original
            && let Err(e) = atomic_write(&valid_path, modified.as_bytes()).await
        {
            return Ok(tool_error(format!(
                "Failed to write {}: {e}",
                valid_path.display()
            )));
        }
        let diff = truncate_text(
            &diff,
            MAX_TOOL_RESULT_BYTES,
            &format!(
                "\n\n[truncated: diff exceeds {MAX_TOOL_RESULT_BYTES} bytes; \
                 consider editing a smaller portion of the file]"
            ),
        );
        Ok(text_result(diff))
    }

    #[tool(
        name = "create_directory",
        title = "Create Directory",
        description = "Create a directory and any missing parent directories inside the allowed filesystem scope. Existing directories succeed without modification.",
        annotations(
            read_only_hint = false,
            idempotent_hint = true,
            destructive_hint = false,
            open_world_hint = false
        )
    )]
    async fn create_directory(
        &self,
        Parameters(args): Parameters<CreateDirectoryArgs>,
    ) -> Result<CallToolResult, McpError> {
        let valid_path = match self.resolve(&args.path).await {
            Ok(p) => p,
            Err(e) => return Ok(tool_error(e)),
        };
        match tokio::fs::create_dir_all(&valid_path).await {
            Ok(()) => Ok(text_result(format!(
                "Successfully created directory {}",
                args.path
            ))),
            Err(e) => Ok(tool_error(format!(
                "Failed to create directory {}: {e}",
                valid_path.display()
            ))),
        }
    }

    #[tool(
        name = "list_directory",
        title = "List Directory",
        description = "List immediate entries of a directory inside the allowed filesystem scope, distinguishing files, directories, and symbolic links.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn list_directory(
        &self,
        Parameters(args): Parameters<ListDirectoryArgs>,
    ) -> Result<CallToolResult, McpError> {
        let valid_path = match self.resolve(&args.path).await {
            Ok(p) => p,
            Err(e) => return Ok(tool_error(e)),
        };
        let mut entries = match tokio::fs::read_dir(&valid_path).await {
            Ok(entries) => entries,
            Err(e) => {
                return Ok(tool_error(format!(
                    "Failed to list directory {}: {e}",
                    valid_path.display()
                )));
            }
        };
        let mut names = Vec::new();
        loop {
            let next = match entries.next_entry().await {
                Ok(next) => next,
                Err(e) => {
                    return Ok(tool_error(format!(
                        "Failed to list directory {}: {e}",
                        valid_path.display()
                    )));
                }
            };
            let Some(entry) = next else { break };
            let file_type = match entry.file_type().await {
                Ok(file_type) => file_type,
                Err(e) => {
                    return Ok(tool_error(format!(
                        "Failed to list directory {}: {e}",
                        valid_path.display()
                    )));
                }
            };
            let kind = if file_type.is_dir() {
                "[DIR]"
            } else if file_type.is_symlink() {
                "[SYMLINK]"
            } else {
                "[FILE]"
            };
            names.push(format!("{kind} {}", entry.file_name().to_string_lossy()));
        }
        names.sort();
        let listing = truncate_text(
            &names.join("\n"),
            MAX_TOOL_RESULT_BYTES,
            &format!(
                "\n\n[truncated: listing exceeds {MAX_TOOL_RESULT_BYTES} bytes; \
                 use search_files or directory_tree with excludePatterns to narrow]"
            ),
        );
        Ok(text_result(listing))
    }

    #[tool(
        name = "list_directory_with_sizes",
        title = "List Directory with Sizes",
        description = "List immediate directory entries with file sizes inside the allowed filesystem scope. Symbolic links are reported without following their targets.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn list_directory_with_sizes(
        &self,
        Parameters(args): Parameters<ListDirectoryWithSizesArgs>,
    ) -> Result<CallToolResult, McpError> {
        let valid_path = match self.resolve(&args.path).await {
            Ok(p) => p,
            Err(e) => return Ok(tool_error(e)),
        };
        let mut entries = match tokio::fs::read_dir(&valid_path).await {
            Ok(entries) => entries,
            Err(e) => {
                return Ok(tool_error(format!(
                    "Failed to list directory {}: {e}",
                    valid_path.display()
                )));
            }
        };

        struct Entry {
            name: String,
            is_directory: bool,
            is_symlink: bool,
            size: u64,
        }

        let mut detailed = Vec::new();
        loop {
            let next = match entries.next_entry().await {
                Ok(next) => next,
                Err(e) => {
                    return Ok(tool_error(format!(
                        "Failed to list directory {}: {e}",
                        valid_path.display()
                    )));
                }
            };
            let Some(entry) = next else { break };
            let name = entry.file_name().to_string_lossy().into_owned();
            let file_type = match entry.file_type().await {
                Ok(file_type) => file_type,
                Err(error) => {
                    return Ok(tool_error(format!(
                        "Failed to inspect directory entry {}: {error}",
                        entry.path().display()
                    )));
                }
            };
            let is_directory = file_type.is_dir();
            let is_symlink = file_type.is_symlink();
            let size = if is_directory {
                0
            } else {
                match tokio::fs::symlink_metadata(entry.path()).await {
                    Ok(metadata) => metadata.len(),
                    Err(error) => {
                        return Ok(tool_error(format!(
                            "Failed to stat {}: {error}",
                            entry.path().display()
                        )));
                    }
                }
            };
            detailed.push(Entry {
                name,
                is_directory,
                is_symlink,
                size,
            });
        }

        detailed.sort_by(|a, b| {
            if args.sort_by == DirectorySort::Size {
                b.size.cmp(&a.size)
            } else {
                a.name.cmp(&b.name)
            }
        });

        let mut formatted = Vec::new();
        for entry in &detailed {
            let size = if entry.is_directory {
                String::new()
            } else {
                format_size(entry.size)
            };
            formatted.push(format!(
                "{} {:<30} {:>10}",
                if entry.is_directory {
                    "[DIR]"
                } else if entry.is_symlink {
                    "[SYMLINK]"
                } else {
                    "[FILE]"
                },
                entry.name,
                size
            ));
        }

        let total_files = detailed.iter().filter(|e| !e.is_directory).count();
        let total_dirs = detailed.iter().filter(|e| e.is_directory).count();
        let total_size = detailed
            .iter()
            .filter(|e| !e.is_directory)
            .map(|e| e.size)
            .sum::<u64>();
        formatted.push(String::new());
        formatted.push(format!(
            "Total: {total_files} files, {total_dirs} directories"
        ));
        formatted.push(format!("Combined size: {}", format_size(total_size)));

        let listing = truncate_text(
            &formatted.join("\n"),
            MAX_TOOL_RESULT_BYTES,
            &format!(
                "\n\n[truncated: listing exceeds {MAX_TOOL_RESULT_BYTES} bytes; \
                 use search_files or directory_tree with excludePatterns to narrow]"
            ),
        );
        Ok(text_result(listing))
    }

    #[tool(
        name = "directory_tree",
        title = "Directory Tree",
        description = "Return a bounded recursive JSON tree inside the allowed filesystem scope. The entry limit uses an explicit marker; JSON that exceeds the result byte limit returns an error. Use excludePatterns to narrow the tree.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn directory_tree(
        &self,
        Parameters(args): Parameters<DirectoryTreeArgs>,
    ) -> Result<CallToolResult, McpError> {
        let valid_path = match self.resolve(&args.path).await {
            Ok(p) => p,
            Err(e) => return Ok(tool_error(e)),
        };
        let access = self.access.clone();
        let tree: Vec<TreeEntry> =
            match directory_tree(&valid_path, &args.exclude_patterns, &access).await {
                Ok(tree) => tree,
                Err(e) => return Ok(tool_error(e)),
            };
        match serialize_tree(&tree) {
            Ok(json) => Ok(text_result(json)),
            Err(e) => Ok(tool_error(e)),
        }
    }

    #[tool(
        name = "move_file",
        title = "Move File",
        description = "Move or rename a file or directory inside the allowed filesystem scope. The destination must not already exist.",
        annotations(
            read_only_hint = false,
            idempotent_hint = false,
            destructive_hint = true,
            open_world_hint = false
        )
    )]
    async fn move_file(
        &self,
        Parameters(args): Parameters<MoveFileArgs>,
    ) -> Result<CallToolResult, McpError> {
        let valid_source = match self.resolve(&args.source).await {
            Ok(p) => p,
            Err(e) => return Ok(tool_error(e)),
        };
        let valid_dest = match self.resolve(&args.destination).await {
            Ok(p) => p,
            Err(e) => return Ok(tool_error(e)),
        };
        match tokio::fs::try_exists(&valid_dest).await {
            Ok(true) => {
                return Ok(tool_error(format!(
                    "Destination already exists: {}",
                    valid_dest.display()
                )));
            }
            Ok(false) => {}
            Err(error) => {
                return Ok(tool_error(format!(
                    "Failed to inspect destination {}: {error}",
                    valid_dest.display()
                )));
            }
        }
        match tokio::fs::rename(&valid_source, &valid_dest).await {
            Ok(()) => Ok(text_result(format!(
                "Successfully moved {} to {}",
                args.source, args.destination
            ))),
            Err(e) => Ok(tool_error(format!(
                "Failed to move {} to {}: {e}",
                args.source, args.destination
            ))),
        }
    }

    #[tool(
        name = "search_files",
        title = "Search Files",
        description = "Recursively search inside the allowed filesystem scope using a glob pattern. Use excludePatterns to omit matching paths; results are bounded.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn search_files(
        &self,
        Parameters(args): Parameters<SearchFilesArgs>,
    ) -> Result<CallToolResult, McpError> {
        let valid_path = match self.resolve(&args.path).await {
            Ok(p) => p,
            Err(e) => return Ok(tool_error(e)),
        };
        let access = self.access.clone();
        let (results, search_truncated) =
            match search_files(&valid_path, &args.pattern, &args.exclude_patterns, &access).await {
                Ok(results) => results,
                Err(error) => return Ok(tool_error(error)),
            };
        if results.is_empty() {
            return Ok(text_result("No matches found"));
        }
        let mut text = results
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join("\n");
        if search_truncated {
            text.push_str("\n[truncated: search safety limit reached]");
        }
        let text = truncate_text(
            &text,
            MAX_TOOL_RESULT_BYTES,
            &format!(
                "\n\n[truncated: too many matches to list in {MAX_TOOL_RESULT_BYTES} bytes; \
                 narrow the pattern or add excludePatterns]"
            ),
        );
        Ok(text_result(text))
    }

    #[tool(
        name = "get_file_info",
        title = "Get File Info",
        description = "Return bounded filesystem metadata for a file or directory inside the allowed filesystem scope.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn get_file_info(
        &self,
        Parameters(args): Parameters<GetFileInfoArgs>,
    ) -> Result<CallToolResult, McpError> {
        let valid_path = match self.resolve(&args.path).await {
            Ok(p) => p,
            Err(e) => return Ok(tool_error(e)),
        };
        let meta = match tokio::fs::metadata(&valid_path).await {
            Ok(meta) => meta,
            Err(e) => {
                return Ok(tool_error(format!(
                    "Failed to stat {}: {e}",
                    valid_path.display()
                )));
            }
        };
        let lines = [
            format!("size: {}", meta.len()),
            format!("created: {}", rfc3339(meta.created())),
            format!("modified: {}", rfc3339(meta.modified())),
            format!("accessed: {}", rfc3339(meta.accessed())),
            format!("isFile: {}", meta.is_file()),
            format!("isDirectory: {}", meta.is_dir()),
            format!("permissions: {}", permissions_string(&meta)),
        ];
        Ok(text_result(lines.join("\n")))
    }

    #[tool(
        name = "list_allowed_directories",
        title = "List Allowed Directories",
        description = "Return the configured filesystem roots available to this server.",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn list_allowed_directories(&self) -> Result<CallToolResult, McpError> {
        let text = format!(
            "Allowed directories:\n{}",
            self.access
                .roots()
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join("\n")
        );
        Ok(text_result(text))
    }
}

impl FilesystemServer {
    /// Resolve a user path against the allowed directories, or return an
    /// access-denied style error message.
    async fn resolve(&self, path: &str) -> Result<PathBuf, String> {
        self.access.validate_path(path).await
    }

    async fn read_text_file_impl(
        &self,
        args: ReadTextFileArgs,
    ) -> Result<CallToolResult, McpError> {
        if args.head.is_some() && args.tail.is_some() {
            return Ok(tool_error(
                "Cannot specify both head and tail parameters simultaneously",
            ));
        }
        let valid_path = match self.resolve(&args.path).await {
            Ok(p) => p,
            Err(e) => return Ok(tool_error(e)),
        };
        let content = match read_text_file(&valid_path).await {
            Ok(content) => content,
            Err(error) => return Ok(tool_error(error)),
        };
        let text = if let Some(n) = args.tail {
            tail_lines(&content, n)
        } else if let Some(n) = args.head {
            head_lines(&content, n)
        } else {
            content
        };
        let text = truncate_text(
            &text,
            MAX_TOOL_RESULT_BYTES,
            &format!(
                "\n\n[truncated: file exceeds {MAX_TOOL_RESULT_BYTES} bytes; \
                 use the head or tail parameter to read a specific portion]"
            ),
        );
        Ok(text_result(text))
    }
}

async fn read_bounded_file(path: &std::path::Path, limit: usize) -> Result<Vec<u8>, String> {
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|error| format!("Failed to open {}: {error}", path.display()))?;
    let mut bytes = Vec::with_capacity(limit.min(64 * 1024));
    let mut reader = file.take(limit.saturating_add(1) as u64);
    reader
        .read_to_end(&mut bytes)
        .await
        .map_err(|error| format!("Failed to read {}: {error}", path.display()))?;
    if bytes.len() > limit {
        return Err(format!(
            "Failed to read {}: file exceeds the {} byte limit",
            path.display(),
            limit
        ));
    }
    Ok(bytes)
}

async fn read_text_file_limited(path: &std::path::Path, limit: usize) -> Result<String, String> {
    let bytes = read_bounded_file(path, limit).await?;
    String::from_utf8(bytes)
        .map_err(|_| format!("Failed to read {}: file is not valid UTF-8", path.display()))
}

async fn read_text_file(path: &std::path::Path) -> Result<String, String> {
    read_text_file_limited(path, MAX_TEXT_FILE_BYTES).await
}

#[cfg(unix)]
fn permissions_string(meta: &std::fs::Metadata) -> String {
    use std::os::unix::fs::PermissionsExt;
    format!("{:o}", meta.permissions().mode() & 0o777)
}

#[cfg(not(unix))]
fn permissions_string(_meta: &std::fs::Metadata) -> String {
    "unknown".to_string()
}

fn access_denied(path: &str) -> String {
    format!("Access denied - path outside allowed directories: {path}")
}

fn rfc3339(time: std::io::Result<std::time::SystemTime>) -> String {
    let Some(seconds) = time.ok().and_then(unix_seconds) else {
        return "unknown".to_string();
    };
    let days = seconds.div_euclid(86_400);
    let rem = seconds.rem_euclid(86_400);
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let (y, mo, d) = civil_from_days(days);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

fn unix_seconds(time: std::time::SystemTime) -> Option<i64> {
    match time.duration_since(std::time::UNIX_EPOCH) {
        Ok(duration) => i64::try_from(duration.as_secs()).ok(),
        Err(error) => i64::try_from(error.duration().as_secs())
            .ok()
            .and_then(i64::checked_neg),
    }
}

/// Civil-from-days algorithm (Howard Hinnant), returns (year, month, day).
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for FilesystemServer {
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
        ServerInfo::new(if self.tool_router.list_all().is_empty() {
            ServerCapabilities::builder().build()
        } else {
            ServerCapabilities::builder().enable_tools().build()
        })
        .with_server_info(Implementation::new(
            "tuls-filesystem",
            env!("CARGO_PKG_VERSION"),
        ))
        .with_instructions(
            "This server provides secure file access restricted to the allowed \
                 directories passed on the command line. Use list_allowed_directories \
                 to see which directories are currently accessible.",
        )
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.tool_router.get(name).cloned()
    }
}

/// Start the filesystem server on stdio.
pub async fn run(options: DirectoryServerOptions) -> anyhow::Result<()> {
    use rmcp::ServiceExt;
    use rmcp::transport::stdio;

    let access = AccessControl::from_args(&options.dirs).map_err(anyhow::Error::msg)?;
    let policy = ToolPolicy::from_selectors(&options.tools.allow, &options.tools.deny, TOOL_SPECS)
        .map_err(anyhow::Error::msg)?;

    let server = FilesystemServer::new(access, policy);
    let service = server
        .serve(stdio())
        .await
        .inspect_err(|e| tracing::error!("serving error: {e:?}"))?;
    tracing::info!("Filesystem MCP server running on stdio (MCP {SPEC_VERSION})");

    service.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests;
