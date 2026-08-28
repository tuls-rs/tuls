use super::*;

#[tokio::test]
async fn retains_output_up_to_limit() {
    let (bytes, truncated) = drain_limited(&b"hello"[..], 1024).await.unwrap();
    assert_eq!(bytes, b"hello");
    assert!(!truncated);
}

#[tokio::test]
async fn truncates_at_limit_and_keeps_draining() {
    let (bytes, truncated) = drain_limited(&b"abcdef"[..], 4).await.unwrap();
    assert_eq!(bytes, b"abcd");
    assert!(truncated);
}

#[tokio::test]
async fn drains_past_the_limit() {
    // 100 KiB of data with a 4 KiB limit: must return quickly with
    // exactly the first 4 KiB retained.
    let data = vec![b'x'; 100 * 1024];
    let (bytes, truncated) = drain_limited(data.as_slice(), 4 * 1024).await.unwrap();
    assert_eq!(bytes.len(), 4 * 1024);
    assert!(truncated);
}

#[tokio::test]
async fn exact_limit_is_not_truncated() {
    let (bytes, truncated) = drain_limited(&b"abcd"[..], 4).await.unwrap();
    assert_eq!(bytes, b"abcd");
    assert!(!truncated);
}

#[tokio::test]
async fn empty_stream_is_not_truncated() {
    let (bytes, truncated) = drain_limited(&b""[..], 1024).await.unwrap();
    assert!(bytes.is_empty());
    assert!(!truncated);
}
