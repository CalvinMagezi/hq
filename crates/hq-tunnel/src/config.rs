//! What this peer serves and what it forwards, and the limits on both. Every entry is a fixed
//! address pair: there are no wildcards, no ranges and no way to say "anything", because the
//! point of the maps is that a tunnel peer can reach exactly what the owner listed.

use crate::error::{Result, invalid};
use crate::join::{JoinConfig, is_plane_address};
use serde::Deserialize;
use std::collections::HashSet;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

pub const MAX_ENTRIES: usize = 16;
pub const MAX_SOURCES_PER_ENTRY: usize = 16;
const DEFAULT_TCP_BUFFER_KB: usize = 256;
const MIN_TCP_BUFFER_KB: usize = 16;
const MAX_TCP_BUFFER_KB: usize = 4096;
const DEFAULT_IDLE_LISTENERS: usize = 4;
const MAX_IDLE_LISTENERS: usize = 8;
const DEFAULT_MAX_CONNECTIONS: usize = 32;
const MAX_MAX_CONNECTIONS: usize = 128;
const DEFAULT_REACH_IDLE_SECS: u64 = 600;
const MIN_REACH_IDLE_SECS: u64 = 30;
const MAX_REACH_IDLE_SECS: u64 = 86_400;
const MAX_CONFIG_BYTES: usize = 16 * 1024;

fn default_tcp_buffer_kb() -> usize {
    DEFAULT_TCP_BUFFER_KB
}
fn default_idle_listeners() -> usize {
    DEFAULT_IDLE_LISTENERS
}
fn default_max_connections() -> usize {
    DEFAULT_MAX_CONNECTIONS
}
fn default_reach_idle_secs() -> u64 {
    DEFAULT_REACH_IDLE_SECS
}

/// A port on this device's tunnel address that is handed to a loopback port.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServeEntry {
    pub port: u16,
    /// Where connections go. Loopback only.
    pub to: SocketAddr,
    /// The tunnel addresses allowed to connect. A connection from any other is refused.
    pub from: Vec<Ipv4Addr>,
}

/// A loopback port that is forwarded to one tunnel destination.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReachEntry {
    /// Loopback only.
    pub listen: SocketAddr,
    pub to: SocketAddrV4,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TunnelConfig {
    #[serde(default)]
    pub serve: Vec<ServeEntry>,
    #[serde(default)]
    pub reach: Vec<ReachEntry>,
    /// Per direction, per connection. 64 KB caps a stream near 3 Mbit/s at 170 ms round trip.
    #[serde(default = "default_tcp_buffer_kb")]
    pub tcp_buffer_kb: usize,
    #[serde(default = "default_idle_listeners")]
    pub idle_listeners: usize,
    /// Per serve entry and per reach entry.
    #[serde(default = "default_max_connections")]
    pub max_connections: usize,
    #[serde(default = "default_reach_idle_secs")]
    pub reach_idle_secs: u64,
}

impl Default for TunnelConfig {
    fn default() -> Self {
        Self {
            serve: vec![],
            reach: vec![],
            tcp_buffer_kb: DEFAULT_TCP_BUFFER_KB,
            idle_listeners: DEFAULT_IDLE_LISTENERS,
            max_connections: DEFAULT_MAX_CONNECTIONS,
            reach_idle_secs: DEFAULT_REACH_IDLE_SECS,
        }
    }
}

fn within(name: &str, value: usize, min: usize, max: usize) -> Result<()> {
    if !(min..=max).contains(&value) {
        return invalid(format!("{name} must be between {min} and {max}"));
    }
    Ok(())
}

impl TunnelConfig {
    pub fn from_toml(text: &str) -> Result<Self> {
        if text.len() > MAX_CONFIG_BYTES {
            return invalid("the tunnel settings are too long");
        }
        toml::from_str(text).or_else(|e| invalid(format!("tunnel settings: {}", e.message())))
    }

    /// Refuses anything the maps are not meant to carry. Called with the join data because some
    /// rules are about this device's own address.
    pub fn validate(&self, join: &JoinConfig) -> Result<()> {
        within(
            "tcp_buffer_kb",
            self.tcp_buffer_kb,
            MIN_TCP_BUFFER_KB,
            MAX_TCP_BUFFER_KB,
        )?;
        within("idle_listeners", self.idle_listeners, 1, MAX_IDLE_LISTENERS)?;
        within(
            "max_connections",
            self.max_connections,
            1,
            MAX_MAX_CONNECTIONS,
        )?;
        if !(MIN_REACH_IDLE_SECS..=MAX_REACH_IDLE_SECS).contains(&self.reach_idle_secs) {
            return invalid(format!(
                "reach_idle_secs must be between {MIN_REACH_IDLE_SECS} and {MAX_REACH_IDLE_SECS}"
            ));
        }
        if self.idle_listeners > self.max_connections {
            return invalid("idle_listeners is more than max_connections");
        }
        self.validate_serve(join)?;
        self.validate_reach(join)
    }

    fn validate_serve(&self, join: &JoinConfig) -> Result<()> {
        if self.serve.len() > MAX_ENTRIES {
            return invalid("too many serve entries");
        }
        let mut ports = HashSet::new();
        for e in &self.serve {
            if e.port == 0 || !ports.insert(e.port) {
                return invalid("a serve port is zero or listed twice");
            }
            if !e.to.ip().is_loopback() || e.to.port() == 0 {
                return invalid("a serve target must be a loopback address with a port");
            }
            if e.from.is_empty() || e.from.len() > MAX_SOURCES_PER_ENTRY {
                return invalid("a serve entry needs between 1 and 16 allowed sources");
            }
            let mut seen = HashSet::new();
            for src in &e.from {
                if !is_plane_address(*src) || *src == join.address || !seen.insert(*src) {
                    return invalid(
                        "an allowed source must be a distinct plane address other than this device's",
                    );
                }
            }
        }
        Ok(())
    }

    fn validate_reach(&self, join: &JoinConfig) -> Result<()> {
        if self.reach.len() > MAX_ENTRIES {
            return invalid("too many reach entries");
        }
        let mut listens = HashSet::new();
        for e in &self.reach {
            if !e.listen.ip().is_loopback() || e.listen.port() == 0 || !listens.insert(e.listen) {
                return invalid("a reach listener must be a distinct loopback address with a port");
            }
            if !is_plane_address(*e.to.ip()) || *e.to.ip() == join.address || e.to.port() == 0 {
                return invalid("a reach destination must be another plane address with a port");
            }
            // A target that is also a listener would loop a connection back into the tunnel.
            if self.serve.iter().any(|s| s.to == e.listen) {
                return invalid("a serve target is also a reach listener");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::join::parse;

    fn join() -> JoinConfig {
        parse(&format!(
            "[Interface]\nAddress = 10.77.0.4/32\n\n[Peer]\nPublicKey = {}\nEndpoint = vpn.example.com:51820\nAllowedIPs = 10.77.0.1/32\n",
            hq_net::key::encode(&[7; 32])
        ))
        .unwrap()
    }

    fn ok(text: &str) -> TunnelConfig {
        let c = TunnelConfig::from_toml(text).unwrap();
        c.validate(&join()).unwrap();
        c
    }

    fn bad(text: &str) {
        match TunnelConfig::from_toml(text) {
            Err(_) => {}
            Ok(c) => assert!(c.validate(&join()).is_err(), "{text}"),
        }
    }

    #[test]
    fn an_empty_config_serves_and_forwards_nothing() {
        let c = ok("");
        assert!(c.serve.is_empty() && c.reach.is_empty());
        assert_eq!(c.tcp_buffer_kb, 256);
    }

    #[test]
    fn a_normal_config_is_accepted() {
        let c = ok(r#"
            [[serve]]
            port = 8443
            to = "127.0.0.1:8443"
            from = ["10.77.0.1", "10.77.0.2"]

            [[reach]]
            listen = "127.0.0.1:47801"
            to = "10.77.0.2:47801"
        "#);
        assert_eq!(c.serve[0].from.len(), 2);
        assert_eq!(c.reach[0].to.port(), 47801);
    }

    #[test]
    fn nothing_that_widens_the_maps_is_accepted() {
        for text in [
            "[[serve]]\nport = 0\nto = \"127.0.0.1:1\"\nfrom = [\"10.77.0.1\"]",
            "[[serve]]\nport = 8443\nto = \"0.0.0.0:8443\"\nfrom = [\"10.77.0.1\"]",
            "[[serve]]\nport = 8443\nto = \"192.168.1.5:80\"\nfrom = [\"10.77.0.1\"]",
            "[[serve]]\nport = 8443\nto = \"127.0.0.1:0\"\nfrom = [\"10.77.0.1\"]",
            "[[serve]]\nport = 8443\nto = \"127.0.0.1:1\"\nfrom = []",
            "[[serve]]\nport = 8443\nto = \"127.0.0.1:1\"\nfrom = [\"8.8.8.8\"]",
            "[[serve]]\nport = 8443\nto = \"127.0.0.1:1\"\nfrom = [\"10.77.0.4\"]",
            "[[serve]]\nport = 8443\nto = \"127.0.0.1:1\"\nfrom = [\"10.77.0.1\", \"10.77.0.1\"]",
            "[[serve]]\nport = 8443\nto = \"127.0.0.1:1\"\nfrom = [\"10.77.0.1\"]\n[[serve]]\nport = 8443\nto = \"127.0.0.1:2\"\nfrom = [\"10.77.0.1\"]",
            "[[serve]]\nport = 8443\nto = \"127.0.0.1:1\"\nfrom = [\"10.77.0.1\"]\nextra = 1",
            "[[reach]]\nlisten = \"0.0.0.0:1\"\nto = \"10.77.0.2:1\"",
            "[[reach]]\nlisten = \"127.0.0.1:1\"\nto = \"8.8.8.8:53\"",
            "[[reach]]\nlisten = \"127.0.0.1:1\"\nto = \"10.77.0.4:1\"",
            "[[reach]]\nlisten = \"127.0.0.1:1\"\nto = \"10.77.0.2:0\"",
            "[[reach]]\nlisten = \"127.0.0.1:1\"\nto = \"10.77.0.2:1\"\n[[reach]]\nlisten = \"127.0.0.1:1\"\nto = \"10.77.0.3:1\"",
            "[[serve]]\nport = 8443\nto = \"127.0.0.1:47801\"\nfrom = [\"10.77.0.1\"]\n[[reach]]\nlisten = \"127.0.0.1:47801\"\nto = \"10.77.0.2:1\"",
            "tcp_buffer_kb = 1",
            "tcp_buffer_kb = 100000",
            "idle_listeners = 0",
            "idle_listeners = 9",
            "max_connections = 0",
            "max_connections = 2\nidle_listeners = 4",
            "reach_idle_secs = 1",
            "mystery = true",
        ] {
            bad(text);
        }
        let many: String = (0..17)
            .map(|i| {
                format!(
                    "[[serve]]\nport = {}\nto = \"127.0.0.1:{}\"\nfrom = [\"10.77.0.1\"]\n",
                    9000 + i,
                    9000 + i
                )
            })
            .collect();
        bad(&many);
        bad(&"#".repeat(MAX_CONFIG_BYTES + 1));
    }

    #[test]
    fn garbage_settings_never_panic() {
        use rand::{Rng, SeedableRng};
        let mut rng = rand::rngs::StdRng::seed_from_u64(0xc0f1);
        let base = b"[[serve]]\nport = 8443\nto = \"127.0.0.1:8443\"\nfrom = [\"10.77.0.1\"]\n[[reach]]\nlisten = \"127.0.0.1:47801\"\nto = \"10.77.0.2:47801\"\ntcp_buffer_kb = 256\n".to_vec();
        for _ in 0..5_000 {
            let mut bytes = base.clone();
            for _ in 0..rng.gen_range(1..6) {
                let at = rng.gen_range(0..bytes.len());
                match rng.gen_range(0..3) {
                    0 => bytes[at] = rng.r#gen(),
                    1 => {
                        bytes.remove(at);
                    }
                    _ => bytes.insert(at, rng.r#gen()),
                }
            }
            if let Ok(text) = String::from_utf8(bytes)
                && let Ok(c) = TunnelConfig::from_toml(&text)
            {
                let _ = c.validate(&join());
            }
        }
    }
}
