//! SSRF guard for model-chosen URLs: a syntactic URL check, an address
//! classifier, and a DNS resolver that only ever returns public addresses.
//!
//! The SearxNG URL (`searxng_url`) is deliberately outside this guard. It is
//! operator configuration, not model input, and is usually a loopback or LAN
//! address. Nothing a model supplies may reach `get_client()`.
//!
//! A proxy set through `HTTP_PROXY`/`HTTPS_PROXY` resolves names itself, so
//! with one in place the proxy's egress policy is the control, not this guard.

use anyhow::{Result, bail};
use std::future::Future;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::pin::Pin;
use std::sync::Arc;

use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use url::{Host, Url};

type LookupFuture = Pin<Box<dyn Future<Output = io::Result<Vec<IpAddr>>> + Send>>;
type Lookup = Arc<dyn Fn(String) -> LookupFuture + Send + Sync>;

/// Reject URLs whose scheme is not http(s) or whose host is, as written,
/// a loopback name or a non-public IP. Hostnames that merely resolve to a
/// private address are caught later by [`GuardedResolver`].
///
/// The `url` crate already folds decimal, octal and hex IPv4 spellings
/// (`2130706433`, `0177.0.0.1`, `0x7f.1`) into dotted form, so they arrive
/// here as `Host::Ipv4`.
pub(super) fn validate_url(url: &str) -> Result<()> {
    let parsed = Url::parse(url).map_err(|e| anyhow::anyhow!("Invalid URL: {}", e))?;

    match parsed.scheme() {
        "http" | "https" => {}
        scheme => bail!(
            "URL scheme '{}' not allowed. Only http and https are permitted.",
            scheme
        ),
    }

    match parsed.host() {
        None => bail!("URL has no host."),
        Some(Host::Domain(name)) if is_local_name(name) => {
            bail!("Fetching localhost URLs is not permitted (SSRF protection).")
        }
        Some(Host::Domain(_)) => {}
        Some(Host::Ipv4(v4)) => reject_ip(IpAddr::V4(v4))?,
        Some(Host::Ipv6(v6)) => reject_ip(IpAddr::V6(v6))?,
    }
    Ok(())
}

fn reject_ip(ip: IpAddr) -> Result<()> {
    if is_non_public_ip(&ip) {
        bail!("Fetching private or reserved IP addresses is not permitted (SSRF protection): {ip}");
    }
    Ok(())
}

/// `localhost`, `*.localhost` (RFC 6761, resolves to loopback in many stacks),
/// each with an optional trailing root dot.
fn is_local_name(host: &str) -> bool {
    let name = host.trim_end_matches('.').to_ascii_lowercase();
    name == "localhost" || name.ends_with(".localhost")
}

/// True for every address that is not globally routable unicast: loopback,
/// RFC 1918, CGNAT, link-local (incl. cloud metadata), multicast, unique-local,
/// documentation and other reserved blocks, plus IPv6 forms that embed an
/// IPv4 address (mapped, compatible, NAT64, 6to4).
pub(super) fn is_non_public_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_non_public_v4(v4),
        IpAddr::V6(v6) => is_non_public_v6(v6),
    }
}

fn is_non_public_v4(v4: &Ipv4Addr) -> bool {
    let [a, b, c, _] = v4.octets();
    v4.is_unspecified()
        || a == 0 // 0.0.0.0/8 "this network"
        || v4.is_private()
        || v4.is_loopback()
        || v4.is_link_local()
        || v4.is_broadcast()
        || v4.is_multicast()
        || (a == 100 && (64..=127).contains(&b)) // CGNAT 100.64.0.0/10
        || (a == 192 && b == 0 && c == 0) // IETF protocol assignments
        || (a == 192 && b == 0 && c == 2) // TEST-NET-1
        || (a == 192 && b == 88 && c == 99) // 6to4 relay anycast
        || (a == 198 && (b == 18 || b == 19)) // benchmarking
        || (a == 198 && b == 51 && c == 100) // TEST-NET-2
        || (a == 203 && b == 0 && c == 113) // TEST-NET-3
        || a >= 240 // reserved 240.0.0.0/4
}

fn is_non_public_v6(v6: &Ipv6Addr) -> bool {
    if let Some(v4) = v6.to_ipv4_mapped() {
        return is_non_public_v4(&v4);
    }
    let seg = v6.segments();
    let first = seg[0];
    let octets = v6.octets();
    let embedded = |range: std::ops::Range<usize>| {
        let o = &octets[range];
        is_non_public_v4(&Ipv4Addr::new(o[0], o[1], o[2], o[3]))
    };
    v6.is_unspecified()
        || v6.is_loopback()
        || v6.is_multicast() // ff00::/8
        || (first & 0xfe00) == 0xfc00 // unique local, fc00::/7
        || (first & 0xffc0) == 0xfe80 // link local, fe80::/10
        || (first & 0xffc0) == 0xfec0 // deprecated site local, fec0::/10
        || (seg[..6].iter().all(|s| *s == 0) && embedded(12..16)) // ::a.b.c.d, deprecated IPv4-compatible
        || (first == 0x64 && seg[1] == 0xff9b && seg[2..6].iter().all(|s| *s == 0) && embedded(12..16)) // NAT64
        || (first == 0x64 && seg[1] == 0xff9b && seg[2] == 1) // 64:ff9b:1::/48 local-use NAT64
        || (first == 0x100 && seg[1..4].iter().all(|s| *s == 0)) // discard 100::/64
        || (first & 0xfffe) == 0x2000 && seg[1] < 0x200 // 2001::/23 protocol assignments, Teredo
        || (first == 0x2001 && seg[1] == 0x0db8) // documentation
        || (seg[..4].iter().all(|s| *s == 0) && seg[4] == 0xffff && seg[5] == 0 && embedded(12..16)) // ::ffff:0:a.b.c.d, SIIT
        || (first == 0x3fff && seg[1] < 0x1000) // 3fff::/20 documentation
        || first == 0x5f00 // 5f00::/16 SRv6 SIDs
        || (first == 0x2002 && embedded(2..6)) // 6to4
}

/// Resolves a hostname, refuses it if any answer is non-public, and returns
/// only the checked addresses. reqwest connects to exactly these, so a second
/// lookup (DNS rebinding) cannot send the socket elsewhere. It runs for every
/// request including each redirect hop; IP-literal URLs skip DNS and are
/// covered by [`validate_url`] in the redirect policy.
#[derive(Clone)]
pub(super) struct GuardedResolver {
    lookup: Lookup,
}

impl GuardedResolver {
    pub(super) fn system() -> Self {
        Self::with_lookup(Arc::new(|host: String| -> LookupFuture {
            Box::pin(async move {
                let addrs = tokio::net::lookup_host((host.as_str(), 0)).await?;
                Ok(addrs.map(|a| a.ip()).collect())
            })
        }))
    }

    pub(super) fn with_lookup(lookup: Lookup) -> Self {
        Self { lookup }
    }
}

impl Resolve for GuardedResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let lookup = self.lookup.clone();
        Box::pin(async move {
            let host = name.as_str().to_string();
            if is_local_name(&host) {
                return Err(blocked(format!("{host} is a loopback name")));
            }
            let ips = lookup(host.clone()).await?;
            if ips.is_empty() {
                return Err(blocked(format!("{host} did not resolve")));
            }
            if let Some(bad) = ips.iter().find(|ip| is_non_public_ip(ip)) {
                return Err(blocked(format!(
                    "{host} resolves to non-public address {bad}"
                )));
            }
            let addrs: Addrs = Box::new(ips.into_iter().map(|ip| SocketAddr::new(ip, 0)));
            Ok(addrs)
        })
    }
}

fn blocked(reason: String) -> Box<dyn std::error::Error + Send + Sync> {
    format!("Fetching non-public hosts is not permitted (SSRF protection): {reason}").into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn non_public_v4_ranges_are_blocked() {
        for s in [
            "0.0.0.0",
            "0.1.2.3",
            "10.0.0.1",
            "10.255.255.255",
            "100.64.0.1",
            "100.127.255.254",
            "127.0.0.1",
            "127.255.255.254",
            "169.254.169.254",
            "169.254.0.1",
            "172.16.0.1",
            "172.31.255.255",
            "192.0.0.8",
            "192.0.2.1",
            "192.168.1.1",
            "192.88.99.1",
            "198.18.0.1",
            "198.19.255.255",
            "198.51.100.1",
            "203.0.113.9",
            "224.0.0.1",
            "239.255.255.250",
            "240.0.0.1",
            "255.255.255.255",
        ] {
            assert!(is_non_public_ip(&ip(s)), "{s} must be blocked");
        }
    }

    #[test]
    fn public_v4_neighbours_are_allowed() {
        for s in [
            "8.8.8.8",
            "1.1.1.1",
            "100.63.255.255",
            "100.128.0.1",
            "172.15.255.255",
            "172.32.0.1",
            "169.253.1.1",
            "192.0.1.1",
            "192.169.0.1",
            "198.17.0.1",
            "198.20.0.1",
            "223.255.255.255",
        ] {
            assert!(!is_non_public_ip(&ip(s)), "{s} must be allowed");
        }
    }

    #[test]
    fn non_public_v6_ranges_are_blocked() {
        for s in [
            "::",
            "::1",
            "fc00::1",
            "fd12::1",
            "fe80::1",
            "febf::1",
            "fec0::1",
            "ff02::1",
            "ff0e::1",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "::ffff:169.254.169.254",
            "::ffff:7f00:1",
            "::127.0.0.1",
            "::10.0.0.1",
            "64:ff9b::7f00:1",
            "64:ff9b::a9fe:a9fe",
            "64:ff9b:1::1",
            "100::1",
            "2001::1",
            "2001:0:4136:e378:8000:63bf:3fff:fdd2",
            "2001:db8::1",
            "2002:7f00:1::",
            "2002:a9fe:a9fe::1",
            "2002:c0a8:101::",
            "::ffff:0:7f00:1",
            "::ffff:0:a9fe:a9fe",
            "3fff::1",
            "3fff:fff::1",
            "5f00::1",
        ] {
            assert!(is_non_public_ip(&ip(s)), "{s} must be blocked");
        }
    }

    #[test]
    fn public_v6_is_allowed() {
        for s in [
            "2606:4700::1111",
            "2001:4860:4860::8888",
            "2a00:1450:4001::200e",
            "::ffff:8.8.8.8",
            "64:ff9b::808:808",
            "2002:808:808::1",
            "2400:cb00::1",
        ] {
            assert!(!is_non_public_ip(&ip(s)), "{s} must be allowed");
        }
    }

    #[test]
    fn alternate_ipv4_spellings_are_blocked_after_url_parsing() {
        for u in [
            "http://2130706433/",   // decimal 127.0.0.1
            "http://0x7f000001/",   // hex
            "http://0x7f.0.0.1/",   // dotted hex
            "http://0177.0.0.1/",   // octal
            "http://017700000001/", // octal integer
            "http://127.1/",        // short form
            "http://0/",            // 0.0.0.0
            "http://2852039166/",   // decimal 169.254.169.254
            "http://0xa9.0xfe.0xa9.0xfe/",
            "http://[::ffff:127.0.0.1]/",
            "http://[::ffff:7f00:1]/",
            "http://[0:0:0:0:0:ffff:a9fe:a9fe]/",
            "http://[::1]:8080/",
            "http://[fd00::1]/",
            "http://localhost./",
            "http://LOCALHOST/",
            "http://app.localhost/",
            "http://a.b.LocalHost./",
            "http://100.64.0.1/",
            "http://169.254.169.254/latest/meta-data/",
            "http://user:pw@127.0.0.1/",
        ] {
            assert!(validate_url(u).is_err(), "{u} must be blocked");
        }
    }

    #[test]
    fn ordinary_public_urls_pass_syntactic_validation() {
        for u in [
            "https://example.com/",
            "http://8.8.8.8/x",
            "https://[2606:4700::1111]/",
            "https://localhost.example.com/",
        ] {
            assert!(validate_url(u).is_ok(), "{u} must pass");
        }
        assert!(validate_url("ftp://example.com/").is_err());
        assert!(validate_url("file:///etc/passwd").is_err());
    }

    fn fake(answer: &'static [&'static str]) -> GuardedResolver {
        GuardedResolver::with_lookup(Arc::new(move |_host| {
            Box::pin(async move { Ok(answer.iter().map(|s| s.parse().unwrap()).collect()) })
        }))
    }

    async fn resolve(r: &GuardedResolver, host: &str) -> Result<Vec<SocketAddr>, String> {
        let name: Name = host.parse().unwrap();
        r.resolve(name)
            .await
            .map(|a| a.collect())
            .map_err(|e| e.to_string())
    }

    #[tokio::test]
    async fn resolver_returns_only_validated_public_addresses() {
        let addrs = resolve(
            &fake(&["93.184.216.34", "2606:2800:220:1::1"]),
            "example.com",
        )
        .await
        .unwrap();
        assert_eq!(addrs.len(), 2);
    }

    #[tokio::test]
    async fn resolver_rejects_a_name_with_any_non_public_answer() {
        for bad in [
            "127.0.0.1",
            "::1",
            "169.254.169.254",
            "10.1.2.3",
            "100.64.1.1",
            "::ffff:127.0.0.1",
            "fd00::1",
            "224.0.0.1",
        ] {
            let answer: &'static [&'static str] =
                Box::leak(vec!["93.184.216.34", bad].into_boxed_slice());
            let err = resolve(&fake(answer), "mixed.example").await.unwrap_err();
            assert!(err.contains("SSRF protection"), "{bad}: {err}");
        }
    }

    #[tokio::test]
    async fn resolver_rejects_empty_answers_and_local_names_without_lookup() {
        assert!(resolve(&fake(&[]), "nothing.example").await.is_err());
        let r = GuardedResolver::with_lookup(Arc::new(|_| panic!("lookup must not run")));
        assert!(resolve(&r, "evil.localhost").await.is_err());
    }
}
