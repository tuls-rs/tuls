use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

use anyhow::Context;

use super::parser::{ParsedSkill, parse_skill_file};

const ROOTS: [&str; 2] = [".agents/skills", ".claude/skills"];
pub(crate) const MAX_DISCOVERED_SKILLS: usize = 256;
pub(crate) const MAX_SKILL_CATALOG_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone)]
pub(crate) struct Skill {
    pub(crate) skill_dir: PathBuf,
    pub(crate) skill_file: PathBuf,
    pub(crate) description: String,
}

#[derive(Debug, Default)]
pub(crate) struct SkillRegistry {
    skills: BTreeMap<String, Skill>,
}

impl SkillRegistry {
    pub(crate) fn discover(workspace: impl AsRef<Path>) -> anyhow::Result<Self> {
        let workspace = fs::canonicalize(workspace)?;
        if !workspace.is_dir() {
            anyhow::bail!("workspace is not a directory")
        }
        let mut candidates = Vec::new();
        for (precedence, relative_root) in ROOTS.into_iter().enumerate() {
            let root_path = workspace.join(relative_root);
            let root = match fs::canonicalize(&root_path) {
                Ok(root) if root.is_dir() && root.starts_with(&workspace) => root,
                Ok(_) => continue,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("reading skill root {}", root_path.display()));
                }
            };
            let entries = fs::read_dir(&root)
                .with_context(|| format!("reading skill root {}", root.display()))?;
            let mut dirs = Vec::new();
            for entry in entries {
                if dirs.len() == MAX_DISCOVERED_SKILLS {
                    anyhow::bail!(
                        "skill candidate count exceeds the limit of {MAX_DISCOVERED_SKILLS}"
                    )
                }
                let entry =
                    entry.with_context(|| format!("reading skill root {}", root.display()))?;
                let Ok(dir) = fs::canonicalize(entry.path()) else {
                    continue;
                };
                dirs.push(dir);
            }
            dirs.sort();
            dirs.dedup();
            for dir in dirs {
                if !dir.starts_with(&root) || !dir.is_dir() {
                    continue;
                }
                let file = dir.join("SKILL.md");
                let Ok(canonical_file) = fs::canonicalize(&file) else {
                    continue;
                };
                if !canonical_file.starts_with(&dir) || !canonical_file.is_file() {
                    continue;
                }
                match parse_skill_file(&canonical_file) {
                    Ok(parsed) => candidates.push((precedence, dir, canonical_file, parsed)),
                    Err(error) => {
                        tracing::warn!(path = %file.display(), %error, "ignoring malformed skill")
                    }
                }
            }
        }
        candidates.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        let mut skills = BTreeMap::new();
        let mut identities = HashSet::new();
        for (
            _,
            skill_dir,
            skill_file,
            ParsedSkill {
                name, description, ..
            },
        ) in candidates
        {
            if !identities.insert(skill_dir.clone()) {
                continue;
            }
            if skills.contains_key(&name) {
                tracing::warn!(skill = %name, path = %skill_dir.display(), "ignoring skill with colliding name");
                continue;
            }
            skills.insert(
                name,
                Skill {
                    skill_dir,
                    skill_file,
                    description,
                },
            );
        }
        if skills.len() > MAX_DISCOVERED_SKILLS {
            anyhow::bail!(
                "discovered skill count {} exceeds the limit of {MAX_DISCOVERED_SKILLS}",
                skills.len()
            )
        }
        let catalog_bytes = skills
            .iter()
            .map(|(name, skill)| 5 + name.len() + skill.description.len())
            .sum::<usize>()
            .saturating_sub(1);
        if catalog_bytes > MAX_SKILL_CATALOG_BYTES {
            anyhow::bail!("skill catalog exceeds the {MAX_SKILL_CATALOG_BYTES}-byte limit")
        }
        Ok(Self { skills })
    }

    pub(crate) fn names(&self) -> impl Iterator<Item = &str> {
        self.skills.keys().map(String::as_str)
    }
    pub(crate) fn descriptions(&self) -> impl Iterator<Item = (&str, &str)> {
        self.skills
            .iter()
            .map(|(n, s)| (n.as_str(), s.description.as_str()))
    }
    pub(crate) fn catalog(&self) -> String {
        self.descriptions()
            .map(|(n, d)| format!("- {n}: {d}"))
            .collect::<Vec<_>>()
            .join("\n")
    }
    pub(crate) fn get(&self, name: &str) -> Option<&Skill> {
        self.skills.get(name)
    }
    pub(crate) fn load(&self, name: &str) -> anyhow::Result<ParsedSkill> {
        let skill = self
            .skills
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("unknown skill: {name}"))?;
        let file = fs::canonicalize(&skill.skill_file)?;
        if !file.starts_with(&skill.skill_dir) || file != skill.skill_file {
            anyhow::bail!("registered SKILL.md changed location")
        }
        let parsed = parse_skill_file(&file)?;
        if parsed.name != name {
            anyhow::bail!("registered SKILL.md name changed")
        }
        Ok(parsed)
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }
}

#[cfg(test)]
mod tests;
