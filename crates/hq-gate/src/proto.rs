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
                Err(e) => {
                    if let Some(failure) = read_failure(&e) {
                        return Err(failure);
                    }
                }
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

/// What a failed read means, or `None` when the read should simply be tried again: a signal that
/// lands while a slow reply is awaited interrupts `read` with EINTR, and the reply still arrives.
fn read_failure(e: &std::io::Error) -> Option<ReadError> {
    if e.kind() == std::io::ErrorKind::Interrupted {
        return None;
    }
    if is_timeout(e) {
        return Some(ReadError::Timeout);
    }
    if is_reset(e) {
        return Some(ReadError::Reset);
    }
    Some(ReadError::Io(e.to_string()))
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
    fn an_interrupted_read_is_retried_and_other_failures_end_the_read() {
        use std::io::{Error, ErrorKind};
        assert_eq!(read_failure(&Error::from(ErrorKind::Interrupted)), None);
        assert_eq!(
            read_failure(&Error::from(ErrorKind::WouldBlock)),
            Some(ReadError::Timeout)
        );
        assert_eq!(
            read_failure(&Error::from(ErrorKind::ConnectionReset)),
            Some(ReadError::Reset)
        );
        assert!(matches!(
            read_failure(&Error::from(ErrorKind::PermissionDenied)),
            Some(ReadError::Io(_))
        ));
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
