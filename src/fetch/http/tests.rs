use super::*;

#[test]
fn robots_url_construction() {
    assert_eq!(
        robots_txt_url("https://example.com/page").unwrap(),
        "https://example.com/robots.txt"
    );
    assert_eq!(
        robots_txt_url("https://example.com/some/deep/path/page.html").unwrap(),
        "https://example.com/robots.txt"
    );
    assert_eq!(
        robots_txt_url("https://example.com/page?foo=bar&baz=qux").unwrap(),
        "https://example.com/robots.txt"
    );
    assert_eq!(
        robots_txt_url("https://example.com:8080/page").unwrap(),
        "https://example.com:8080/robots.txt"
    );
    assert_eq!(
        robots_txt_url("https://example.com/page#section").unwrap(),
        "https://example.com/robots.txt"
    );
    assert_eq!(
        robots_txt_url("http://example.com/page").unwrap(),
        "http://example.com/robots.txt"
    );
}

#[test]
fn robots_url_rejects_garbage() {
    assert!(robots_txt_url("not a url").is_err());
}

#[test]
fn extract_simple_html() {
    let html = "<html><body><article><h1>Hello World</h1><p>This is a test paragraph.</p></article></body></html>";
    let result = extract_content_from_html(html);
    assert!(result.contains("Hello World"), "got: {result}");
    assert!(result.contains("test paragraph"), "got: {result}");
}

#[test]
fn extract_html_with_link() {
    let html = "<html><body><article><p>Visit <a href=\"https://example.com\">Example</a> for more.</p></article></body></html>";
    let result = extract_content_from_html(html);
    assert!(result.contains("Example"), "got: {result}");
}

#[test]
fn extract_empty_returns_error() {
    let result = extract_content_from_html("");
    assert!(result.contains("<error>"));
}

#[test]
fn extract_drops_style_and_script_noise() {
    let html = "<html><head><style>body{color:red;background:#eee}</style>\
    <script>console.log('x')</script></head>\
    <body><article><h1>Real Title</h1><p>Real content.</p></article></body></html>";
    let result = extract_content_from_html(html);
    assert!(result.contains("Real Title"), "got: {result}");
    assert!(result.contains("Real content"), "got: {result}");
    assert!(!result.contains("color:red"), "style leaked: {result}");
    assert!(!result.contains("console.log"), "script leaked: {result}");
}

#[test]
fn truncate_basic_slice() {
    // Content fully consumed: no continuation hint.
    assert_eq!(truncate("abc", 0, 3), "abc");
    // Partial consumption with more content: hint added.
    let result = truncate("abcdef", 0, 3);
    assert!(result.starts_with("abc"));
    assert!(result.contains("startIndex of 3"));
    let result = truncate("abcdef", 2, 2);
    assert!(result.starts_with("cd"));
    assert!(result.contains("startIndex of 4"));
}

#[test]
fn truncate_with_continuation_hint() {
    let result = truncate("abcdefgh", 0, 5);
    assert!(result.starts_with("abcde"));
    assert!(result.contains("Content truncated"));
    assert!(result.contains("startIndex of 5"));
}

#[test]
fn truncate_exact_boundary_has_no_hint() {
    let result = truncate("abcdef", 0, 6);
    assert_eq!(result, "abcdef");
}

#[test]
fn truncate_past_end_returns_error() {
    assert_eq!(
        truncate("abcdef", 6, 5),
        "<error>No more content available.</error>"
    );
    assert_eq!(
        truncate("abcdef", 100, 5),
        "<error>No more content available.</error>"
    );
}

#[test]
fn truncate_is_character_indexed() {
    // Multibyte characters: "héllo" is 5 chars but more bytes.
    let result = truncate("héllo wörld", 0, 5);
    assert!(result.starts_with("héllo"));
    assert!(result.contains("startIndex of 5"));
    // Re-fetch from the continuation point.
    let result = truncate("héllo wörld", 6, 5);
    assert_eq!(result, "wörld");
}

#[test]
fn validates_user_agent() {
    assert!(validate_user_agent(DEFAULT_USER_AGENT).is_ok());
    assert!(validate_user_agent("").is_err());
    assert!(validate_user_agent("bad\nheader").is_err());
    assert!(validate_user_agent(&"x".repeat(257)).is_err());
}

#[test]
fn url_validation_rejects_credentials_and_non_http_schemes() {
    assert!(validate_http_url("https://example.com/path").is_ok());
    assert!(validate_http_url("https://user:secret@example.com/path").is_err());
    assert!(validate_http_url("file:///tmp/data").is_err());
}

#[tokio::test]
async fn public_network_policy_rejects_non_public_literal_addresses() {
    use crate::cli::NetworkPolicy;

    for url in [
        "http://127.0.0.1/",
        "http://10.0.0.1/",
        "http://169.254.169.254/",
        "http://[::1]/",
        "http://[fe80::1]/",
        "http://[fec0::1]/",
        "http://[64:ff9b::7f00:1]/",
        "http://[64:ff9b:1::a00:1]/",
        "http://[2001::1]/",
        "http://[2002:7f00:1::]/",
        "http://[::ffff:127.0.0.1]/",
        "http://[::ffff:10.0.0.1]/",
    ] {
        assert!(
            resolve_network_target(url, NetworkPolicy::Public)
                .await
                .is_err()
        );
        assert!(
            resolve_network_target(url, NetworkPolicy::Unrestricted)
                .await
                .is_ok()
        );
    }
}

#[test]
fn public_address_policy_has_auditable_boundaries() {
    use std::str::FromStr;

    for address in [
        "8.8.8.8",
        "100.63.255.255",
        "100.128.0.0",
        "223.255.255.254",
        "223.255.255.255",
        "2001:4860:4860::8888",
        "2606:4700:4700::1111",
    ] {
        assert!(
            validate_public_ip(IpAddr::from_str(address).unwrap()).is_ok(),
            "global address {address} was blocked"
        );
    }
    for address in [
        "0.255.255.255",
        "100.64.0.0",
        "100.127.255.255",
        "192.88.99.0",
        "192.88.99.255",
        "224.0.0.0",
        "255.255.255.255",
        "::",
        "::1",
        "::7f00:1",
        "64:ff9b::7f00:1",
        "64:ff9b:1::a00:1",
        "100::1",
        "2001::1",
        "2001:db8:ffff:ffff:ffff:ffff:ffff:ffff",
        "2002::1",
        "3fff:fff::1",
        "5f00::1",
        "fc00::1",
        "fe80::1",
        "fec0::1",
        "feff:ffff:ffff:ffff:ffff:ffff:ffff:ffff",
        "ff00::1",
    ] {
        assert!(
            validate_public_ip(IpAddr::from_str(address).unwrap()).is_err(),
            "special address {address} was allowed"
        );
    }
}

#[tokio::test]
async fn public_policy_blocks_local_hostnames_before_dns() {
    use crate::cli::NetworkPolicy;

    for url in [
        "http://localhost/",
        "http://localhost:8080/",
        "http://printer.local/",
        "http://host.localhost/",
    ] {
        let error = resolve_network_target(url, NetworkPolicy::Public)
            .await
            .unwrap_err();
        assert!(error.contains("local hostnames"), "hostname {url}: {error}");
    }
}

#[tokio::test]
async fn named_destinations_are_resolved_and_validated() {
    use crate::cli::NetworkPolicy;

    // `.invalid` is reserved (RFC 2606) and must never resolve; the
    // public-policy path fails closed either with a resolution error or
    // with the DNS timeout, and never proceeds to the request.
    let error = resolve_network_target("http://no-such-host.invalid/", NetworkPolicy::Public)
        .await
        .expect_err("reserved .invalid name must not resolve");
    assert!(
        error.contains("resolve") || error.contains("timed out"),
        "error: {error}"
    );

    // Unrestricted mode does not resolve or validate the destination at all.
    assert!(
        resolve_network_target("http://no-such-host.invalid/", NetworkPolicy::Unrestricted)
            .await
            .is_ok()
    );
}
