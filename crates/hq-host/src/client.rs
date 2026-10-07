//! Client for the control socket, used by HQ's native backend and `hq host`.

use crate::proto::{MAX_LINE_BYTES, PROTOCOL_VERSION, Request, Response};
use crate::server::socket_path;
use crate::token;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

/// How long the handshake may take before the host is taken for wedged.
const HELLO_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("host unreachable: {0}")]
    Unreachable(String),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("{code}: {message}")]
    Remote { code: String, message: String },
}

impl ClientError {
    pub fn code(&self) -> Option<&str> {
        match self {
            ClientError::Remote { code, .. } => Some(code),
            _ => None,
        }
    }
}

pub struct Client {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    next_id: u64,
    /// Set when a reply could not be read whole; the stream is out of step.
    broken: bool,
}

impl Client {
    /// Connects to `<dir>/host.sock` with the token from `<dir>/operator.token`.
    pub fn connect(dir: &Path) -> Result<Self, ClientError> {
        let token = token::load_trusted(dir)
            .map_err(|e| ClientError::Unreachable(format!("operator token: {e}")))?;
        Self::connect_with_token(dir, &token)
    }

    pub fn connect_with_token(dir: &Path, token: &str) -> Result<Self, ClientError> {
        token::check_trusted_dir(dir)
            .map_err(|e| ClientError::Unreachable(format!("host directory: {e}")))?;
        let stream = UnixStream::connect(socket_path(dir))
            .map_err(|e| ClientError::Unreachable(e.to_string()))?;
        let writer = stream
            .try_clone()
            .map_err(|e| ClientError::Unreachable(e.to_string()))?;
        let mut client = Self {
            reader: BufReader::new(stream),
            writer,
            next_id: 1,
            broken: false,
        };
        client.set_timeout(Some(HELLO_TIMEOUT));
        client.call(
            "hello",
            json!({ "protocol_version": PROTOCOL_VERSION, "token": token }),
        )?;
        client.set_timeout(None);
        Ok(client)
    }

    pub fn set_timeout(&self, timeout: Option<Duration>) {
        let _ = self.writer.set_read_timeout(timeout);
    }

    pub fn call(&mut self, method: &str, params: Value) -> Result<Value, ClientError> {
        if self.broken {
            return Err(ClientError::Unreachable(
                "connection lost sync after an oversized reply; reconnect".into(),
            ));
        }
        let id = self.next_id;
        self.next_id += 1;
        let req = Request {
            id: json!(id),
            method: method.to_string(),
            params,
        };
        let mut line =
            serde_json::to_vec(&req).map_err(|e| ClientError::Protocol(e.to_string()))?;
        line.push(b'\n');
        self.writer
            .write_all(&line)
            .map_err(|e| ClientError::Unreachable(e.to_string()))?;
        let mut reply = Vec::new();
        let read = self
            .reader
            .by_ref()
            .take(MAX_LINE_BYTES as u64 + 1)
            .read_until(b'\n', &mut reply);
        match read {
            Ok(0) => {
                return Err(ClientError::Unreachable(
                    "host closed the connection".into(),
                ));
            }
            Err(e) => return Err(ClientError::Unreachable(e.to_string())),
            Ok(_) => {}
        }
        // A reply cut off at the limit leaves its tail in the stream; the next
        // call would read that tail as its answer, so refuse to continue.
        if reply.last() != Some(&b'\n') {
            self.broken = true;
            return Err(ClientError::Protocol(
                "reply longer than the line limit".into(),
            ));
        }
        let resp: Response =
            serde_json::from_slice(&reply).map_err(|e| ClientError::Protocol(e.to_string()))?;
        if resp.id != json!(id) {
            return Err(ClientError::Protocol(format!(
                "reply id {} does not match request {id}",
                resp.id
            )));
        }
        match (resp.result, resp.error) {
            (_, Some(e)) => Err(ClientError::Remote {
                code: e.code,
                message: e.message,
            }),
            (Some(v), None) => Ok(v),
            (None, None) => Err(ClientError::Protocol(
                "reply has neither result nor error".into(),
            )),
        }
    }
}
