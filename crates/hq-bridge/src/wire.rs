//! What the two machines say to each other during pairing: one JSON object per line on a single
//! stream, in this order. The joiner speaks first.
//!
//! ```text
//! joiner  Hello   the protocol version, the ticket's code, a commitment to a nonce
//! host    Reply   a refusal with a reason, or its own nonce and a binding of its long-term id
//! joiner  Reveal  the nonce it committed to and a name for the person to read
//! host    Final   the answer, and on acceptance the host's identity card and the key that opens
//!                 its binding
//! joiner  Details how to dial the joiner back, sent only once it has checked the card
//! ```

use crate::code::{BIND_KEY_BYTES, BindKey, Binding, COMMIT_BYTES, NONCE_BYTES, Nonce, printable};
use anyhow::{Result, anyhow, bail};
use data_encoding::BASE32_NOPAD;
use iroh::{
    EndpointId, RelayUrl,
    endpoint::{RecvStream, SendStream},
};
use serde::{Deserialize, Serialize};

/// The longest line either side will read.
pub const MAX_LINE_BYTES: usize = 4_096;
/// The longest device name that is shown to a person.
pub const MAX_NAME_CHARS: usize = 40;
/// Relays and addresses kept from the other machine, and the longest one.
pub const MAX_DIAL_ITEMS: usize = 4;
pub const MAX_DIAL_ITEM_CHARS: usize = 300;

#[derive(Debug, Serialize, Deserialize)]
pub struct Hello {
    pub v: u16,
    pub code: String,
    pub commit: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Reply {
    pub ok: bool,
    #[serde(default)]
    pub why: String,
    #[serde(default)]
    pub nonce: String,
    #[serde(default)]
    pub bind: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Reveal {
    pub nonce: String,
    #[serde(default)]
    pub name: String,
}

/// How to dial the joiner back. Sent after the host accepted, so a host that was never going to
/// accept (a rogue QR code, say) learns no address of the joiner.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Details {
    #[serde(default)]
    pub relays: Vec<String>,
    #[serde(default)]
    pub addrs: Vec<String>,
}

/// Who a machine is and how to dial it. Sent by the host only after the digits matched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Card {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub relays: Vec<String>,
    #[serde(default)]
    pub addrs: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Accepted,
    Declined,
    Timeout,
    Gone,
    Error,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Final {
    pub result: Outcome,
    #[serde(default)]
    pub card: Option<Card>,
    /// The secret the binding was made with, released with the card.
    #[serde(default)]
    pub bind_key: String,
    /// What the registrar of a purpose has to tell the joiner (an `hq edge` says which name it
    /// gave the device). Not used by pairing two devices.
    #[serde(default)]
    pub info: Option<serde_json::Value>,
}

pub fn b32(bytes: &[u8]) -> String {
    BASE32_NOPAD.encode(bytes)
}

pub fn unb32(text: &str) -> Vec<u8> {
    BASE32_NOPAD.decode(text.as_bytes()).unwrap_or_default()
}

pub fn nonce_from(text: &str) -> Option<Nonce> {
    unb32(text).try_into().ok()
}

pub fn commitment_from(text: &str) -> Option<[u8; COMMIT_BYTES]> {
    unb32(text).try_into().ok()
}

pub fn binding_from(text: &str) -> Option<Binding> {
    unb32(text).try_into().ok()
}

pub fn bind_key_from(text: &str) -> Option<BindKey> {
    unb32(text).try_into().ok()
}

const _: () = assert!(NONCE_BYTES == 16 && BIND_KEY_BYTES == 16);

/// Keeps the entries of a list from the other machine that parse, a few of them, each short.
pub fn clean_list(items: Vec<String>, valid: impl Fn(&str) -> bool) -> Vec<String> {
    items
        .into_iter()
        .filter(|s| s.len() <= MAX_DIAL_ITEM_CHARS && valid(s))
        .take(MAX_DIAL_ITEMS)
        .collect()
}

pub fn clean_relays(items: Vec<String>) -> Vec<String> {
    clean_list(items, |s| s.parse::<RelayUrl>().is_ok())
}

pub fn clean_addrs(items: Vec<String>) -> Vec<String> {
    clean_list(items, |s| s.parse::<std::net::SocketAddr>().is_ok())
}

/// A card from the other machine made safe to store: the id in the one spelling the gate compares
/// (iroh reads hex and base32, and prints hex), a printable name, and clean dial-back lists.
pub fn normalize_card(card: Card) -> Result<Card> {
    let id: EndpointId = card
        .id
        .parse()
        .map_err(|_| anyhow!("the card's id is not a key"))?;
    Ok(Card {
        id: id.to_string(),
        name: printable(&card.name, MAX_NAME_CHARS),
        relays: clean_relays(card.relays),
        addrs: clean_addrs(card.addrs),
    })
}

pub async fn read_line(recv: &mut RecvStream) -> Result<String> {
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        if line.len() > MAX_LINE_BYTES {
            bail!("a line longer than {MAX_LINE_BYTES} bytes");
        }
        let n = recv.read(&mut byte).await?.unwrap_or(0);
        if n == 0 || byte[0] == b'\n' {
            return Ok(String::from_utf8(line)?);
        }
        line.push(byte[0]);
    }
}

pub async fn send_line<T: Serialize>(send: &mut SendStream, value: &T) -> Result<()> {
    let mut text = serde_json::to_string(value)?;
    text.push('\n');
    send.write_all(text.as_bytes()).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_outcome_reads_as_unknown_not_as_accepted() {
        let last: Final = serde_json::from_str(r#"{"result":"something-new"}"#).unwrap();
        assert_eq!(last.result, Outcome::Unknown);
        let last: Final = serde_json::from_str(r#"{"result":"accepted"}"#).unwrap();
        assert_eq!(last.result, Outcome::Accepted);
    }

    #[test]
    fn binary_fields_round_trip_and_wrong_sizes_are_refused() {
        let nonce = [7u8; NONCE_BYTES];
        assert_eq!(nonce_from(&b32(&nonce)), Some(nonce));
        assert_eq!(nonce_from(&b32(&nonce[..8])), None);
        assert_eq!(nonce_from("!!!"), None);
        assert_eq!(
            commitment_from(&b32(&[1u8; COMMIT_BYTES])),
            Some([1u8; COMMIT_BYTES])
        );
        assert_eq!(binding_from(""), None);
        assert_eq!(
            bind_key_from(&b32(&[3u8; BIND_KEY_BYTES])),
            Some([3u8; BIND_KEY_BYTES])
        );
    }

    #[test]
    fn a_card_is_stored_in_one_spelling_with_a_clean_name_and_lists() {
        let id = iroh::SecretKey::generate().public();
        let base32 = data_encoding::BASE32_NOPAD
            .encode(id.as_bytes())
            .to_lowercase();
        assert_ne!(base32, id.to_string(), "the test needs the other spelling");
        let many: Vec<String> = (0..10).map(|n| format!("192.0.2.1:{}", 7000 + n)).collect();
        let card = Card {
            id: base32,
            name: "\u{1b}[2Jdesk\u{202E}".into(),
            relays: vec!["https://relay.example.com".into(), "not a url".into()],
            addrs: many,
        };
        let card = normalize_card(card).unwrap();
        assert_eq!(card.id, id.to_string());
        assert_eq!(card.name, "?[2Jdesk?");
        assert_eq!(card.relays, ["https://relay.example.com"]);
        assert_eq!(card.addrs.len(), MAX_DIAL_ITEMS);
        let bad = Card {
            id: "nope".into(),
            name: String::new(),
            relays: vec![],
            addrs: vec![],
        };
        assert!(normalize_card(bad).is_err());
    }
}
