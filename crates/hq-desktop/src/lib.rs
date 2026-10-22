//! The logic behind the desktop app, with no GUI dependency: the local host's lifecycle, device
//! pairing on top of `hq-bridge`, the Connect Claude backend and the loopback proxy. The Tauri
//! shell in `apps/desktop` only turns its calls into window commands.
//!
//! Nothing here starts on its own. Each listener (the proxy, the long-term pairing endpoint, one
//! throwaway endpoint per QR code) exists only after a caller asks for it; see
//! `docs/security/DESKTOP.md`.

pub mod connect;
pub mod desktop;
pub mod host;
pub mod pairing;
pub mod proxy;
pub mod qr;
pub mod settings;
pub mod window;

pub use desktop::{Desktop, DesktopInit};
