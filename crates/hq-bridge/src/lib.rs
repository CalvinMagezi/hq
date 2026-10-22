//! Native Bridge pairing: a one-time ticket shown as a QR code, six digits typed on the machine
//! that shows it, then reconnecting by key alone over iroh. The protocol and its attack notes are
//! in `docs/security/PAIRING.md`; how to use it is in `docs/PAIRING.md`.
//!
//! Nothing here starts on its own. A caller builds a `Bridge`, and a listener exists only between
//! `Bridge::start` with a handler (the long-term service) and `Bridge::open_ticket` (one
//! throwaway endpoint per ticket, closed when the ticket is spent, expired or burned).

pub mod bridge;
pub mod code;
pub mod host;
pub mod join;
pub mod key;
pub mod limits;
pub mod qr;
pub mod ticket;
pub mod wire;

pub use bridge::{
    Bridge, BridgeConfig, PeerStream, SERVICE_ALPN, StreamHandler, echo, echo_handler,
};
pub use host::{Joiner, OpenTicket, PairRequest, PairingSpec, Registrar, Tuning};
pub use join::{JoinOptions, JoinResult, Joined};
pub use ticket::{LINK_SCHEME, Purpose, Ticket};
