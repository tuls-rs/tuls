use crate::agents::{
    definition::{
        AgentDefinition, MAX_AGENT_CATALOG_BYTES, MAX_AGENT_FILE_BYTES, MAX_DISCOVERED_AGENTS,
    },
    markdown::parse_markdown,
};
use anyhow::{Context, Result, bail};
use std::{
    collections::{BTreeMap, BTreeSet, btree_map::Entry},
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};

const MAX_SCAN_DEPTH: usize = 8;
const MAX_SCAN_ENTRIES: usize = 4096;

#[derive(Clone)]
pub(crate) struct AgentRegistry {
    workspace: PathBuf,
    agents: BTreeMap<String, Arc<AgentDefinition>>,
}

impl AgentRegistry {
    pub(crate) fn discover(workspace: impl AsRef<Path>) -> Result<Self> {
        let workspace = fs::canonicalize(workspace.as_ref()).context("canonicalizing workspace")?;
        if !workspace.is_dir() {
            bail!("workspace must be a directory")
        }
        let mut candidates = Vec::new();
        collect(
            &workspace.join(".agents/agents"),
            &workspace,
            &mut candidates,
        )?;
        candidates.sort();
        let mut paths = BTreeSet::new();
        let mut agents = BTreeMap::new();
        let mut warned = BTreeSet::new();
        for path in candidates {
            if !paths.insert(path.clone()) {
                continue;
            }
            let input = match read_agent_file(&path) {
                Ok(value) => value,
                Err(error) => {
                    tracing::warn!(path=%path.display(), %error, "cannot read agent");
                    continue;
                }
            };
            match parse_markdown(path.clone(), &input) {
                Ok(agent) => {
                    let name = agent.name.clone();
                    match agents.entry(name) {
                        Entry::Occupied(entry) => {
                            if warned.insert(entry.key().clone()) {
                                tracing::warn!(name=%entry.key(), "agent name collision; retaining lexically first definition");
                            }
                        }
                        Entry::Vacant(entry) => {
                            entry.insert(Arc::new(agent));
                        }
                    }
                }
                Err(error) => {
                    tracing::warn!(path=%path.display(), %error, "ignoring malformed agent")
                }
            }
        }
        if agents.len() > MAX_DISCOVERED_AGENTS {
            bail!(
                "discovered agent count {} exceeds the limit of {MAX_DISCOVERED_AGENTS}",
                agents.len()
            );
        }
        let catalog_bytes = agents
            .values()
            .filter(|agent| agent.subagent)
            .map(|agent| 5 + agent.name.len() + agent.description.len())
            .sum::<usize>()
            .saturating_sub(1);
        if catalog_bytes > MAX_AGENT_CATALOG_BYTES {
            bail!("agent catalog exceeds the {MAX_AGENT_CATALOG_BYTES}-byte limit");
        }
        Ok(Self { workspace, agents })
    }

    pub(crate) fn names(&self) -> Vec<String> {
        self.agents
            .values()
            .filter(|agent| agent.subagent)
            .map(|agent| agent.name.clone())
            .collect()
    }

    pub(crate) fn catalog(&self) -> Vec<Arc<AgentDefinition>> {
        self.agents
            .values()
            .filter(|agent| agent.subagent)
            .cloned()
            .collect()
    }

    pub(crate) fn get_subagent(&self, name: &str) -> Option<Arc<AgentDefinition>> {
        self.agents
            .get(name)
            .filter(|agent| agent.subagent)
            .cloned()
    }

    pub(crate) fn is_empty(&self) -> bool {
        !self.agents.values().any(|agent| agent.subagent)
    }

    pub(crate) fn workspace(&self) -> &Path {
        &self.workspace
    }
}

fn read_agent_file(path: &Path) -> Result<String> {
    let file = fs::File::open(path)?;
    if file.metadata()?.len() > MAX_AGENT_FILE_BYTES as u64 {
        bail!("agent file exceeds the 1 MiB limit")
    }
    let mut bytes = Vec::with_capacity(MAX_AGENT_FILE_BYTES.min(64 * 1024));
    file.take((MAX_AGENT_FILE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_AGENT_FILE_BYTES {
        bail!("agent file exceeds the 1 MiB limit")
    }
    String::from_utf8(bytes).context("agent file is not valid UTF-8")
}

fn collect(root: &Path, workspace: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let root = match fs::canonicalize(root) {
        Ok(root) => root,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error).context("canonicalizing agent root"),
    };
    if !root.is_dir() || !root.starts_with(workspace) {
        return Ok(());
    }
    let mut visited = BTreeSet::new();
    ScanContext {
        root: &root,
        workspace,
        visited: &mut visited,
        scanned: 0,
        out,
    }
    .collect_dir(&root, 0)
}

struct ScanContext<'a> {
    root: &'a Path,
    workspace: &'a Path,
    visited: &'a mut BTreeSet<PathBuf>,
    scanned: usize,
    out: &'a mut Vec<PathBuf>,
}

impl ScanContext<'_> {
    fn collect_dir(&mut self, dir: &Path, depth: usize) -> Result<()> {
        let dir = fs::canonicalize(dir)
            .with_context(|| format!("canonicalizing agent directory {}", dir.display()))?;
        if !dir.starts_with(self.workspace)
            || !dir.starts_with(self.root)
            || !self.visited.insert(dir.clone())
        {
            return Ok(());
        }
        let entries = fs::read_dir(&dir)
            .with_context(|| format!("reading agent directory {}", dir.display()))?;
        let mut paths = Vec::new();
        for entry in entries {
            self.scanned = self.scanned.saturating_add(1);
            if self.scanned > MAX_SCAN_ENTRIES {
                bail!("agent discovery exceeds the {MAX_SCAN_ENTRIES}-entry scan limit")
            }
            paths.push(
                entry
                    .with_context(|| format!("reading agent directory {}", dir.display()))?
                    .path(),
            );
        }
        paths.sort();
        for path in paths {
            let target = match fs::canonicalize(&path) {
                Ok(target) => target,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("canonicalizing agent path {}", path.display()));
                }
            };
            if !target.starts_with(self.workspace) || !target.starts_with(self.root) {
                continue;
            }
            let metadata = fs::metadata(&target)
                .with_context(|| format!("inspecting agent path {}", target.display()))?;
            if metadata.is_dir() {
                if depth < MAX_SCAN_DEPTH {
                    self.collect_dir(&target, depth + 1)?;
                }
            } else if metadata.is_file()
                && target.extension().and_then(|value| value.to_str()) == Some("md")
            {
                if self.out.len() == MAX_DISCOVERED_AGENTS {
                    bail!("agent candidate count exceeds the limit of {MAX_DISCOVERED_AGENTS}")
                }
                self.out.push(target);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
