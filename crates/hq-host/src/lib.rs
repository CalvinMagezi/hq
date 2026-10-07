//! Built-in host for long-lived coding agents: it runs each agent in a
//! pseudo-terminal, keeps an emulated screen and scrollback for reading, and
//! accepts typed input. See docs/provenance/herdr.md for how work adapted from
//! herdr is recorded.

mod client;
mod detect;
mod emu;
mod env;
mod error;
mod events;
mod gate;
mod host;
mod hooks;
mod keys;
mod pane;
mod proto;
mod report;
mod server;
mod state;
mod token;

pub use client::{Client, ClientError};
pub use detect::{AgentState, Detection, Detector, ENGINE_VERSION, Input as DetectInput};
pub use emu::{Emulator, Row, VtEmulator};
pub use env::pane_env;
pub use error::HostError;
pub use events::{Event, EventKind, EventLog, Poll};
pub use host::{
    AwaitingInfo, Host, PANE_TOKEN_ENV, RUN_DIR_ENV, PaneInfo, PaneStatus, ReadSource, RestoreReport, SpawnSpec, valid_name,
};
pub use gate::{Denied, GATE_DENIED_EXIT, MAX_GATE_INPUT, forward as gate_forward, parse_request as gate_parse_request};
pub use hooks::{claude_settings, report_params, write_claude_settings};
pub use keys::encode_key;
pub use report::{STALE_WORKING_AFTER, combine as combine_state, state_for};
pub use proto::{MAX_LINE_BYTES, PROTOCOL_VERSION, Request, Response};
pub use server::{Limits, Server, StopHandle, socket_path};
pub use token::{load_or_create as load_or_create_token, token_path};
