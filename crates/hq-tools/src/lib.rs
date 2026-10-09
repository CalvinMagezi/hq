//! Tool definitions and registry for Agent-HQ.
//!
//! Provides the `HqTool` trait, a `ToolRegistry`, and built-in tool
//! implementations for vault operations, skills, agents, Google Workspace,
//! image generation, TTS, DrawIt diagrams, benchmarking, webmail, planning,
//! browser automation, and workflow orchestration.

pub mod a2a;
pub mod agent_comm;
pub mod agents;
pub mod ask;
pub mod background_turns;
pub mod subagent_runs;

pub mod brand;
pub mod coding;
pub mod convert;
pub mod copilot_credits;
pub mod external_runner;
pub mod family_confirm;
pub mod family_guest;
pub mod file_edit;
#[cfg(feature = "gws")]
pub mod gws;
pub mod harness_chunk;
pub mod harness_session;
pub mod agent_host;
pub mod imagegen;
pub mod model_control;
pub mod prose_lint;
pub mod registry;
pub mod remote_mcp;
pub mod self_update;
pub mod session_search;
pub mod shortcuts;
pub mod skill_audit;
mod skill_bundle;
pub mod skill_edit;
pub mod skill_manage_tool;
pub mod skills;
pub mod slash_commands;
pub mod budget_status;
pub mod system_info;
pub mod usage_forecast;
pub mod task_classifier;
pub mod tasks;
pub mod util;
pub mod vault;
pub mod vault_reorg;
pub mod web;

pub use external_runner::run_external_cli_harness_strict;
pub use registry::{HqTool, ToolRegistry, ToolSummary};
