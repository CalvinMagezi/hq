//! The language the root applier reads, and the only thing it will ever hand to `wg`.
//!
//! ```text
//! # hq-net peers v1
//! # a comment on a line of its own
//! [Peer]
//! PublicKey = <44 characters of base64>
//! PresharedKey = <44 characters of base64>     (optional)
//! AllowedIPs = <an address in the subnet>/32
//! # hq-net end <number of peers>
//! ```
//!
//! Anything else is refused and nothing is applied: other sections (`[Interface]`), other keys
//! (`PostUp`, `PreUp`, `Table`, `Endpoint`, `PrivateKey`), a key spelled in another case, two
//! values on one line, inline comments, tabs, carriage returns, bytes outside printable ASCII.
//! The first line must be the exact header and the last the trailer with the right count, so an
//! empty or truncated file applies nothing. The writer puts `AllowedIPs` last in each block, so a
//! cut inside a block loses a required line.
//!
//! A refusal names the line and the rule, never the text of the line: the file may be a stranger's
//! file that root was tricked into reading, and the message goes to the journal.

use crate::key;
use crate::subnet::{Subnet, parse_canonical_ipv4};
use std::collections::HashSet;
use std::fmt;
use std::net::Ipv4Addr;

pub const HEADER: &str = "# hq-net peers v1";
/// The last line is this followed by the number of peers, so a file cut at a line boundary is caught.
pub const TRAILER_PREFIX: &str = "# hq-net end ";
pub const MAX_PEERS: usize = 64;
/// 64 peers at about 200 bytes each is 13 KiB; anything near this is not a peers file.
pub const MAX_INPUT_BYTES: usize = 64 * 1024;
pub const MAX_LINE_BYTES: usize = 200;
const ADDRESS_SUFFIX: &str = "/32";
const PEER_HEADER: &str = "[Peer]";

/// One admitted device: its public key, the one address it may use, and an optional preshared key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Peer {
    pub public_key: String,
    pub address: Ipv4Addr,
    pub preshared_key: Option<String>,
}

/// What the applier is told by root, never by the file: the subnet and the hub's own address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    subnet: Subnet,
    hub: Ipv4Addr,
}

impl Policy {
    pub fn new(subnet: Subnet, hub: Ipv4Addr) -> Result<Self, String> {
        if !subnet.is_private() {
            return Err(
                "the subnet must be inside 10.0.0.0/8, 172.16.0.0/12 or 192.168.0.0/16".into(),
            );
        }
        if !subnet.is_usable_host(hub) {
            return Err("the hub address must be a host address inside the subnet".into());
        }
        Ok(Self { subnet, hub })
    }

    pub fn subnet(&self) -> Subnet {
        self.subnet
    }

    pub fn hub(&self) -> Ipv4Addr {
        self.hub
    }
}

/// The rule a line or a file broke.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    TooLarge,
    NotAscii,
    ControlCharacter,
    NoFinalNewline,
    LineTooLong,
    MissingHeader,
    BadTrailer,
    UnknownSection,
    KeyOutsidePeer,
    NotKeyValue,
    UnknownKey,
    DuplicateKey,
    BadPublicKey,
    BadPresharedKey,
    BadAddress,
    AddressOutsideSubnet,
    AddressIsHub,
    AddressNotUsable,
    MissingPublicKey,
    MissingAddress,
    DuplicatePublicKey,
    DuplicateAddress,
    TooManyPeers,
}

impl Why {
    fn text(self) -> &'static str {
        match self {
            Why::TooLarge => "the file is larger than a peers file can be",
            Why::NotAscii => "a byte is outside printable ASCII",
            Why::ControlCharacter => {
                "a control character (tab, carriage return, NUL or similar) is not allowed"
            }
            Why::NoFinalNewline => "the file does not end with a newline, so it may be cut short",
            Why::LineTooLong => "the line is too long",
            Why::MissingHeader => "the first line is not the exact header",
            Why::BadTrailer => {
                "the last line is not the exact trailer for the number of peers read"
            }
            Why::UnknownSection => "only [Peer] sections are allowed",
            Why::KeyOutsidePeer => "a setting appears before any [Peer] section",
            Why::NotKeyValue => "the line is neither a comment, a [Peer] header nor Key = Value",
            Why::UnknownKey => "only PublicKey, AllowedIPs and PresharedKey are allowed",
            Why::DuplicateKey => "the setting appears twice in one peer",
            Why::BadPublicKey => {
                "PublicKey is not 44 characters of base64 holding a non-zero 32 byte key"
            }
            Why::BadPresharedKey => {
                "PresharedKey is not 44 characters of base64 holding a non-zero 32 byte key"
            }
            Why::BadAddress => "AllowedIPs is not exactly one IPv4 address written a.b.c.d/32",
            Why::AddressOutsideSubnet => "AllowedIPs is outside the subnet",
            Why::AddressIsHub => "AllowedIPs is the hub's own address",
            Why::AddressNotUsable => "AllowedIPs is the network or broadcast address",
            Why::MissingPublicKey => "the peer has no PublicKey",
            Why::MissingAddress => "the peer has no AllowedIPs",
            Why::DuplicatePublicKey => "another peer already has this PublicKey",
            Why::DuplicateAddress => "another peer already has this address",
            Why::TooManyPeers => "more than 64 peers",
        }
    }
}

/// A refusal: the 1-based line (0 for the file as a whole) and the rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reject {
    pub line: usize,
    pub why: Why,
}

impl fmt::Display for Reject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.line == 0 {
            write!(f, "refused: {}", self.why.text())
        } else {
            write!(f, "refused at line {}: {}", self.line, self.why.text())
        }
    }
}

impl std::error::Error for Reject {}

fn reject<T>(line: usize, why: Why) -> Result<T, Reject> {
    Err(Reject { line, why })
}

/// A peer being read: its header line and what it has said so far.
struct Draft {
    line: usize,
    public_key: Option<String>,
    address: Option<Ipv4Addr>,
    preshared_key: Option<String>,
}

impl Draft {
    fn new(line: usize) -> Self {
        Self {
            line,
            public_key: None,
            address: None,
            preshared_key: None,
        }
    }

    fn finish(self) -> Result<(usize, Peer), Reject> {
        let Some(public_key) = self.public_key else {
            return reject(self.line, Why::MissingPublicKey);
        };
        let Some(address) = self.address else {
            return reject(self.line, Why::MissingAddress);
        };
        let peer = Peer {
            public_key,
            address,
            preshared_key: self.preshared_key,
        };
        Ok((self.line, peer))
    }
}

/// Bytes the language allows: printable ASCII and the newline.
fn check_bytes(input: &[u8]) -> Result<(), Reject> {
    let mut line = 1;
    for &byte in input {
        match byte {
            b'\n' => line += 1,
            0x20..=0x7e => {}
            0x80..=0xff => return reject(line, Why::NotAscii),
            _ => return reject(line, Why::ControlCharacter),
        }
    }
    Ok(())
}

fn parse_address(value: &str, policy: &Policy) -> Result<Ipv4Addr, Why> {
    let text = value.strip_suffix(ADDRESS_SUFFIX).ok_or(Why::BadAddress)?;
    let addr = parse_canonical_ipv4(text).ok_or(Why::BadAddress)?;
    if !policy.subnet.contains(addr) {
        return Err(Why::AddressOutsideSubnet);
    }
    if addr == policy.hub {
        return Err(Why::AddressIsHub);
    }
    if !policy.subnet.is_usable_host(addr) {
        return Err(Why::AddressNotUsable);
    }
    Ok(addr)
}

fn set_once<T>(slot: &mut Option<T>, value: T) -> Result<(), Why> {
    if slot.is_some() {
        return Err(Why::DuplicateKey);
    }
    *slot = Some(value);
    Ok(())
}

fn apply_setting(
    draft: &mut Draft,
    key_name: &str,
    value: &str,
    policy: &Policy,
) -> Result<(), Why> {
    match key_name {
        "PublicKey" => {
            let key = key::is_usable(value).then(|| value.to_string());
            set_once(&mut draft.public_key, key.ok_or(Why::BadPublicKey)?)
        }
        "PresharedKey" => {
            let key = key::is_usable(value).then(|| value.to_string());
            set_once(&mut draft.preshared_key, key.ok_or(Why::BadPresharedKey)?)
        }
        "AllowedIPs" => set_once(&mut draft.address, parse_address(value, policy)?),
        _ => Err(Why::UnknownKey),
    }
}

/// Reads a peers file. Either every line is acceptable and the peers come back in file order, or
/// the first rule broken comes back and nothing else does.
pub fn parse(input: &[u8], policy: &Policy) -> Result<Vec<Peer>, Reject> {
    if input.len() > MAX_INPUT_BYTES {
        return reject(0, Why::TooLarge);
    }
    check_bytes(input)?;
    let Ok(text) = std::str::from_utf8(input) else {
        return reject(0, Why::NotAscii);
    };
    if !text.ends_with('\n') {
        return reject(
            0,
            if text.is_empty() {
                Why::MissingHeader
            } else {
                Why::NoFinalNewline
            },
        );
    }
    let all: Vec<&str> = text.lines().collect();
    if all.first() != Some(&HEADER) {
        return reject(1, Why::MissingHeader);
    }
    let Some((&trailer, body)) = all[1..].split_last() else {
        return reject(all.len(), Why::BadTrailer);
    };
    let lines = body.iter().enumerate().map(|(i, l)| (i + 2, *l));
    let mut peers: Vec<(usize, Peer)> = Vec::new();
    let mut draft: Option<Draft> = None;
    for (n, raw) in lines {
        if raw.len() > MAX_LINE_BYTES {
            return reject(n, Why::LineTooLong);
        }
        let line = raw.trim_matches(' ');
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line == PEER_HEADER {
            if let Some(done) = draft.take() {
                peers.push(done.finish()?);
            }
            if peers.len() >= MAX_PEERS {
                return reject(n, Why::TooManyPeers);
            }
            draft = Some(Draft::new(n));
        } else if line.starts_with('[') {
            return reject(n, Why::UnknownSection);
        } else {
            let Some((name, value)) = line.split_once('=') else {
                return reject(n, Why::NotKeyValue);
            };
            let Some(current) = draft.as_mut() else {
                return reject(n, Why::KeyOutsidePeer);
            };
            apply_setting(
                current,
                name.trim_matches(' '),
                value.trim_matches(' '),
                policy,
            )
            .or_else(|why| reject(n, why))?;
        }
    }
    if let Some(done) = draft.take() {
        peers.push(done.finish()?);
    }
    if trailer != format!("{TRAILER_PREFIX}{}", peers.len()) {
        return reject(all.len(), Why::BadTrailer);
    }
    check_unique(&peers)?;
    Ok(peers.into_iter().map(|(_, peer)| peer).collect())
}

fn check_unique(peers: &[(usize, Peer)]) -> Result<(), Reject> {
    let mut keys = HashSet::new();
    let mut addresses = HashSet::new();
    for (line, peer) in peers {
        if !keys.insert(peer.public_key.as_str()) {
            return reject(*line, Why::DuplicatePublicKey);
        }
        if !addresses.insert(peer.address) {
            return reject(*line, Why::DuplicateAddress);
        }
    }
    Ok(())
}

/// The canonical text for `peers`: the header, then one block per peer, in the order given. This
/// is what the applier passes to `wg`, so `wg` never sees a byte the file supplied except through
/// a value this module already accepted.
pub fn render(peers: &[Peer]) -> String {
    render_labeled(&peers.iter().map(|p| ("", p)).collect::<Vec<_>>())
}

/// `render` with a `# label` comment above each peer that has one. A label is reduced to
/// letters, digits, `-` and `_` so a hostile one cannot add a line.
pub fn render_labeled(peers: &[(&str, &Peer)]) -> String {
    let mut out = format!("{HEADER}\n");
    for (label, peer) in peers {
        let label: String = label
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .take(MAX_LINE_BYTES / 2)
            .collect();
        out.push('\n');
        if !label.is_empty() {
            out.push_str(&format!("# {label}\n"));
        }
        out.push_str(&format!("{PEER_HEADER}\nPublicKey = {}\n", peer.public_key));
        if let Some(psk) = &peer.preshared_key {
            out.push_str(&format!("PresharedKey = {psk}\n"));
        }
        out.push_str(&format!("AllowedIPs = {}{ADDRESS_SUFFIX}\n", peer.address));
    }
    out.push_str(&format!("{TRAILER_PREFIX}{}\n", peers.len()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn policy() -> Policy {
        Policy::new(
            Subnet::parse("10.77.0.0/24").unwrap(),
            Ipv4Addr::new(10, 77, 0, 1),
        )
        .unwrap()
    }

    pub(crate) fn key(seed: u8) -> String {
        key::encode(&[seed; 32])
    }

    /// A whole file around `body`, with the trailer counting its `[Peer]` lines.
    fn file(body: &str) -> Vec<u8> {
        let count = body
            .lines()
            .filter(|l| l.trim_matches(' ') == PEER_HEADER)
            .count();
        format!("{HEADER}\n{body}{TRAILER_PREFIX}{count}\n").into_bytes()
    }

    fn peer_block(seed: u8, host: u8) -> String {
        format!(
            "[Peer]\nPublicKey = {}\nAllowedIPs = 10.77.0.{host}/32\n",
            key(seed)
        )
    }

    fn refused(input: &[u8]) -> Why {
        parse(input, &policy()).unwrap_err().why
    }

    #[test]
    fn a_minimal_file_parses_and_comments_and_blank_lines_are_ignored() {
        let text = file(&format!("# laptop\n\n{}  \n# end\n", peer_block(1, 2)));
        let peers = parse(&text, &policy()).unwrap();
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].address, Ipv4Addr::new(10, 77, 0, 2));
        assert_eq!(peers[0].preshared_key, None);
        assert_eq!(parse(&file(""), &policy()).unwrap(), vec![]);
    }

    #[test]
    fn spaces_around_the_equals_sign_are_fine_and_a_preshared_key_is_kept() {
        let text = file(&format!(
            "[Peer]\n  PublicKey=   {}\nAllowedIPs =10.77.0.9/32 \nPresharedKey = {}\n",
            key(1),
            key(2)
        ));
        let peers = parse(&text, &policy()).unwrap();
        assert_eq!(peers[0].preshared_key, Some(key(2)));
    }

    #[test]
    fn the_header_is_required_so_an_empty_or_foreign_file_applies_nothing() {
        assert_eq!(refused(b""), Why::MissingHeader);
        assert_eq!(refused(b"\n"), Why::MissingHeader);
        assert_eq!(refused(b"# hq-net peers v2\n"), Why::MissingHeader);
        assert_eq!(refused(b"# hq-net peers v1 \n"), Why::MissingHeader);
        assert_eq!(
            refused(format!("\n{HEADER}\n").as_bytes()),
            Why::MissingHeader
        );
        assert_eq!(refused(&peer_block(1, 2).into_bytes()), Why::MissingHeader);
        assert_eq!(refused(HEADER.as_bytes()), Why::NoFinalNewline);
    }

    #[test]
    fn the_trailer_must_state_how_many_peers_were_read() {
        let one = peer_block(1, 2);
        let wrong_counts = [
            format!("{HEADER}\n"),
            format!("{HEADER}\n{one}{TRAILER_PREFIX}0\n"),
            format!("{HEADER}\n{one}{TRAILER_PREFIX}2\n"),
            format!("{HEADER}\n{one}{TRAILER_PREFIX}01\n"),
            format!("{HEADER}\n{one}{TRAILER_PREFIX}1 \n"),
            format!("{HEADER}\n{one}{TRAILER_PREFIX}1\n\n"),
            format!("{HEADER}\n{one}{TRAILER_PREFIX}1\n# more\n"),
            format!("{HEADER}\n{TRAILER_PREFIX}1\n"),
        ];
        for text in wrong_counts {
            assert_eq!(refused(text.as_bytes()), Why::BadTrailer, "{text:?}");
        }
        assert!(parse(format!("{HEADER}\n{one}").as_bytes(), &policy()).is_err());
        let ok = format!("{HEADER}\n{one}{TRAILER_PREFIX}1\n");
        assert_eq!(parse(ok.as_bytes(), &policy()).unwrap().len(), 1);
    }

    #[test]
    fn a_file_cut_at_any_byte_is_refused() {
        let mut peers = vec![];
        for i in 0..3u8 {
            peers.push(Peer {
                public_key: key(i + 1),
                address: Ipv4Addr::new(10, 77, 0, i + 2),
                preshared_key: (i == 1).then(|| key(9)),
            });
        }
        let whole = render(&peers).into_bytes();
        assert!(parse(&whole, &policy()).is_ok());
        for cut in 0..whole.len() {
            assert!(
                parse(&whole[..cut], &policy()).is_err(),
                "a file cut at byte {cut} was accepted"
            );
        }
    }

    #[test]
    fn other_sections_are_refused_and_the_message_never_echoes_the_line() {
        for section in [
            "[Interface]",
            "[peer]",
            "[Peer]x",
            "[Peer x",
            "[]",
            "[Peer] x",
        ] {
            let r = parse(&file(&format!("{section}\n")), &policy()).unwrap_err();
            assert_eq!((r.why, r.line), (Why::UnknownSection, 2), "{section:?}");
        }
        let r = parse(&file("PostUp = /bin/secret-command\n"), &policy()).unwrap_err();
        assert_eq!((r.why, r.line), (Why::KeyOutsidePeer, 2));
        assert!(!r.to_string().contains("secret-command"));
        assert!(!r.to_string().contains("PostUp"));
    }

    #[test]
    fn every_hook_and_interface_key_inside_a_peer_is_refused() {
        for forbidden in [
            "PostUp = wg set %i private-key /root/x",
            "PreUp = x",
            "PostDown = x",
            "PreDown = x",
            "SaveConfig = true",
            "Table = off",
            "PrivateKey = abc",
            "ListenPort = 1",
            "Address = 10.77.0.5/32",
            "DNS = 1.1.1.1",
            "Endpoint = example.com:51820",
            "PersistentKeepalive = 25",
            "FwMark = 1",
            "MTU = 1280",
            "publickey = x",
            "PUBLICKEY = x",
            "PublicKey2 = x",
            "Public Key = x",
            "= x",
        ] {
            let r = parse(&file(&format!("[Peer]\n{forbidden}\n")), &policy()).unwrap_err();
            assert_eq!((r.why, r.line), (Why::UnknownKey, 3), "{forbidden:?}");
        }
        let r = parse(&file("[Peer]\nnot a setting\n"), &policy()).unwrap_err();
        assert_eq!(r.why, Why::NotKeyValue);
    }

    #[test]
    fn newline_and_option_injection_in_a_value_is_refused() {
        for evil in [
            format!("{}\nPostUp = x", key(1)),
            format!("{} # c", key(1)),
            format!("{}\rPostUp = x", key(1)),
            format!("{};PostUp=x", key(1)),
            format!("{},{}", key(1), key(2)),
            format!("\"{}\"", key(1)),
        ] {
            let text = file(&format!(
                "[Peer]\nPublicKey = {evil}\nAllowedIPs = 10.77.0.2/32\n"
            ));
            assert!(parse(&text, &policy()).is_err(), "{evil:?} must not parse");
        }
    }

    #[test]
    fn allowed_ips_must_be_one_plain_slash_32_inside_the_subnet() {
        let cases: [(&str, Why); 16] = [
            ("10.77.0.2/24", Why::BadAddress),
            ("10.77.0.2/31", Why::BadAddress),
            ("10.77.0.2", Why::BadAddress),
            ("10.77.0.2/32,10.77.0.3/32", Why::BadAddress),
            ("0.0.0.0/0", Why::BadAddress),
            ("::/0", Why::BadAddress),
            ("fd00::2/128", Why::BadAddress),
            ("10.77.0.02/32", Why::BadAddress),
            ("10.77.0.+2/32", Why::BadAddress),
            ("10.77.0.2/032", Why::BadAddress),
            ("10.77.0.1/32", Why::AddressIsHub),
            ("10.77.0.0/32", Why::AddressNotUsable),
            ("10.77.0.255/32", Why::AddressNotUsable),
            ("10.77.1.2/32", Why::AddressOutsideSubnet),
            ("192.168.1.2/32", Why::AddressOutsideSubnet),
            ("10.77.0.256/32", Why::BadAddress),
        ];
        for (value, why) in cases {
            let text = file(&format!(
                "[Peer]\nPublicKey = {}\nAllowedIPs = {value}\n",
                key(1)
            ));
            assert_eq!(refused(&text), why, "{value:?}");
        }
    }

    #[test]
    fn keys_must_be_44_canonical_base64_characters_of_32_non_zero_bytes() {
        let good = key(3);
        let bad_keys = [
            good[..43].to_string(),
            format!("{good}="),
            good.replace('=', ""),
            "A".repeat(44),
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".to_string(),
            format!("{}B=", &good[..42]),
            good.replace('A', "-"),
            String::new(),
        ];
        for bad in bad_keys {
            let text = file(&format!(
                "[Peer]\nPublicKey = {bad}\nAllowedIPs = 10.77.0.2/32\n"
            ));
            assert_eq!(refused(&text), Why::BadPublicKey, "{bad:?}");
            let text = file(&format!(
                "[Peer]\nPublicKey = {good}\nAllowedIPs = 10.77.0.2/32\nPresharedKey = {bad}\n"
            ));
            assert_eq!(refused(&text), Why::BadPresharedKey, "{bad:?}");
        }
    }

    #[test]
    fn a_peer_needs_each_required_setting_exactly_once() {
        let k = key(1);
        assert_eq!(
            refused(&file("[Peer]\nAllowedIPs = 10.77.0.2/32\n")),
            Why::MissingPublicKey
        );
        assert_eq!(
            refused(&file(&format!("[Peer]\nPublicKey = {k}\n"))),
            Why::MissingAddress
        );
        assert_eq!(refused(&file("[Peer]\n")), Why::MissingPublicKey);
        let twice =
            format!("[Peer]\nPublicKey = {k}\nPublicKey = {k}\nAllowedIPs = 10.77.0.2/32\n");
        assert_eq!(refused(&file(&twice)), Why::DuplicateKey);
        let twice = format!(
            "[Peer]\nPublicKey = {k}\nAllowedIPs = 10.77.0.2/32\nAllowedIPs = 10.77.0.3/32\n"
        );
        assert_eq!(refused(&file(&twice)), Why::DuplicateKey);
        let twice = format!(
            "[Peer]\nPublicKey = {k}\nAllowedIPs = 10.77.0.2/32\nPresharedKey = {k}\nPresharedKey = {k}\n"
        );
        assert_eq!(refused(&file(&twice)), Why::DuplicateKey);
    }

    #[test]
    fn duplicate_addresses_and_keys_across_peers_are_refused() {
        let same_address = file(&format!("{}\n{}", peer_block(1, 2), peer_block(2, 2)));
        let r = parse(&same_address, &policy()).unwrap_err();
        assert_eq!((r.why, r.line), (Why::DuplicateAddress, 6));
        let same_key = file(&format!("{}\n{}", peer_block(1, 2), peer_block(1, 3)));
        assert_eq!(refused(&same_key), Why::DuplicatePublicKey);
    }

    #[test]
    fn sixty_four_peers_pass_and_sixty_five_do_not() {
        let build = |count: u8| {
            let blocks: String = (0..count)
                .map(|i| peer_block(i + 1, i + 2) + "\n")
                .collect();
            file(&blocks)
        };
        assert_eq!(parse(&build(64), &policy()).unwrap().len(), 64);
        assert_eq!(refused(&build(65)), Why::TooManyPeers);
    }

    #[test]
    fn size_line_length_bytes_and_line_endings_are_bounded() {
        let big = vec![b'#'; MAX_INPUT_BYTES + 1];
        assert_eq!(refused(&big), Why::TooLarge);
        let long = file(&format!("# {}\n", "x".repeat(MAX_LINE_BYTES)));
        assert_eq!(refused(&long), Why::LineTooLong);
        let exact = file(&format!("#{}\n", "x".repeat(MAX_LINE_BYTES - 1)));
        assert!(parse(&exact, &policy()).is_ok());
        for (bytes, why) in [
            (b"# a\tb\n".to_vec(), Why::ControlCharacter),
            (b"# a\r\n".to_vec(), Why::ControlCharacter),
            (b"# a\0b\n".to_vec(), Why::ControlCharacter),
            (b"# a\x1bb\n".to_vec(), Why::ControlCharacter),
            (b"# caf\xc3\xa9\n".to_vec(), Why::NotAscii),
            (b"# \xff\xfe\n".to_vec(), Why::NotAscii),
        ] {
            let mut text = format!("{HEADER}\n").into_bytes();
            text.extend(bytes);
            assert_eq!(refused(&text), why);
        }
        assert_eq!(refused(b"\xef\xbb\xbf# hq-net peers v1\n"), Why::NotAscii);
    }

    #[test]
    fn render_then_parse_returns_the_same_peers() {
        let peers = vec![
            Peer {
                public_key: key(1),
                address: Ipv4Addr::new(10, 77, 0, 2),
                preshared_key: None,
            },
            Peer {
                public_key: key(2),
                address: Ipv4Addr::new(10, 77, 0, 200),
                preshared_key: Some(key(9)),
            },
        ];
        let text = render(&peers);
        assert_eq!(parse(text.as_bytes(), &policy()).unwrap(), peers);
        let labeled = render_labeled(&[("laptop", &peers[0]), ("x\nPostUp = y", &peers[1])]);
        assert!(labeled.contains("# laptop\n"));
        assert!(!labeled.contains("PostUp ="));
        assert_eq!(parse(labeled.as_bytes(), &policy()).unwrap(), peers);
    }

    #[test]
    fn a_policy_needs_a_private_subnet_and_a_host_address_for_the_hub() {
        let net = Subnet::parse("10.77.0.0/24").unwrap();
        assert!(Policy::new(net, Ipv4Addr::new(10, 77, 0, 0)).is_err());
        assert!(Policy::new(net, Ipv4Addr::new(10, 77, 0, 255)).is_err());
        assert!(Policy::new(net, Ipv4Addr::new(10, 78, 0, 1)).is_err());
        assert!(
            Policy::new(
                Subnet::parse("100.64.0.0/24").unwrap(),
                Ipv4Addr::new(100, 64, 0, 1)
            )
            .is_err()
        );
        assert!(Policy::new(net, Ipv4Addr::new(10, 77, 0, 1)).is_ok());
    }
}
