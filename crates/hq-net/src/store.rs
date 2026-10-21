//! What `hq net` remembers, under `~/.hq/net`: the plane's public settings (`net.toml`), the
//! admitted devices (`peers.toml`), the addresses of revoked devices (`retired`) and, until the
//! owner moves it, the hub's private key. Every file is read strictly: an unknown field, a bad
//! name or a damaged file is an error and never an empty list.

use crate::files;
use crate::grammar::{self, Link, MAX_LINKS, MAX_PEERS, Peer, Policy};
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
        if !valid_interface(&self.interface) || self.interface != DEFAULT_INTERFACE {
            // hq net only ever makes the default interface, so any other value was edited in. It
            // reaches printed root commands, unit instance names and a file path.
            return fail(format!(
                "{NET_FILE}: interface must be {DEFAULT_INTERFACE}; delete {NET_FILE} and run `hq net init` again only if you mean to start a new plane"
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
    /// Seconds between the keepalive packets the device sends. Absent means the default for its
    /// kind (a laptop 25, a phone none); 0 means none. Only the device config carries it: the
    /// applier never sees it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keepalive: Option<u16>,
}

/// Two devices, by label, that the hub may forward between in both directions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LinkRecord {
    pub a: String,
    pub b: String,
}

impl LinkRecord {
    fn joins(&self, x: &str, y: &str) -> bool {
        (self.a == x && self.b == y) || (self.a == y && self.b == x)
    }

    fn touches(&self, label: &str) -> bool {
        self.a == label || self.b == label
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// `peers.toml`: the admitted devices. An unknown table or key is an error, so a typo cannot read
/// back as "no devices".
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PeersFile {
    #[serde(default)]
    pub peer: Vec<PeerRecord>,
    /// Pairs of devices the hub may forward between. Everything else between devices is dropped.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub link: Vec<LinkRecord>,
    /// Set by the first `hq net link add` and never cleared. Until then the applier's input is a
    /// version 1 file, which an applier built before links existed still reads; from then on it is
    /// version 2, even with no links in it, so removing the last link also empties the hub's list.
    #[serde(default, skip_serializing_if = "is_false")]
    pub links_enabled: bool,
    /// Test files from before the `retired` file existed kept the revoked addresses here. They
    /// are read once, moved into `retired`, and never written back.
    #[serde(default, rename = "retired", skip_serializing)]
    legacy_retired: Vec<String>,
    /// Things `load_peers` did that the owner should be told about (a damaged `retired` file met
    /// during the migration). The caller prints them; they are never written.
    #[serde(skip)]
    pub notices: Vec<String>,
}

/// The retired addresses that could be read, and whether anything about the file was wrong.
pub struct RetiredRead {
    pub addresses: Vec<String>,
    pub damaged: bool,
}

/// What rebuilding the retired file did.
#[derive(Debug)]
pub struct RetireOutcome {
    pub damaged: bool,
    /// Where the damaged original went, if there was one and it could be moved.
    pub kept_as: Option<PathBuf>,
}

/// A retired file larger than this is read only as far as this; the rest stays in the copy that is
/// set aside.
const MAX_SALVAGE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_ASIDE_COPIES: usize = 100;

fn classify_retired(text: &str, already_damaged: bool) -> RetiredRead {
    let mut damaged = already_damaged;
    let mut addresses: Vec<String> = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if crate::subnet::parse_canonical_ipv4(line).is_some() {
            if !addresses.iter().any(|a| a == line) {
                addresses.push(line.to_string());
            }
        } else {
            damaged = true;
        }
    }
    RetiredRead { addresses, damaged }
}

impl PeersFile {
    /// Every field that is printed back to the owner or put in a file, checked on load. A label
    /// from an edited file could otherwise carry a terminal escape into `peer list`; the message
    /// names the device by position and never repeats the bad value.
    fn validate(&self) -> Result<()> {
        for (i, record) in self.peer.iter().enumerate() {
            let n = i + 1;
            if !valid_label(&record.label) {
                return fail(format!(
                    "{PEERS_FILE}: device number {n} has a label that is not valid (1 to 32 lowercase letters, digits, - or _)"
                ));
            }
            if !key::is_usable(&record.public_key) {
                return fail(format!(
                    "{PEERS_FILE}: device number {n} has a public key that is not a WireGuard key"
                ));
            }
            if record
                .preshared_key
                .as_deref()
                .is_some_and(|k| !key::is_usable(k))
            {
                return fail(format!(
                    "{PEERS_FILE}: device number {n} has a preshared key that is not a WireGuard key"
                ));
            }
            if crate::subnet::parse_canonical_ipv4(&record.address).is_none() {
                return fail(format!(
                    "{PEERS_FILE}: device number {n} has an address that is not an IPv4 address"
                ));
            }
        }
        self.validate_links()
    }

    /// Every link joins two different devices that exist, once, and there are not too many. The
    /// message names the link by position and never repeats the bad value.
    fn validate_links(&self) -> Result<()> {
        if self.link.len() > MAX_LINKS {
            return fail(format!(
                "{PEERS_FILE}: more than {MAX_LINKS} links; the applier accepts no more"
            ));
        }
        for (i, link) in self.link.iter().enumerate() {
            let n = i + 1;
            if !self.has_device(&link.a) || !self.has_device(&link.b) {
                return fail(format!(
                    "{PEERS_FILE}: link number {n} names a device that is not in the list"
                ));
            }
            if link.a == link.b {
                return fail(format!(
                    "{PEERS_FILE}: link number {n} joins a device to itself"
                ));
            }
            if self.link[..i]
                .iter()
                .any(|earlier| earlier.joins(&link.a, &link.b))
            {
                return fail(format!(
                    "{PEERS_FILE}: link number {n} repeats an earlier link"
                ));
            }
        }
        Ok(())
    }

    fn has_device(&self, label: &str) -> bool {
        self.peer.iter().any(|p| p.label == label)
    }

    /// Whether the applier's input is version 2: links were ever used here, or are listed now.
    pub fn uses_links(&self) -> bool {
        self.links_enabled || !self.link.is_empty()
    }

    /// The addresses of the devices linked to `label`, lowest first.
    pub fn linked_addresses(&self, label: &str) -> Vec<String> {
        let mut found: Vec<Ipv4Addr> = self
            .link
            .iter()
            .filter(|l| l.touches(label))
            .filter_map(|l| {
                let other = if l.a == label { &l.b } else { &l.a };
                let record = self.peer.iter().find(|p| &p.label == other)?;
                record.address.parse().ok()
            })
            .collect();
        found.sort();
        found.into_iter().map(|a| a.to_string()).collect()
    }
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
        let mut file: PeersFile =
            toml::from_str(&text).map_err(|e| toml_error(PEERS_FILE, &text, &e))?;
        file.validate()?;
        if !file.legacy_retired.is_empty() {
            let legacy = std::mem::take(&mut file.legacy_retired);
            if legacy
                .iter()
                .any(|a| crate::subnet::parse_canonical_ipv4(a).is_none())
            {
                return fail(format!(
                    "{PEERS_FILE}: retired holds a value that is not an address"
                ));
            }
            // One write for the whole list, however long it is.
            let outcome = self.retire_all_lenient(&legacy)?;
            if outcome.damaged {
                file.notices.push(self.damaged_notice(&outcome));
            }
            self.save_peers(&file)?;
        }
        Ok(file)
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

    /// What can be read of the retired file, without changing anything. A file the strict reader
    /// refuses (other users can read it, a byte that is not UTF-8, too large) is still read as far
    /// as it goes, line by line, so one bad byte costs one line and not the whole record.
    pub fn read_retired_salvage(&self) -> RetiredRead {
        let path = self.dir.join(RETIRED_FILE);
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(meta) => meta,
            Err(e) => {
                return RetiredRead {
                    addresses: Vec::new(),
                    damaged: e.kind() != std::io::ErrorKind::NotFound,
                };
            }
        };
        if let Ok(Some(text)) = files::read_private(&path) {
            return classify_retired(&text, false);
        }
        if !meta.file_type().is_file() {
            return RetiredRead {
                addresses: Vec::new(),
                damaged: true,
            };
        }
        let mut raw = Vec::new();
        let read = std::fs::File::open(&path).and_then(|f| {
            std::io::Read::read_to_end(&mut std::io::Read::take(f, MAX_SALVAGE_BYTES), &mut raw)
        });
        if read.is_err() && raw.is_empty() {
            return RetiredRead {
                addresses: Vec::new(),
                damaged: true,
            };
        }
        classify_retired(&String::from_utf8_lossy(&raw), true)
    }

    /// Adds an address to the retired list (once). A damaged list is an error.
    pub fn retire(&self, address: &str) -> Result<()> {
        let mut retired = self.load_retired()?;
        if !retired.iter().any(|a| a == address) {
            retired.push(address.to_string());
        }
        self.write_retired(&retired)
    }

    /// Adds an address to the retired list even if the file is damaged. See `retire_all_lenient`.
    pub fn retire_lenient(&self, address: &str) -> Result<RetireOutcome> {
        self.retire_all_lenient(&[address.to_string()])
    }

    /// Adds addresses to the retired list in one write. If the file is damaged, the original is
    /// first set aside untouched (never over an earlier copy), the lines that can be read are
    /// kept, and the outcome says so. Nothing already recorded is lost unless it cannot be read at
    /// all, and then the set-aside copy still holds it.
    pub fn retire_all_lenient(&self, addresses: &[String]) -> Result<RetireOutcome> {
        self.retire_all_with(addresses, &|from, to| std::fs::rename(from, to))
    }

    /// `retire_all_lenient` with the rename it uses passed in, so a test can make one fail.
    ///
    /// The new content is written to a temporary file first, so a full disk or a missing
    /// permission fails before the old file is touched. A damaged original is then moved aside and
    /// the new file moved in; if that second move fails the original is moved back. If the
    /// original cannot be set aside at all, nothing is rewritten: replacing it would destroy the
    /// lines that could not be read.
    fn retire_all_with(
        &self,
        addresses: &[String],
        rename: &dyn Fn(&Path, &Path) -> std::io::Result<()>,
    ) -> Result<RetireOutcome> {
        let path = self.dir.join(RETIRED_FILE);
        let read = self.read_retired_salvage();
        let mut list = read.addresses;
        for address in addresses {
            if !list.contains(address) {
                list.push(address.clone());
            }
        }
        let staged = files::stage_private(&path, &(list.join("\n") + "\n"))?;
        let discard = |staged: &Path| {
            let _ = std::fs::remove_file(staged);
        };
        let aside = if read.damaged {
            match self.free_aside_name() {
                Some(target) => match rename(&path, &target) {
                    Ok(()) => Some(target),
                    Err(e) => {
                        discard(&staged);
                        return fail(format!(
                            "{} is damaged and could not be moved aside ({e}); it was left untouched. Move it by hand (for example to {RETIRED_FILE}.damaged) and run the command again",
                            path.display()
                        ));
                    }
                },
                None => {
                    discard(&staged);
                    return fail(format!(
                        "{} is damaged and cannot be set aside because the {MAX_ASIDE_COPIES} names for copies of it are taken; it was left untouched. Move or delete the old {RETIRED_FILE}.damaged files, or move it by hand, and run the command again",
                        path.display()
                    ));
                }
            }
        } else {
            None
        };
        if let Err(e) = rename(&staged, &path) {
            discard(&staged);
            if let Some(target) = &aside {
                // Put the original back, so a failed write never leaves the record missing.
                let _ = rename(target, &path);
            }
            return fail(format!("could not write {}: {e}", path.display()));
        }
        Ok(RetireOutcome {
            damaged: read.damaged,
            kept_as: aside,
        })
    }

    /// The first name for a set-aside copy that is not taken.
    fn free_aside_name(&self) -> Option<PathBuf> {
        (0..MAX_ASIDE_COPIES)
            .map(|n| {
                if n == 0 {
                    format!("{RETIRED_FILE}.damaged")
                } else {
                    format!("{RETIRED_FILE}.damaged.{n}")
                }
            })
            .map(|name| self.dir.join(name))
            .find(|target| std::fs::symlink_metadata(target).is_err())
    }

    /// The warning for a retired file that had to be rebuilt.
    pub fn damaged_notice(&self, outcome: &RetireOutcome) -> String {
        let copy = match &outcome.kept_as {
            Some(path) => format!("The damaged file was kept as {}.", path.display()),
            None => "A copy of the damaged file could not be kept.".to_string(),
        };
        format!(
            "{}/{RETIRED_FILE} was damaged; it was rebuilt from the lines that could be read. {copy} Addresses whose lines could not be read can be handed out again by `hq net peer add`: compare with that copy before adding",
            self.dir.display()
        )
    }

    fn write_retired(&self, retired: &[String]) -> Result<()> {
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
    // A key name or a value in the message comes from the file; a control character in it would
    // reach the terminal.
    let reason: String = words
        .join(" ")
        .chars()
        .take(MAX_ERROR_CHARS)
        .map(|c| if c.is_control() { '?' } else { c })
        .collect();
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
    /// See `PeerRecord::keepalive`.
    pub keepalive: Option<u16>,
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
        // An explicit value is kept on purpose, except a phone's 0, which is the same as the
        // default for its kind: storing it would keep an older `hq net` from reading the file
        // for nothing.
        keepalive: new
            .keepalive
            .filter(|&s| s != 0 || new.kind == Kind::Laptop),
    };
    peers.peer.push(record.clone());
    Ok(record)
}

/// Removes a device from the list, and every link that names it, so a revoked device cannot leave
/// a pair behind for whatever takes its place. The caller retires its address before it saves.
pub fn revoke_peer(peers: &mut PeersFile, label: &str) -> Result<(PeerRecord, Vec<LinkRecord>)> {
    let Some(at) = peers.peer.iter().position(|p| p.label == label) else {
        return fail(format!(
            "there is no device called {label}; `hq net peer list` shows them"
        ));
    };
    let record = peers.peer.remove(at);
    let (dropped, kept) = std::mem::take(&mut peers.link)
        .into_iter()
        .partition(|l| l.touches(label));
    peers.link = kept;
    Ok((record, dropped))
}

fn require_device(peers: &PeersFile, label: &str) -> Result<()> {
    if peers.has_device(label) {
        return Ok(());
    }
    fail(format!(
        "there is no device called {label}; `hq net peer list` shows them"
    ))
}

/// Joins two devices so the hub forwards between them, in both directions and for every port.
/// Nothing is written; the caller saves `peers` and applies it.
pub fn add_link(peers: &mut PeersFile, a: &str, b: &str) -> Result<LinkRecord> {
    require_device(peers, a)?;
    require_device(peers, b)?;
    if a == b {
        return fail("a link joins two different devices");
    }
    if peers.link.iter().any(|l| l.joins(a, b)) {
        return fail(format!("{a} and {b} are linked already"));
    }
    if peers.link.len() >= MAX_LINKS {
        return fail(format!(
            "the applier accepts at most {MAX_LINKS} links; remove one first"
        ));
    }
    let (first, second) = if a < b { (a, b) } else { (b, a) };
    let record = LinkRecord {
        a: first.to_string(),
        b: second.to_string(),
    };
    peers.link.push(record.clone());
    peers.links_enabled = true;
    Ok(record)
}

/// Takes a link away. The devices stay.
pub fn remove_link(peers: &mut PeersFile, a: &str, b: &str) -> Result<LinkRecord> {
    let Some(at) = peers.link.iter().position(|l| l.joins(a, b)) else {
        return fail(format!(
            "{a} and {b} are not linked; `hq net link list` shows the links"
        ));
    };
    Ok(peers.link.remove(at))
}

/// The links as the applier's grammar states them: addresses, the lower first, in order.
fn grammar_links(peers: &PeersFile) -> Result<Vec<Link>> {
    let address = |label: &str| -> Result<Ipv4Addr> {
        peers
            .peer
            .iter()
            .find(|p| p.label == label)
            .and_then(|p| crate::subnet::parse_canonical_ipv4(&p.address))
            .ok_or_else(|| {
                NetError(format!(
                    "{PEERS_FILE}: a link names a device that is not in the list"
                ))
            })
    };
    let mut links: Vec<Link> = peers
        .link
        .iter()
        .map(|l| {
            Link::new(address(&l.a)?, address(&l.b)?)
                .ok_or_else(|| NetError(format!("{PEERS_FILE}: a link joins a device to itself")))
        })
        .collect::<Result<_>>()?;
    links.sort();
    Ok(links)
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
    let links = peers
        .uses_links()
        .then(|| grammar_links(peers))
        .transpose()?;
    let text = grammar::render_file(&labeled, links.as_deref());
    grammar::parse_file(text.as_bytes(), &net.policy()?).map_err(|e| {
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
            keepalive: None,
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
        let (gone, dropped) = revoke_peer(&mut peers, "a").unwrap();
        assert!(dropped.is_empty());
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
            ("interface", "wg_hq"),
            ("interface", "hq1"),
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
    fn a_keepalive_is_optional_in_peers_toml_and_survives_a_save() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        let key = device_key();
        let entry = format!(
            "label = \"pixel\"\nkind = \"phone\"\npublic_key = \"{key}\"\naddress = \"10.77.0.2\"\n"
        );
        // A file written before the field existed still loads, with no keepalive.
        write_private(&store, PEERS_FILE, &format!("[[peer]]\n{entry}"));
        let old = store.load_peers().unwrap();
        assert_eq!(old.peer[0].keepalive, None);
        // None is not written at all, so an older binary can still read what this one saves.
        assert!(!toml::to_string_pretty(&old).unwrap().contains("keepalive"));
        write_private(
            &store,
            PEERS_FILE,
            &format!("[[peer]]\n{entry}keepalive = 25\n"),
        );
        let mut peers = store.load_peers().unwrap();
        assert_eq!(peers.peer[0].keepalive, Some(25));
        peers.peer[0].keepalive = Some(0);
        store.save_peers(&peers).unwrap();
        assert_eq!(store.load_peers().unwrap().peer[0].keepalive, Some(0));
        for bad in [
            "keepalive = -1\n",
            "keepalive = 70000\n",
            "keepalive = \"25\"\n",
        ] {
            write_private(&store, PEERS_FILE, &format!("[[peer]]\n{entry}{bad}"));
            assert!(store.load_peers().is_err(), "{bad}");
        }
    }

    #[test]
    fn a_new_device_keeps_the_keepalive_it_was_added_with() {
        let net = sample_net();
        let mut peers = PeersFile::default();
        let key = device_key();
        let asked = NewPeer {
            keepalive: Some(25),
            ..request("pixel", &key, Kind::Phone)
        };
        let record = add_peer(&net, &mut peers, &[], asked).unwrap();
        assert_eq!(record.keepalive, Some(25));
        assert_eq!(peers.peer[0].keepalive, Some(25));
        assert_eq!(
            add(&net, &mut peers, "mac", Kind::Laptop)
                .unwrap()
                .keepalive,
            None
        );
        let off = |kind, label| {
            let key = device_key();
            let ask = NewPeer {
                keepalive: Some(0),
                ..request(label, &key, kind)
            };
            add_peer(&net, &mut PeersFile::default(), &[], ask)
                .unwrap()
                .keepalive
        };
        assert_eq!(
            off(Kind::Laptop, "a"),
            Some(0),
            "0 turns the laptop's default off"
        );
        assert_eq!(off(Kind::Phone, "b"), None, "0 on a phone is the default");
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

    #[test]
    fn a_peers_file_from_before_the_retired_file_is_migrated_once() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        let key = device_key();
        let entry = format!(
            "label = \"mac\"\nkind = \"laptop\"\npublic_key = \"{key}\"\naddress = \"10.77.0.3\"\n"
        );
        write_private(
            &store,
            PEERS_FILE,
            &format!("retired = [\"10.77.0.2\", \"10.77.0.9\"]\n\n[[peer]]\n{entry}"),
        );
        store.retire("10.77.0.9").unwrap();
        let peers = store.load_peers().unwrap();
        assert_eq!(peers.peer.len(), 1);
        assert_eq!(
            store.load_retired().unwrap(),
            vec!["10.77.0.9", "10.77.0.2"]
        );
        // The old key is gone from the file, so the next load has nothing left to migrate.
        let rewritten = std::fs::read_to_string(dir.path().join(PEERS_FILE)).unwrap();
        assert!(!rewritten.contains("retired"), "{rewritten}");
        assert_eq!(store.load_peers().unwrap(), peers);
        write_private(&store, PEERS_FILE, "retired = [\"not an address\"]\n");
        assert!(store.load_peers().is_err());
    }

    #[test]
    fn a_label_with_a_terminal_escape_or_a_bad_key_is_refused_on_load_without_being_echoed() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        let key = device_key();
        let record = |label: &str, key: &str, address: &str| {
            format!(
                "[[peer]]\nlabel = \"{label}\"\nkind = \"phone\"\npublic_key = \"{key}\"\naddress = \"{address}\"\n"
            )
        };
        for (text, hidden) in [
            (record("mac\\u001b[2J", &key, "10.77.0.2"), "\u{1b}"),
            (record("MAC", &key, "10.77.0.2"), "MAC"),
            (record("", &key, "10.77.0.2"), "zzz"),
            (record("ok", "nope", "10.77.0.2"), "nope"),
            (record("ok", &key, "10.77.0.02"), "10.77.0.02"),
            (
                format!(
                    "{}preshared_key = \"short\"\n",
                    record("ok", &key, "10.77.0.2")
                ),
                "short",
            ),
        ] {
            write_private(&store, PEERS_FILE, &text);
            let err = store.load_peers().unwrap_err().to_string();
            assert!(err.starts_with("peers.toml: device number 1 "), "{err}");
            assert!(!err.contains(hidden), "the bad value was echoed: {err:?}");
        }
        write_private(&store, PEERS_FILE, &record("ok", &key, "10.77.0.2"));
        assert!(store.load_peers().is_ok());
    }

    fn retired_path(store: &Store) -> std::path::PathBuf {
        store.dir().join(RETIRED_FILE)
    }

    #[test]
    fn a_retired_file_with_a_junk_line_keeps_its_good_lines_and_the_original_is_set_aside() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        write_private(
            &store,
            RETIRED_FILE,
            "10.77.0.2\n\u{1b}[31m junk\n10.77.0.5\n",
        );
        assert!(store.load_retired().is_err());
        let read = store.read_retired_salvage();
        assert_eq!((read.addresses.len(), read.damaged), (2, true));
        assert!(
            store.retire("10.77.0.7").is_err(),
            "the strict path still refuses"
        );
        let outcome = store.retire_lenient("10.77.0.7").unwrap();
        assert!(outcome.damaged);
        let kept = outcome
            .kept_as
            .clone()
            .expect("the damaged original is kept");
        assert!(
            std::fs::read_to_string(&kept).unwrap().contains("junk"),
            "set aside untouched"
        );
        assert_eq!(
            store.load_retired().unwrap(),
            vec!["10.77.0.2", "10.77.0.5", "10.77.0.7"]
        );
        assert!(
            !store.retire_lenient("10.77.0.7").unwrap().damaged,
            "a clean file is not reported as damaged"
        );
        assert!(store.damaged_notice(&outcome).contains(".damaged"));
    }

    #[test]
    fn a_retired_file_that_is_merely_unreadable_loses_no_earlier_address() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        write_private(&store, RETIRED_FILE, "10.77.0.2\n10.77.0.3\n10.77.0.4\n");
        // Other users can read it, so the strict reader refuses it.
        std::fs::set_permissions(retired_path(&store), std::fs::Permissions::from_mode(0o644))
            .unwrap();
        assert!(store.load_retired().is_err());
        let outcome = store.retire_lenient("10.77.0.9").unwrap();
        assert!(outcome.damaged && outcome.kept_as.is_some());
        assert_eq!(
            store.load_retired().unwrap(),
            vec!["10.77.0.2", "10.77.0.3", "10.77.0.4", "10.77.0.9"],
            "every earlier address survives"
        );
        let mode = std::fs::metadata(retired_path(&store))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "the rebuilt file is private again");
    }

    #[test]
    fn one_byte_that_is_not_utf8_costs_one_line_not_the_record() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        store.ensure_dir().unwrap();
        let mut bytes = b"10.77.0.2\n10.77.0.3\n".to_vec();
        bytes.extend_from_slice(b"10.77.0.\xff\n");
        bytes.extend_from_slice(b"10.77.0.5\n");
        std::fs::write(retired_path(&store), &bytes).unwrap();
        std::fs::set_permissions(
            retired_path(&store),
            std::os::unix::fs::PermissionsExt::from_mode(0o600),
        )
        .unwrap();
        let read = store.read_retired_salvage();
        assert!(read.damaged);
        assert_eq!(read.addresses, vec!["10.77.0.2", "10.77.0.3", "10.77.0.5"]);
        let outcome = store.retire_lenient("10.77.0.9").unwrap();
        assert_eq!(
            store.load_retired().unwrap(),
            vec!["10.77.0.2", "10.77.0.3", "10.77.0.5", "10.77.0.9"]
        );
        assert_eq!(
            std::fs::read(outcome.kept_as.unwrap()).unwrap(),
            bytes,
            "the original is byte for byte"
        );
    }

    #[test]
    fn a_second_damage_never_overwrites_the_first_copy() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        write_private(&store, RETIRED_FILE, "10.77.0.2\njunk one\n");
        let first = store.retire_lenient("10.77.0.3").unwrap().kept_as.unwrap();
        write_private_again(&store, "10.77.0.2\njunk two\n");
        let second = store.retire_lenient("10.77.0.4").unwrap().kept_as.unwrap();
        assert_ne!(first, second);
        assert!(
            std::fs::read_to_string(&first)
                .unwrap()
                .contains("junk one")
        );
        assert!(
            std::fs::read_to_string(&second)
                .unwrap()
                .contains("junk two")
        );
    }

    fn write_private_again(store: &Store, text: &str) {
        crate::files::write_private_atomic(&retired_path(store), text).unwrap();
    }

    #[test]
    fn a_retired_file_that_is_a_link_or_a_directory_is_set_aside_not_followed() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        let secret = dir.path().join("elsewhere");
        std::fs::write(&secret, "10.77.0.2\n").unwrap();
        std::os::unix::fs::symlink(&secret, retired_path(&store)).unwrap();
        let read = store.read_retired_salvage();
        assert!(
            read.damaged && read.addresses.is_empty(),
            "a link is not followed"
        );
        let outcome = store.retire_lenient("10.77.0.9").unwrap();
        assert!(outcome.damaged);
        assert_eq!(store.load_retired().unwrap(), vec!["10.77.0.9"]);
        assert_eq!(
            std::fs::read_to_string(&secret).unwrap(),
            "10.77.0.2\n",
            "the link target is untouched"
        );
    }

    #[test]
    fn the_migration_writes_the_retired_file_once_keeps_the_damage_notice_and_loses_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        write_private(&store, RETIRED_FILE, "10.77.0.2\nnot an address\n");
        let many: Vec<String> = (3..=200).map(|n| format!("\"10.77.0.{n}\"")).collect();
        write_private(
            &store,
            PEERS_FILE,
            &format!("retired = [{}]\n", many.join(", ")),
        );
        let peers = store.load_peers().unwrap();
        assert_eq!(
            peers.notices.len(),
            1,
            "the damage is reported through the migration"
        );
        assert!(peers.notices[0].contains(".damaged"));
        let retired = store.load_retired().unwrap();
        assert_eq!(
            retired.len(),
            199,
            "10.77.0.2 from the old record plus the 198 migrated"
        );
        assert!(
            retired.contains(&"10.77.0.2".to_string())
                && retired.contains(&"10.77.0.200".to_string())
        );
        assert!(store.load_peers().unwrap().notices.is_empty(), "told once");
    }

    #[test]
    fn a_toml_error_never_carries_a_control_character() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        for bad in [
            "\"a\\u001b[2Jb\" = 1\n".to_string(),
            "[[peer]]\n\"x\\u0007y\" = \"v\"\n".to_string(),
            "[[peer]]\nlabel = 5\n".to_string(),
        ] {
            write_private(&store, PEERS_FILE, &bad);
            let err = store.load_peers().unwrap_err().to_string();
            assert!(!err.chars().any(char::is_control), "{err:?}");
        }
    }

    fn damaged_retired(store: &Store, text: &str) {
        write_private(store, RETIRED_FILE, text);
    }

    #[test]
    fn a_damaged_file_that_cannot_be_set_aside_is_not_rewritten() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        let text = "10.77.0.2\n\u{1b}[31m junk\n10.77.0.5\n";
        damaged_retired(&store, text);
        std::fs::write(dir.path().join("retired.damaged"), "x").unwrap();
        for n in 1..MAX_ASIDE_COPIES {
            std::fs::write(dir.path().join(format!("retired.damaged.{n}")), "x").unwrap();
        }
        let err = store.retire_lenient("10.77.0.9").unwrap_err().to_string();
        assert!(
            err.contains("cannot be set aside") && err.contains("left untouched"),
            "{err}"
        );
        assert_eq!(std::fs::read_to_string(retired_path(&store)).unwrap(), text);
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".tmp"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "no staged file stays behind: {leftovers:?}"
        );
        // A rename that fails does the same.
        let dir2 = tempfile::tempdir().unwrap();
        let store2 = Store::new(dir2.path());
        damaged_retired(&store2, text);
        let refuse_aside = |from: &Path, to: &Path| -> std::io::Result<()> {
            if from == retired_path(&store2) {
                return Err(std::io::Error::other("no"));
            }
            std::fs::rename(from, to)
        };
        let err = store2
            .retire_all_with(&["10.77.0.9".to_string()], &refuse_aside)
            .unwrap_err()
            .to_string();
        assert!(err.contains("could not be moved aside"), "{err}");
        assert_eq!(
            std::fs::read_to_string(retired_path(&store2)).unwrap(),
            text
        );
    }

    #[test]
    fn a_write_that_fails_after_the_original_was_set_aside_puts_the_original_back() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        let text = "10.77.0.2\njunk\n10.77.0.5\n";
        damaged_retired(&store, text);
        let path = retired_path(&store);
        // The move of the original aside works; the move of the new file into place fails.
        let fail_install = |from: &Path, to: &Path| -> std::io::Result<()> {
            // Only the move of the staged file into place fails; putting the original back works.
            if to == path && from.to_string_lossy().ends_with(".tmp") {
                return Err(std::io::Error::other("disk full"));
            }
            std::fs::rename(from, to)
        };
        let err = store
            .retire_all_with(&["10.77.0.9".to_string()], &fail_install)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("could not write") && err.contains("disk full"),
            "{err}"
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            text,
            "the record is back where it was"
        );
        assert!(
            !dir.path().join("retired.damaged").exists(),
            "no stray copy"
        );
        assert!(
            std::fs::read_dir(dir.path()).unwrap().all(|e| !e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp"))
        );
    }

    #[test]
    fn a_directory_that_cannot_be_written_fails_before_the_record_is_touched() {
        use std::os::unix::fs::PermissionsExt;
        if crate::files::effective_uid() == 0 {
            return; // root ignores directory permissions
        }
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        let text = "10.77.0.2\njunk\n";
        damaged_retired(&store, text);
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o500)).unwrap();
        let result = store.retire_lenient("10.77.0.9");
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(result.is_err());
        assert_eq!(
            std::fs::read_to_string(retired_path(&store)).unwrap(),
            text,
            "untouched"
        );
        assert!(!dir.path().join("retired.damaged").exists());
        // Once the directory can be written again the same call works and nothing was lost.
        assert!(store.retire_lenient("10.77.0.9").unwrap().damaged);
        assert_eq!(
            store.load_retired().unwrap(),
            vec!["10.77.0.2", "10.77.0.9"]
        );
    }

    #[test]
    fn a_clean_file_is_replaced_in_one_step_with_no_stray_files() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        store.retire_lenient("10.77.0.2").unwrap();
        store.retire_lenient("10.77.0.3").unwrap();
        assert_eq!(
            store.load_retired().unwrap(),
            vec!["10.77.0.2", "10.77.0.3"]
        );
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names.len(), 1, "{names:?}");
    }

    fn two(net: &Net) -> PeersFile {
        let mut peers = PeersFile::default();
        for label in ["a", "b", "c"] {
            add(net, &mut peers, label, Kind::Laptop).unwrap();
        }
        peers
    }

    #[test]
    fn a_link_is_stored_once_with_its_labels_in_order_and_needs_both_devices() {
        let net = sample_net();
        let mut peers = two(&net);
        let link = add_link(&mut peers, "c", "a").unwrap();
        assert_eq!((link.a.as_str(), link.b.as_str()), ("a", "c"));
        assert!(peers.links_enabled && peers.uses_links());
        for (x, y) in [
            ("a", "c"),
            ("c", "a"),
            ("a", "a"),
            ("a", "zzz"),
            ("zzz", "a"),
        ] {
            assert!(add_link(&mut peers, x, y).is_err(), "{x} {y}");
        }
        assert_eq!(peers.link.len(), 1);
        assert_eq!(peers.linked_addresses("a"), vec!["10.77.0.4"]);
        assert_eq!(peers.linked_addresses("b"), Vec::<String>::new());
        assert!(remove_link(&mut peers, "b", "c").is_err());
        remove_link(&mut peers, "c", "a").unwrap();
        assert!(
            peers.link.is_empty() && peers.uses_links(),
            "version 2 stays once used"
        );
    }

    #[test]
    fn revoking_a_device_removes_only_its_links() {
        let net = sample_net();
        let mut peers = two(&net);
        add_link(&mut peers, "a", "b").unwrap();
        add_link(&mut peers, "b", "c").unwrap();
        add_link(&mut peers, "a", "c").unwrap();
        let (_, dropped) = revoke_peer(&mut peers, "b").unwrap();
        assert_eq!(dropped.len(), 2);
        assert_eq!(peers.link.len(), 1);
        assert!(peers.link[0].joins("a", "c"));
    }

    #[test]
    fn the_applier_input_is_version_1_until_links_are_used_then_version_2_with_sorted_links() {
        let net = sample_net();
        let mut peers = two(&net);
        let v1 = render_applier_input(&net, &peers).unwrap();
        assert!(v1.starts_with(grammar::HEADER) && !v1.contains("[Link]"));
        add_link(&mut peers, "b", "c").unwrap();
        add_link(&mut peers, "a", "b").unwrap();
        let v2 = render_applier_input(&net, &peers).unwrap();
        assert!(v2.starts_with(grammar::HEADER_V2));
        let first = v2.find("A = 10.77.0.2/32").unwrap();
        let second = v2.find("A = 10.77.0.3/32").unwrap();
        assert!(first < second, "{v2}");
        assert!(v2.ends_with("# hq-net end 3 links 2\n"), "{v2}");
    }

    #[test]
    fn at_most_the_applier_limit_of_links_can_be_added() {
        let net = sample_net();
        let mut peers = PeersFile::default();
        for i in 0..20 {
            add(&net, &mut peers, &format!("d{i}"), Kind::Laptop).unwrap();
        }
        let mut made = 0;
        'outer: for i in 0..20 {
            for j in (i + 1)..20 {
                if made == MAX_LINKS {
                    break 'outer;
                }
                add_link(&mut peers, &format!("d{i}"), &format!("d{j}")).unwrap();
                made += 1;
            }
        }
        let err = add_link(&mut peers, "d18", "d19").unwrap_err().to_string();
        assert!(err.contains("at most"), "{err}");
    }
}
