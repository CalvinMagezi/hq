//! The `edge:` section of `config.yaml`. Everything is off unless `enabled` is set.

use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use figment::{
    Figment,
    providers::{Env, Format, Yaml},
};
use serde::{Deserialize, Serialize};

use crate::LABEL_BYTES;

const MIB: u64 = 1024 * 1024;
const HOSTNAME_MAX: usize = 253;
const LABEL_MAX: usize = 63;
/// The default UDP port for the iroh relay's address discovery (QUIC).
const RELAY_QUIC_PORT: u16 = 7842;

/// What `hq edge run` needs. Unknown keys are refused, so a typo in a security setting is an error
/// and not a silently ignored line.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EdgeConfig {
    /// Master switch. `hq edge run` refuses to start while it is false.
    pub enabled: bool,
    /// The edge domain. A device is reached at `<label>.<domain>`; every name under it must
    /// resolve to this machine.
    pub domain: String,
    /// The one TCP listener. Port 443 needs `CAP_NET_BIND_SERVICE` (the systemd unit grants it).
    pub listen: SocketAddr,
    /// Where the edge keeps its identity key, the device allowlist and the relay's certificate
    /// cache.
    pub dir: PathBuf,
    /// The UDP address of the edge's iroh endpoint. Set a fixed port to open it in a firewall.
    pub iroh_bind: SocketAddr,
    /// The relay URL the edge dials devices through. Left empty it is the co-hosted relay when
    /// that is on, and n0's public relays otherwise (development only: they are rate limited and
    /// not for production).
    pub relay_url: Option<String>,
    /// The relay the edge can host for the household.
    pub relay: RelayHostConfig,
    pub limits: LimitsConfig,
}

/// The co-hosted `iroh-relay`. Off by default.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RelayHostConfig {
    pub enabled: bool,
    /// The name the relay answers on, for example `relay.example.com`. It must resolve to this
    /// machine. The edge routes this server name to the relay, which ends TLS itself, so a
    /// Let's Encrypt challenge for it arrives on port 443 like any other connection.
    pub hostname: String,
    /// Contact address for Let's Encrypt.
    pub acme_email: String,
    /// Use Let's Encrypt's production directory. Staging (the default) issues certificates no
    /// client trusts, which is the safe way to try a setup before the rate limits apply.
    pub acme_production: bool,
    /// UDP address of the relay's QUIC address discovery, which helps devices find direct paths.
    /// `null` turns it off.
    pub quic_bind: Option<SocketAddr>,
}

/// Caps. A connection that breaks one is closed, never queued.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LimitsConfig {
    /// Total time from accept to a complete ClientHello.
    pub hello_timeout_secs: u64,
    /// Connections that have not finished their ClientHello, across all sources.
    pub max_pending: usize,
    /// The same, from one source address (one /64 for IPv6).
    pub max_pending_per_source: usize,
    /// Concurrent forwarded connections per device.
    pub max_connections_per_device: usize,
    /// Sustained bytes per second per device, both directions together.
    pub bytes_per_second: u64,
    /// Bytes a device may move at once before the rate applies.
    pub burst_bytes: u64,
    /// A forwarded connection with no bytes either way for this long is closed.
    pub idle_timeout_secs: u64,
    /// How long to wait for the device to accept a stream.
    pub dial_timeout_secs: u64,
    /// Concurrent connections to the co-hosted relay.
    pub max_relay_connections: usize,
}

impl Default for EdgeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            domain: String::new(),
            listen: SocketAddr::from(([0, 0, 0, 0], 443)),
            dir: default_dir(),
            iroh_bind: SocketAddr::from(([0, 0, 0, 0], 0)),
            relay_url: None,
            relay: RelayHostConfig::default(),
            limits: LimitsConfig::default(),
        }
    }
}

impl Default for RelayHostConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            hostname: String::new(),
            acme_email: String::new(),
            acme_production: false,
            quic_bind: Some(SocketAddr::from(([0, 0, 0, 0], RELAY_QUIC_PORT))),
        }
    }
}

impl Default for LimitsConfig {
    fn default() -> Self {
        Self {
            hello_timeout_secs: 10,
            max_pending: 256,
            max_pending_per_source: 16,
            max_connections_per_device: 16,
            bytes_per_second: 2 * MIB,
            burst_bytes: MIB,
            idle_timeout_secs: 120,
            dial_timeout_secs: 10,
            max_relay_connections: 128,
        }
    }
}

impl LimitsConfig {
    pub fn hello_timeout(&self) -> Duration {
        Duration::from_secs(self.hello_timeout_secs)
    }
    pub fn idle_timeout(&self) -> Duration {
        Duration::from_secs(self.idle_timeout_secs)
    }
    pub fn dial_timeout(&self) -> Duration {
        Duration::from_secs(self.dial_timeout_secs)
    }
}

fn default_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".hq")
        .join("edge")
}

impl EdgeConfig {
    /// Reads the `edge:` section of the YAML file at `path` (a missing file is the defaults),
    /// with `HQ_EDGE__...` environment variables on top, as the rest of `hq` reads its config.
    pub fn load(path: &Path) -> Result<Self> {
        let mut figment = Figment::new();
        if path.exists() {
            figment = figment.merge(Yaml::file(path));
        }
        figment
            .merge(Env::prefixed("HQ_").split("__"))
            .focus("edge")
            .extract()
            .with_context(|| format!("reading the edge: section of {}", path.display()))
    }

    pub fn key_path(&self) -> PathBuf {
        self.dir.join("edge.key")
    }

    pub fn allowlist_path(&self) -> PathBuf {
        self.dir.join("devices.txt")
    }

    pub fn acme_cache_dir(&self) -> PathBuf {
        self.dir.join("relay-acme")
    }

    /// Checks the settings `enabled: true` depends on. The listener never starts on a config this
    /// refuses.
    pub fn validate(&self) -> Result<()> {
        if !self.enabled {
            bail!("the edge is off: set edge.enabled: true to run it");
        }
        let domain = normalize_hostname(&self.domain).context(
            "edge.domain must be the domain devices are reached under, such as edge.example.com",
        )?;
        if domain.split('.').count() < 2 {
            bail!("edge.domain needs at least two labels, for example edge.example.com");
        }
        let limits = &self.limits;
        let zero = [
            ("hello_timeout_secs", limits.hello_timeout_secs == 0),
            ("max_pending", limits.max_pending == 0),
            ("max_pending_per_source", limits.max_pending_per_source == 0),
            (
                "max_connections_per_device",
                limits.max_connections_per_device == 0,
            ),
            ("bytes_per_second", limits.bytes_per_second == 0),
            ("burst_bytes", limits.burst_bytes == 0),
            ("idle_timeout_secs", limits.idle_timeout_secs == 0),
            ("dial_timeout_secs", limits.dial_timeout_secs == 0),
            ("max_relay_connections", limits.max_relay_connections == 0),
        ];
        if let Some((name, _)) = zero.iter().find(|(_, is_zero)| *is_zero) {
            bail!("edge.limits.{name} must not be 0");
        }
        if let Some(url) = &self.relay_url {
            url.parse::<iroh::RelayUrl>()
                .with_context(|| "edge.relay_url is not a relay URL")?;
        }
        if self.relay.enabled {
            let host = normalize_hostname(&self.relay.hostname)
                .context("edge.relay.hostname must be a name that resolves to this machine")?;
            if looks_like_device_name(&host, &domain) {
                bail!(
                    "edge.relay.hostname would shadow a device name; choose a name such as relay.{domain}"
                );
            }
            if self.relay.acme_email.trim().is_empty() || !self.relay.acme_email.contains('@') {
                bail!("edge.relay.acme_email must be an email address for Let's Encrypt");
            }
        }
        Ok(())
    }
}

/// Lowercases and checks a hostname: letters, digits and hyphens in dot-separated labels, no
/// trailing dot, no IP literal.
pub(crate) fn normalize_hostname(name: &str) -> Result<String> {
    let name = name.trim().to_ascii_lowercase();
    if name.is_empty() || name.len() > HOSTNAME_MAX {
        bail!("not a hostname");
    }
    for label in name.split('.') {
        let ok = !label.is_empty()
            && label.len() <= LABEL_MAX
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-');
        if !ok {
            bail!("{name:?} is not a hostname");
        }
    }
    if name.parse::<std::net::IpAddr>().is_ok()
        || name.bytes().all(|b| b.is_ascii_digit() || b == b'.')
    {
        bail!("{name:?} is an address, not a hostname");
    }
    Ok(name)
}

fn looks_like_device_name(host: &str, domain: &str) -> bool {
    let Some(label) = host.strip_suffix(domain).and_then(|h| h.strip_suffix('.')) else {
        return false;
    };
    label.len() == LABEL_BYTES * 2 && label.bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on() -> EdgeConfig {
        EdgeConfig {
            enabled: true,
            domain: "edge.example.com".into(),
            ..EdgeConfig::default()
        }
    }

    #[test]
    fn everything_is_off_by_default() {
        let config = EdgeConfig::default();
        assert!(!config.enabled);
        assert!(!config.relay.enabled);
        assert!(
            config.validate().is_err(),
            "a default config must not start"
        );
    }

    #[test]
    fn a_minimal_config_validates() {
        on().validate().unwrap();
    }

    #[test]
    fn a_relay_that_would_shadow_a_device_is_refused() {
        let mut config = on();
        config.relay.enabled = true;
        config.relay.acme_email = "owner@example.com".into();
        config.relay.hostname = "0123456789abcdef.edge.example.com".into();
        assert!(config.validate().is_err());
        config.relay.hostname = "relay.edge.example.com".into();
        config.validate().unwrap();
    }

    #[test]
    fn zero_limits_and_bad_names_are_refused() {
        let mut config = on();
        config.limits.max_pending = 0;
        assert!(config.validate().is_err());
        for bad in [
            "",
            "edge",
            "10.0.0.1",
            "a..b.com",
            "-a.example.com",
            "a b.example.com",
        ] {
            let mut config = on();
            config.domain = bad.into();
            assert!(config.validate().is_err(), "{bad:?} should be refused");
        }
    }

    #[test]
    fn unknown_keys_and_the_yaml_section_are_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        std::fs::write(
            &path,
            "other: 1\nedge:\n  enabled: true\n  domain: edge.example.com\n  limits:\n    max_pending: 7\n",
        )
        .unwrap();
        let config = EdgeConfig::load(&path).unwrap();
        assert!(config.enabled);
        assert_eq!(config.limits.max_pending, 7);
        assert_eq!(config.limits.max_pending_per_source, 16);

        std::fs::write(&path, "edge:\n  enabeld: true\n").unwrap();
        assert!(EdgeConfig::load(&path).is_err(), "a typo must be an error");
    }
}
