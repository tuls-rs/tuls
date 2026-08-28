use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

pub(crate) const MAX_RESOURCES: usize = 1000;
pub(crate) const MAX_DEPTH: usize = 8;
pub(crate) const MAX_RESOURCE_MANIFEST_BYTES: usize = 64 * 1024;

pub(crate) fn resource_manifest(skill_dir: &Path) -> anyhow::Result<Vec<String>> {
    let root = fs::canonicalize(skill_dir)?;
    let mut resources = Vec::new();
    let mut identities = HashSet::new();
    collect(&root, &root, 0, &mut resources, &mut identities)?;
    resources.sort();
    Ok(resources)
}

fn collect(
    root: &Path,
    dir: &Path,
    depth: usize,
    out: &mut Vec<String>,
    identities: &mut HashSet<PathBuf>,
) -> anyhow::Result<()> {
    let mut entries = fs::read_dir(dir)?.collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .map_err(|_| anyhow::anyhow!("resource is outside skill directory"))?;
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_dir() {
            if depth >= MAX_DEPTH {
                anyhow::bail!("resource directory depth exceeds {MAX_DEPTH}")
            }
            collect(root, &path, depth + 1, out, identities)?;
        } else {
            if relative == Path::new("SKILL.md") {
                continue;
            }
            let target = fs::canonicalize(&path)?;
            if !target.starts_with(root) {
                anyhow::bail!(
                    "resource target escapes skill directory: {}",
                    relative.display()
                )
            }
            if !fs::metadata(&target)?.is_file() {
                continue;
            }
            if depth + 1 > MAX_DEPTH {
                anyhow::bail!("resource depth exceeds {MAX_DEPTH}")
            }
            if !identities.insert(target) {
                continue;
            }
            if out.len() >= MAX_RESOURCES {
                anyhow::bail!("resource count exceeds {MAX_RESOURCES}")
            }
            let resource = slash_path(relative);
            let manifest_bytes = out
                .iter()
                .map(String::len)
                .sum::<usize>()
                .saturating_add(out.len())
                .saturating_add(resource.len());
            if manifest_bytes > MAX_RESOURCE_MANIFEST_BYTES {
                anyhow::bail!("resource manifest exceeds {MAX_RESOURCE_MANIFEST_BYTES} bytes")
            }
            out.push(resource);
        }
    }
    Ok(())
}

fn slash_path(path: &Path) -> String {
    path.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests;
