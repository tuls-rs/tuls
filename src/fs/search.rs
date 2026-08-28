use std::path::{Path, PathBuf};

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};

use crate::support::{AccessControl, MAX_TOOL_RESULT_BYTES};

const MAX_SEARCH_RESULTS: usize = 4096;
const MAX_SEARCH_ENTRIES: usize = 100_000;
pub const MAX_TREE_ENTRIES: usize = 1024;

fn build_glob_set(patterns: impl IntoIterator<Item = String>) -> Result<GlobSet, String> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        let glob = GlobBuilder::new(&pattern)
            .literal_separator(true)
            .backslash_escape(false)
            .build()
            .map_err(|error| format!("Invalid pattern {pattern:?}: {error}"))?;
        builder.add(glob);
    }
    builder
        .build()
        .map_err(|error| format!("Failed to build pattern set: {error}"))
}

fn build_excludes(patterns: &[String]) -> Result<Option<GlobSet>, String> {
    if patterns.is_empty() {
        return Ok(None);
    }
    let expanded = patterns.iter().flat_map(|pattern| {
        if pattern.contains('*') {
            vec![pattern.clone()]
        } else {
            vec![
                pattern.clone(),
                format!("**/{pattern}"),
                format!("**/{pattern}/**"),
            ]
        }
    });
    build_glob_set(expanded).map(Some)
}

pub async fn search_files(
    root: &Path,
    pattern: &str,
    exclude_patterns: &[String],
    access: &AccessControl,
) -> Result<(Vec<PathBuf>, bool), String> {
    let matcher = build_glob_set([pattern.to_string()])?;
    let excludes = build_excludes(exclude_patterns)?;
    let mut results = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    let mut visited = 0usize;
    let mut truncated = false;

    while let Some(current) = stack.pop() {
        let mut reader = tokio::fs::read_dir(&current)
            .await
            .map_err(|error| format!("Failed to read directory {}: {error}", current.display()))?;
        let mut entries = Vec::new();
        while let Some(entry) = reader
            .next_entry()
            .await
            .map_err(|error| error.to_string())?
        {
            entries.push(entry);
        }
        entries.sort_by_key(|entry| entry.file_name());

        for entry in entries {
            visited += 1;
            if visited > MAX_SEARCH_ENTRIES {
                truncated = true;
                break;
            }
            let path = entry.path();
            if access.validate_path(&path.to_string_lossy()).await.is_err() {
                continue;
            }
            let Ok(relative) = path.strip_prefix(root) else {
                continue;
            };
            let relative = relative.to_string_lossy().replace('\\', "/");
            if excludes.as_ref().is_some_and(|set| set.is_match(&relative)) {
                continue;
            }
            if matcher.is_match(&relative) {
                if results.len() == MAX_SEARCH_RESULTS {
                    truncated = true;
                    break;
                }
                results.push(path.clone());
            }
            if entry
                .file_type()
                .await
                .map_err(|error| error.to_string())?
                .is_dir()
            {
                stack.push(path);
            }
        }
        if truncated {
            break;
        }
    }

    results.sort();
    Ok((results, truncated))
}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub struct TreeEntry {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<TreeEntry>>,
}

/// Serialize a bounded tree to pretty JSON, erroring instead of emitting
/// truncated JSON when the result would exceed the tool-result limit. The
/// tree is already bounded to [`MAX_TREE_ENTRIES`] entries and oversized
/// trees carry a structured `truncated` marker entry, but a wide tree can
/// still serialize past the limit; callers must never append a truncation
/// notice to raw JSON because the client would receive invalid JSON.
pub fn serialize_tree(tree: &[TreeEntry]) -> Result<String, String> {
    let json = serde_json::to_string_pretty(tree)
        .map_err(|error| format!("Failed to serialize tree: {error}"))?;
    if json.len() > MAX_TOOL_RESULT_BYTES {
        return Err(format!(
            "directory tree exceeds the {MAX_TOOL_RESULT_BYTES} byte tool-result limit; \
             pass excludePatterns to exclude e.g. node_modules or target, \
             or use list_directory for a single directory"
        ));
    }
    Ok(json)
}

pub async fn directory_tree(
    root: &Path,
    exclude_patterns: &[String],
    access: &AccessControl,
) -> Result<Vec<TreeEntry>, String> {
    let excludes = build_excludes(exclude_patterns)?;
    let mut budget = MAX_TREE_ENTRIES;
    let mut truncated = false;
    build_tree(
        root,
        root,
        excludes.as_ref(),
        access,
        &mut budget,
        &mut truncated,
    )
    .await
}

async fn build_tree(
    root: &Path,
    current: &Path,
    excludes: Option<&GlobSet>,
    access: &AccessControl,
    budget: &mut usize,
    truncated: &mut bool,
) -> Result<Vec<TreeEntry>, String> {
    let mut reader = tokio::fs::read_dir(current)
        .await
        .map_err(|error| format!("Failed to read directory {}: {error}", current.display()))?;
    let mut entries = Vec::new();
    while let Some(entry) = reader
        .next_entry()
        .await
        .map_err(|error| error.to_string())?
    {
        entries.push(entry);
    }
    entries.sort_by_key(|entry| entry.file_name());

    let mut result = Vec::new();
    for entry in entries {
        if *budget == 0 {
            *truncated = true;
            result.push(TreeEntry {
                name: format!("... (tree truncated at {MAX_TREE_ENTRIES} entries)"),
                kind: "truncated",
                children: None,
            });
            break;
        }
        let path = entry.path();
        if access.validate_path(&path.to_string_lossy()).await.is_err() {
            continue;
        }
        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };
        let relative = relative.to_string_lossy().replace('\\', "/");
        if excludes.is_some_and(|set| set.is_match(&relative)) {
            continue;
        }

        *budget -= 1;
        let is_dir = entry
            .file_type()
            .await
            .map_err(|error| error.to_string())?
            .is_dir();
        let children = if is_dir {
            Some(Box::pin(build_tree(root, &path, excludes, access, budget, truncated)).await?)
        } else {
            None
        };
        result.push(TreeEntry {
            name: entry.file_name().to_string_lossy().into_owned(),
            kind: if is_dir { "directory" } else { "file" },
            children,
        });
        if *truncated {
            break;
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests;
