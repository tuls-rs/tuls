//! Neutral support shared by the concrete MCP servers.
//!
//! Concrete servers depend on this module for shared filesystem and MCP
//! primitives. Server-specific behavior remains in each server module.

mod access;

pub use access::AccessControl;

use std::path::Path;

use uuid::Uuid;

use rmcp::model::{CallToolResult, ContentBlock, ProtocolVersion};
use tokio::io::AsyncWriteExt;

/// The MCP protocol version implemented by every server in this binary.
pub const SPEC_VERSION: &str = "2026-07-28";

/// The only protocol revision supported by this binary.
pub static SUPPORTED_PROTOCOL_VERSIONS: &[ProtocolVersion] = &[ProtocolVersion::V_2026_07_28];

/// Suggested MCP Tasks polling interval in milliseconds used by tasks `tuls`
/// creates and by the child MCP client when a server omits `pollIntervalMs`.
pub const DEFAULT_TASK_POLL_INTERVAL_MS: u64 = 1000;

/// Reject initialize requests that do not use the only supported protocol lifecycle.
pub fn reject_unsupported_initialize()
-> std::future::Ready<Result<rmcp::model::InitializeResult, rmcp::ErrorData>> {
    std::future::ready(Err(rmcp::ErrorData::method_not_found::<
        rmcp::model::InitializeResultMethod,
    >()))
}

/// Clear the child environment and restore only variables required for normal
/// process discovery and platform operation. Credentials and unrelated parent
/// process state are not inherited implicitly.
pub fn configure_minimal_process_environment(command: &mut tokio::process::Command) {
    command.env_clear();
    for name in [
        "PATH",
        "HOME",
        "USERPROFILE",
        "SYSTEMROOT",
        "PATHEXT",
        "TEMP",
        "TMP",
        "TMPDIR",
        "LANG",
        "LC_ALL",
        "LC_CTYPE",
    ] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
}

/// A tool-level error result carrying a plain-text message.
pub fn tool_error(message: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(message)])
}

/// A successful tool result carrying plain text content.
pub fn text_result(text: impl Into<String>) -> CallToolResult {
    CallToolResult::success(vec![ContentBlock::text(text)])
}

/// Maximum size in bytes of a single tool result's text content. Tool
/// outputs that exceed this are truncated (see [`truncate_text`]) so a
/// single call — e.g. `directory_tree` on a huge tree or `read_text_file`
/// on a large file — cannot overflow the client's context window.
pub const MAX_TOOL_RESULT_BYTES: usize = 64 * 1024;

/// Truncate `text` so it fits within `max_bytes` bytes, cutting on a UTF-8
/// character boundary. When anything is cut, `notice` is appended after the
/// truncated content so callers can explain what was dropped and how to
/// retrieve the rest. Returns `text` unchanged when it already fits.
pub fn truncate_text(text: &str, max_bytes: usize, notice: &str) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    if max_bytes == 0 {
        return String::new();
    }

    let notice = truncate_utf8_prefix(notice, max_bytes);
    if notice.len() == max_bytes {
        return notice.to_string();
    }

    let content_budget = max_bytes - notice.len();
    let content = truncate_utf8_prefix(text, content_budget);
    let mut truncated = String::with_capacity(content.len() + notice.len());
    truncated.push_str(content);
    truncated.push_str(notice);
    truncated
}

fn truncate_utf8_prefix(value: &str, max_bytes: usize) -> &str {
    let mut end = max_bytes.min(value.len());
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

/// Replace `path` contents atomically: write a unique temp file in the same
/// directory as the target, flush it to disk, then rename it over the
/// target. The previous target contents are never replaced before the new
/// contents have been fully written, and are preserved when the temporary
/// write fails. The temporary file is removed on failure where possible.
///
/// When the target already exists as a regular file, its permissions are
/// captured and applied to the temp file before the rename so an existing
/// file keeps its mode (e.g. a 0600 memory file stays private). A brand-new
/// target uses the normal permissions produced by file creation under the
/// process umask.
///
/// Windows note: `tokio::fs::rename` replaces an existing destination file
/// on Windows (via `MoveFileExW` with `MOVEFILE_REPLACE_EXISTING`), so
/// file-for-file replacement works on every CI target. Renames do not follow
/// symlinks.
pub async fn atomic_write(path: &Path, content: &[u8]) -> std::io::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let file_name = path.file_name().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "atomic write target must have a file name",
        )
    })?;
    let file_name = file_name.to_string_lossy();
    let existing_permissions = capture_existing_permissions(path).await;

    let (temp, mut file) = loop {
        let temp = parent.join(format!(".{file_name}.{}.tmp", Uuid::now_v7().simple()));
        let open = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .await;
        match open {
            Ok(file) => break (temp, file),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    };

    let result = async {
        file.write_all(content).await?;
        if let Some(permissions) = existing_permissions {
            file.set_permissions(permissions).await?;
        }
        file.sync_all().await?;
        drop(file);
        tokio::fs::rename(&temp, path).await?;
        sync_parent_directory(parent).await
    }
    .await;

    if result.is_err() {
        let _ = tokio::fs::remove_file(&temp).await;
    }
    result
}

#[cfg(unix)]
async fn sync_parent_directory(parent: &Path) -> std::io::Result<()> {
    tokio::fs::File::open(parent).await?.sync_all().await
}

#[cfg(not(unix))]
async fn sync_parent_directory(_parent: &Path) -> std::io::Result<()> {
    Ok(())
}

/// Capture an existing regular file's permissions for reuse on the temp
/// file. Returns `None` when the target does not exist or is not a regular
/// file (e.g. a symlink); callers must not fail on that.
#[cfg(unix)]
async fn capture_existing_permissions(path: &Path) -> Option<std::fs::Permissions> {
    let metadata = tokio::fs::symlink_metadata(path).await.ok()?;
    metadata.is_file().then(|| metadata.permissions())
}

#[cfg(not(unix))]
async fn capture_existing_permissions(_path: &Path) -> Option<std::fs::Permissions> {
    None
}

#[cfg(all(test, unix))]
mod tests;

#[cfg(test)]
mod truncate_tests;
