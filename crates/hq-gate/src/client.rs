//! The calling side: connect, answer the challenge, send one request, read one reply.

use crate::keys::ClientKey;
use crate::proto::{
    GateError, HELLO_LINE_MAX, Lines, MAGIC, MAX_REPLY_BYTES, MAX_REQUEST_BYTES, NONCE_BYTES,
    ReadError, decode_exact, encode, is_reset, remaining, signed_message,
};
use std::io::Write;
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

const CONNECT_CAP: Duration = Duration::from_secs(5);

/// One request through the gate at `addr`, signed with `key`, answered within `timeout` in total.
/// `request` is a single line (no newline) of at most `MAX_REQUEST_BYTES`.
pub fn call(
    addr: SocketAddr,
    key: &ClientKey,
    request: &str,
    timeout: Duration,
) -> Result<String, GateError> {
    if request.contains('\n') || request.len() > MAX_REQUEST_BYTES {
        return Err(GateError::Protocol(
            "the request must be one line within the size limit".into(),
        ));
    }
    let deadline = Instant::now() + timeout;
    let connect_within = remaining(deadline).min(CONNECT_CAP);
    if connect_within.is_zero() {
        return Err(GateError::Timeout);
    }
    let mut stream = TcpStream::connect_timeout(&addr, connect_within).map_err(connect_error)?;
    stream.set_nodelay(true)?;
    stream.set_write_timeout(Some(remaining(deadline).max(Duration::from_millis(1))))?;
    let mut lines = Lines::new(stream.try_clone()?);
    let nonce = read_challenge(&mut lines, deadline)?;
    let signature = key.sign(&signed_message(&nonce, request));
    let auth = format!("{} {}\n", key.public_key(), encode(&signature.to_bytes()));
    let sent = stream
        .write_all(auth.as_bytes())
        .and_then(|()| stream.write_all(request.as_bytes()))
        .and_then(|()| stream.write_all(b"\n"));
    if let Err(e) = sent {
        return Err(if is_reset(&e) { refused() } else { e.into() });
    }
    lines
        .next_line(MAX_REPLY_BYTES, deadline)
        .map_err(|e| reply_error(&e))
}

fn read_challenge(lines: &mut Lines, deadline: Instant) -> Result<[u8; NONCE_BYTES], GateError> {
    let hello = lines
        .next_line(HELLO_LINE_MAX, deadline)
        .map_err(|e| reply_error(&e))?;
    let nonce = hello
        .strip_prefix(MAGIC)
        .and_then(|rest| rest.strip_prefix(' '))
        .and_then(decode_exact::<NONCE_BYTES>)
        .ok_or_else(|| GateError::Protocol("that is not an HQ gate".into()))?;
    Ok(nonce)
}

fn connect_error(e: std::io::Error) -> GateError {
    match e.kind() {
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => GateError::Timeout,
        _ => GateError::Io(e),
    }
}

/// A server that refuses a caller closes without a word, so a close before any reply is a refusal.
fn reply_error(e: &ReadError) -> GateError {
    match e {
        ReadError::Timeout => GateError::Timeout,
        ReadError::Closed | ReadError::Reset => refused(),
        ReadError::TooLong => GateError::Protocol("the reply was too long".into()),
        ReadError::Io(why) => GateError::Protocol(why.clone()),
    }
}

fn refused() -> GateError {
    GateError::Refused(
        "the connection closed with no reply (the key is not authorized, or this address is not allowed)"
            .into(),
    )
}
