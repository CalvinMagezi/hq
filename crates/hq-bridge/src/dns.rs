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
/// iroh gives each lookup about 3 s in all. The platform has to answer well inside that, or the public
/// resolvers (which are for a platform that is stuck) never get their turn.
const PLATFORM_TIMEOUT: Duration = Duration::from_millis(1_500);

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

/// What `getaddrinfo` said. `None` when it failed or timed out, the one case where the public resolvers
/// may help; `Some` when it answered, with every address it has, even if there is none of the family
/// that was asked for (a name with only an IPv4 address has no IPv6 one, and asking someone else would
/// bypass the network's own DNS, private DNS and VPN).
async fn system_lookup(host: &str) -> Option<Vec<IpAddr>> {
    let lookup = tokio::net::lookup_host((host, 0));
    match tokio::time::timeout(PLATFORM_TIMEOUT, lookup).await {
        Ok(Ok(found)) => Some(found.map(|addr| addr.ip()).collect()),
        _ => None,
    }
}

fn as_v4(ip: IpAddr) -> Option<Ipv4Addr> {
    match ip {
        IpAddr::V4(v4) => Some(v4),
        IpAddr::V6(_) => None,
    }
}

fn as_v6(ip: IpAddr) -> Option<Ipv6Addr> {
    match ip {
        IpAddr::V6(v6) => Some(v6),
        IpAddr::V4(_) => None,
    }
}

impl Resolver for Platform {
    fn lookup_ipv4(&self, host: String) -> BoxFuture<Result<BoxIter<Ipv4Addr>, DnsError>> {
        let fallback = self.fallback.clone();
        Box::pin(async move {
            if let Some(all) = system_lookup(&host).await {
                let found: BoxIter<Ipv4Addr> = Box::new(
                    all.into_iter()
                        .filter_map(as_v4)
                        .collect::<Vec<_>>()
                        .into_iter(),
                );
                return Ok(found);
            }
            let list = fallback.lookup_ipv4(host, FALLBACK_TIMEOUT).await?;
            let found: BoxIter<Ipv4Addr> =
                Box::new(list.filter_map(as_v4).collect::<Vec<_>>().into_iter());
            Ok(found)
        })
    }

    fn lookup_ipv6(&self, host: String) -> BoxFuture<Result<BoxIter<Ipv6Addr>, DnsError>> {
        let fallback = self.fallback.clone();
        Box::pin(async move {
            if let Some(all) = system_lookup(&host).await {
                let found: BoxIter<Ipv6Addr> = Box::new(
                    all.into_iter()
                        .filter_map(as_v6)
                        .collect::<Vec<_>>()
                        .into_iter(),
                );
                return Ok(found);
            }
            let list = fallback.lookup_ipv6(host, FALLBACK_TIMEOUT).await?;
            let found: BoxIter<Ipv6Addr> =
                Box::new(list.filter_map(as_v6).collect::<Vec<_>>().into_iter());
            Ok(found)
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
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

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

    /// A fallback that counts how often it is asked, and always fails.
    #[derive(Debug)]
    struct Counting(Arc<AtomicUsize>);

    impl Resolver for Counting {
        fn lookup_ipv4(&self, _: String) -> BoxFuture<Result<BoxIter<Ipv4Addr>, DnsError>> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Box::pin(async {
                Err(DnsError::NxDomain {
                    meta: Default::default(),
                })
            })
        }
        fn lookup_ipv6(&self, _: String) -> BoxFuture<Result<BoxIter<Ipv6Addr>, DnsError>> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Box::pin(async {
                Err(DnsError::NxDomain {
                    meta: Default::default(),
                })
            })
        }
        fn lookup_txt(&self, _: String) -> BoxFuture<Result<BoxIter<TxtRecordData>, DnsError>> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Box::pin(async {
                Err(DnsError::NxDomain {
                    meta: Default::default(),
                })
            })
        }
        fn clear_cache(&self) {}
        fn reset(&self) -> Box<dyn Resolver> {
            Box::new(Counting(self.0.clone()))
        }
    }

    fn counted() -> (DnsResolver, Arc<AtomicUsize>) {
        let asked = Arc::new(AtomicUsize::new(0));
        let fallback = DnsResolver::custom(Counting(asked.clone()));
        (DnsResolver::custom(Platform { fallback }), asked)
    }

    #[tokio::test]
    async fn a_family_the_answered_name_lacks_is_empty_and_the_public_resolvers_are_not_asked() {
        let (dns, asked) = counted();
        // The platform answers for this literal with one IPv4 address. Asking for IPv6 is "none", not a
        // reason to ask someone else (which would bypass the network's own DNS, private DNS and VPN).
        let mut v6 = dns
            .lookup_ipv6("127.0.0.1", Duration::from_secs(5))
            .await
            .expect("an answered lookup is not an error");
        assert!(v6.next().is_none());
        assert_eq!(asked.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn a_lookup_the_platform_could_not_answer_goes_to_the_public_resolvers() {
        let (dns, asked) = counted();
        // `.invalid` never resolves: the platform fails (or times out), so the fallback is asked.
        let result = dns
            .lookup_ipv4("hq-bridge-test.invalid", Duration::from_secs(5))
            .await;
        assert!(result.is_err());
        assert!(asked.load(Ordering::SeqCst) >= 1);
    }
}
