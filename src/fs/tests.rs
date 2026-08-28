use std::time::{Duration, UNIX_EPOCH};

use super::{read_bounded_file, read_text_file_limited, rfc3339};

#[test]
fn timestamps_before_unix_epoch_are_formatted_correctly() {
    let time = UNIX_EPOCH
        .checked_sub(Duration::from_secs(1))
        .expect("valid time");
    assert_eq!(rfc3339(Ok(time)), "1969-12-31T23:59:59Z");
}

#[test]
fn timestamp_io_errors_are_reported_as_unknown() {
    let error = std::io::Error::other("metadata unavailable");
    assert_eq!(rfc3339(Err(error)), "unknown");
}

#[tokio::test]
async fn bounded_file_reader_rejects_oversized_input() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("large.txt");
    tokio::fs::write(&path, b"12345").await.expect("write");

    let error = read_bounded_file(&path, 4).await.expect_err("size limit");
    assert!(error.contains("4 byte limit"), "{error}");
}

#[tokio::test]
async fn text_reader_rejects_invalid_utf8() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("binary.txt");
    tokio::fs::write(&path, [0xff, 0xfe]).await.expect("write");

    let error = read_text_file_limited(&path, 16)
        .await
        .expect_err("invalid UTF-8");
    assert!(error.contains("not valid UTF-8"), "{error}");
}
