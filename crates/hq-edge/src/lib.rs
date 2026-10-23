//! `hq edge`: a blind public forwarder.
//!
//! One TCP listener reads only the TLS server name of each connection, never decrypts, looks up
//! the registered device that owns the name, opens one iroh stream to it and copies bytes both
//! ways. The device ends TLS. An unknown name is closed without any answer.
//!
//! The device side contract (ALPN, name, framing) is [`EDGE_ALPN`] and [`device_label`]; the
//! security model is in `docs/security/EDGE.md`.

mod config;
mod hello;
mod identity;
#[cfg(feature = "invite")]
mod invite;
mod limits;
mod pipe;
mod registry;
mod relay;
mod server;

pub use config::{EdgeConfig, LimitsConfig, RelayHostConfig};
pub use hello::{ClientHello, HelloError, MAX_HELLO_BYTES, read_client_hello};
pub use identity::{edge_id, load_or_create_key};
#[cfg(feature = "invite")]
pub use invite::{EdgeRegistrar, InviteSpec, default_invite_spec, open_invite};
pub use limits::TokenBucket;
pub use pipe::{PipeEnd, PipeLimits, pipe};
pub use registry::{
    AllowlistFile, ListError, Registered, Registry, allow_device, device_label, list_devices,
    revoke_device, set_device_relay,
};
pub use relay::{RelayHost, RelayParams, RelayTls};
pub use server::{Edge, Stats, StatsSnapshot};

/// The protocol the edge speaks to a device. Each forwarded TCP connection is one bidirectional
/// stream carrying the client's raw TLS bytes from the first byte, with no framing of our own.
pub const EDGE_ALPN: &[u8] = b"hq/edge/1";

/// How many leading bytes of a device's endpoint id make its DNS label. The full 32-byte id is 64
/// hex characters, longer than the 63 a DNS label may have; 8 bytes give 16 characters and a
/// space too large to grind for a collision (and the registry refuses a collision outright).
pub const LABEL_BYTES: usize = 8;
