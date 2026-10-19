//! What `hq net` remembers, under `~/.hq/net`: the plane's public settings (`net.toml`), the
//! admitted devices (`peers.toml`), the addresses of revoked devices (`retired`) and, until the
//! owner moves it, the hub's private key. Every file is read strictly: an unknown field, a bad
//! name or a damaged file is an error and never an empty list.

use crate::files;
use crate::grammar::{self, MAX_PEERS, Peer, Policy};
use crate::key;
use crate::names::{valid_endpoint, valid_interface};
use crate::subnet::Subnet;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};

pub const NET_FILE: &str = "net.toml";
pub const PEERS_FILE: &str = "peers.toml";
pub const HUB_KEY_FILE: &str = "hub.key";
/// Addresses of revoked devices, one per line. Apart from `peers.toml` so that damage to that file
/// cannot also lose the record of which addresses must never be handed out again.
pub const RETIRED_FILE: &str = "retired";
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
#[serde(deny_unknown_fields)]
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

    /// Every value that is printed into a root command, a path or a device config, checked with
    /// the rule it is used under. A hand-edited `net.toml` cannot carry a newline, a quote or a
    /// path component through any of them.
    pub fn validate(&self) -> Result<()> {
        self.policy()?;
        if !valid_interface(&self.interface) {
            return fail(format!(
                "{NET_FILE}: interface must be a lowercase letter then lowercase letters, digits or _, at most 15 characters"
            ));
        }
        if self.port == 0 {
            return fail(format!("{NET_FILE}: port must be between 1 and 65535"));
        }
        if self.endpoint.as_deref().is_some_and(|e| !valid_endpoint(e)) {
            return fail(format!(
                "{NET_FILE}: endpoint must be host:port (letters, digits, dots and dashes only)"
            ));
        }
        if !key::is_usable(&self.hub_public_key) {
            return fail(format!("{NET_FILE}: hub_public_key is not a WireGuard key"));
        }
        Ok(())
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
#[serde(deny_unknown_fields)]
pub struct PeerRecord {
    pub label: String,
    pub kind: Kind,
    pub public_key: String,
    pub address: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preshared_key: Option<String>,
}

/// `peers.toml`: the admitted devices. An unknown table or key is an error, so a typo cannot read
/// back as "no devices".
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PeersFile {
    #[serde(default)]
    pub peer: Vec<PeerRecord>,
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
        let net: Net = toml::from_str(&text).map_err(|e| toml_error(NET_FILE, &text, &e))?;
        net.validate()?;
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
        toml::from_str(&text).map_err(|e| toml_error(PEERS_FILE, &text, &e))
    }

    /// The staged hub private key is still in the HQ user's directory.
    pub fn hub_key_staged(&self) -> bool {
        self.hub_key_path().exists()
    }

    /// Addresses that were given to a device and taken back. A line that is not an address is an
    /// error, never skipped.
    pub fn load_retired(&self) -> Result<Vec<String>> {
        let Some(text) = files::read_private(&self.dir.join(RETIRED_FILE))? else {
            return Ok(Vec::new());
        };
        text.lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| {
                let line = l.trim();
                crate::subnet::parse_canonical_ipv4(line)
                    .map(|_| line.to_string())
                    .ok_or_else(|| {
                        NetError(format!(
                            "{RETIRED_FILE} holds a line that is not an address"
                        ))
                    })
            })
            .collect()
    }

    /// Adds an address to the retired list (once).
    pub fn retire(&self, address: &str) -> Result<()> {
        let mut retired = self.load_retired()?;
        if retired.iter().any(|a| a == address) {
            return Ok(());
        }
        retired.push(address.to_string());
        let text = retired.join("\n") + "\n";
        files::write_private_atomic(&self.dir.join(RETIRED_FILE), &text).map_err(NetError::from)
    }

    pub fn save_peers(&self, peers: &PeersFile) -> Result<()> {
        let text = toml::to_string_pretty(peers).map_err(|e| NetError(e.to_string()))?;
        files::write_private_atomic(&self.dir.join(PEERS_FILE), &text).map_err(NetError::from)
    }
}

/// Words this long made only of base64 characters are keys; an error message must never carry one.
const REDACT_FROM_CHARS: usize = 32;
const MAX_ERROR_CHARS: usize = 160;

/// A toml error as a file name, a line number and the parser's reason, with anything shaped like
/// a key removed. The parser's own `Display` quotes the offending source line, which in
/// `peers.toml` can be a preshared key.
fn toml_error(file: &str, text: &str, error: &toml::de::Error) -> NetError {
    let line = error
        .span()
        .map(|span| text[..span.start.min(text.len())].matches('\n').count() + 1);
    let words: Vec<String> = error
        .message()
        .split_whitespace()
        .map(|word| {
            let bare = word.trim_matches(|c: char| {
                !c.is_ascii_alphanumeric() && c != '+' && c != '/' && c != '='
            });
            let keyish = bare.len() >= REDACT_FROM_CHARS
                && bare
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=');
            if keyish {
                "[key]".to_string()
            } else {
                word.to_string()
            }
        })
        .collect();
    let reason: String = words.join(" ").chars().take(MAX_ERROR_CHARS).collect();
    match line {
        Some(line) => NetError(format!("{file}: line {line}: {reason}")),
        None => NetError(format!("{file}: {reason}")),
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
    /// The caller says the key is a public key even though it has the shape of a private one.
    pub confirm_public: bool,
}

/// Adds a device: checks the request, takes the lowest address nobody has or ever had, and
/// returns the record. Nothing is written; the caller saves `peers` and applies it.
pub fn add_peer(
    net: &Net,
    peers: &mut PeersFile,
    retired: &[String],
    new: NewPeer<'_>,
) -> Result<PeerRecord> {
    if !valid_label(new.label) {
        return fail(
            "a label is 1 to 32 characters: lowercase letters, digits, - and _, starting with a letter or digit",
        );
    }
    if !key::is_usable(new.public_key) {
        return fail("--pubkey is not a WireGuard public key (44 characters of base64)");
    }
    if key::looks_like_private_key(new.public_key) && !new.confirm_public {
        return fail(
            "that key has the shape of a WireGuard PRIVATE key. A device's private key must never leave the device, and anything you paste here is stored and sent to the hub. If the device gave you its private key, do not use it: make a new key pair on the device and give only the public half. About one real public key in sixteen looks like this by chance; if you are sure this is a public key, repeat the command with --confirm-public",
        );
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
        *addr == hub || peers.peer.iter().any(|p| p.address == text) || retired.contains(&text)
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

/// Removes a device from the list. The caller retires its address before it saves.
pub fn revoke_peer(peers: &mut PeersFile, label: &str) -> Result<PeerRecord> {
    let Some(at) = peers.peer.iter().position(|p| p.label == label) else {
        return fail(format!(
            "there is no device called {label}; `hq net peer list` shows them"
        ));
    };
    Ok(peers.peer.remove(at))
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

    /// A public key that does not happen to have the private-key shape, so tests are not flaky.
    pub fn device_key() -> String {
        loop {
            let key = PrivateKey::generate().public();
            if !key::looks_like_private_key(&key) {
                return key;
            }
        }
    }

    fn request<'a>(label: &'a str, public_key: &'a str, kind: Kind) -> NewPeer<'a> {
        NewPeer {
            label,
            public_key,
            kind,
            preshared: false,
            confirm_public: false,
        }
    }

    fn add(net: &Net, peers: &mut PeersFile, label: &str, kind: Kind) -> Result<PeerRecord> {
        add_peer(net, peers, &[], request(label, &device_key(), kind))
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
        let retired = vec![gone.address];
        let key = device_key();
        let next = add_peer(&net, &mut peers, &retired, request("c", &key, Kind::Phone)).unwrap();
        assert_eq!(next.address, "10.77.0.4");
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
                &[],
                request(label, public_key, Kind::Laptop),
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
        add_peer(&net, &mut peers, &[], request("mac", &good, Kind::Laptop)).unwrap();
        assert!(add(&net, &mut peers, "mac", Kind::Laptop).is_err());
        assert!(add_peer(&net, &mut peers, &[], request("other", &good, Kind::Laptop)).is_err());
    }

    #[test]
    fn a_pasted_private_key_is_refused_unless_the_caller_confirms_it_is_public() {
        let net = sample_net();
        let private = PrivateKey::generate().reveal();
        let mut peers = PeersFile::default();
        let err = add_peer(
            &net,
            &mut peers,
            &[],
            request("mac", &private, Kind::Laptop),
        )
        .unwrap_err();
        assert!(err.to_string().contains("PRIVATE key"), "{err}");
        assert!(
            err.to_string().contains("--confirm-public"),
            "the way out is named: {err}"
        );
        assert!(peers.peer.is_empty());
        assert!(
            !err.to_string().contains(&private),
            "the key is never echoed"
        );
        let confirmed = NewPeer {
            confirm_public: true,
            ..request("mac", &private, Kind::Laptop)
        };
        assert!(add_peer(&net, &mut peers, &[], confirmed).is_ok());
        // A real public key (RFC 7748, Alice) is not private-shaped and needs no flag.
        let public = "hSDwCYkwp1R0i33ctD73Wg2/Og0mOBr066SpjqqbTmo=";
        assert!(add_peer(&net, &mut peers, &[], request("alice", public, Kind::Phone)).is_ok());
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
        let ask = NewPeer {
            preshared: true,
            ..request("b", &key, Kind::Laptop)
        };
        let with = add_peer(&net, &mut peers, &[], ask).unwrap();
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

    fn write_private(store: &Store, name: &str, text: &str) {
        store.ensure_dir().unwrap();
        crate::files::write_private_atomic(&store.dir().join(name), text).unwrap();
    }

    #[test]
    fn a_hostile_net_file_is_refused_on_load_whatever_field_carries_the_payload() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        let good = toml::to_string_pretty(&sample_net()).unwrap();
        assert!(toml::from_str::<Net>(&good).is_ok());
        for (field, value) in [
            ("interface", "hq0\\nEOF\\nsudo id"),
            ("interface", "../../etc/x"),
            ("interface", "Hq0"),
            ("interface", "hq0;id"),
            ("interface", ""),
            ("endpoint", "vpn.example.com:51820\\nPostUp = id"),
            ("endpoint", "bad host:51820"),
            ("endpoint", "$(id).example.com:1"),
            ("subnet", "8.8.8.0/24"),
            ("hub", "10.77.0.0"),
            ("hub_public_key", "x"),
        ] {
            let edited: String = good
                .lines()
                .map(|l| {
                    if l.starts_with(&format!("{field} =")) {
                        format!("{field} = \"{value}\"")
                    } else {
                        l.to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");
            write_private(&store, NET_FILE, &edited);
            assert!(store.load_net().is_err(), "{field} = {value}");
        }
        let with_port_zero = good.replace("port = 51820", "port = 0");
        write_private(&store, NET_FILE, &with_port_zero);
        assert!(store.load_net().is_err());
        write_private(&store, NET_FILE, &good);
        assert!(store.load_net().unwrap().is_some());
    }

    #[test]
    fn unknown_fields_and_tables_are_errors_never_an_empty_list() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        let key = device_key();
        let entry = format!(
            "label = \"mac\"\nkind = \"laptop\"\npublic_key = \"{key}\"\naddress = \"10.77.0.2\"\n"
        );
        for bad in [
            format!("[[peers]]\n{entry}"),
            format!("[[peer]]\n{entry}adress = \"10.77.0.3\"\n"),
            format!("[[peer]]\n{entry}retired = \"x\"\n"),
            "retired = [\"10.77.0.2\"]\n".to_string(),
        ] {
            write_private(&store, PEERS_FILE, &bad);
            assert!(store.load_peers().is_err(), "{bad}");
        }
        write_private(&store, PEERS_FILE, &format!("[[peer]]\n{entry}"));
        assert_eq!(store.load_peers().unwrap().peer.len(), 1);
        let mut net = toml::to_string_pretty(&sample_net()).unwrap();
        net.push_str("extra = 1\n");
        write_private(&store, NET_FILE, &net);
        assert!(store.load_net().is_err());
    }

    #[test]
    fn a_zero_byte_peers_file_is_an_empty_list_but_the_retired_record_survives_it() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        store.ensure_dir().unwrap();
        store.retire("10.77.0.2").unwrap();
        store.retire("10.77.0.2").unwrap();
        store.retire("10.77.0.5").unwrap();
        write_private(&store, PEERS_FILE, "");
        assert!(store.load_peers().unwrap().peer.is_empty());
        assert_eq!(
            store.load_retired().unwrap(),
            vec!["10.77.0.2", "10.77.0.5"]
        );
        write_private(&store, RETIRED_FILE, "10.77.0.2\nnot an address\n");
        assert!(
            store.load_retired().is_err(),
            "a damaged record is an error, never skipped"
        );
    }

    #[test]
    fn a_toml_error_never_carries_a_key() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        let psk = crate::key::new_preshared_key();
        for bad in [
            format!("[[peer]]\npreshared_key = {psk}\n"),
            format!("[[peer]]\nlabel = \"a\"\npreshared_key = \"{psk}\" trailing\n"),
            format!("[[peer]]\nkind = {psk}\n"),
            "[[peer]]\npublic_key = 12345678901234567890123456789012345678901234\n".to_string(),
        ] {
            write_private(&store, PEERS_FILE, &bad);
            let err = store.load_peers().unwrap_err().to_string();
            assert!(!err.contains(&psk), "a key was echoed: {err}");
            assert!(err.starts_with("peers.toml: line "), "{err}");
            assert!(err.len() < 400);
        }
    }
}
