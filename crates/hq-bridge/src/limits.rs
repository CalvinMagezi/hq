//! Who may try a pairing ticket, and how often. The source of a connection is its network
//! address: endpoint ids and ports cost an attacker nothing, an address does. Over a relay the
//! address is the relay's, so every relayed source behind one relay share one bucket (a residual
//! risk, written up in `docs/security/PAIRING.md`). A pure struct with no clock of its own, like
//! the code window, so it is tested with times made up.

use std::{
    collections::HashMap,
    net::{IpAddr, Ipv6Addr, SocketAddr},
    time::Instant,
};

/// Strikes (a wrong code, a bad reveal, a stalled exchange) one address may collect before it is
/// shut out for the rest of the ticket. An honest scan never collects one.
pub const DEFAULT_IP_STRIKES: u32 = 5;
/// The same for everything that arrives through one relay: many people share it, so it is looser.
pub const DEFAULT_RELAY_STRIKES: u32 = 20;
/// Pairing exchanges one source may run at once.
pub const DEFAULT_PER_SOURCE_ACTIVE: u32 = 2;
/// Pairing exchanges served at once in all. Larger than a few sources can fill together.
pub const DEFAULT_MAX_ACTIVE: usize = 8;
/// Sources remembered. When the table is full of sources with strikes, new ones are refused, which
/// fails closed instead of forgetting the offenders.
pub const DEFAULT_MAX_SOURCES: usize = 1_024;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Source {
    /// An IPv4 address, or the /64 an IPv6 address is in (one subscriber owns a whole /64).
    Ip(IpAddr),
    /// Everything that comes through this relay.
    Relay(String),
    /// A transport this build has no address for. One bucket.
    Other,
}

impl Source {
    pub fn from_socket(addr: SocketAddr) -> Self {
        let ip = match addr.ip() {
            IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
                Some(v4) => IpAddr::V4(v4),
                None => IpAddr::V6(prefix64(v6)),
            },
            v4 => v4,
        };
        Source::Ip(ip)
    }
}

fn prefix64(addr: Ipv6Addr) -> Ipv6Addr {
    let mut octets = addr.octets();
    octets[8..].fill(0);
    Ipv6Addr::from(octets)
}

#[derive(Debug, Clone, Copy)]
pub struct LimitConfig {
    pub ip_strikes: u32,
    pub relay_strikes: u32,
    pub per_source_active: u32,
    pub max_active: usize,
    pub max_sources: usize,
}

impl Default for LimitConfig {
    fn default() -> Self {
        Self {
            ip_strikes: DEFAULT_IP_STRIKES,
            relay_strikes: DEFAULT_RELAY_STRIKES,
            per_source_active: DEFAULT_PER_SOURCE_ACTIVE,
            max_active: DEFAULT_MAX_ACTIVE,
            max_sources: DEFAULT_MAX_SOURCES,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    Allowed,
    /// This source has used up its strikes, or the table has no room for a new one.
    Blocked,
    /// Too many exchanges are running, for this source or in all.
    Busy,
}

#[derive(Debug)]
struct Entry {
    strikes: u32,
    active: u32,
    last: Instant,
}

#[derive(Debug)]
pub struct Limiter {
    cfg: LimitConfig,
    sources: HashMap<Source, Entry>,
    active: usize,
}

impl Limiter {
    pub fn new(cfg: LimitConfig) -> Self {
        Self {
            cfg,
            sources: HashMap::new(),
            active: 0,
        }
    }

    fn strike_limit(&self, source: &Source) -> u32 {
        match source {
            Source::Ip(_) => self.cfg.ip_strikes,
            Source::Relay(_) | Source::Other => self.cfg.relay_strikes,
        }
    }

    /// A cheap look for the gate, before any handler runs. A source the table has no room for is
    /// not blocked here; `admit` decides that.
    pub fn is_blocked(&self, source: &Source) -> bool {
        self.sources
            .get(source)
            .is_some_and(|e| e.strikes >= self.strike_limit(source))
    }

    /// Takes a place for one exchange. A place that was taken must be given back with `release`.
    pub fn admit(&mut self, source: &Source, now: Instant) -> Admission {
        if self.is_blocked(source) {
            return Admission::Blocked;
        }
        if self.active >= self.cfg.max_active {
            return Admission::Busy;
        }
        if !self.sources.contains_key(source) && !self.make_room() {
            return Admission::Blocked;
        }
        let per_source = self.cfg.per_source_active;
        let entry = self.sources.entry(source.clone()).or_insert(Entry {
            strikes: 0,
            active: 0,
            last: now,
        });
        if entry.active >= per_source {
            return Admission::Busy;
        }
        entry.active += 1;
        entry.last = now;
        self.active += 1;
        Admission::Allowed
    }

    /// Forgets the oldest source that has no strikes and nothing running. False if there is none.
    fn make_room(&mut self) -> bool {
        if self.sources.len() < self.cfg.max_sources {
            return true;
        }
        let oldest = self
            .sources
            .iter()
            .filter(|(_, e)| e.strikes == 0 && e.active == 0)
            .min_by_key(|(_, e)| e.last)
            .map(|(s, _)| s.clone());
        match oldest {
            Some(source) => {
                self.sources.remove(&source);
                true
            }
            None => false,
        }
    }

    pub fn release(&mut self, source: &Source) {
        if let Some(entry) = self.sources.get_mut(source) {
            if entry.active > 0 {
                entry.active -= 1;
                self.active = self.active.saturating_sub(1);
            }
        }
    }

    pub fn strike(&mut self, source: &Source, now: Instant) {
        if let Some(entry) = self.sources.get_mut(source) {
            entry.strikes += 1;
            entry.last = now;
        }
    }

    pub fn active(&self) -> usize {
        self.active
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(text: &str) -> Source {
        Source::from_socket(format!("{text}:7000").parse().unwrap())
    }

    fn small() -> LimitConfig {
        LimitConfig {
            ip_strikes: 2,
            relay_strikes: 4,
            per_source_active: 1,
            max_active: 3,
            max_sources: 4,
        }
    }

    #[test]
    fn one_source_shut_out_does_not_touch_another() {
        let t0 = Instant::now();
        let mut limiter = Limiter::new(small());
        let (a, b) = (ip("192.0.2.1"), ip("192.0.2.2"));
        for _ in 0..2 {
            assert_eq!(limiter.admit(&a, t0), Admission::Allowed);
            limiter.strike(&a, t0);
            limiter.release(&a);
        }
        assert!(limiter.is_blocked(&a));
        assert_eq!(limiter.admit(&a, t0), Admission::Blocked);
        assert!(!limiter.is_blocked(&b));
        assert_eq!(limiter.admit(&b, t0), Admission::Allowed, "B is unaffected");
    }

    #[test]
    fn one_source_cannot_fill_the_concurrency_cap() {
        let t0 = Instant::now();
        let mut limiter = Limiter::new(small());
        let a = ip("192.0.2.1");
        assert_eq!(limiter.admit(&a, t0), Admission::Allowed);
        assert_eq!(
            limiter.admit(&a, t0),
            Admission::Busy,
            "one at a time per source"
        );
        assert_eq!(limiter.admit(&ip("192.0.2.2"), t0), Admission::Allowed);
        assert_eq!(limiter.admit(&ip("192.0.2.3"), t0), Admission::Allowed);
        assert_eq!(limiter.active(), 3);
        assert_eq!(
            limiter.admit(&ip("192.0.2.4"), t0),
            Admission::Busy,
            "the global cap holds"
        );
        limiter.release(&a);
        assert_eq!(limiter.active(), 2);
        assert_eq!(limiter.admit(&ip("192.0.2.4"), t0), Admission::Allowed);
    }

    #[test]
    fn addresses_in_one_ipv6_subnet_are_one_source_and_mapped_ones_are_ipv4() {
        let t0 = Instant::now();
        let a = ip("[2001:db8:1:2:aaaa::1]");
        let b = ip("[2001:db8:1:2:bbbb::9]");
        assert_eq!(a, b, "a /64 is one subscriber");
        assert_ne!(a, ip("[2001:db8:1:3::1]"));
        assert_eq!(ip("[::ffff:192.0.2.1]"), ip("192.0.2.1"));
        let mut limiter = Limiter::new(small());
        assert_eq!(limiter.admit(&a, t0), Admission::Allowed);
        assert_eq!(
            limiter.admit(&b, t0),
            Admission::Busy,
            "same source, same cap"
        );
    }

    #[test]
    fn a_relay_is_one_looser_bucket() {
        let t0 = Instant::now();
        let mut limiter = Limiter::new(small());
        let relay = Source::Relay("https://relay.example.com".into());
        for n in 0..4 {
            assert!(!limiter.is_blocked(&relay), "strike {n}");
            assert_eq!(limiter.admit(&relay, t0), Admission::Allowed);
            limiter.strike(&relay, t0);
            limiter.release(&relay);
        }
        assert!(limiter.is_blocked(&relay));
    }

    #[test]
    fn a_full_table_forgets_only_the_harmless_and_otherwise_refuses_new_sources() {
        let t0 = Instant::now();
        let mut limiter = Limiter::new(small());
        for n in 1..=4 {
            let s = ip(&format!("192.0.2.{n}"));
            assert_eq!(limiter.admit(&s, t0), Admission::Allowed);
            limiter.release(&s);
        }
        let fifth = ip("192.0.2.5");
        assert_eq!(
            limiter.admit(&fifth, t0),
            Admission::Allowed,
            "an idle source with no strikes made room"
        );
        limiter.release(&fifth);

        // Everything now has a strike: nobody can be forgotten.
        let mut limiter = Limiter::new(small());
        for n in 1..=4 {
            let s = ip(&format!("192.0.2.{n}"));
            limiter.admit(&s, t0);
            limiter.strike(&s, t0);
            limiter.release(&s);
        }
        assert_eq!(limiter.admit(&ip("192.0.2.9"), t0), Admission::Blocked);
        assert_eq!(
            limiter.admit(&ip("192.0.2.1"), t0),
            Admission::Allowed,
            "a known source with strikes left still gets in"
        );
    }

    #[test]
    fn releasing_twice_does_not_free_a_place_that_was_never_taken() {
        let t0 = Instant::now();
        let mut limiter = Limiter::new(small());
        let a = ip("192.0.2.1");
        limiter.admit(&a, t0);
        limiter.release(&a);
        limiter.release(&a);
        assert_eq!(limiter.active(), 0);
        let b = ip("192.0.2.2");
        limiter.admit(&b, t0);
        limiter.release(&a);
        assert_eq!(
            limiter.active(),
            1,
            "A's stray release did not free B's place"
        );
    }
}
