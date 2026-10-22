//! Name resolution for the bridge's endpoints.
//!
//! iroh's default resolver reads the system's DNS servers. On Android that read goes through JNI
//! (`ndk-context`), and the `hq` child process of the phone app has no JVM, so it panics. With
//! `panic = "abort"` (the release profiles) a panic is the end of the process, however much
//! the callers try to catch it. [`resolver`] therefore skips the system read on Android and asks
//! the platform resolver instead (`getaddrinfo`, which honours the network's DNS, private DNS and
//! a VPN). Names it cannot answer go to iroh's built-in public resolvers, as they would if the
//! system read had failed. Everywhere else it is iroh's default.

use iroh::dns::{BoxIter, DnsError, DnsResolver, Resolver, TxtRecordData};
use std::{
    future::Future,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    pin::Pin,
    time::Duration,
};

type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;

/// Per attempt, for the built-in public resolvers.
const FALLBACK_TIMEOUT: Duration = Duration::from_secs(5);
/// A phone on a bad network can wait on `getaddrinfo` for a long time; the fallback tier answers
/// while it does.
const PLATFORM_TIMEOUT: Duration = Duration::from_secs(5);

/// The resolver every endpoint of the bridge is built with.
pub fn resolver() -> DnsResolver {
    if cfg!(target_os = "android") {
        platform_resolver()
    } else {
        DnsResolver::new()
    }
}

/// A resolver that asks the platform, then iroh's public resolvers. Public so tests can use it on
/// any target.
pub fn platform_resolver() -> DnsResolver {
    DnsResolver::custom(Platform::new())
}

#[derive(Debug)]
struct Platform {
    /// iroh's public resolvers, with no system read (that is the call that cannot be made).
    fallback: DnsResolver,
}

impl Platform {
    fn new() -> Self {
        Self {
            fallback: DnsResolver::builder().build(),
        }
    }
}

/// Every address `getaddrinfo` returns for `host`, or none when it fails or times out.
async fn system_lookup(host: &str) -> Vec<IpAddr> {
    let lookup = tokio::net::lookup_host((host, 0));
    match tokio::time::timeout(PLATFORM_TIMEOUT, lookup).await {
        Ok(Ok(found)) => found.map(|addr| addr.ip()).collect(),
        _ => Vec::new(),
    }
}

impl Resolver for Platform {
    fn lookup_ipv4(&self, host: String) -> BoxFuture<Result<BoxIter<Ipv4Addr>, DnsError>> {
        let fallback = self.fallback.clone();
        Box::pin(async move {
            let found: Vec<Ipv4Addr> = system_lookup(&host)
                .await
                .into_iter()
                .filter_map(|ip| match ip {
                    IpAddr::V4(v4) => Some(v4),
                    IpAddr::V6(_) => None,
                })
                .collect();
            if !found.is_empty() {
                let iter: BoxIter<Ipv4Addr> = Box::new(found.into_iter());
                return Ok(iter);
            }
            let list = fallback.lookup_ipv4(host, FALLBACK_TIMEOUT).await?;
            let v4: Vec<Ipv4Addr> = list
                .filter_map(|ip| match ip {
                    IpAddr::V4(v4) => Some(v4),
                    IpAddr::V6(_) => None,
                })
                .collect();
            let iter: BoxIter<Ipv4Addr> = Box::new(v4.into_iter());
            Ok(iter)
        })
    }

    fn lookup_ipv6(&self, host: String) -> BoxFuture<Result<BoxIter<Ipv6Addr>, DnsError>> {
        let fallback = self.fallback.clone();
        Box::pin(async move {
            let found: Vec<Ipv6Addr> = system_lookup(&host)
                .await
                .into_iter()
                .filter_map(|ip| match ip {
                    IpAddr::V6(v6) => Some(v6),
                    IpAddr::V4(_) => None,
                })
                .collect();
            if !found.is_empty() {
                let iter: BoxIter<Ipv6Addr> = Box::new(found.into_iter());
                return Ok(iter);
            }
            let list = fallback.lookup_ipv6(host, FALLBACK_TIMEOUT).await?;
            let v6: Vec<Ipv6Addr> = list
                .filter_map(|ip| match ip {
                    IpAddr::V6(v6) => Some(v6),
                    IpAddr::V4(_) => None,
                })
                .collect();
            let iter: BoxIter<Ipv6Addr> = Box::new(v6.into_iter());
            Ok(iter)
        })
    }

    /// TXT records are for address discovery, which the bridge does not use; the public resolvers
    /// answer if something asks.
    fn lookup_txt(&self, host: String) -> BoxFuture<Result<BoxIter<TxtRecordData>, DnsError>> {
        let fallback = self.fallback.clone();
        Box::pin(async move {
            let list: Vec<TxtRecordData> =
                fallback.lookup_txt(host, FALLBACK_TIMEOUT).await?.collect();
            let iter: BoxIter<TxtRecordData> = Box::new(list.into_iter());
            Ok(iter)
        })
    }

    fn clear_cache(&self) {
        self.fallback.clear_cache();
    }

    fn reset(&self) -> Box<dyn Resolver> {
        self.fallback.reset();
        Box::new(Platform {
            fallback: self.fallback.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_platform_resolver_answers_without_the_system_dns_read() {
        let dns = platform_resolver();
        let found: Vec<IpAddr> = dns
            .lookup_ipv4_ipv6("localhost", Duration::from_secs(5))
            .await
            .expect("localhost resolves through getaddrinfo")
            .collect();
        assert!(found.iter().any(|ip| ip.is_loopback()), "{found:?}");
    }

    #[tokio::test]
    async fn an_address_literal_needs_no_lookup() {
        let dns = platform_resolver();
        let found: Vec<IpAddr> = dns
            .lookup_ipv4("127.0.0.1", Duration::from_secs(5))
            .await
            .unwrap()
            .collect();
        assert_eq!(found, vec![IpAddr::from([127, 0, 0, 1])]);
    }

    #[tokio::test]
    async fn a_reset_keeps_answering() {
        let dns = platform_resolver();
        dns.reset();
        let mut found = dns
            .lookup_ipv4("localhost", Duration::from_secs(5))
            .await
            .unwrap();
        assert!(found.next().is_some());
    }
}
