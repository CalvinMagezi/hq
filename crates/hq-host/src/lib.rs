//! Built-in host for long-lived coding agents: it runs each agent in a
//! pseudo-terminal, keeps an emulated screen and scrollback for reading, and
//! accepts typed input. See docs/provenance/herdr.md for how work adapted from
//! herdr is recorded.

mod emu;
mod env;
mod error;
mod host;
mod keys;
mod pane;

pub use emu::{Emulator, Row, VtEmulator};
pub use env::pane_env;
pub use error::HostError;
pub use host::{Host, PaneInfo, PaneStatus, ReadSource, SpawnSpec, valid_name};
pub use keys::encode_key;
