use std::ffi::OsString;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

/// Lexically normalize a path: collapses `.`, resolves `..` where possible,
/// and strips redundant separators. Does not touch the filesystem.
pub fn normalize_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => match out.components().next_back() {
                Some(Component::RootDir | Component::Prefix(_)) => {}
                Some(Component::ParentDir) | None => out.push(".."),
                Some(_) => {
                    out.pop();
                }
            },
            other => out.push(other),
        }
    }
    out
}

/// Expand a leading `~` to the user's home directory.
pub fn expand_home(path: &Path) -> PathBuf {
    if path == Path::new("~") {
        return home_dir().unwrap_or_else(|| path.to_path_buf());
    }
    if let (Ok(rest), Some(home)) = (path.strip_prefix("~"), home_dir()) {
        return home.join(rest);
    }
    path.to_path_buf()
}

fn home_dir() -> Option<PathBuf> {
    home_dir_from(|key| std::env::var_os(key))
}

/// Resolve the home directory from an environment lookup, trying the
/// platform conventions in order: `HOME`, then `USERPROFILE`, then
/// `HOMEDRIVE` + `HOMEPATH` (both present and non-empty). Standard library
/// only, so it works on Unix and Windows CI runners alike.
fn home_dir_from(mut get: impl FnMut(&str) -> Option<OsString>) -> Option<PathBuf> {
    if let Some(home) = get("HOME").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(home));
    }
    if let Some(profile) = get("USERPROFILE").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(profile));
    }
    let (Some(drive), Some(path)) = (get("HOMEDRIVE"), get("HOMEPATH")) else {
        return None;
    };
    if drive.is_empty() || path.is_empty() {
        return None;
    }
    Some(PathBuf::from(drive).join(path))
}

/// True when `path` is lexically inside at least one of `roots`.
///
/// Component-wise so that `/foo/bar` is not considered inside `/foo/bar2`.
pub fn is_within(path: &Path, roots: &[PathBuf]) -> bool {
    roots.iter().any(|root| path.starts_with(root))
}

/// Controls which directories a server may access.
///
/// Configured roots retain user order for display and relative-path semantics.
/// Validation roots additionally contain canonical targets so platform aliases
/// and symlinked directory arguments are checked consistently.
#[derive(Debug, Clone)]
pub struct AccessControl {
    default_root: PathBuf,
    configured_roots: Vec<PathBuf>,
    validation_roots: Vec<PathBuf>,
}

impl AccessControl {
    /// Build access control from command-line arguments.
    ///
    /// Every configured directory must exist, be accessible, and be a
    /// directory. Invalid security configuration fails closed.
    pub fn from_args(dirs: &[PathBuf]) -> Result<Self, String> {
        if dirs.is_empty() {
            return Err(
                "no allowed directories provided - pass at least one directory".to_string(),
            );
        }

        let current_dir = std::env::current_dir()
            .map_err(|error| format!("failed to resolve current directory: {error}"))?;
        let mut configured_roots = Vec::new();
        let mut validation_roots = Vec::new();

        for dir in dirs {
            let expanded = expand_home(dir);
            let absolute = if expanded.is_absolute() {
                expanded
            } else {
                current_dir.join(expanded)
            };
            let normalized = normalize_path(&absolute);
            let real = std::fs::canonicalize(&normalized).map_err(|error| {
                format!(
                    "failed to access allowed directory {}: {error}",
                    normalized.display()
                )
            })?;
            let metadata = std::fs::metadata(&real).map_err(|error| {
                format!(
                    "failed to inspect allowed directory {}: {error}",
                    real.display()
                )
            })?;
            if !metadata.is_dir() {
                return Err(format!(
                    "allowed path is not a directory: {}",
                    normalized.display()
                ));
            }

            if !configured_roots.contains(&normalized) {
                configured_roots.push(normalized.clone());
            }
            validation_roots.push(normalized);
            validation_roots.push(real);
        }

        validation_roots.sort();
        validation_roots.dedup();
        let default_root = configured_roots
            .first()
            .cloned()
            .ok_or_else(|| "server has no allowed directories".to_string())?;

        Ok(Self {
            default_root,
            configured_roots,
            validation_roots,
        })
    }

    /// Configured roots in command-line order, without canonical aliases.
    pub fn roots(&self) -> &[PathBuf] {
        &self.configured_roots
    }

    /// Resolve and verify a user-supplied path against the allowed roots.
    ///
    /// Returns the canonical path for existing entries, the absolute path for
    /// entries that do not exist yet (their deepest existing ancestor must be
    /// allowed), or an error describing why access is denied.
    pub async fn validate_path(&self, requested: &str) -> Result<PathBuf, String> {
        let expanded = expand_home(Path::new(requested));
        let absolute = if expanded.is_absolute() {
            expanded
        } else {
            self.default_root.join(expanded)
        };
        let normalized = normalize_path(&absolute);

        if !is_within(&normalized, &self.validation_roots) {
            return Err(format!(
                "Access denied - path outside allowed directories: {} not in {}",
                normalized.display(),
                self.display_roots()
            ));
        }

        match tokio::fs::canonicalize(&normalized).await {
            Ok(real) => {
                if !is_within(&real, &self.validation_roots) {
                    return Err(format!(
                        "Access denied - symlink target outside allowed directories: {} not in {}",
                        real.display(),
                        self.display_roots()
                    ));
                }
                Ok(real)
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {
                let mut ancestor = normalized.as_path();
                loop {
                    match tokio::fs::canonicalize(ancestor).await {
                        Ok(real) => {
                            if !is_within(&real, &self.validation_roots) {
                                return Err(format!(
                                    "Access denied - parent directory outside allowed directories: {} not in {}",
                                    real.display(),
                                    self.display_roots()
                                ));
                            }
                            return Ok(normalized);
                        }
                        Err(error) if error.kind() == ErrorKind::NotFound => {
                            match ancestor.parent() {
                                Some(parent) if parent != ancestor => ancestor = parent,
                                _ => {
                                    return Err(format!(
                                        "Parent directory does not exist: {}",
                                        normalized.display()
                                    ));
                                }
                            }
                        }
                        Err(error) => {
                            return Err(format!(
                                "Failed to access {}: {error}",
                                normalized.display()
                            ));
                        }
                    }
                }
            }
            Err(error) => Err(format!(
                "Failed to access {}: {error}",
                normalized.display()
            )),
        }
    }

    fn display_roots(&self) -> String {
        self.configured_roots
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

#[cfg(test)]
mod tests;
