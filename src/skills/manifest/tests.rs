use super::*;
use tempfile::tempdir;

#[test]
fn lists_sorted_resources_without_skill() {
    let temp = tempdir().unwrap();
    fs::write(temp.path().join("SKILL.md"), "x").unwrap();
    fs::create_dir(temp.path().join("nested")).unwrap();
    fs::write(temp.path().join("nested/z.txt"), "z").unwrap();
    fs::write(temp.path().join("a.txt"), "a").unwrap();
    assert_eq!(
        resource_manifest(temp.path()).unwrap(),
        ["a.txt", "nested/z.txt"]
    );
}

#[cfg(unix)]
#[test]
fn rejects_file_symlinks_that_escape_the_skill() {
    use std::os::unix::fs::symlink;
    let skill = tempdir().unwrap();
    let outside = tempdir().unwrap();
    fs::write(outside.path().join("secret"), "x").unwrap();
    symlink(outside.path().join("secret"), skill.path().join("escape")).unwrap();
    assert!(resource_manifest(skill.path()).is_err());
}

#[test]
fn enforces_resource_count_and_depth() {
    let temp = tempdir().unwrap();
    for index in 0..=MAX_RESOURCES {
        fs::write(temp.path().join(format!("{index}.txt")), "x").unwrap();
    }
    assert!(resource_manifest(temp.path()).is_err());

    let deep = tempdir().unwrap();
    let mut dir = deep.path().to_path_buf();
    for index in 0..MAX_DEPTH {
        dir.push(format!("d{index}"));
        fs::create_dir(&dir).unwrap();
    }
    fs::write(dir.join("too-deep"), "x").unwrap();
    assert!(resource_manifest(deep.path()).is_err());
}

#[test]
fn enforces_resource_manifest_byte_limit() {
    let temp = tempdir().unwrap();
    for index in 0..400 {
        let name = format!("{index:03}-{}.txt", "x".repeat(180));
        fs::write(temp.path().join(name), "x").unwrap();
    }
    assert!(resource_manifest(temp.path()).is_err());
}
