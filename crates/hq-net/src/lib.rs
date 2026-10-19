//! `hq net`: a WireGuard private plane for HQ, and the strict-grammar program that applies its peers.
//!
//! Two halves with different trust:
//!
//! - The unprivileged half (`tools` feature, on by default): key generation, the peer store under
//!   `~/.hq/net`, the renderers for hub and device configs, the QR code. It never runs as root.
//! - The root half: `grammar` (the only language the applier reads), `wgconf` (what it hands to
//!   `wg`) and, on Linux only, `applier` (the file opener and the `wg` caller). A build with
//!   `--no-default-features` links these and nothing else.
//!
//! Everything that needs root is `cfg(target_os = "linux")`, never `cfg(unix)`, so a build for
//! Android or any other unix target does not contain it.

pub mod grammar;
pub mod key;
pub mod subnet;
pub mod wgconf;

#[cfg(target_os = "linux")]
pub mod applier;

#[cfg(all(feature = "tools", unix))]
pub mod device;
#[cfg(all(feature = "tools", unix))]
pub mod files;
#[cfg(all(feature = "tools", unix))]
pub mod init;
#[cfg(all(feature = "tools", unix))]
pub mod ops;
#[cfg(feature = "tools")]
pub mod qr;
#[cfg(all(feature = "tools", unix))]
pub mod status;
#[cfg(all(feature = "tools", unix))]
pub mod store;
