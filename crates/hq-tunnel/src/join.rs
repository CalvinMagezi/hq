//! The join text: what `hq net peer add` prints for a device and its QR code carries. It is
//! parsed strictly. Only the lines `hq net` writes are accepted, so a QR code from anywhere else
//! cannot smuggle in a setting this peer does not model (a private key, a DNS server, a hook).

use crate::error::{Result, invalid};
use crate::secret::Secret32;
use hq_net::{key, names, subnet::parse_canonical_ipv4};
use std::net::{Ipv4Addr, SocketAddr, ToSocketAddrs};

/// The device config is a few hundred bytes; anything near this is not one.
pub const MAX_JOIN_BYTES: usize = 4096;
const HOST_PREFIX: &str = "/32";
/// A keepalive keeps the radio busy; the join text is not trusted to set it to a second.
use hq_net::keepalive::{
    KEEPALIVE_MAX_SECS as MAX_KEEPALIVE_SECS, KEEPALIVE_MIN_SECS as MIN_KEEPALIVE_SECS,
};
const MAX_NAMED_CHARS: usize = 32;
const CGNAT_FIRST_OCTET: u8 = 100;
const CGNAT_SECOND_MASK: u8 = 0b1100_0000;
const CGNAT_SECOND_VALUE: u8 = 0b0100_0000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    host: String,
    port: u16,
}

impl Endpoint {
    pub fn parse(text: &str) -> Result<Self> {
        if text.starts_with('[') || !names::valid_endpoint(text) {
            return invalid("Endpoint is not host:port with a DNS name or an IPv4 address");
        }
        let (host, port) = text.rsplit_once(':').unwrap_or_default();
        // A host that ends in digits or `0x` is an address, and only the plain dotted form is one:
        // `127.1` and `0x7f000001` would be read as loopback by some resolvers.
        let numeric = host
            .rsplit('.')
            .next()
            .is_some_and(|l| l.bytes().all(|b| b.is_ascii_digit()) || l.starts_with("0x"));
        if numeric && parse_canonical_ipv4(host).is_none() {
            return invalid("Endpoint looks like an address but is not a plain a.b.c.d");
        }
        let port = port.parse().unwrap_or_default();
        Ok(Self {
            host: host.to_string(),
            port,
        })
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// The first IPv4 address the name resolves to. Called at start and again when the network
    /// changes, because a name can move and a phone changes networks all day.
    pub fn resolve(&self) -> std::io::Result<SocketAddr> {
        (self.host.as_str(), self.port)
            .to_socket_addrs()?
            .find(SocketAddr::is_ipv4)
            .ok_or_else(|| std::io::Error::other("the endpoint has no IPv4 address"))
    }
}

impl std::fmt::Display for Endpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.host, self.port)
    }
}

#[derive(Debug, Clone)]
pub struct JoinConfig {
    /// This device's address in the plane.
    pub address: Ipv4Addr,
    pub hub_key: [u8; 32],
    pub hub_address: Ipv4Addr,
    pub preshared_key: Option<Secret32>,
    pub endpoint: Endpoint,
    /// Seconds between keepalive packets, or none.
    pub keepalive_secs: Option<u16>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    None,
    Interface,
    Peer,
}

#[derive(Default)]
struct Seen {
    interface: bool,
    peer: bool,
    address: Option<Ipv4Addr>,
    public_key: Option<[u8; 32]>,
    preshared_key: Option<[u8; 32]>,
    endpoint: Option<Endpoint>,
    allowed: Option<Ipv4Addr>,
    keepalive: Option<u16>,
    keepalive_seen: bool,
}

fn host_address(text: &str, what: &str) -> Result<Ipv4Addr> {
    let Some(bare) = text.strip_suffix(HOST_PREFIX) else {
        return invalid(format!(
            "{what} must be a single address written a.b.c.d/32"
        ));
    };
    let Some(addr) = parse_canonical_ipv4(bare) else {
        return invalid(format!("{what} is not an IPv4 address"));
    };
    if !is_plane_address(addr) {
        return invalid(format!(
            "{what} is not a private plane address (RFC 1918 or 100.64.0.0/10)"
        ));
    }
    Ok(addr)
}

pub(crate) fn is_plane_address(addr: Ipv4Addr) -> bool {
    let o = addr.octets();
    let cgnat = o[0] == CGNAT_FIRST_OCTET && o[1] & CGNAT_SECOND_MASK == CGNAT_SECOND_VALUE;
    (addr.is_private() || cgnat) && !addr.is_broadcast() && o[3] != 0 && o[3] != u8::MAX
}

fn once<T>(slot: &mut Option<T>, value: T, key: &str) -> Result<()> {
    if slot.replace(value).is_some() {
        return invalid(format!("{key} appears twice"));
    }
    Ok(())
}

fn read_key(value: &str, name: &str) -> Result<[u8; 32]> {
    match key::decode(value) {
        Some(k) if k != [0; 32] => Ok(k),
        _ => invalid(format!(
            "{name} is not a WireGuard key (44 characters of base64)"
        )),
    }
}

/// The start of a name from the text, for an error message. The text is not trusted, and the
/// message may end up on a screen.
fn named(name: &str) -> String {
    name.chars().take(MAX_NAMED_CHARS).collect()
}

/// Seconds, written the plain way: 0 for none, or from 10 to 300.
fn keepalive(value: &str) -> Result<Option<u16>> {
    let plain = !value.is_empty()
        && value.len() <= 3
        && value.bytes().all(|b| b.is_ascii_digit())
        && (value == "0" || !value.starts_with('0'));
    let secs: u16 = if plain {
        value.parse().unwrap_or(u16::MAX)
    } else {
        u16::MAX
    };
    match secs {
        0 => Ok(None),
        MIN_KEEPALIVE_SECS..=MAX_KEEPALIVE_SECS => Ok(Some(secs)),
        _ => invalid(format!(
            "PersistentKeepalive must be 0 or between {MIN_KEEPALIVE_SECS} and {MAX_KEEPALIVE_SECS} seconds"
        )),
    }
}

fn interface_line(seen: &mut Seen, key_name: &str, value: &str) -> Result<()> {
    match key_name {
        "Address" => once(&mut seen.address, host_address(value, "Address")?, key_name),
        // The line is named, never its value: it may be a secret that has no business here.
        other => invalid(format!(
            "the [Interface] line {} is not one `hq net` writes (a private key is made on this device, not carried in the join text)",
            named(other)
        )),
    }
}

fn peer_line(seen: &mut Seen, key_name: &str, value: &str) -> Result<()> {
    match key_name {
        "PublicKey" => once(
            &mut seen.public_key,
            read_key(value, "PublicKey")?,
            key_name,
        ),
        "PresharedKey" => once(
            &mut seen.preshared_key,
            read_key(value, "PresharedKey")?,
            key_name,
        ),
        "Endpoint" => once(&mut seen.endpoint, Endpoint::parse(value)?, key_name),
        "AllowedIPs" => once(
            &mut seen.allowed,
            host_address(value, "AllowedIPs")?,
            key_name,
        ),
        "PersistentKeepalive" => {
            if std::mem::replace(&mut seen.keepalive_seen, true) {
                return invalid("PersistentKeepalive appears twice");
            }
            seen.keepalive = keepalive(value)?;
            Ok(())
        }
        other => invalid(format!(
            "the [Peer] line {} is not one `hq net` writes",
            named(other)
        )),
    }
}

/// The join text inside what `hq net peer add` prints, which wraps it in prose and a QR code: from
/// the `[Interface]` line to the end of the `[Peer]` section (the first blank line after it). Text
/// with no `[Interface]` line comes back as it is, for the parser to refuse. Only this block is
/// ever parsed or kept; whatever surrounds it is dropped.
pub fn extract_block(text: &str) -> &str {
    let mut start = None;
    let mut in_peer = false;
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_end();
        match (start, trimmed) {
            (None, "[Interface]") => start = Some(offset),
            (Some(_), "[Peer]") => in_peer = true,
            (Some(from), "") if in_peer => return &text[from..offset],
            _ => {}
        }
        offset += line.len();
    }
    start.map_or(text, |from| &text[from..])
}

pub fn parse(text: &str) -> Result<JoinConfig> {
    if text.len() > MAX_JOIN_BYTES {
        return invalid("the join text is too long to be a device config");
    }
    if !text
        .bytes()
        .all(|b| b == b'\n' || (0x20..0x7f).contains(&b))
    {
        return invalid("the join text has a character `hq net` never writes");
    }
    let mut section = Section::None;
    let mut seen = Seen::default();
    for line in text.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        match line {
            "[Interface]" if !seen.interface && !seen.peer => {
                seen.interface = true;
                section = Section::Interface;
            }
            "[Peer]" if seen.interface && !seen.peer => {
                seen.peer = true;
                section = Section::Peer;
            }
            "[Interface]" | "[Peer]" => {
                return invalid("the sections must be one [Interface] followed by one [Peer]");
            }
            _ => {
                let Some((name, value)) = line.split_once(" = ") else {
                    return invalid("a line is not `Name = value`");
                };
                if name.trim() != name || value.trim() != value || value.is_empty() {
                    return invalid("a line has stray spaces or an empty value");
                }
                match section {
                    Section::Interface => interface_line(&mut seen, name, value)?,
                    Section::Peer => peer_line(&mut seen, name, value)?,
                    Section::None => return invalid("a setting comes before any section"),
                }
            }
        }
    }
    finish(seen)
}

fn finish(seen: Seen) -> Result<JoinConfig> {
    let (Some(address), Some(hub_key), Some(endpoint), Some(hub_address)) =
        (seen.address, seen.public_key, seen.endpoint, seen.allowed)
    else {
        return invalid("the join text needs Address, PublicKey, Endpoint and AllowedIPs");
    };
    if address == hub_address {
        return invalid("Address and AllowedIPs are the same address");
    }
    Ok(JoinConfig {
        address,
        hub_key,
        hub_address,
        preshared_key: seen.preshared_key.map(Secret32::new),
        endpoint,
        keepalive_secs: seen.keepalive,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_join_text_is_found_inside_what_peer_add_prints() {
        let config = "[Interface]\n# PrivateKey is not here\nAddress = 10.77.0.2/32\n\n[Peer]\nPublicKey = k\nEndpoint = h:1\nAllowedIPs = 10.77.0.1/32\n";
        let printed = format!(
            "Adding phone \"p\" at 10.77.0.2.\n\nConfig for the device:\n{config}\nScan this:\n\u{2588}\u{2580} qr\n\nTreat it like a password.\n"
        );
        assert_eq!(extract_block(&printed), config);
        assert_eq!(
            extract_block(config),
            config,
            "a bare join text is unchanged"
        );
        assert_eq!(extract_block("nothing here"), "nothing here");
        assert_eq!(extract_block(""), "");
        // A second block, or lines after the blank one, are never part of what is kept.
        let two = format!("{config}\n{config}");
        assert_eq!(extract_block(&two), config);
        let late = format!("{config}\nPrivateKey = x\n");
        assert_eq!(extract_block(&late), config);
        // No blank line after the peer section: the block runs to the end.
        let bare = config.trim_end();
        assert_eq!(extract_block(bare), bare);
        // CRLF line ends.
        let crlf = config.replace('\n', "\r\n");
        assert_eq!(extract_block(&crlf), crlf);
    }

    fn k(n: u8) -> String {
        key::encode(&[n; 32])
    }

    fn sample() -> String {
        format!(
            "[Interface]\n# PrivateKey is not here: this device made its own.\nAddress = 10.77.0.4/32\n\n[Peer]\nPublicKey = {}\nEndpoint = vpn.example.com:51820\nAllowedIPs = 10.77.0.1/32\nPersistentKeepalive = 25\n",
            k(7)
        )
    }

    #[test]
    fn the_text_hq_net_writes_is_read_back() {
        let join = parse(&sample()).unwrap();
        assert_eq!(join.address, Ipv4Addr::new(10, 77, 0, 4));
        assert_eq!(join.hub_address, Ipv4Addr::new(10, 77, 0, 1));
        assert_eq!(join.hub_key, [7; 32]);
        assert_eq!(join.endpoint.host(), "vpn.example.com");
        assert_eq!(join.endpoint.port(), 51820);
        assert_eq!(join.keepalive_secs, Some(25));
        assert!(join.preshared_key.is_none());
    }

    #[test]
    fn a_preshared_key_and_no_keepalive_are_read() {
        let text = sample()
            .replace("PersistentKeepalive = 25\n", "")
            .replace("Endpoint", &format!("PresharedKey = {}\nEndpoint", k(9)));
        let join = parse(&text).unwrap();
        assert!(join.preshared_key.is_some());
        assert_eq!(join.keepalive_secs, None);
        assert!(
            !format!("{join:?}").contains(&k(9)),
            "a secret never reaches Debug"
        );
        let zero = sample().replace("= 25", "= 0");
        assert_eq!(parse(&zero).unwrap().keepalive_secs, None);
    }

    #[test]
    fn anything_hq_net_does_not_write_is_refused() {
        let bad = [
            sample().replace("Address", "PrivateKey"),
            sample().replace("[Interface]\n", "[Interface]\nDNS = 10.0.0.1\n"),
            sample().replace("[Interface]\n", "[Interface]\nPostUp = true\n"),
            sample().replace("Endpoint", "ListenPort = 1\nEndpoint"),
            sample().replace("/32\n\n[Peer]", "/24\n\n[Peer]"),
            sample().replace("AllowedIPs = 10.77.0.1/32", "AllowedIPs = 0.0.0.0/0"),
            sample().replace(
                "AllowedIPs = 10.77.0.1/32",
                "AllowedIPs = 10.77.0.1/32, 10.77.0.2/32",
            ),
            sample().replace("AllowedIPs = 10.77.0.1/32", "AllowedIPs = 10.77.0.4/32"),
            sample().replace("10.77.0.4", "8.8.8.8"),
            sample().replace("10.77.0.4", "10.77.0.0"),
            sample().replace("10.77.0.4", "10.77.0.255"),
            sample().replace("10.77.0.4", "010.77.0.4"),
            sample().replace("vpn.example.com:51820", "[::1]:51820"),
            sample().replace("vpn.example.com:51820", "vpn.example.com"),
            sample().replace("vpn.example.com:51820", "vpn.example.com:0"),
            sample().replace(&k(7), "short"),
            sample().replace(&k(7), &key::encode(&[0; 32])),
            sample().replace("\n[Peer]", "\n[Peer]\n[Peer]"),
            sample() + "[Peer]\nPublicKey = x\n",
            sample().replace("= 25", "= twenty"),
            sample().replace("= 25", "= 70000"),
            sample().replace("= 25", "= 1"),
            sample().replace("= 25", "= 9"),
            sample().replace("= 25", "= 301"),
            sample().replace("= 25", "= +25"),
            sample().replace("= 25", "= 025"),
            sample().replace("= 25", "= 00"),
            sample().replace("vpn.example.com:51820", "127.1:51820"),
            sample().replace("vpn.example.com:51820", "2130706433:51820"),
            sample().replace("vpn.example.com:51820", "0x7f000001:51820"),
            sample().replace("vpn.example.com:51820", "010.0.0.1:51820"),
            sample().replace("vpn.example.com:51820", "example.123:51820"),
            sample() + "PersistentKeepalive = 30\n",
            sample().replace(" = ", "="),
            sample().replace("Address = ", "Address =  "),
            sample().replace("\n", "\r\n"),
            sample().replace("Address", "Addr\tess"),
            sample().replace("[Peer]", "Orphan = 1\n[Peer]"),
            "Address = 10.77.0.4/32\n".to_string() + &sample(),
            String::new(),
        ];
        for text in bad {
            assert!(parse(&text).is_err(), "{text}");
        }
        assert!(parse(&"#".repeat(MAX_JOIN_BYTES + 1)).is_err());
    }

    #[test]
    fn the_keepalive_range_and_plain_addresses_are_accepted() {
        for (value, expected) in [
            ("10", Some(10)),
            ("25", Some(25)),
            ("300", Some(300)),
            ("0", None),
        ] {
            let text = sample().replace("= 25", &format!("= {value}"));
            assert_eq!(parse(&text).unwrap().keepalive_secs, expected, "{value}");
        }
        for host in [
            "1.2.3.4:51820",
            "vpn1.example.com:51820",
            "123.example.com:51820",
            "hub:51820",
        ] {
            let text = sample().replace("vpn.example.com:51820", host);
            assert!(parse(&text).is_ok(), "{host}");
        }
    }

    #[test]
    fn an_overlong_line_name_is_cut_in_the_message() {
        let long = "A".repeat(500);
        let text = sample().replace("[Peer]", &format!("[Peer]\n{long} = 1"));
        let err = parse(&text).unwrap_err().to_string();
        assert!(err.len() < 200, "{}", err.len());
    }

    #[test]
    fn a_refusal_names_the_line_and_never_the_value() {
        let secret = k(5);
        let text = sample().replace("Address", &format!("PrivateKey = {secret}\nAddress"));
        let err = parse(&text).unwrap_err().to_string();
        assert!(err.contains("PrivateKey"));
        assert!(!err.contains(&secret));
    }

    #[test]
    fn a_plane_address_is_private_or_shared_space_and_not_a_network_or_broadcast() {
        for ok in [
            "10.77.0.4",
            "192.168.5.9",
            "172.16.0.2",
            "100.64.0.2",
            "100.127.255.2",
        ] {
            assert!(is_plane_address(ok.parse().unwrap()), "{ok}");
        }
        for bad in [
            "8.8.8.8",
            "100.128.0.2",
            "100.63.0.2",
            "127.0.0.1",
            "0.0.0.0",
            "10.0.0.0",
            "10.0.0.255",
        ] {
            assert!(!is_plane_address(bad.parse().unwrap()), "{bad}");
        }
    }

    #[test]
    fn garbage_never_panics() {
        use rand::{Rng, SeedableRng};
        let mut rng = rand::rngs::StdRng::seed_from_u64(0x6a01);
        let base = sample().into_bytes();
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
            if let Ok(text) = String::from_utf8(bytes) {
                let _ = parse(&text);
            }
        }
    }

    #[test]
    fn the_text_the_hub_renderer_writes_is_accepted() {
        // The emitter and this parser are separate code; keep them in step. The record is read
        // from TOML so the test does not name fields a later `hq net` may add.
        use hq_net::device::config_text;
        use hq_net::store::{Net, PeerRecord};
        let net = Net {
            interface: "hq0".into(),
            subnet: "10.77.0.0/24".into(),
            hub: "10.77.0.1".into(),
            port: 51820,
            endpoint: Some("vpn.example.com:51820".into()),
            hub_public_key: k(3),
        };
        // The keepalive column is what the record asks for; the last column is what the join text
        // must then carry: the laptop default, none, or the exact value at each end of the range.
        for (kind, psk, asked, keepalive) in [
            ("laptop", None, None, Some(25)),
            ("laptop", None, Some(0), None),
            ("phone", None, None, None),
            ("phone", Some(k(4)), None, None),
            (
                "phone",
                None,
                Some(hq_net::keepalive::KEEPALIVE_MIN_SECS),
                Some(hq_net::keepalive::KEEPALIVE_MIN_SECS),
            ),
            ("phone", None, Some(25), Some(25)),
            (
                "phone",
                None,
                Some(hq_net::keepalive::KEEPALIVE_MAX_SECS),
                Some(hq_net::keepalive::KEEPALIVE_MAX_SECS),
            ),
        ] {
            let psk_line = psk
                .as_ref()
                .map(|p| format!("preshared_key = \"{p}\"\n"))
                .unwrap_or_default();
            let keepalive_line = asked
                .map(|s| format!("keepalive = {s}\n"))
                .unwrap_or_default();
            let record: PeerRecord = toml::from_str(&format!(
                "label = \"dev\"\nkind = \"{kind}\"\npublic_key = \"{}\"\naddress = \"10.77.0.4\"\n{psk_line}{keepalive_line}",
                k(5)
            ))
            .unwrap();
            let join = parse(&config_text(&net, &record).unwrap()).unwrap();
            assert_eq!(join.address, Ipv4Addr::new(10, 77, 0, 4));
            assert_eq!(join.hub_address, Ipv4Addr::new(10, 77, 0, 1));
            assert_eq!(join.hub_key, [3; 32]);
            assert_eq!(join.preshared_key.is_some(), psk.is_some());
            assert_eq!(join.endpoint.to_string(), "vpn.example.com:51820");
            assert_eq!(join.keepalive_secs, keepalive, "{kind} asked {asked:?}");
        }
    }
}
