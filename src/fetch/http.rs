use std::{net::IpAddr, time::Duration};

use url::{Host, Url};

use crate::cli::NetworkPolicy;

pub const DEFAULT_USER_AGENT: &str = concat!("tuls/", env!("CARGO_PKG_VERSION"));

const MAX_USER_AGENT_BYTES: usize = 256;
pub const MAX_URL_CHARS: usize = 8 * 1024;
const MAX_CONTENT_TYPE_BYTES: usize = 256;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Bound on DNS resolution of a named destination so a broken or hanging
/// resolver cannot stall a public-policy fetch indefinitely. The HTTP
/// request itself keeps its own 30-second timeout.
const DNS_RESOLUTION_TIMEOUT: Duration = Duration::from_secs(10);

/// Hard upper bound on the raw HTTP response body the fetch client will
/// buffer. `maxLength` in the fetch tool is character-based output
/// truncation and must never be treated as the network safety limit, so an
/// oversized remote body is aborted here regardless of `maxLength`.
pub const MAX_RESPONSE_BODY_BYTES: usize = 8 * 1024 * 1024;

/// HTTP client with a fixed timeout, bounded responses, no automatic
/// redirects, and an explicit outbound network policy.
pub struct FetchClient {
    client: reqwest::Client,
    network: NetworkPolicy,
}

impl FetchClient {
    pub fn new(proxy_url: Option<&str>, network: NetworkPolicy) -> Result<Self, String> {
        if proxy_url.is_some() && network != NetworkPolicy::Unrestricted {
            return Err(
                "HTTP proxies require --network unrestricted because proxy-side DNS and routing cannot be constrained by the local public-network policy"
                    .to_string(),
            );
        }
        let mut builder = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none());
        if let Some(proxy) = proxy_url {
            let proxy = reqwest::Proxy::all(proxy).map_err(|_| "invalid proxy URL".to_string())?;
            builder = builder.proxy(proxy);
        }
        let client = builder
            .build()
            .map_err(|error| format!("Failed to build HTTP client: {error}"))?;
        Ok(Self { client, network })
    }

    /// GET a URL with the given User-Agent. Returns body, content type, and status.
    pub async fn get(&self, url: &str, user_agent: &str) -> Result<(String, String, u16), String> {
        let dns_override = resolve_network_target(url, self.network).await?;
        let client = if let Some((host, addresses)) = dns_override {
            reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .resolve_to_addrs(&host, &addresses)
                .build()
                .map_err(|error| format!("Failed to build HTTP client: {error}"))?
        } else {
            self.client.clone()
        };
        let response = client
            .get(url)
            .header(reqwest::header::USER_AGENT, user_agent)
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await
            .map_err(|error| {
                if error.is_timeout() {
                    "HTTP request timed out".to_string()
                } else {
                    "HTTP request failed".to_string()
                }
            })?;
        read_bounded_response(response).await
    }
}

/// Resolve and validate a public destination. Domain resolutions are returned
/// so the request client can pin the validated addresses and avoid a second
/// DNS lookup between authorization and connection establishment.
async fn resolve_network_target(
    url: &str,
    policy: NetworkPolicy,
) -> Result<Option<(String, Vec<std::net::SocketAddr>)>, String> {
    if policy == NetworkPolicy::Unrestricted {
        return Ok(None);
    }
    let parsed = validate_http_url(url)?;
    match parsed.host() {
        Some(Host::Ipv4(address)) => {
            validate_public_ip(IpAddr::V4(address))?;
            Ok(None)
        }
        Some(Host::Ipv6(address)) => {
            validate_public_ip(IpAddr::V6(address))?;
            Ok(None)
        }
        Some(Host::Domain(host)) => {
            let lower = host.to_ascii_lowercase();
            if lower == "localhost" || lower.ends_with(".localhost") || lower.ends_with(".local") {
                return Err("network policy blocks local hostnames".to_string());
            }
            let port = parsed
                .port_or_known_default()
                .ok_or_else(|| "URL has no usable port".to_string())?;
            let addresses = tokio::time::timeout(
                DNS_RESOLUTION_TIMEOUT,
                tokio::net::lookup_host((host, port)),
            )
            .await
            .map_err(|_| "DNS resolution timed out".to_string())?
            .map_err(|_| "failed to resolve destination hostname".to_string())?
            .collect::<Vec<_>>();
            if addresses.is_empty() {
                return Err("destination hostname resolved to no addresses".to_string());
            }
            for address in &addresses {
                validate_public_ip(address.ip())?;
            }
            Ok(Some((host.to_owned(), addresses)))
        }
        None => Err("URL must include a host".to_string()),
    }
}

fn validate_public_ip(address: IpAddr) -> Result<(), String> {
    let public = match address {
        IpAddr::V4(address) => !IPV4_NON_PUBLIC_RANGES
            .iter()
            .any(|&(network, prefix)| ipv4_in_prefix(address, network, prefix)),
        IpAddr::V6(address) => {
            if let Some(mapped) = address.to_ipv4_mapped() {
                return validate_public_ip(IpAddr::V4(mapped));
            }
            !IPV6_NON_PUBLIC_RANGES
                .iter()
                .any(|&(network, prefix)| ipv6_in_prefix(address, network, prefix))
        }
    };
    if public {
        Ok(())
    } else {
        Err(format!(
            "network policy blocks non-public destination address {address}"
        ))
    }
}

const IPV4_NON_PUBLIC_RANGES: &[(u32, u8)] = &[
    (0x0000_0000, 8),
    (0x0a00_0000, 8),
    (0x6440_0000, 10),
    (0x7f00_0000, 8),
    (0xa9fe_0000, 16),
    (0xac10_0000, 12),
    (0xc000_0000, 24),
    (0xc000_0200, 24),
    (0xc058_6300, 24),
    (0xc0a8_0000, 16),
    (0xc612_0000, 15),
    (0xc633_6400, 24),
    (0xcb00_7100, 24),
    (0xe000_0000, 4),
    (0xf000_0000, 4),
];

const IPV6_NON_PUBLIC_RANGES: &[(u128, u8)] = &[
    (0, 96),                                         // IPv4-compatible and unspecified space
    (1, 128),                                        // loopback
    (0x0064_ff9b_0000_0000_0000_0000_0000_0000, 96), // NAT64 well-known
    (0x0064_ff9b_0001_0000_0000_0000_0000_0000, 48), // NAT64 local-use
    (0x0100_0000_0000_0000_0000_0000_0000_0000, 64), // discard-only
    (0x2001_0000_0000_0000_0000_0000_0000_0000, 32), // Teredo
    (0x2001_0002_0000_0000_0000_0000_0000_0000, 48), // benchmarking
    (0x2001_0010_0000_0000_0000_0000_0000_0000, 28), // ORCHID
    (0x2001_0020_0000_0000_0000_0000_0000_0000, 28), // ORCHIDv2
    (0x2001_0db8_0000_0000_0000_0000_0000_0000, 32), // documentation
    (0x2002_0000_0000_0000_0000_0000_0000_0000, 16), // 6to4
    (0x3fff_0000_0000_0000_0000_0000_0000_0000, 20), // documentation
    (0x5f00_0000_0000_0000_0000_0000_0000_0000, 16), // segment-routing SIDs
    (0xfc00_0000_0000_0000_0000_0000_0000_0000, 7),  // unique-local
    (0xfe80_0000_0000_0000_0000_0000_0000_0000, 10), // link-local
    (0xfec0_0000_0000_0000_0000_0000_0000_0000, 10), // deprecated site-local
    (0xff00_0000_0000_0000_0000_0000_0000_0000, 8),  // multicast
];

fn ipv4_in_prefix(address: std::net::Ipv4Addr, network: u32, prefix: u8) -> bool {
    let mask = u32::MAX.checked_shl(u32::from(32 - prefix)).unwrap_or(0);
    u32::from(address) & mask == network & mask
}

fn ipv6_in_prefix(address: std::net::Ipv6Addr, network: u128, prefix: u8) -> bool {
    let mask = u128::MAX.checked_shl(u32::from(128 - prefix)).unwrap_or(0);
    u128::from(address) & mask == network & mask
}

/// Read a response body with the hard [`MAX_RESPONSE_BODY_BYTES`] bound: reject an oversized `Content-Length` up front (without relying
/// on it), then read chunk-by-chunk and abort as soon as the limit would be
/// exceeded. Never allocates an unbounded buffer and never includes remote
/// body content in error messages.
async fn read_bounded_response(
    mut response: reqwest::Response,
) -> Result<(String, String, u16), String> {
    let status = response.status().as_u16();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();

    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BODY_BYTES as u64)
    {
        return Err(response_too_large());
    }

    let mut body = Vec::with_capacity(
        response
            .content_length()
            .map(|length| length.min(MAX_RESPONSE_BODY_BYTES as u64) as usize)
            .unwrap_or(0),
    );
    loop {
        let chunk = response
            .chunk()
            .await
            .map_err(|_| "Failed to read response body".to_string())?;
        let Some(chunk) = chunk else {
            break;
        };
        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BODY_BYTES {
            return Err(response_too_large());
        }
        body.extend_from_slice(&chunk);
    }
    Ok((decode_text(&body), content_type, status))
}

fn response_too_large() -> String {
    format!("Response body exceeds the {MAX_RESPONSE_BODY_BYTES} byte safety limit")
}

/// Decode a bounded response body as text. Strips a UTF-8 byte-order mark
/// and decodes lossily (invalid sequences become U+FFFD). Declared non-UTF-8
/// charsets are decoded as UTF-8 to keep the binary small and deterministic.
fn decode_text(body: &[u8]) -> String {
    let body = body.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(body);
    String::from_utf8_lossy(body).into_owned()
}

/// Build the robots.txt URL for a website URL.
pub fn validate_user_agent(user_agent: &str) -> Result<(), String> {
    if user_agent.is_empty() {
        return Err("user agent must not be empty".to_string());
    }
    if user_agent.len() > MAX_USER_AGENT_BYTES {
        return Err(format!(
            "user agent exceeds the {MAX_USER_AGENT_BYTES} byte limit"
        ));
    }
    reqwest::header::HeaderValue::from_str(user_agent)
        .map(|_| ())
        .map_err(|_| "user agent contains invalid HTTP header characters".to_string())
}

pub fn validate_http_url(url: &str) -> Result<Url, String> {
    if url.chars().count() > MAX_URL_CHARS {
        return Err(format!("URL exceeds the {MAX_URL_CHARS}-character limit"));
    }
    let parsed = Url::parse(url).map_err(|error| format!("Invalid URL {url:?}: {error}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(format!(
            "Unsupported URL scheme {:?}: only http and https are supported",
            parsed.scheme()
        ));
    }
    if parsed.host().is_none() {
        return Err(format!("URL must include a host: {url:?}"));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(
            "URL credentials are not supported; use request configuration instead".to_string(),
        );
    }
    Ok(parsed)
}

pub fn robots_txt_url(url: &str) -> Result<String, String> {
    let mut parsed = validate_http_url(url)?;
    parsed.set_path("/robots.txt");
    parsed.set_query(None);
    parsed.set_fragment(None);
    parsed
        .set_username("")
        .map_err(|()| format!("Invalid URL for robots.txt: {url:?}"))?;
    parsed
        .set_password(None)
        .map_err(|()| format!("Invalid URL for robots.txt: {url:?}"))?;
    Ok(parsed.to_string())
}

/// Check that the given user agent is allowed to autonomously fetch `url`,
/// according to the site's robots.txt. Returns an error describing the
/// rejection. Unreachable or redirected robots files fail closed; 401/403
/// deny autonomous fetching, other 4xx responses are treated as no policy,
/// and successful responses are evaluated for the configured user agent.
pub async fn check_may_fetch(
    client: &FetchClient,
    url: &str,
    user_agent: &str,
) -> Result<(), String> {
    let robots_url = robots_txt_url(url)?;
    let (body, _content_type, status) = client.get(&robots_url, user_agent).await.map_err(|e| {
        format!("Failed to fetch robots.txt {robots_url} due to a connection issue: {e}")
    })?;

    if (300..400).contains(&status) {
        return Err(format!(
            "robots.txt at {robots_url} redirected with status {status}; redirects are not followed"
        ));
    }
    if status == 401 || status == 403 {
        return Err(format!(
            "When fetching robots.txt ({robots_url}), received status {status} so assuming \
             that autonomous fetching is not allowed, the user can try manually fetching \
             by using the fetch prompt"
        ));
    }
    if (400..500).contains(&status) {
        return Ok(());
    }
    if !(200..300).contains(&status) {
        return Err(format!(
            "robots.txt at {robots_url} returned status {status}; failing closed"
        ));
    }

    // Full-line comments do not affect robots matching.
    let processed: String = body
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n");

    let mut matcher = robotstxt::DefaultMatcher::default();
    if !matcher.one_agent_allowed_by_robots(&processed, user_agent, url) {
        return Err(format!(
            "robots.txt at {robots_url} disallows autonomous fetching of {url} for user agent {user_agent:?}"
        ));
    }
    Ok(())
}

/// Fetch a URL and return content ready for the model, plus an optional
/// status prefix (used when the body could not be simplified to markdown).
pub async fn fetch_url(
    client: &FetchClient,
    url: &str,
    user_agent: &str,
    force_raw: bool,
) -> Result<(String, String), String> {
    let (page_raw, content_type, status) = client.get(url, user_agent).await?;
    let content_type = bounded_utf8(&content_type, MAX_CONTENT_TYPE_BYTES);
    if status >= 300 {
        return Err(format!("Failed to fetch {url} - status code {status}"));
    }

    let first_100 = page_raw
        .chars()
        .take(100)
        .collect::<String>()
        .to_ascii_lowercase();
    let is_page_html = first_100.contains("<html")
        || content_type.to_ascii_lowercase().contains("text/html")
        || content_type.is_empty();

    if is_page_html && !force_raw {
        Ok((extract_content_from_html(&page_raw), String::new()))
    } else {
        Ok((
            page_raw,
            format!(
                "Content type {content_type} cannot be simplified to markdown, but here is \
                 the raw content:\n"
            ),
        ))
    }
}

fn bounded_utf8(value: &str, max_bytes: usize) -> &str {
    if value.len() <= max_bytes {
        return value;
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

/// Convert HTML to a simplified markdown form. Returns an error marker when
/// nothing could be extracted.
pub fn extract_content_from_html(html: &str) -> String {
    let content = parse_html(html);
    let content = content.trim();
    if content.is_empty() {
        "<error>Page failed to be simplified from HTML</error>".to_string()
    } else {
        content.to_string()
    }
}

/// HTML→markdown conversion that drops non-content tags (`style`,
/// `script`, ...) instead of leaking their raw text into the output.
fn parse_html(html: &str) -> String {
    htmd::HtmlToMarkdown::builder()
        .skip_tags(vec!["style", "script", "noscript", "template"])
        .build()
        .convert(html)
        .unwrap_or_default()
}

/// Apply character-indexed `maxLength`/`startIndex` truncation semantics.
/// Returns an error marker beyond the end and a continuation hint when more
/// content remains.
#[cfg(test)]
pub fn truncate(content: &str, start_index: usize, max_length: usize) -> String {
    let mut chars = content.chars().skip(start_index);
    let mut out = String::new();
    for _ in 0..max_length {
        let Some(character) = chars.next() else {
            return if out.is_empty() && start_index > 0 {
                "<error>No more content available.</error>".to_string()
            } else {
                out
            };
        };
        out.push(character);
    }
    if chars.next().is_some() {
        let next_start = start_index.saturating_add(max_length);
        out.push_str(&format!(
            "\n\n<error>Content truncated. Call the fetch tool with a startIndex of {next_start} to get more content.</error>"
        ));
    }
    out
}

#[cfg(test)]
mod tests;
