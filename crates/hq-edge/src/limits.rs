//! The caps: a byte-rate bucket per device, and a count of connections that have not yet shown a
//! server name, in total and per source address.

use std::{
    collections::HashMap,
    net::{IpAddr, Ipv6Addr},
    sync::{Arc, Mutex},
    time::Duration,
};

use tokio::time::Instant;

/// IPv6 sources are counted by /64: one subscriber is handed a whole /64, so counting single
/// addresses would let one machine use as many "sources" as it likes.
const V6_PREFIX_BITS: u32 = 64;

/// A token bucket in bytes. `take` waits until the bytes are available; the rate is shared by
/// everyone holding the same bucket, which is how a device's connections share one budget.
#[derive(Debug)]
pub struct TokenBucket {
    bytes_per_second: f64,
    burst: f64,
    state: Mutex<(f64, Instant)>,
}

impl TokenBucket {
    pub fn new(bytes_per_second: u64, burst_bytes: u64) -> Self {
        let burst = burst_bytes.max(1) as f64;
        Self {
            bytes_per_second: bytes_per_second.max(1) as f64,
            burst,
            state: Mutex::new((burst, Instant::now())),
        }
    }

    /// The largest `take` that can ever succeed in one piece; callers move data in pieces no
    /// larger than this.
    pub fn burst(&self) -> usize {
        self.burst as usize
    }

    pub async fn take(&self, bytes: usize) {
        let need = (bytes as f64).min(self.burst);
        loop {
            let wait = {
                let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
                let now = Instant::now();
                let refill = now.duration_since(state.1).as_secs_f64() * self.bytes_per_second;
                state.0 = (state.0 + refill).min(self.burst);
                state.1 = now;
                if state.0 >= need {
                    state.0 -= need;
                    return;
                }
                (need - state.0) / self.bytes_per_second
            };
            tokio::time::sleep(Duration::from_secs_f64(wait)).await;
        }
    }
}

#[derive(Default)]
struct PendingState {
    total: usize,
    per_source: HashMap<IpAddr, usize>,
}

/// Counts connections that have not finished their ClientHello. A new one is refused when the
/// total or its source's count is at the cap.
pub struct PendingLimiter {
    max_total: usize,
    max_per_source: usize,
    state: Mutex<PendingState>,
}

/// Holds one pending slot; dropping it frees the slot.
pub struct PendingGuard {
    limiter: Arc<PendingLimiter>,
    source: IpAddr,
}

impl PendingLimiter {
    pub fn new(max_total: usize, max_per_source: usize) -> Arc<Self> {
        Arc::new(Self {
            max_total,
            max_per_source,
            state: Mutex::new(PendingState::default()),
        })
    }

    pub fn try_enter(self: &Arc<Self>, from: IpAddr) -> Option<PendingGuard> {
        let source = source_key(from);
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let from_source = state.per_source.get(&source).copied().unwrap_or(0);
        if state.total >= self.max_total || from_source >= self.max_per_source {
            return None;
        }
        state.total += 1;
        *state.per_source.entry(source).or_insert(0) += 1;
        Some(PendingGuard {
            limiter: self.clone(),
            source,
        })
    }

    #[cfg(test)]
    fn total(&self) -> usize {
        self.state.lock().unwrap().total
    }
}

impl Drop for PendingGuard {
    fn drop(&mut self) {
        let mut state = self.limiter.state.lock().unwrap_or_else(|e| e.into_inner());
        state.total = state.total.saturating_sub(1);
        if let Some(count) = state.per_source.get_mut(&self.source) {
            *count -= 1;
            if *count == 0 {
                state.per_source.remove(&self.source);
            }
        }
    }
}

fn source_key(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V4(_) => ip,
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => {
                let mask = u128::MAX << (128 - V6_PREFIX_BITS);
                IpAddr::V6(Ipv6Addr::from(u128::from(v6) & mask))
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    #[test]
    fn the_total_cap_refuses_the_next_connection_and_frees_on_drop() {
        let limiter = PendingLimiter::new(2, 10);
        let a = limiter.try_enter(ip("192.0.2.1")).unwrap();
        let _b = limiter.try_enter(ip("192.0.2.2")).unwrap();
        assert!(limiter.try_enter(ip("192.0.2.3")).is_none());
        drop(a);
        assert_eq!(limiter.total(), 1);
        assert!(limiter.try_enter(ip("192.0.2.3")).is_some());
    }

    #[test]
    fn the_per_source_cap_stops_one_address_but_not_others() {
        let limiter = PendingLimiter::new(100, 2);
        let _a = limiter.try_enter(ip("192.0.2.1")).unwrap();
        let _b = limiter.try_enter(ip("192.0.2.1")).unwrap();
        assert!(limiter.try_enter(ip("192.0.2.1")).is_none());
        assert!(limiter.try_enter(ip("192.0.2.2")).is_some());
    }

    #[test]
    fn ipv6_is_counted_by_prefix_and_mapped_addresses_as_ipv4() {
        let limiter = PendingLimiter::new(100, 1);
        let _a = limiter.try_enter(ip("2001:db8:1:2::1")).unwrap();
        assert!(limiter.try_enter(ip("2001:db8:1:2:ffff::9")).is_none());
        assert!(limiter.try_enter(ip("2001:db8:1:3::1")).is_some());
        let _b = limiter.try_enter(ip("192.0.2.9")).unwrap();
        assert!(limiter.try_enter(ip("::ffff:192.0.2.9")).is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn the_bucket_lets_a_burst_through_then_holds_to_the_rate() {
        let bucket = TokenBucket::new(1000, 500);
        let started = Instant::now();
        bucket.take(500).await;
        assert!(
            started.elapsed() < Duration::from_millis(1),
            "a burst is free"
        );
        bucket.take(1000).await;
        // 500 tokens short of 1000 at 1000/s is half a second; take() caps at the burst, so it
        // waits for 500 tokens only.
        let waited = started.elapsed();
        assert!(
            waited >= Duration::from_millis(450) && waited <= Duration::from_millis(700),
            "waited {waited:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn the_bucket_is_shared_between_takers() {
        let bucket = Arc::new(TokenBucket::new(1000, 100));
        bucket.take(100).await;
        let started = Instant::now();
        let handles: Vec<_> = (0..4)
            .map(|_| {
                let bucket = bucket.clone();
                tokio::spawn(async move { bucket.take(100).await })
            })
            .collect();
        for handle in handles {
            handle.await.unwrap();
        }
        assert!(
            started.elapsed() >= Duration::from_millis(390),
            "four takers of 100 at 1000/s share one budget, took {:?}",
            started.elapsed()
        );
    }
}
