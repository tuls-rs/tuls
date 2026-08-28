use super::*;
use std::os::unix::fs::PermissionsExt;

#[tokio::test]
async fn atomic_rewrite_preserves_existing_mode() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("target.txt");
    std::fs::write(&path, "old").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();

    atomic_write(&path, b"new contents").await.unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "new contents");
    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "mode {mode:#o}");
}

#[tokio::test]
async fn new_file_uses_normal_creation_mode() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fresh.txt");
    atomic_write(&path, b"x").await.unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    // A normal file create always grants the owner read/write under any
    // reasonable umask; a too-narrow mode would mean we pinned a stale
    // permission instead of creating fresh.
    assert_eq!(mode & 0o600, 0o600, "mode {mode:#o}");
}
