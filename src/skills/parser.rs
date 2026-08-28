use std::{fs, io::Read, path::Path};

use serde::Deserialize;

pub(crate) const MAX_SKILL_BYTES: u64 = 1024 * 1024;
pub(crate) const MAX_SKILL_DESCRIPTION_BYTES: usize = 4 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParsedSkill {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) instructions: String,
}

#[derive(Deserialize)]
struct Frontmatter {
    name: String,
    description: String,
}

pub(crate) fn parse_skill_file(path: &Path) -> anyhow::Result<ParsedSkill> {
    let file = fs::File::open(path)?;
    if file.metadata()?.len() > MAX_SKILL_BYTES {
        anyhow::bail!("SKILL.md exceeds the 1 MiB limit")
    }
    let mut bytes = Vec::with_capacity((MAX_SKILL_BYTES as usize).min(64 * 1024));
    file.take(MAX_SKILL_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_SKILL_BYTES {
        anyhow::bail!("SKILL.md exceeds the 1 MiB limit")
    }
    let text = String::from_utf8(bytes).map_err(|_| anyhow::anyhow!("SKILL.md is not UTF-8"))?;
    parse_skill(&text)
}

pub(crate) fn parse_skill(text: &str) -> anyhow::Result<ParsedSkill> {
    let mut lines = text.split_inclusive('\n');
    let Some(first) = lines.next() else {
        anyhow::bail!("SKILL.md is empty")
    };
    if first.trim_end_matches(['\r', '\n']) != "---" {
        anyhow::bail!("SKILL.md must begin with YAML frontmatter")
    }
    let mut yaml = String::new();
    let mut closed = false;
    for line in lines.by_ref() {
        if line.trim_end_matches(['\r', '\n']) == "---" {
            closed = true;
            break;
        }
        yaml.push_str(line);
    }
    if !closed {
        anyhow::bail!("SKILL.md frontmatter is not closed")
    }
    let frontmatter: Frontmatter = noyalib::from_str(&yaml)
        .map_err(|e| anyhow::anyhow!("invalid SKILL.md frontmatter: {e}"))?;
    if !valid_name(&frontmatter.name) {
        anyhow::bail!("skill name must match ^[a-z0-9]+(-[a-z0-9]+)*$ and be at most 64 characters")
    }
    if frontmatter.description.trim().is_empty()
        || frontmatter.description.len() > MAX_SKILL_DESCRIPTION_BYTES
    {
        anyhow::bail!("skill description must contain 1 to {MAX_SKILL_DESCRIPTION_BYTES} bytes")
    }
    Ok(ParsedSkill {
        name: frontmatter.name,
        description: frontmatter.description,
        instructions: lines.collect(),
    })
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--")
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

#[cfg(test)]
mod tests;
