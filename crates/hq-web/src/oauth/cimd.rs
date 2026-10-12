//! Client ID Metadata Documents: the client identifies itself with an https URL whose JSON
//! names it and its redirect URIs. HQ fetches only from known hosts, so the client id cannot
//! steer a request into the owner's network.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Hosts whose metadata documents HQ trusts: claude.ai's hosted apps and Claude Code.
pub(crate) const CIMD_HOSTS: &[&str] = &["claude.ai"];
const FETCH_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_DOCUMENT_BYTES: usize = 64 * 1024;
const CACHE_TTL: Duration = Duration::from_secs(60 * 60);
/// claude.ai's edge refuses library-default agents (slice 0), so HQ names itself.
const USER_AGENT: &str = concat!(
    "agent-hq/",
    env!("CARGO_PKG_VERSION"),
    " (oauth client lookup)"
);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClientMetadata {
    pub client_id: String,
    pub client_name: String,
    pub redirect_uris: Vec<String>,
}

fn cache() -> &'static Mutex<HashMap<String, (Instant, ClientMetadata)>> {
    static CACHE: OnceLock<Mutex<HashMap<String, (Instant, ClientMetadata)>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// Seeds the cache, so tests resolve a client without the network.
#[cfg(test)]
pub(crate) fn remember(meta: ClientMetadata) {
    let mut cache = cache().lock().unwrap();
    cache.insert(meta.client_id.clone(), (Instant::now(), meta));
}

/// The metadata for `client_id`, from the cache or a fresh fetch. The error is safe to show.
pub(crate) async fn client_metadata(client_id: &str) -> Result<ClientMetadata, &'static str> {
    let url = checked_client_url(client_id)?;
    if let Some(meta) = cached(client_id) {
        return Ok(meta);
    }
    let bytes = fetch(&url).await?;
    let meta = parse(&bytes, client_id)?;
    cache()
        .lock()
        .map_err(|_| "client lookup failed")?
        .insert(client_id.to_string(), (Instant::now(), meta.clone()));
    Ok(meta)
}

fn cached(client_id: &str) -> Option<ClientMetadata> {
    let cache = cache().lock().ok()?;
    let (at, meta) = cache.get(client_id)?;
    (at.elapsed() < CACHE_TTL).then(|| meta.clone())
}

/// The client id as a URL HQ may fetch: https, a trusted host, the default port, a path.
pub(crate) fn checked_client_url(client_id: &str) -> Result<reqwest::Url, &'static str> {
    let url = reqwest::Url::parse(client_id).map_err(|_| "the client id is not a URL")?;
    let host = url.host_str().unwrap_or_default();
    let trusted = url.scheme() == "https"
        && CIMD_HOSTS.contains(&host)
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none()
        && url.path() != "/";
    if !trusted {
        return Err("this client is not one HQ accepts (only Claude's published identity)");
    }
    Ok(url)
}

async fn fetch(url: &reqwest::Url) -> Result<Vec<u8>, &'static str> {
    resolve_public(url).await?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(FETCH_TIMEOUT)
        .user_agent(USER_AGENT)
        .build()
        .map_err(|_| "client lookup failed")?;
    let mut res = client
        .get(url.clone())
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|_| "could not fetch the client's metadata")?;
    if !res.status().is_success() {
        return Err("the client's metadata could not be read");
    }
    let mut body = Vec::new();
    while let Some(chunk) = res
        .chunk()
        .await
        .map_err(|_| "could not fetch the client's metadata")?
    {
        body.extend_from_slice(&chunk);
        if body.len() > MAX_DOCUMENT_BYTES {
            return Err("the client's metadata is too large");
        }
    }
    Ok(body)
}

/// Refuses a host that resolves to a private, loopback or otherwise internal address.
async fn resolve_public(url: &reqwest::Url) -> Result<(), &'static str> {
    let host = url.host_str().ok_or("the client id has no host")?;
    let addrs: Vec<_> = tokio::net::lookup_host((host, 443))
        .await
        .map_err(|_| "could not resolve the client's host")?
        .collect();
    if addrs.is_empty() || addrs.iter().any(|a| internal(a.ip())) {
        return Err("the client's host resolves to an internal address");
    }
    Ok(())
}

fn internal(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || (a == 100 && (64..128).contains(&b))
        }
        IpAddr::V6(v6) => {
            let first = v6.segments()[0];
            v6.is_loopback()
                || v6.is_unspecified()
                || (first & 0xfe00) == 0xfc00
                || (first & 0xffc0) == 0xfe80
                || v6
                    .to_ipv4_mapped()
                    .is_some_and(|v4| internal(IpAddr::V4(v4)))
        }
    }
}

/// The document's id must be the URL it came from, or a host could speak for another client.
fn parse(bytes: &[u8], client_id: &str) -> Result<ClientMetadata, &'static str> {
    let doc: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| "the client's metadata is not JSON")?;
    if doc["client_id"].as_str() != Some(client_id) {
        return Err("the client's metadata names a different client");
    }
    let redirect_uris: Vec<String> = doc["redirect_uris"]
        .as_array()
        .ok_or("the client's metadata lists no redirect URIs")?
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    let client_name = doc["client_name"]
        .as_str()
        .map(|n| n.chars().take(80).collect())
        .unwrap_or_else(|| "Unnamed client".to_string());
    Ok(ClientMetadata {
        client_id: client_id.to_string(),
        client_name,
        redirect_uris,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLAUDE: &str = "https://claude.ai/oauth/mcp-oauth-client-metadata";

    #[test]
    fn only_https_urls_on_trusted_hosts_are_fetched() {
        assert!(checked_client_url(CLAUDE).is_ok());
        assert!(checked_client_url("https://claude.ai/oauth/claude-code-client-metadata").is_ok());
        for bad in [
            "http://claude.ai/oauth/x",
            "https://claude.ai:8443/oauth/x",
            "https://evil.example/oauth/x",
            "https://claude.ai.evil.example/x",
            "https://user@claude.ai/x",
            "https://claude.ai/",
            "https://claude.ai/x#frag",
            "stub_client_123",
            "https://10.0.0.1/x",
        ] {
            assert!(checked_client_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn internal_addresses_are_refused() {
        for ip in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "100.127.255.1",
            "0.0.0.0",
            "::1",
            "fd00::1",
            "fe80::1",
            "::ffff:10.0.0.1",
        ] {
            assert!(internal(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["160.79.106.168", "8.8.8.8", "100.128.0.1", "2606:4700::1"] {
            assert!(!internal(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn the_slice_0_document_parses_and_a_mismatched_id_is_refused() {
        let doc = br#"{"client_id":"https://claude.ai/oauth/mcp-oauth-client-metadata","client_name":"Claude","client_uri":"https://claude.ai","redirect_uris":["https://claude.ai/api/mcp/auth_callback"],"grant_types":["authorization_code","refresh_token"],"response_types":["code"],"token_endpoint_auth_method":"none"}"#;
        let meta = parse(doc, CLAUDE).unwrap();
        assert_eq!(meta.client_name, "Claude");
        assert_eq!(
            meta.redirect_uris,
            ["https://claude.ai/api/mcp/auth_callback"]
        );
        assert!(parse(doc, "https://claude.ai/oauth/other").is_err());
        assert!(parse(b"not json", CLAUDE).is_err());
        assert!(
            parse(
                br#"{"client_id":"https://claude.ai/oauth/mcp-oauth-client-metadata"}"#,
                CLAUDE
            )
            .is_err()
        );
    }
}
