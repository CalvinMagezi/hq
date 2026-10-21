//! The WireGuard session with the hub: boringtun's `Tunn` behind a few owned-buffer methods, so
//! the rest of the crate never holds a borrow into its scratch space.

use crate::secret::Secret32;
use boringtun::noise::errors::WireGuardError;
use boringtun::noise::{Tunn, TunnResult};
use boringtun::x25519::{PublicKey, StaticSecret};
use std::net::IpAddr;
use std::time::Duration;
use zeroize::Zeroize;

/// A WireGuard packet is an IP packet plus a 32 byte header and a tag; this leaves room.
const SCRATCH_BYTES: usize = 4096;

/// What one datagram from the hub produced.
#[derive(Default)]
pub struct Decapsulated {
    /// Datagrams to send back to the hub (a handshake response, a keepalive).
    pub replies: Vec<Vec<u8>>,
    /// IPv4 packets for the stack.
    pub packets: Vec<Vec<u8>>,
    /// Datagrams the session refused. The reason is kept for the event log, never the bytes.
    pub refused: Vec<String>,
}

pub struct Session {
    tunn: Tunn,
    scratch: Vec<u8>,
}

impl Session {
    pub fn new(
        private: &Secret32,
        hub_key: [u8; 32],
        preshared: Option<&Secret32>,
        keepalive: Option<u16>,
    ) -> Self {
        let mut bytes = private.bytes();
        let secret = StaticSecret::from(bytes);
        bytes.zeroize();
        Self {
            tunn: Tunn::new(
                secret,
                PublicKey::from(hub_key),
                preshared.map(Secret32::bytes),
                keepalive,
                0,
                None,
            ),
            scratch: vec![0; SCRATCH_BYTES],
        }
    }

    /// A handshake initiation. `force` sends one even if one went out a moment ago, which is what a
    /// change of network wants.
    pub fn handshake(&mut self, force: bool) -> Option<Vec<u8>> {
        match self
            .tunn
            .format_handshake_initiation(&mut self.scratch, force)
        {
            TunnResult::WriteToNetwork(b) => Some(b.to_vec()),
            _ => None,
        }
    }

    /// An IP packet for the hub, as the datagram to send, or `None` while the session is not up
    /// (boringtun queues the packet and starts a handshake itself).
    pub fn encapsulate(&mut self, ip_packet: &[u8]) -> Option<Vec<u8>> {
        match self.tunn.encapsulate(ip_packet, &mut self.scratch) {
            TunnResult::WriteToNetwork(b) => Some(b.to_vec()),
            _ => None,
        }
    }

    pub fn decapsulate(&mut self, source: Option<IpAddr>, datagram: &[u8], out: &mut Decapsulated) {
        let mut input = datagram;
        let mut source = source;
        loop {
            match self.tunn.decapsulate(source, input, &mut self.scratch) {
                TunnResult::WriteToNetwork(b) => {
                    out.replies.push(b.to_vec());
                    // Calling again with nothing drains whatever the session queued.
                    input = &[];
                    source = None;
                }
                TunnResult::WriteToTunnelV4(p, _) => {
                    out.packets.push(p.to_vec());
                    return;
                }
                TunnResult::Err(e) => {
                    out.refused.push(format!("{e:?}"));
                    return;
                }
                TunnResult::Done | TunnResult::WriteToTunnelV6(..) => return,
            }
        }
    }

    /// Runs the protocol's timers and returns the datagram they want sent, if any. When the
    /// session has given up (no answer to its initiations for 90 s, or no session for 540 s)
    /// boringtun stays expired until someone asks for a new handshake, so that is done here;
    /// asking again within five seconds of the last try is a no-op, so this cannot flood.
    pub fn update_timers(&mut self) -> Option<Vec<u8>> {
        match self.tunn.update_timers(&mut self.scratch) {
            TunnResult::WriteToNetwork(b) => Some(b.to_vec()),
            TunnResult::Err(WireGuardError::ConnectionExpired) => self.handshake(false),
            _ => None,
        }
    }

    /// How long ago the last handshake completed, or `None` if none has.
    pub fn handshake_age(&self) -> Option<Duration> {
        self.tunn.stats().0
    }

    pub fn is_established(&self) -> bool {
        self.handshake_age().is_some()
    }
}
