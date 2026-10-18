//! The host gate over TCP, for a private tunnel address.
//!
//! `hq host gate` lets a server call a laptop's host API through ssh. This crate carries the same
//! one-request exchange over a TCP connection on a WireGuard address, with no ssh and no shared
//! secret. The protocol (version 1), all ASCII lines:
//!
//! 1. Server to client, at once: `HQGATE1 <nonce>`. The nonce is 32 random bytes, base64url without
//!    padding, new for every connection.
//! 2. Client to server: `<public-key> <signature>`. The signature is Ed25519 over
//!    `"hq-gate-v1\0" || nonce || SHA-256(request line)`.
//! 3. Client to server: the request line, which the handler alone understands.
//! 4. Server to client: one reply line, then the connection closes.
//!
//! The server checks the source address, then that the public key is in its authorized keys file,
//! then reads the request, then verifies the signature. The handler sees nothing until the
//! signature is valid. A refused caller gets no reply at all, only a closed connection.

#![cfg(unix)]

mod client;
mod keys;
mod proto;
mod server;

pub use client::call;
pub use keys::{AuthorizedKey, ClientKey, append_authorized, load_authorized};
pub use proto::{GateError, MAGIC, MAX_REPLY_BYTES, MAX_REQUEST_BYTES};
pub use server::{Handler, Limits, Listener, ServerConfig, ShutdownHandle};
