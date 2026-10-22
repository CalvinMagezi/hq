//! A userspace WireGuard peer for a phone. It joins the plane `hq net` sets up without a VPN
//! service and without root: boringtun speaks the protocol, smoltcp terminates TCP, and two fixed
//! maps bridge the tunnel and loopback. A *serve* entry hands a port on this device's tunnel
//! address to a loopback port, for the tunnel addresses it names. A *reach* entry forwards a
//! loopback port to one tunnel destination. There is no general proxy and no route-anywhere mode.

pub mod config;
pub mod error;
mod fsutil;
pub mod join;
pub mod keyin;
mod packet;
pub mod pipe;
pub mod run;
pub mod secret;
pub mod session;
pub mod stack;
pub mod status;
pub mod store;
mod throttle;
mod timers;

pub use error::{Error, Result};
