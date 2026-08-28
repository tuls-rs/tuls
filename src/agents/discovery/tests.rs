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
fn same_precedence_uses_lexical_canonical_path_and_skips_malformed() {
    let temp = tempfile::tempdir().unwrap();
    let canonical = "---\nname: same\ndescription: x\nmodel: g\nmodel_provider: openai\n---\na";
    write(temp.path(), ".agents/agents/a.md", canonical);
    write(temp.path(), ".agents/agents/z.md", canonical);
    write(
        temp.path(),
        ".claude/agents/bad.md",
        "---\nname: bad\ndescription: x\nmodel: c\nisolation: vm\n---\nx",
    );
    let registry = AgentRegistry::discover(temp.path()).unwrap();
    assert!(registry.get("bad").is_none());
    assert!(registry.get("same").unwrap().source_path.ends_with("a.md"));
}
#[cfg(unix)]
#[test]
fn file_symlink_alias_is_deduplicated() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    let body = "---\nname: a\ndescription: x\nmodel: g\nmodel_provider: openai\n---\nx";
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
        "---\nname: escaped\ndescription: x\nmodel: g\nmodel_provider: openai\n---\nx",
    );
    let root = temp.path().join(".agents/agents");
    fs::create_dir_all(&root).unwrap();
    symlink(&outside, root.join("escaped")).unwrap();

    let registry = AgentRegistry::discover(temp.path()).unwrap();
    assert!(registry.get("escaped").is_none());
}

fn populate(root: &Path, count: usize) {
    for index in 0..count {
        let name = format!("agent-{index:03}");
        let body =
            format!("---\nname: {name}\ndescription: x\nmodel: g\nmodel_provider: openai\n---\nx");
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
            "---\nname: {name}\ndescription: {}\nmodel: g\nmodel_provider: openai\n---\nx",
            "d".repeat(MAX_AGENT_DESCRIPTION_BYTES)
        );
        write(temp.path(), &format!(".agents/agents/{name}.md"), &body);
    }
    assert!(AgentRegistry::discover(temp.path()).is_err());
}
