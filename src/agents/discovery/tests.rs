use super::*;
use crate::agents::definition::MAX_AGENT_DESCRIPTION_BYTES;
use std::io::Write;
fn write(root: &Path, name: &str, body: &str) {
    let path = root.join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::File::create(path)
        .unwrap()
        .write_all(body.as_bytes())
        .unwrap();
}
#[test]
fn duplicate_names_use_lexical_canonical_path_and_skip_malformed_files() {
    let temp = tempfile::tempdir().unwrap();
    let canonical = "---\nname: same\ndescription: x\nprovider: openai\nmodel: g\n---\na";
    write(temp.path(), ".agents/agents/a.md", canonical);
    write(temp.path(), ".agents/agents/z.md", canonical);
    write(temp.path(), ".agents/agents/bad.md", "not frontmatter");
    let registry = AgentRegistry::discover(temp.path()).unwrap();
    assert!(registry.get_subagent("bad").is_none());
    assert!(
        registry
            .get_subagent("same")
            .unwrap()
            .source_path
            .ends_with("a.md")
    );
}

#[test]
fn discovers_supported_recursive_markdown_layouts_only() {
    let temp = tempfile::tempdir().unwrap();
    for (path, name) in [
        (".agents/agents/reviewer.md", "reviewer"),
        (".agents/agents/security/auditor.md", "auditor"),
        (".agents/agents/writer/agent.md", "writer"),
    ] {
        write(
            temp.path(),
            path,
            &format!("---\nname: {name}\ndescription: x\nprovider: openai\nmodel: g\n---\nx"),
        );
    }
    write(
        temp.path(),
        ".agents/agents/ignored.toml",
        "name = \"toml-agent\"",
    );
    write(
        temp.path(),
        ".claude/agents/claude.md",
        "---\nname: claude\ndescription: x\nprovider: openai\nmodel: g\n---\nx",
    );
    assert_eq!(
        AgentRegistry::discover(temp.path()).unwrap().names(),
        vec!["auditor", "reviewer", "writer"]
    );
}
#[cfg(unix)]
#[test]
fn file_symlink_alias_is_deduplicated() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    let body = "---\nname: a\ndescription: x\nprovider: openai\nmodel: g\n---\nx";
    write(temp.path(), ".agents/agents/real.md", body);
    symlink(
        temp.path().join(".agents/agents/real.md"),
        temp.path().join(".agents/agents/alias.md"),
    )
    .unwrap();
    assert_eq!(
        AgentRegistry::discover(temp.path()).unwrap().names(),
        vec!["a"]
    );
}

#[cfg(unix)]
#[test]
fn directory_symlink_cannot_escape_supported_agent_root() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let outside = temp.path().join("elsewhere");
    fs::create_dir_all(&outside).unwrap();
    write(
        &outside,
        "escaped.md",
        "---\nname: escaped\ndescription: x\nprovider: openai\nmodel: g\n---\nx",
    );
    let root = temp.path().join(".agents/agents");
    fs::create_dir_all(&root).unwrap();
    symlink(&outside, root.join("escaped")).unwrap();

    let registry = AgentRegistry::discover(temp.path()).unwrap();
    assert!(registry.get_subagent("escaped").is_none());
}

#[test]
fn subagent_eligibility_filters_exposure_after_full_discovery() {
    let temp = tempfile::tempdir().unwrap();
    write(
        temp.path(),
        ".agents/agents/default.md",
        "---\nname: default\ndescription: visible by default\nprovider: openai\nmodel: g\n---\nx",
    );
    write(
        temp.path(),
        ".agents/agents/explicit.md",
        "---\nname: explicit\ndescription: explicitly visible\nprovider: openai\nmodel: g\nsubagent: true\n---\nx",
    );
    write(
        temp.path(),
        ".agents/agents/nested/leader.md",
        "---\nname: leader\ndescription: hidden leader\nprovider: openai\nmodel: g\nsubagent: false\n---\nx",
    );

    let registry = AgentRegistry::discover(temp.path()).unwrap();
    assert_eq!(registry.names(), vec!["default", "explicit"]);
    assert_eq!(
        registry
            .catalog()
            .iter()
            .map(|agent| agent.name.as_str())
            .collect::<Vec<_>>(),
        ["default", "explicit"]
    );
    assert!(registry.get_subagent("leader").is_none());
    assert!(!registry.agents["leader"].subagent);
    assert!(!registry.is_empty());
}

#[test]
fn hidden_lexical_winner_cannot_be_resurrected_by_visible_duplicate() {
    let temp = tempfile::tempdir().unwrap();
    write(
        temp.path(),
        ".agents/agents/a.md",
        "---\nname: leader\ndescription: hidden winner\nprovider: openai\nmodel: g\nsubagent: false\n---\nx",
    );
    write(
        temp.path(),
        ".agents/agents/z.md",
        "---\nname: leader\ndescription: visible duplicate\nprovider: openai\nmodel: g\nsubagent: true\n---\nx",
    );

    let registry = AgentRegistry::discover(temp.path()).unwrap();
    assert!(registry.is_empty());
    assert!(registry.names().is_empty());
    assert!(registry.get_subagent("leader").is_none());
    assert!(registry.agents["leader"].source_path.ends_with("a.md"));
}

fn populate(root: &Path, count: usize) {
    for index in 0..count {
        let name = format!("agent-{index:03}");
        let body = format!("---\nname: {name}\ndescription: x\nprovider: openai\nmodel: g\n---\nx");
        write(root, &format!(".agents/agents/{name}.md"), &body);
    }
}

#[test]
fn discovered_agent_count_is_fail_closed_at_256() {
    let at_limit = tempfile::tempdir().unwrap();
    populate(at_limit.path(), MAX_DISCOVERED_AGENTS);
    assert_eq!(
        AgentRegistry::discover(at_limit.path())
            .unwrap()
            .names()
            .len(),
        MAX_DISCOVERED_AGENTS
    );

    let over_limit = tempfile::tempdir().unwrap();
    populate(over_limit.path(), MAX_DISCOVERED_AGENTS + 1);
    assert!(AgentRegistry::discover(over_limit.path()).is_err());
}

#[test]
fn hidden_definitions_still_count_toward_discovery_limits() {
    let at_limit = tempfile::tempdir().unwrap();
    for index in 0..MAX_DISCOVERED_AGENTS {
        let name = format!("hidden-{index:03}");
        write(
            at_limit.path(),
            &format!(".agents/agents/{name}.md"),
            &format!(
                "---\nname: {name}\ndescription: x\nprovider: openai\nmodel: g\nsubagent: false\n---\nx"
            ),
        );
    }
    let registry = AgentRegistry::discover(at_limit.path()).unwrap();
    assert_eq!(registry.agents.len(), MAX_DISCOVERED_AGENTS);
    assert!(registry.is_empty());

    let over_limit = tempfile::tempdir().unwrap();
    for index in 0..=MAX_DISCOVERED_AGENTS {
        let name = format!("hidden-{index:03}");
        write(
            over_limit.path(),
            &format!(".agents/agents/{name}.md"),
            &format!(
                "---\nname: {name}\ndescription: x\nprovider: openai\nmodel: g\nsubagent: false\n---\nx"
            ),
        );
    }
    assert!(AgentRegistry::discover(over_limit.path()).is_err());
}

#[test]
fn oversized_agent_file_is_rejected_by_the_bounded_reader() {
    use std::io::{Seek, SeekFrom};

    let mut file = tempfile::NamedTempFile::new().unwrap();
    file.write_all(b"x").unwrap();
    file.seek(SeekFrom::Start(MAX_AGENT_FILE_BYTES as u64))
        .unwrap();
    file.write_all(b"x").unwrap();
    assert!(
        read_agent_file(file.path())
            .unwrap_err()
            .to_string()
            .contains("1 MiB")
    );
}

#[test]
fn combined_agent_catalog_is_bounded() {
    let temp = tempfile::tempdir().unwrap();
    for index in 0..17 {
        let name = format!("agent-{index}");
        let body = format!(
            "---\nname: {name}\ndescription: {}\nprovider: openai\nmodel: g\n---\nx",
            "d".repeat(MAX_AGENT_DESCRIPTION_BYTES)
        );
        write(temp.path(), &format!(".agents/agents/{name}.md"), &body);
    }
    assert!(AgentRegistry::discover(temp.path()).is_err());
}

#[test]
fn hidden_descriptions_do_not_consume_exposed_catalog_budget() {
    let temp = tempfile::tempdir().unwrap();
    for index in 0..17 {
        let name = format!("hidden-{index}");
        let body = format!(
            "---\nname: {name}\ndescription: {}\nprovider: openai\nmodel: g\nsubagent: false\n---\nx",
            "d".repeat(MAX_AGENT_DESCRIPTION_BYTES)
        );
        write(temp.path(), &format!(".agents/agents/{name}.md"), &body);
    }
    let registry = AgentRegistry::discover(temp.path()).unwrap();
    assert_eq!(registry.agents.len(), 17);
    assert!(registry.is_empty());
}
