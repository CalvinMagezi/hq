//! Built-in host for long-lived coding agents: it runs each agent in a
//! pseudo-terminal, keeps an emulated screen and scrollback for reading, and
//! accepts typed input. See docs/provenance/herdr.md for how work adapted from
//! herdr is recorded.

// The host runs coding agents in terminals and talks over Unix sockets, so it exists on Unix
// only. On other platforms (HQ Lite on Windows) the crate keeps just the small pure helpers other
// crates call, and the rest is a plain "not available" at their call sites.
mod bypass;
mod names;
pub use bypass::{find_bypass, strip_bypass};
pub use names::check_folder_name;

#[cfg(unix)]
mod client;
#[cfg(unix)]
mod detect;
#[cfg(unix)]
mod egress;
#[cfg(unix)]
mod emu;
#[cfg(unix)]
mod env;
#[cfg(unix)]
mod error;
#[cfg(unix)]
mod events;
#[cfg(unix)]
mod gate;
#[cfg(unix)]
mod host;
#[cfg(unix)]
mod hooks;
#[cfg(unix)]
mod keys;
#[cfg(unix)]
mod pane;
#[cfg(unix)]
mod proto;
#[cfg(unix)]
mod relay;
#[cfg(unix)]
mod report;
#[cfg(unix)]
mod sandbox;
#[cfg(unix)]
mod server;
#[cfg(unix)]
mod state;
#[cfg(unix)]
mod token;
#[cfg(unix)]
mod workspace;

#[cfg(unix)]
pub use client::{Client, ClientError};
#[cfg(unix)]
pub use detect::{AgentState, Detection, Detector, ENGINE_VERSION, Input as DetectInput};
#[cfg(unix)]
pub use egress::{Decision, Egress, Rule, Verdict, decide as decide_egress, is_private};
#[cfg(unix)]
pub use emu::{Emulator, Row, VtEmulator};
#[cfg(unix)]
pub use env::pane_env;
#[cfg(unix)]
pub use error::HostError;
#[cfg(unix)]
pub use events::{Event, EventKind, EventLog, Poll};
#[cfg(unix)]
pub use host::{
    AwaitingInfo, Host, PANE_TOKEN_ENV, RUN_DIR_ENV, PaneInfo, PaneStatus, ReadSource, RestoreReport, SpawnSpec, valid_name,
};
#[cfg(unix)]
pub use gate::{Denied, GATE_DENIED_EXIT, MAX_GATE_INPUT, forward as gate_forward, parse_request as gate_parse_request};
#[cfg(unix)]
pub use hooks::{claude_mcp_config, claude_settings, write_claude_mcp_config, report_params, write_claude_settings};
#[cfg(unix)]
pub use keys::encode_key;
#[cfg(unix)]
pub use relay::{run_relay, sandbox_init};
#[cfg(unix)]
pub use report::{STALE_WORKING_AFTER, combine as combine_state, state_for};
#[cfg(unix)]
pub use proto::{MAX_LINE_BYTES, PROTOCOL_VERSION, Request, Response};
#[cfg(unix)]
pub use sandbox::{Allow, Mode as SandboxMode, SandboxSpec};
#[cfg(unix)]
pub use server::{Limits, Server, StopHandle, agent_socket_path, socket_path};
#[cfg(unix)]
pub use token::{load_or_create as load_or_create_token, token_path};
#[cfg(unix)]
pub use workspace::{describe as describe_workspace, ensure_workspace, list_dirs, make_dir, resolve_inside, workspace_root};
