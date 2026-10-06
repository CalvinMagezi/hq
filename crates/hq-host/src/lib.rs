//! Built-in host for long-lived coding agents: it runs each agent in a
//! pseudo-terminal, keeps an emulated screen and scrollback for reading, and
//! accepts typed input. See docs/provenance/herdr.md for how work adapted from
//! herdr is recorded.

mod client;
mod detect;
mod emu;
mod env;
mod error;
mod host;
mod keys;
mod pane;
mod proto;
mod server;
mod state;
mod token;

pub use client::{Client, ClientError};
pub use detect::{AgentState, Detection, Detector, ENGINE_VERSION, Input as DetectInput};
pub use emu::{Emulator, Row, VtEmulator};
pub use env::pane_env;
pub use error::HostError;
pub use host::{
    AwaitingInfo, Host, PaneInfo, PaneStatus, ReadSource, RestoreReport, SpawnSpec, valid_name,
};
pub use keys::encode_key;
pub use proto::{MAX_LINE_BYTES, PROTOCOL_VERSION, Request, Response};
pub use server::{Limits, Server, StopHandle, socket_path};
pub use token::{load_or_create as load_or_create_token, token_path};
