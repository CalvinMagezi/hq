//! A probe of a URL a task says it deployed: does it answer, and with what status.
//!
//! The URL is text an agent wrote, and this fetches it from the machine HQ runs on, so the probe
//! is built not to be a way into that machine's network: https only, a public address only, the
//! address pinned after it is checked so DNS cannot change its answer between the check and the
//! request, no redirects followed, a short timeout, and the body never read.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::{Duration, Instant};

use serde_json::json;

use super::observation::ObservationLog;

const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// What a probe may reach. The default is the strict one; the looser one exists for tests, which
/// serve from loopback.
#[derive(Debug, Clone, Copy)]
pub struct ProbePolicy {
    pub https_only: bool,
    pub allow_private: bool,
}

impl Default for ProbePolicy {
    fn default() -> Self {
        Self {
            https_only: true,
            allow_private: false,
        }
    }
}

/// Whether an address is one an ordinary internet host could have. Loopback, private, link-local
/// (cloud metadata lives there), carrier-grade NAT, unique-local, multicast and unspecified
/// addresses are not.
pub fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => is_public_v6(v6),
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    !(ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_documentation()
        || (a == 100 && (64..=127).contains(&b))
        || a == 0
        || a >= 240
        || (a == 198 && (b == 18 || b == 19)))
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    if let Some(v4) = ip.to_ipv4_mapped() {
        return is_public_v4(v4);
    }
    let first = ip.segments()[0];
    !(ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        || (first & 0xfe00) == 0xfc00
        || (first & 0xffc0) == 0xfe80
        || (first == 0x2001 && ip.segments()[1] == 0x0db8))
}

/// Probes `raw` and records what happened. Anything refused or unreachable is an `unavailable`
/// observation, never a pass.
pub async fn probe_url(log: &mut ObservationLog, raw: &str, policy: ProbePolicy) {
    let url = match url::Url::parse(raw.trim()) {
        Ok(u) => u,
        Err(_) => {
            log.unavailable("http", "url_probe", "that is not a URL");
            return;
        }
    };
    if policy.https_only && url.scheme() != "https" {
        log.unavailable("http", "url_probe", "only https URLs are probed");
        return;
    }
    if !matches!(url.scheme(), "http" | "https") {
        log.unavailable("http", "url_probe", "only http and https URLs are probed");
        return;
    }
    if !url.username().is_empty() || url.password().is_some() {
        log.unavailable(
            "http",
            "url_probe",
            "a URL with credentials in it is not probed",
        );
        return;
    }
    let Some(parsed_host) = url.host() else {
        log.unavailable("http", "url_probe", "the URL has no host");
        return;
    };
    let port = url.port_or_known_default().unwrap_or(443);
    let host = parsed_host.to_string();
    let lower = host.to_ascii_lowercase();
    if !policy.allow_private
        && (lower == "localhost"
            || lower.ends_with(".localhost")
            || lower.ends_with(".local")
            || lower.ends_with(".internal"))
    {
        log.unavailable(
            "http",
            "url_probe",
            "that host name is not on the public internet",
        );
        return;
    }

    // An address written in the URL is checked as it is; a name is resolved once, every address
    // it resolves to is checked, and the one used is pinned.
    let addrs: Vec<SocketAddr> = match parsed_host {
        url::Host::Ipv4(ip) => vec![SocketAddr::new(IpAddr::V4(ip), port)],
        url::Host::Ipv6(ip) => vec![SocketAddr::new(IpAddr::V6(ip), port)],
        url::Host::Domain(name) => match tokio::net::lookup_host((name, port)).await {
            Ok(a) => a.collect(),
            Err(e) => {
                log.unavailable("http", "url_probe", format!("{host} did not resolve ({e})"));
                return;
            }
        },
    };
    if addrs.is_empty() {
        log.unavailable("http", "url_probe", format!("{host} did not resolve"));
        return;
    }
    if !policy.allow_private && addrs.iter().any(|a| !is_public_ip(a.ip())) {
        log.unavailable(
            "http",
            "url_probe",
            format!("{host} is a private or reserved address, which is not probed"),
        );
        return;
    }
    let pinned = addrs[0];

    let client = match reqwest::Client::builder()
        .timeout(PROBE_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("hq-verify")
        .resolve(&host, pinned)
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            log.unavailable(
                "http",
                "url_probe",
                format!("could not build a client ({e})"),
            );
            return;
        }
    };
    let started = Instant::now();
    match client.get(url.as_str()).send().await {
        Ok(resp) => {
            let status = resp.status();
            let ms = started.elapsed().as_millis() as u64;
            // The body is dropped unread: the status is the fact.
            drop(resp);
            let ok = status.is_success();
            let fact = if status.is_redirection() {
                format!(
                    "{} answered {status}, a redirect that was not followed",
                    url
                )
            } else {
                format!("{} answered {status}", url)
            };
            log.record(
                "http",
                "url_probe",
                fact,
                if status.is_redirection() {
                    None
                } else {
                    Some(ok)
                },
                json!({ "url": url.as_str(), "status": status.as_u16(), "ms": ms }),
            );
        }
        Err(e) => {
            log.unavailable(
                "http",
                "url_probe",
                format!("{url} could not be reached ({})", short(&e)),
            );
        }
    }
}

fn short(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        "timed out".into()
    } else if e.is_connect() {
        "connection failed".into()
    } else {
        "request failed".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn serve(status_line: &'static str) -> u16 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            if let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = [0u8; 1024];
                let _ = sock.read(&mut buf).await;
                let reply = format!(
                    "HTTP/1.1 {status_line}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                );
                let _ = sock.write_all(reply.as_bytes()).await;
            }
        });
        port
    }

    const LOOSE: ProbePolicy = ProbePolicy {
        https_only: false,
        allow_private: true,
    };

    #[test]
    fn only_public_addresses_pass() {
        for ip in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "224.0.0.1",
            "255.255.255.255",
            "240.0.0.1",
            "198.18.0.1",
            "::1",
            "::",
            "fc00::1",
            "fd12::1",
            "fe80::1",
            "ff02::1",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "2001:db8::1",
        ] {
            assert!(!is_public_ip(ip.parse().unwrap()), "{ip}");
        }
        for ip in [
            "8.8.8.8",
            "1.1.1.1",
            "93.184.216.34",
            "2606:4700:4700::1111",
            "::ffff:8.8.8.8",
        ] {
            assert!(is_public_ip(ip.parse().unwrap()), "{ip}");
        }
    }

    async fn refused(raw: &str) -> String {
        let mut log = ObservationLog::new();
        probe_url(&mut log, raw, ProbePolicy::default()).await;
        let o = &log.items()[0];
        assert!(o.unavailable && o.ok.is_none(), "{raw}: {o:?}");
        o.fact.clone()
    }

    #[tokio::test]
    async fn nothing_private_or_odd_is_probed_by_default() {
        assert!(refused("http://example.com/").await.contains("https"));
        assert!(refused("ftp://example.com/").await.contains("https"));
        assert!(
            refused("https://localhost/health")
                .await
                .contains("public internet")
        );
        assert!(
            refused("https://api.corp.internal/x")
                .await
                .contains("public internet")
        );
        assert!(refused("https://127.0.0.1/").await.contains("private"));
        assert!(
            refused("https://169.254.169.254/latest/meta-data/")
                .await
                .contains("private")
        );
        assert!(refused("https://[::1]/").await.contains("private"));
        assert!(
            refused("https://user:pw@example.com/")
                .await
                .contains("credentials")
        );
        assert!(refused("not a url").await.contains("not a URL"));
    }

    #[tokio::test]
    async fn a_2xx_supports_and_an_error_status_contradicts() {
        let port = serve("200 OK").await;
        let mut log = ObservationLog::new();
        probe_url(&mut log, &format!("http://127.0.0.1:{port}/health"), LOOSE).await;
        let o = &log.items()[0];
        assert_eq!(o.ok, Some(true), "{o:?}");
        assert_eq!(o.data["status"], 200);

        let port = serve("503 Service Unavailable").await;
        let mut log = ObservationLog::new();
        probe_url(&mut log, &format!("http://127.0.0.1:{port}/health"), LOOSE).await;
        assert_eq!(log.items()[0].ok, Some(false));
        assert!(log.contradicts("url_probe"));
    }

    #[tokio::test]
    async fn a_redirect_is_reported_and_not_followed() {
        let port = serve("302 Found").await;
        let mut log = ObservationLog::new();
        probe_url(&mut log, &format!("http://127.0.0.1:{port}/"), LOOSE).await;
        let o = &log.items()[0];
        assert_eq!(o.ok, None);
        assert!(o.fact.contains("not followed"), "{}", o.fact);
        assert!(!log.supports("url_probe"));
    }

    #[tokio::test]
    async fn a_refused_connection_is_unavailable_not_a_pass() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let mut log = ObservationLog::new();
        probe_url(&mut log, &format!("http://127.0.0.1:{port}/"), LOOSE).await;
        assert!(log.items()[0].unavailable);
        assert!(!log.supports("url_probe"));
    }
}
