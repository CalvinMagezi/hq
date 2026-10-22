//! The ticket the machine showing the QR code hands to the one scanning it, the link it travels
//! in, the two things a ticket can be for, and the protocol versions both sides negotiate.

use crate::code::{CODE_BYTES, Code};
use anyhow::{Result, anyhow, bail};
use data_encoding::{BASE32_NOPAD, BASE64URL_NOPAD};
use iroh::{EndpointId, RelayUrl};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;

/// The scheme of the link in the QR code. The stock camera hands a link like this to the app that
/// registered the scheme. Whether to move to an https link is an open owner decision, so every
/// place that builds or reads the prefix goes through this constant.
pub const LINK_SCHEME: &str = "hqpair";
const LINK_ROUTE: &str = "ticket";
/// The older copy-paste form. Still read; the same body follows the prefix.
pub const FALLBACK_PREFIX: &str = "hqpair1.";
/// The first ticket layout of the product. Spike F's tickets were layout 1.
pub const TICKET_FORMAT: u8 = 3;
/// Pairing protocol versions this build speaks, newest first. Each is an ALPN suffix.
pub const WIRE_VERSIONS: &[u16] = &[3];
/// A link this long cannot be a ticket; it is refused before anything is decoded.
const MAX_LINK_CHARS: usize = 2_000;
const MAX_LIST: usize = 8;
const MAX_ITEM_CHARS: usize = 300;

/// What a ticket opens the door to. The purpose is part of the protocol name, so a pairing ticket
/// can never finish a registration with an `hq edge` and the other way round.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Purpose {
    /// Two machines of one person become peers of each other.
    Pair,
    /// A device registers itself with an `hq edge` (the public forwarder).
    Register,
}

impl Purpose {
    fn word(self) -> &'static str {
        match self {
            Purpose::Pair => "pair",
            Purpose::Register => "register",
        }
    }

    /// The protocol name for one version, e.g. `hq/pair/3`.
    pub fn alpn(self, version: u16) -> Vec<u8> {
        format!("hq/{}/{version}", self.word()).into_bytes()
    }

    /// Every protocol name this build speaks for the purpose, newest first.
    pub fn alpns(self) -> Vec<Vec<u8>> {
        WIRE_VERSIONS.iter().map(|v| self.alpn(*v)).collect()
    }

    /// The purpose and version a protocol name stands for.
    pub fn from_alpn(alpn: &[u8]) -> Option<(Purpose, u16)> {
        let text = std::str::from_utf8(alpn).ok()?;
        let mut parts = text.split('/');
        let (hq, word, version, rest) = (parts.next()?, parts.next()?, parts.next()?, parts.next());
        if hq != "hq" || rest.is_some() {
            return None;
        }
        let purpose = [Purpose::Pair, Purpose::Register]
            .into_iter()
            .find(|p| p.word() == word)?;
        Some((purpose, version.parse().ok()?))
    }
}

/// The newest version both sides speak. Without one, the message says which side is older, so the
/// person knows which device to update.
pub fn negotiate(theirs: &[u16], ours: &[u16]) -> Result<u16> {
    if let Some(version) = ours.iter().filter(|v| theirs.contains(v)).max() {
        return Ok(*version);
    }
    let their_newest = theirs.iter().max().copied().unwrap_or(0);
    let our_newest = ours.iter().max().copied().unwrap_or(0);
    if their_newest > our_newest {
        bail!("the other device speaks a newer pairing version than this one: update HQ here");
    }
    bail!("the other device speaks an older pairing version than this one: update HQ there");
}

/// What the machine showing the QR code hands to the one scanning it. It holds no secret key. The
/// `id` is a throwaway endpoint made for this one ticket, not the machine's long-term identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ticket {
    pub v: u8,
    pub k: Purpose,
    pub id: String,
    pub code: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relays: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub addrs: Vec<String>,
    /// The protocol versions the host speaks.
    pub proto: Vec<u16>,
}

#[derive(Deserialize)]
struct Peek {
    v: Option<u64>,
}

impl Ticket {
    pub fn new(
        purpose: Purpose,
        id: EndpointId,
        code: &Code,
        relays: Vec<String>,
        addrs: Vec<String>,
    ) -> Self {
        Self {
            v: TICKET_FORMAT,
            k: purpose,
            id: id.to_string(),
            code: BASE32_NOPAD.encode(code),
            relays,
            addrs,
            proto: WIRE_VERSIONS.to_vec(),
        }
    }

    fn body(&self) -> String {
        let json = serde_json::to_vec(self).expect("a ticket always serializes");
        BASE64URL_NOPAD.encode(&json)
    }

    /// The text for the QR code and for copy and paste.
    pub fn link(&self) -> String {
        format!("{LINK_SCHEME}://{LINK_ROUTE}/{}", self.body())
    }

    /// Reads a link, or the older copy-paste form, and checks everything a dial depends on.
    pub fn parse(text: &str) -> Result<Self> {
        let text = text.trim();
        if text.len() > MAX_LINK_CHARS {
            bail!("that is too long to be a pairing ticket");
        }
        let link_prefix = format!("{LINK_SCHEME}://{LINK_ROUTE}/");
        let body = strip_prefix_ignoring_case(text, &link_prefix)
            .or_else(|| strip_prefix_ignoring_case(text, FALLBACK_PREFIX))
            .ok_or_else(|| anyhow!("a pairing ticket starts with {link_prefix}"))?;
        let json = BASE64URL_NOPAD
            .decode(body.as_bytes())
            .map_err(|_| anyhow!("the ticket is damaged"))?;
        let peek: Peek =
            serde_json::from_slice(&json).map_err(|_| anyhow!("the ticket is damaged"))?;
        match peek.v {
            Some(v) if v == u64::from(TICKET_FORMAT) => {}
            Some(v) if v < u64::from(TICKET_FORMAT) => bail!(
                "this ticket is from an older version of HQ and cannot be used: show a new one"
            ),
            Some(_) => bail!("this ticket is from a newer version of HQ: update HQ here"),
            None => bail!("the ticket is damaged"),
        }
        let ticket: Ticket =
            serde_json::from_slice(&json).map_err(|_| anyhow!("the ticket is damaged"))?;
        ticket.check()?;
        Ok(ticket)
    }

    fn check(&self) -> Result<()> {
        self.endpoint_id()?;
        self.code_bytes()?;
        let list_ok = |items: &[String]| {
            items.len() <= MAX_LIST && items.iter().all(|s| s.len() <= MAX_ITEM_CHARS)
        };
        if !list_ok(&self.relays) || !list_ok(&self.addrs) || self.proto.len() > MAX_LIST {
            bail!("the ticket is damaged");
        }
        if self.proto.is_empty() {
            bail!("the ticket names no pairing version");
        }
        for relay in &self.relays {
            relay
                .parse::<RelayUrl>()
                .map_err(|_| anyhow!("the ticket names a bad relay"))?;
        }
        for addr in &self.addrs {
            addr.parse::<SocketAddr>()
                .map_err(|_| anyhow!("the ticket names a bad address"))?;
        }
        Ok(())
    }

    pub fn endpoint_id(&self) -> Result<EndpointId> {
        self.id
            .parse()
            .map_err(|_| anyhow!("the ticket names no machine"))
    }

    pub fn code_bytes(&self) -> Result<Code> {
        let raw = BASE32_NOPAD
            .decode(self.code.as_bytes())
            .map_err(|_| anyhow!("the ticket's code is damaged"))?;
        let code: Code = raw
            .try_into()
            .map_err(|_| anyhow!("the ticket's code is damaged"))?;
        debug_assert_eq!(code.len(), CODE_BYTES);
        Ok(code)
    }

    /// The protocol version to dial with, or why there is none.
    pub fn version(&self) -> Result<u16> {
        negotiate(&self.proto, WIRE_VERSIONS)
    }
}

fn strip_prefix_ignoring_case<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let head = text.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &text[prefix.len()..])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code::new_code;
    use iroh::SecretKey;

    fn id() -> EndpointId {
        SecretKey::generate().public()
    }

    fn sample() -> Ticket {
        Ticket::new(
            Purpose::Pair,
            id(),
            &new_code(),
            vec!["https://relay.example.com".into()],
            vec!["192.0.2.1:7000".into()],
        )
    }

    fn with_body(json: &str) -> String {
        format!(
            "{LINK_SCHEME}://{LINK_ROUTE}/{}",
            BASE64URL_NOPAD.encode(json.as_bytes())
        )
    }

    #[test]
    fn a_ticket_survives_a_link_and_holds_no_secret_key() {
        let ticket = sample();
        let link = ticket.link();
        assert!(
            link.starts_with(&format!("{LINK_SCHEME}://")) && link.len() < 600,
            "{} chars",
            link.len()
        );
        let back = Ticket::parse(&link).unwrap();
        assert_eq!(back, ticket);
        assert_eq!(back.endpoint_id().unwrap().to_string(), ticket.id);
        assert!(
            Ticket::parse(&format!("  {link}\n")).is_ok(),
            "whitespace around it is fine"
        );
    }

    #[test]
    fn the_scheme_is_matched_without_regard_to_case_and_the_old_prefix_still_reads() {
        let ticket = sample();
        let link = ticket.link();
        let shouted = link.replacen(LINK_SCHEME, &LINK_SCHEME.to_uppercase(), 1);
        assert_eq!(Ticket::parse(&shouted).unwrap(), ticket);
        let body = link.rsplit('/').next().unwrap();
        assert_eq!(
            Ticket::parse(&format!("{FALLBACK_PREFIX}{body}")).unwrap(),
            ticket
        );
        assert!(
            Ticket::parse(&format!("{FALLBACK_PREFIX}{body}\n")).is_ok(),
            "a pasted line ends in a newline"
        );
    }

    #[test]
    fn a_ticket_from_the_spike_is_refused_as_older() {
        // The spike wrote layout 1: the host's long-term id and no purpose.
        let spike = format!(
            "{FALLBACK_PREFIX}{}",
            BASE64URL_NOPAD.encode(
                format!(
                    r#"{{"v":1,"id":"{}","code":"AAAAAAAAAAAAA","relays":["https://relay.example.com"]}}"#,
                    id()
                )
                .as_bytes()
            )
        );
        let err = Ticket::parse(&spike).unwrap_err().to_string();
        assert!(err.contains("older version"), "{err}");
        let newer = with_body(&format!(r#"{{"v":9,"id":"{}"}}"#, id()));
        let err = Ticket::parse(&newer).unwrap_err().to_string();
        assert!(err.contains("newer version"), "{err}");
    }

    #[test]
    fn what_is_not_a_ticket_is_refused_without_a_panic() {
        let good = sample().link();
        let ok_id = id();
        for bad in [
            String::new(),
            format!("{LINK_SCHEME}://ticket/"),
            format!("{LINK_SCHEME}://other/AAAA"),
            "https://example.com/ticket/AAAA".into(),
            good.replace(LINK_SCHEME, "other"),
            format!("{good}!"),
            format!("{good}{}", "A".repeat(MAX_LINK_CHARS)),
            with_body("not json"),
            with_body("{}"),
            with_body(r#"{"v":3,"k":"pair","id":"nope","code":"AAAAAAAAAAAAA","proto":[3]}"#),
            with_body(&format!(
                r#"{{"v":3,"k":"pair","id":"{ok_id}","code":"A","proto":[3]}}"#
            )),
            with_body(&format!(
                r#"{{"v":3,"k":"pair","id":"{ok_id}","code":"AAAAAAAAAAAAA","proto":[]}}"#
            )),
            with_body(&format!(
                r#"{{"v":3,"k":"pair","id":"{ok_id}","code":"AAAAAAAAAAAAA","proto":[3],"addrs":["not an address"]}}"#
            )),
            with_body(&format!(
                r#"{{"v":3,"k":"pair","id":"{ok_id}","code":"AAAAAAAAAAAAA","proto":[3],"relays":["nope"]}}"#
            )),
            with_body(&format!(
                r#"{{"v":3,"k":"other","id":"{ok_id}","code":"AAAAAAAAAAAAA","proto":[3]}}"#
            )),
        ] {
            assert!(Ticket::parse(&bad).is_err(), "{bad}");
        }
        let many: Vec<String> = (0..=MAX_LIST)
            .map(|n| format!("192.0.2.1:{}", 7000 + n))
            .collect();
        let crowded = Ticket {
            addrs: many,
            ..sample()
        };
        assert!(
            Ticket::parse(&crowded.link()).is_err(),
            "too many addresses"
        );
    }

    #[test]
    fn the_newest_common_version_wins_and_the_error_says_which_side_is_old() {
        assert_eq!(negotiate(&[1, 2, 3], &[3, 2]).unwrap(), 3);
        assert_eq!(negotiate(&[2], &[3, 2]).unwrap(), 2);
        let newer = negotiate(&[9], &[3]).unwrap_err().to_string();
        assert!(
            newer.contains("newer") && newer.contains("update HQ here"),
            "{newer}"
        );
        let older = negotiate(&[1], &[3]).unwrap_err().to_string();
        assert!(
            older.contains("older") && older.contains("update HQ there"),
            "{older}"
        );
        assert!(negotiate(&[], &[3]).is_err());
        let future = Ticket {
            proto: vec![42],
            ..sample()
        };
        assert!(future.version().is_err());
        assert_eq!(sample().version().unwrap(), WIRE_VERSIONS[0]);
    }

    #[test]
    fn protocol_names_carry_the_purpose_and_the_version() {
        for purpose in [Purpose::Pair, Purpose::Register] {
            for version in [3, 4] {
                assert_eq!(
                    Purpose::from_alpn(&purpose.alpn(version)),
                    Some((purpose, version))
                );
            }
        }
        assert_ne!(Purpose::Pair.alpn(3), Purpose::Register.alpn(3));
        for bad in [
            &b"hq/pair"[..],
            b"hq/pair/x",
            b"hq/echo/3",
            b"x/pair/3",
            b"hq/pair/3/4",
            b"",
        ] {
            assert_eq!(Purpose::from_alpn(bad), None);
        }
    }
}
