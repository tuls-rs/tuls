use super::*;

#[test]
fn normalize_collapses_dots() {
    assert_eq!(
        normalize_path(Path::new("/a/b/../c/./d")),
        PathBuf::from("/a/c/d")
    );
    assert_eq!(normalize_path(Path::new("/a/././b")), PathBuf::from("/a/b"));
}

#[test]
fn normalize_keeps_leading_parents_for_relative_paths() {
    assert_eq!(normalize_path(Path::new("../a/b")), PathBuf::from("../a/b"));
    assert_eq!(normalize_path(Path::new("../..")), PathBuf::from("../.."));
    assert_eq!(
        normalize_path(Path::new("../../a")),
        PathBuf::from("../../a")
    );
    assert_eq!(
        normalize_path(Path::new("a/../../b")),
        PathBuf::from("../b")
    );
}

#[test]
fn is_within_is_component_aware() {
    let roots = vec![PathBuf::from("/foo/bar")];
    assert!(is_within(Path::new("/foo/bar"), &roots));
    assert!(is_within(Path::new("/foo/bar/baz"), &roots));
    assert!(!is_within(Path::new("/foo/bar2"), &roots));
    assert!(!is_within(Path::new("/foo"), &roots));
}

#[test]
fn expand_home_replaces_tilde() {
    // Resolves via HOME on Unix, USERPROFILE on Windows runners.
    let home = home_dir().expect("home dir resolvable in test env");
    assert_eq!(expand_home(Path::new("~")), home);
    assert_eq!(expand_home(Path::new("~/doc.txt")), home.join("doc.txt"));
    assert_eq!(expand_home(Path::new("/abs")), PathBuf::from("/abs"));
    assert_eq!(
        expand_home(Path::new("rel/path")),
        PathBuf::from("rel/path")
    );
}

// The lookups below use an injected env closure instead of touching the
// process-global environment, so tests can run in parallel safely.
fn env_of<'a>(entries: &'a [(&'a str, &'a str)]) -> impl FnMut(&str) -> Option<OsString> + 'a {
    let entries: Vec<(&'a str, &'a str)> = entries.to_vec();
    move |key| {
        entries
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| OsString::from(*v))
    }
}

#[test]
fn home_dir_prefers_home() {
    let get = env_of(&[("HOME", "/home/usr"), ("USERPROFILE", "C:\\Users\\bob")]);
    assert_eq!(home_dir_from(get), Some(PathBuf::from("/home/usr")));
}

#[test]
fn home_dir_falls_back_to_userprofile() {
    let get = env_of(&[("USERPROFILE", "C:\\Users\\bob")]);
    assert_eq!(home_dir_from(get), Some(PathBuf::from("C:\\Users\\bob")));
}

#[test]
fn home_dir_joins_homedrive_and_homepath() {
    let get = env_of(&[("HOMEDRIVE", "C:"), ("HOMEPATH", "\\Users\\bob")]);
    // `\` is a separator on Windows but a plain character on Unix, so
    // the joined result differs per platform.
    #[cfg(windows)]
    let expected = PathBuf::from("C:\\Users\\bob");
    #[cfg(not(windows))]
    let expected = PathBuf::from("C:/\\Users\\bob");
    assert_eq!(home_dir_from(get), Some(expected));
}

#[test]
fn home_dir_requires_both_homedrive_and_homepath() {
    let get = env_of(&[("HOMEDRIVE", "C:")]);
    assert_eq!(home_dir_from(get), None);
}

#[test]
fn home_dir_skips_empty_values() {
    let get = env_of(&[("HOME", ""), ("HOMEDRIVE", "C:"), ("HOMEPATH", "")]);
    assert_eq!(home_dir_from(get), None);
}

#[tokio::test]
async fn relative_paths_use_first_configured_root_not_sorted_root() {
    let temp = tempfile::tempdir().unwrap();
    let first = temp.path().join("z-first");
    let second = temp.path().join("a-second");
    std::fs::create_dir_all(&first).unwrap();
    std::fs::create_dir_all(&second).unwrap();
    std::fs::write(first.join("target.txt"), "first").unwrap();
    std::fs::write(second.join("target.txt"), "second").unwrap();

    let access = AccessControl::from_args(&[first.clone(), second]).unwrap();
    let resolved = access.validate_path("target.txt").await.unwrap();
    assert_eq!(
        resolved,
        std::fs::canonicalize(first.join("target.txt")).unwrap()
    );
}
