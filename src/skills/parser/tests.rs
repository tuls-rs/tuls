use super::*;
use std::io::{Seek, SeekFrom, Write};
use tempfile::NamedTempFile;

#[test]
fn parses_and_removes_frontmatter() {
    let skill =
        parse_skill("---\nname: web-search\ndescription: Search web\n---\nUse this.\n").unwrap();
    assert_eq!(skill.name, "web-search");
    assert_eq!(skill.instructions, "Use this.\n");
}

#[test]
fn rejects_invalid_names_and_missing_frontmatter() {
    for name in ["Upper", "two--dash", "-start", "end-", ""] {
        assert!(parse_skill(&format!("---\nname: {name:?}\ndescription: x\n---\n")).is_err());
    }
    assert!(parse_skill("name: x").is_err());
    assert!(parse_skill("---\nname: demo\n---\n").is_err());
    assert!(parse_skill("---\ndescription: Demo\n---\n").is_err());
}

#[test]
fn permits_extra_frontmatter_metadata() {
    let skill = parse_skill(
        "---\nname: demo\ndescription: Demo\nlicense: MIT\nmetadata:\n  author: agent\n---\nbody\n",
    )
    .unwrap();
    assert_eq!(skill.name, "demo");
}

#[test]
fn rejects_oversized_files_without_reading_them() {
    let mut file = NamedTempFile::new().unwrap();
    file.write_all(b"x").unwrap();
    file.as_file_mut()
        .seek(SeekFrom::Start(MAX_SKILL_BYTES))
        .unwrap();
    file.write_all(b"x").unwrap();
    assert!(
        parse_skill_file(file.path())
            .unwrap_err()
            .to_string()
            .contains("1 MiB")
    );
}

#[test]
fn rejects_oversized_descriptions() {
    let input = format!(
        "---\nname: demo\ndescription: {}\n---\nbody\n",
        "d".repeat(MAX_SKILL_DESCRIPTION_BYTES + 1)
    );
    assert!(parse_skill(&input).is_err());
}
