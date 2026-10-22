//! What the two machines say to each other during pairing: one JSON object per line on a single
//! stream, in this order. The joiner speaks first.
//!
//! ```text
//! joiner  Hello   the protocol version, the ticket's code, a commitment to a nonce
//! host    Reply   a refusal with a reason, or its own nonce and a binding of its long-term id
//! joiner  Reveal  the nonce it committed to, a name for the person to read, how to dial it back
//! host    Final   the answer, and on acceptance the host's identity card
//! ```

use crate::code::{Binding, COMMIT_BYTES, NONCE_BYTES, Nonce};
use anyhow::{Result, bail};
use data_encoding::BASE32_NOPAD;
use iroh::endpoint::{RecvStream, SendStream};
use serde::{Deserialize, Serialize};

/// The longest line either side will read.
pub const MAX_LINE_BYTES: usize = 4_096;
/// The longest device name that is shown to a person.
pub const MAX_NAME_CHARS: usize = 40;

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

const _: () = assert!(NONCE_BYTES == 16);

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
    }
}
