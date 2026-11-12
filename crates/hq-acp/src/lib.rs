//! Agent Client Protocol client for coding agents. It turns an agent's JSON-RPC stream into the
//! facts the host needs (state, dialogs, finished turns, edited files, failures) so HQ does not
//! have to read terminal screens. Wire notes and per-agent differences: docs/plans/acp-lane.md.

mod client;
mod dialog;
mod tracker;
mod update;
mod wire;

pub use client::{
    Capabilities, ClientError, Connection, ModelChoice, ModelMenu, parse_capabilities,
    parse_model_menu,
};
pub use dialog::{Choice, Dialog, DialogOption, OptionKind, outcome, parse_dialog};
pub use tracker::{
    AgentState, AnswerError, FailureKind, Pending, StopReason, TokenUsage, Tracker, TurnEnd,
    failure_kind,
};
pub use update::{ToolCall, ToolKind, Update, parse_update, touched_paths};
pub use wire::{Incoming, RpcError, WireError, parse_line};
