//! Wire constants, the signed message, and bounded line reading.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::net::TcpStream;
use std::time::{Duration, Instant};

pub const MAGIC: &str = "HQGATE1";
/// Same bound as `hq_host::MAX_GATE_INPUT`, the ssh gate's request limit.
pub const MAX_REQUEST_BYTES: usize = 1_000_000;
pub const MAX_REPLY_BYTES: usize = 2_000_000;
pub(crate) const NONCE_BYTES: usize = 32;
pub(crate) const PUBLIC_KEY_BYTES: usize = 32;
pub(crate) const SIGNATURE_BYTES: usize = 64;
/// Two base64url fields, a space and slack: nothing legitimate is longer.
pub(crate) const AUTH_LINE_MAX: usize = 200;
pub(crate) const HELLO_LINE_MAX: usize = 64;
const SIGN_CONTEXT: &[u8] = b"hq-gate-v1\0";
const READ_CHUNK: usize = 4096;

#[derive(Debug, thiserror::Error)]
pub enum GateError {
    #[error("{0}")]
    Config(String),
    #[error("gate i/o: {0}")]
    Io(#[from] std::io::Error),
    #[error("the gate did not answer in time")]
    Timeout,
    #[error("the gate refused or dropped the request: {0}")]
    Refused(String),
    #[error("gate protocol error: {0}")]
    Protocol(String),
}

pub(crate) fn encode(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Decodes base64url into exactly `N` bytes, or `None`.
pub(crate) fn decode_exact<const N: usize>(text: &str) -> Option<[u8; N]> {
    let bytes = URL_SAFE_NO_PAD.decode(text).ok()?;
    <[u8; N]>::try_from(bytes).ok()
}

/// What the client signs: the context, the connection's nonce, and the request's digest, so a
/// signature is good for one request on one connection.
pub(crate) fn signed_message(nonce: &[u8; NONCE_BYTES], request_line: &str) -> Vec<u8> {
    let mut message = Vec::with_capacity(SIGN_CONTEXT.len() + NONCE_BYTES + 32);
    message.extend_from_slice(SIGN_CONTEXT);
    message.extend_from_slice(nonce);
    message.extend_from_slice(&Sha256::digest(request_line.as_bytes()));
    message
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ReadError {
    Timeout,
    TooLong,
    Closed,
    /// The peer reset the connection: what closing a socket with unread input looks like.
    Reset,
    Io(String),
}

/// Reads `\n`-terminated lines from a socket, each bounded in length, all against a deadline. It
/// keeps what a read brought beyond the first line for the next call.
pub(crate) struct Lines {
    stream: TcpStream,
    buf: Vec<u8>,
}

impl Lines {
    pub(crate) fn new(stream: TcpStream) -> Self {
        Self {
            stream,
            buf: Vec::new(),
        }
    }

    pub(crate) fn next_line(&mut self, max: usize, deadline: Instant) -> Result<String, ReadError> {
        loop {
            if let Some(line) = self.take_line(max)? {
                return Ok(line);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(ReadError::Timeout);
            }
            self.stream
                .set_read_timeout(Some(remaining))
                .map_err(|e| ReadError::Io(e.to_string()))?;
            let mut chunk = [0u8; READ_CHUNK];
            match self.stream.read(&mut chunk) {
                Ok(0) => return Err(ReadError::Closed),
                Ok(n) => self.buf.extend_from_slice(&chunk[..n]),
                Err(e) if is_timeout(&e) => return Err(ReadError::Timeout),
                Err(e) if is_reset(&e) => return Err(ReadError::Reset),
                Err(e) => return Err(ReadError::Io(e.to_string())),
            }
        }
    }

    /// A complete line within `max` bytes, `None` if more input is needed, an error if the line
    /// is already longer than `max`.
    fn take_line(&mut self, max: usize) -> Result<Option<String>, ReadError> {
        let newline = self.buf.iter().position(|&b| b == b'\n');
        match newline {
            Some(at) if at > max => Err(ReadError::TooLong),
            Some(at) => {
                let rest = self.buf.split_off(at + 1);
                let mut line = std::mem::replace(&mut self.buf, rest);
                line.pop();
                String::from_utf8(line)
                    .map(Some)
                    .map_err(|_| ReadError::Io("a line was not valid text".into()))
            }
            None if self.buf.len() > max => Err(ReadError::TooLong),
            None => Ok(None),
        }
    }
}

/// A connection the peer tore down, on read or on write.
pub(crate) fn is_reset(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::BrokenPipe
    )
}

fn is_timeout(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    )
}

pub(crate) fn remaining(deadline: Instant) -> Duration {
    deadline.saturating_duration_since(Instant::now())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_signed_message_depends_on_the_nonce_and_the_request() {
        let nonce = [7u8; NONCE_BYTES];
        let a = signed_message(&nonce, "request one");
        assert_ne!(a, signed_message(&nonce, "request two"));
        assert_ne!(a, signed_message(&[8u8; NONCE_BYTES], "request one"));
        assert!(a.starts_with(b"hq-gate-v1\0"));
    }

    #[test]
    fn base64_must_decode_to_exactly_the_expected_length() {
        let text = encode(&[1u8; 32]);
        assert_eq!(decode_exact::<32>(&text), Some([1u8; 32]));
        assert_eq!(decode_exact::<32>(&encode(&[1u8; 31])), None);
        assert_eq!(decode_exact::<32>("not base64!"), None);
        assert_eq!(decode_exact::<32>(&format!("{text}=")), None);
    }
}
