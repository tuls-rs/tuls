use super::*;
use tempfile::tempdir;

fn write(root: &Path, source: &str, dir: &str, name: &str) {
    let path = root.join(source).join(dir);
    fs::create_dir_all(&path).unwrap();
    fs::write(
        path.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: {source}\n---\nbody"),
    )
    .unwrap();
}

#[test]
fn precedence_and_catalog_are_deterministic() {
    let temp = tempdir().unwrap();
    write(temp.path(), ".agents/skills", "a", "same");
    write(temp.path(), ".claude/skills", "b", "other");
    write(temp.path(), ".claude/skills", "z", "same");
    let registry = SkillRegistry::discover(temp.path()).unwrap();
    assert_eq!(
        registry.catalog(),
        "- other: .claude/skills\n- same: .agents/skills"
    );
}

#[test]
fn discovers_only_supported_roots() {
    let temp = tempdir().unwrap();
    write(temp.path(), ".agents/skills", "agent", "agent");
    write(temp.path(), ".claude/skills", "claude", "claude");
    write(temp.path(), ".unused/skills", "ignored", "ignored");
    assert_eq!(
        SkillRegistry::discover(temp.path())
            .unwrap()
            .names()
            .collect::<Vec<_>>(),
        ["agent", "claude"]
    );
}

#[test]
fn malformed_candidate_does_not_block_valid_one() {
    let temp = tempdir().unwrap();
    write(temp.path(), ".agents/skills", "good", "good");
    let bad = temp.path().join(".claude/skills/bad");
    fs::create_dir_all(&bad).unwrap();
    fs::write(bad.join("SKILL.md"), "bad").unwrap();
    assert_eq!(
        SkillRegistry::discover(temp.path())
            .unwrap()
            .names()
            .collect::<Vec<_>>(),
        ["good"]
    );
}

#[test]
fn lexical_path_breaks_same_root_name_ties() {
    let temp = tempdir().unwrap();
    write(temp.path(), ".agents/skills", "z", "same");
    write(temp.path(), ".agents/skills", "a", "same");
    let registry = SkillRegistry::discover(temp.path()).unwrap();
    assert!(registry.get("same").unwrap().skill_dir.ends_with("a"));
}

#[cfg(unix)]
#[test]
fn canonical_identity_is_deduplicated_before_name_deduplication() {
    use std::os::unix::fs::symlink;
    let temp = tempdir().unwrap();
    write(temp.path(), ".agents/skills", "real", "real");
    let root = temp.path().join(".agents/skills");
    symlink(root.join("real"), root.join("alias")).unwrap();
    let registry = SkillRegistry::discover(temp.path()).unwrap();
    assert_eq!(registry.names().collect::<Vec<_>>(), ["real"]);
}

#[cfg(unix)]
#[test]
fn symlinked_skill_cannot_escape_supported_skill_root() {
    use std::os::unix::fs::symlink;

    let temp = tempdir().unwrap();
    let outside = temp.path().join("elsewhere");
    fs::create_dir_all(&outside).unwrap();
    fs::write(
        outside.join("SKILL.md"),
        "---\nname: escaped\ndescription: escaped\n---\nbody",
    )
    .unwrap();
    let root = temp.path().join(".agents/skills");
    fs::create_dir_all(&root).unwrap();
    symlink(&outside, root.join("escaped")).unwrap();

    let registry = SkillRegistry::discover(temp.path()).unwrap();
    assert!(registry.get("escaped").is_none());
}

#[test]
fn discovered_skill_count_and_combined_catalog_are_bounded() {
    let count = tempdir().unwrap();
    for index in 0..=MAX_DISCOVERED_SKILLS {
        let name = format!("skill-{index}");
        write(count.path(), ".agents/skills", &name, &name);
    }
    assert!(SkillRegistry::discover(count.path()).is_err());

    let catalog = tempdir().unwrap();
    for index in 0..17 {
        let name = format!("skill-{index}");
        let path = catalog.path().join(".agents/skills").join(&name);
        fs::create_dir_all(&path).unwrap();
        fs::write(
            path.join("SKILL.md"),
            format!(
                "---\nname: {name}\ndescription: {}\n---\nbody",
                "d".repeat(super::super::parser::MAX_SKILL_DESCRIPTION_BYTES)
            ),
        )
        .unwrap();
    }
    assert!(SkillRegistry::discover(catalog.path()).is_err());
}
