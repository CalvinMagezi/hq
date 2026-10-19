//! What `hq net` remembers, under `~/.hq/net`: the plane's public settings (`net.toml`), the
//! admitted devices (`peers.toml`) and, until the owner moves it, the hub's private key.

use crate::files;
use crate::grammar::{self, MAX_PEERS, Peer, Policy};
use crate::key;
use crate::subnet::Subnet;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};

pub const NET_FILE: &str = "net.toml";
pub const PEERS_FILE: &str = "peers.toml";
pub const HUB_KEY_FILE: &str = "hub.key";
pub const DEFAULT_INTERFACE: &str = "hq0";
pub const DEFAULT_PORT: u16 = 51820;
/// Where the applier's input goes: a directory the HQ user owns, created by the owner as root.
pub const SPOOL_DIR: &str = "/var/lib/hq-net";
pub const MAX_LABEL_LEN: usize = 32;

#[derive(Debug)]
pub struct NetError(pub String);

impl fmt::Display for NetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for NetError {}

pub type Result<T> = std::result::Result<T, NetError>;

pub fn fail<T>(message: impl Into<String>) -> Result<T> {
    Err(NetError(message.into()))
}

impl From<std::io::Error> for NetError {
    fn from(e: std::io::Error) -> Self {
        NetError(e.to_string())
    }
}

/// The plane's settings. Nothing in here is secret.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Net {
    pub interface: String,
    pub subnet: String,
    pub hub: String,
    pub port: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    pub hub_public_key: String,
}

impl Net {
    pub fn subnet(&self) -> Result<Subnet> {
        Subnet::parse(&self.subnet).map_err(|e| NetError(format!("net.toml subnet: {e}")))
    }

    pub fn hub_addr(&self) -> Result<Ipv4Addr> {
        crate::subnet::parse_canonical_ipv4(&self.hub)
            .ok_or_else(|| NetError("net.toml hub is not an IPv4 address".into()))
    }

    pub fn policy(&self) -> Result<Policy> {
        Policy::new(self.subnet()?, self.hub_addr()?)
            .map_err(|e| NetError(format!("net.toml: {e}")))
    }

    /// The address a device calls: the setting, or an error that says how to set it.
    pub fn endpoint(&self) -> Result<&str> {
        self.endpoint.as_deref().ok_or_else(|| {
            NetError("no public address is set for this server; run `hq net init --endpoint host:port` (for example example.com:51820)".into())
        })
    }

    pub fn spool_file(&self, spool_dir: &Path) -> PathBuf {
        spool_dir.join(format!("{}.conf", self.interface))
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Laptop,
    Phone,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Laptop => "laptop",
            Kind::Phone => "phone",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PeerRecord {
    pub label: String,
    pub kind: Kind,
    pub public_key: String,
    pub address: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preshared_key: Option<String>,
}

/// `peers.toml`. `retired` holds the addresses of revoked devices: they are never handed out
/// again, because gate allow-lists and `from=` pins elsewhere name devices by address.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PeersFile {
    #[serde(default)]
    pub peer: Vec<PeerRecord>,
    #[serde(default)]
    pub retired: Vec<String>,
}

pub struct Store {
    dir: PathBuf,
}

impl Store {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn hub_key_path(&self) -> PathBuf {
        self.dir.join(HUB_KEY_FILE)
    }

    pub fn ensure_dir(&self) -> Result<()> {
        files::ensure_private_dir(&self.dir).map_err(NetError::from)
    }

    pub fn load_net(&self) -> Result<Option<Net>> {
        let Some(text) = files::read_private(&self.dir.join(NET_FILE))? else {
            return Ok(None);
        };
        let net: Net = toml::from_str(&text).map_err(|e| NetError(format!("{NET_FILE}: {e}")))?;
        net.policy()?;
        if !key::is_usable(&net.hub_public_key) {
            return fail(format!("{NET_FILE}: hub_public_key is not a WireGuard key"));
        }
        Ok(Some(net))
    }

    pub fn require_net(&self) -> Result<Net> {
        self.load_net()?.ok_or_else(|| {
            NetError("this server has no private plane yet; run `hq net init` first".into())
        })
    }

    pub fn save_net(&self, net: &Net) -> Result<()> {
        let text = toml::to_string_pretty(net).map_err(|e| NetError(e.to_string()))?;
        files::write_private_atomic(&self.dir.join(NET_FILE), &text).map_err(NetError::from)
    }

    /// Whether `peers.toml` exists. A missing file is not the same as a file with no devices.
    pub fn has_peers_file(&self) -> bool {
        self.dir.join(PEERS_FILE).exists()
    }

    pub fn load_peers(&self) -> Result<PeersFile> {
        let Some(text) = files::read_private(&self.dir.join(PEERS_FILE))? else {
            return Ok(PeersFile::default());
        };
        toml::from_str(&text).map_err(|e| NetError(format!("{PEERS_FILE}: {e}")))
    }

    pub fn save_peers(&self, peers: &PeersFile) -> Result<()> {
        let text = toml::to_string_pretty(peers).map_err(|e| NetError(e.to_string()))?;
        files::write_private_atomic(&self.dir.join(PEERS_FILE), &text).map_err(NetError::from)
    }
}

/// Letters, digits, `-` and `_`, starting with a letter or digit, lowercase, at most 32.
pub fn valid_label(label: &str) -> bool {
    let mut chars = label.chars();
    label.len() <= MAX_LABEL_LEN
        && chars
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

pub struct NewPeer<'a> {
    pub label: &'a str,
    pub public_key: &'a str,
    pub kind: Kind,
    pub preshared: bool,
}

/// Adds a device: checks the request, takes the lowest address nobody has or ever had, and
/// returns the record. Nothing is written; the caller saves `peers` and applies it.
pub fn add_peer(net: &Net, peers: &mut PeersFile, new: NewPeer<'_>) -> Result<PeerRecord> {
    if !valid_label(new.label) {
        return fail(
            "a label is 1 to 32 characters: lowercase letters, digits, - and _, starting with a letter or digit",
        );
    }
    if !key::is_usable(new.public_key) {
        return fail("--pubkey is not a WireGuard public key (44 characters of base64)");
    }
    if new.public_key == net.hub_public_key {
        return fail("--pubkey is the hub's own public key; a device needs a key of its own");
    }
    if peers.peer.iter().any(|p| p.label == new.label) {
        return fail(format!("there is already a device called {}", new.label));
    }
    if peers.peer.iter().any(|p| p.public_key == new.public_key) {
        return fail("another device already uses that public key");
    }
    if peers.peer.len() >= MAX_PEERS {
        return fail(format!(
            "the applier accepts at most {MAX_PEERS} devices; revoke one first"
        ));
    }
    let subnet = net.subnet()?;
    let hub = net.hub_addr()?;
    let taken = |addr: &Ipv4Addr| {
        let text = addr.to_string();
        *addr == hub
            || peers.peer.iter().any(|p| p.address == text)
            || peers.retired.contains(&text)
    };
    let Some(address) = subnet.hosts().find(|a| !taken(a)) else {
        return fail(
            "every address in the subnet is in use or retired; start a new plane with a larger subnet",
        );
    };
    let record = PeerRecord {
        label: new.label.to_string(),
        kind: new.kind,
        public_key: new.public_key.to_string(),
        address: address.to_string(),
        preshared_key: new.preshared.then(key::new_preshared_key),
    };
    peers.peer.push(record.clone());
    Ok(record)
}

/// Removes a device and retires its address.
pub fn revoke_peer(peers: &mut PeersFile, label: &str) -> Result<PeerRecord> {
    let Some(at) = peers.peer.iter().position(|p| p.label == label) else {
        return fail(format!(
            "there is no device called {label}; `hq net peer list` shows them"
        ));
    };
    let record = peers.peer.remove(at);
    if !peers.retired.contains(&record.address) {
        peers.retired.push(record.address.clone());
    }
    Ok(record)
}

fn to_grammar(record: &PeerRecord) -> Result<Peer> {
    let address = crate::subnet::parse_canonical_ipv4(&record.address)
        .ok_or_else(|| NetError(format!("{PEERS_FILE}: {} has a bad address", record.label)))?;
    Ok(Peer {
        public_key: record.public_key.clone(),
        address,
        preshared_key: record.preshared_key.clone(),
    })
}

/// The text the applier reads, with each device's label as a comment. It is parsed back with the
/// applier's own grammar before it is returned, so a `peers.toml` that the applier would refuse
/// (a hand edit that broke a rule) is caught here, with the same reason, and nothing is written.
pub fn render_applier_input(net: &Net, peers: &PeersFile) -> Result<String> {
    let mut records: Vec<&PeerRecord> = peers.peer.iter().collect();
    records.sort_by_key(|r| r.address.parse::<Ipv4Addr>().ok());
    let converted: Vec<Peer> = records
        .iter()
        .map(|r| to_grammar(r))
        .collect::<Result<_>>()?;
    let labeled: Vec<(&str, &Peer)> = records
        .iter()
        .map(|r| r.label.as_str())
        .zip(converted.iter())
        .collect();
    let text = grammar::render_labeled(&labeled);
    grammar::parse(text.as_bytes(), &net.policy()?).map_err(|e| {
        NetError(format!(
            "{PEERS_FILE} would be refused by the applier ({e}); fix it before applying"
        ))
    })?;
    Ok(text)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::key::PrivateKey;

    pub fn sample_net() -> Net {
        Net {
            interface: DEFAULT_INTERFACE.into(),
            subnet: "10.77.0.0/24".into(),
            hub: "10.77.0.1".into(),
            port: DEFAULT_PORT,
            endpoint: Some("vpn.example.com:51820".into()),
            hub_public_key: PrivateKey::generate().public(),
        }
    }

    pub fn device_key() -> String {
        PrivateKey::generate().public()
    }

    fn add(net: &Net, peers: &mut PeersFile, label: &str, kind: Kind) -> Result<PeerRecord> {
        add_peer(
            net,
            peers,
            NewPeer {
                label,
                public_key: &device_key(),
                kind,
                preshared: false,
            },
        )
    }

    #[test]
    fn devices_get_the_lowest_free_address_after_the_hub() {
        let net = sample_net();
        let mut peers = PeersFile::default();
        assert_eq!(
            add(&net, &mut peers, "mac", Kind::Laptop).unwrap().address,
            "10.77.0.2"
        );
        assert_eq!(
            add(&net, &mut peers, "pixel", Kind::Phone).unwrap().address,
            "10.77.0.3"
        );
    }

    #[test]
    fn a_revoked_address_is_never_handed_out_again() {
        let net = sample_net();
        let mut peers = PeersFile::default();
        add(&net, &mut peers, "a", Kind::Laptop).unwrap();
        add(&net, &mut peers, "b", Kind::Laptop).unwrap();
        let gone = revoke_peer(&mut peers, "a").unwrap();
        assert_eq!(gone.address, "10.77.0.2");
        assert_eq!(peers.retired, vec!["10.77.0.2".to_string()]);
        assert_eq!(
            add(&net, &mut peers, "c", Kind::Phone).unwrap().address,
            "10.77.0.4"
        );
        assert!(revoke_peer(&mut peers, "a").is_err());
    }

    #[test]
    fn bad_labels_keys_and_repeats_are_refused() {
        let net = sample_net();
        let mut peers = PeersFile::default();
        let good = device_key();
        let req = |label: &str, public_key: &str| {
            let mut scratch = PeersFile::default();
            add_peer(
                &net,
                &mut scratch,
                NewPeer {
                    label,
                    public_key,
                    kind: Kind::Laptop,
                    preshared: false,
                },
            )
        };
        let too_long = "x".repeat(33);
        for label in [
            "",
            "Mac",
            "a b",
            "a\nb",
            "-a",
            "_a",
            "a.b",
            too_long.as_str(),
            "é",
        ] {
            assert!(req(label, &good).is_err(), "{label:?}");
        }
        assert!(req("ok", "short").is_err());
        assert!(req("ok", "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=").is_err());
        assert!(req("ok", &net.hub_public_key).is_err());
        add_peer(
            &net,
            &mut peers,
            NewPeer {
                label: "mac",
                public_key: &good,
                kind: Kind::Laptop,
                preshared: false,
            },
        )
        .unwrap();
        assert!(add(&net, &mut peers, "mac", Kind::Laptop).is_err());
        let again = add_peer(
            &net,
            &mut peers,
            NewPeer {
                label: "other",
                public_key: &good,
                kind: Kind::Laptop,
                preshared: false,
            },
        );
        assert!(again.is_err());
    }

    #[test]
    fn the_sixty_fifth_device_is_refused_here_not_only_by_the_applier() {
        let net = sample_net();
        let mut peers = PeersFile::default();
        for i in 0..MAX_PEERS {
            add(&net, &mut peers, &format!("d{i}"), Kind::Laptop).unwrap();
        }
        assert!(add(&net, &mut peers, "one-too-many", Kind::Laptop).is_err());
    }

    #[test]
    fn a_preshared_key_is_made_only_when_asked() {
        let net = sample_net();
        let mut peers = PeersFile::default();
        let plain = add(&net, &mut peers, "a", Kind::Laptop).unwrap();
        assert!(plain.preshared_key.is_none());
        let key = device_key();
        let with = add_peer(
            &net,
            &mut peers,
            NewPeer {
                label: "b",
                public_key: &key,
                kind: Kind::Laptop,
                preshared: true,
            },
        )
        .unwrap();
        assert!(key::is_usable(with.preshared_key.as_deref().unwrap()));
    }

    #[test]
    fn the_applier_input_is_sorted_labeled_and_passes_the_applier_grammar() {
        let net = sample_net();
        let mut peers = PeersFile::default();
        add(&net, &mut peers, "mac", Kind::Laptop).unwrap();
        add(&net, &mut peers, "pixel", Kind::Phone).unwrap();
        peers.peer.reverse();
        let text = render_applier_input(&net, &peers).unwrap();
        assert!(text.starts_with(&format!("{}\n", grammar::HEADER)));
        assert!(text.find("# mac").unwrap() < text.find("# pixel").unwrap());
        let parsed = grammar::parse(text.as_bytes(), &net.policy().unwrap()).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(
            render_applier_input(&net, &PeersFile::default())
                .unwrap()
                .lines()
                .count(),
            2,
            "header and trailer only"
        );
    }

    #[test]
    fn a_hand_edit_the_applier_would_refuse_is_caught_before_anything_is_written() {
        let net = sample_net();
        let mut peers = PeersFile::default();
        add(&net, &mut peers, "mac", Kind::Laptop).unwrap();
        for bad in ["10.77.0.1", "10.99.0.2", "10.77.0.0", "not-an-address"] {
            let mut edited = peers.clone();
            edited.peer[0].address = bad.into();
            assert!(render_applier_input(&net, &edited).is_err(), "{bad}");
        }
        let mut edited = peers.clone();
        edited.peer[0].public_key = "nope".into();
        assert!(render_applier_input(&net, &edited).is_err());
        let mut dup = peers.clone();
        let mut second = dup.peer[0].clone();
        second.label = "twin".into();
        dup.peer.push(second);
        assert!(render_applier_input(&net, &dup).is_err());
    }

    #[test]
    fn settings_round_trip_through_private_files() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join("net"));
        assert!(store.load_net().unwrap().is_none());
        assert!(store.require_net().is_err());
        store.ensure_dir().unwrap();
        let net = sample_net();
        store.save_net(&net).unwrap();
        assert_eq!(store.load_net().unwrap(), Some(net.clone()));
        let mut peers = PeersFile::default();
        add(&net, &mut peers, "mac", Kind::Laptop).unwrap();
        store.save_peers(&peers).unwrap();
        assert_eq!(store.load_peers().unwrap(), peers);
    }

    #[test]
    fn a_net_file_with_a_public_subnet_or_a_bad_key_is_refused_on_load() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        for (field, value) in [
            ("subnet", "8.8.8.0/24"),
            ("hub", "10.77.0.0"),
            ("hub_public_key", "x"),
        ] {
            let mut net = sample_net();
            match field {
                "subnet" => net.subnet = value.into(),
                "hub" => net.hub = value.into(),
                _ => net.hub_public_key = value.into(),
            }
            store.save_net(&net).unwrap();
            assert!(store.load_net().is_err(), "{field}");
        }
    }
}
