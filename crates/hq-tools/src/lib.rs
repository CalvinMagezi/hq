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
pub mod lane_quotas;
pub mod subagent_runs;

pub mod agent_host;
pub mod approval_note;
pub mod brand;
pub mod budget_status;
pub mod calendar;
pub mod capabilities;
pub mod coding;
pub mod collection_tip;
pub mod contacts;
pub mod context_gateway;
pub mod convert;
pub mod copilot_credits;
pub mod deliver;
pub mod device_actions;
pub mod external_runner;
pub mod file_edit;
pub mod github;
#[cfg(feature = "gws")]
pub mod gws;
pub mod harness_chunk;
pub mod harness_session;
pub mod harness_usage;
pub mod harness_usage_collect;
pub mod helper_setup;
pub mod link_capture;
pub mod model_control;
pub mod note_index;
pub mod notify;
pub mod office;
pub mod prose_lint;
pub mod provenance;
pub mod self_update;
pub mod session_policy;
pub mod session_search;
pub mod shortcuts;
pub mod skill_audit;
mod skill_bundle;
pub mod skill_edit;
pub mod skill_manage_tool;
pub mod skill_model;
pub mod skills;
pub mod slash_commands;
pub mod spill;
pub mod system_info;
pub mod task_classifier;
pub mod task_placement;
pub mod tasks;
pub mod usage_forecast;
pub mod vault;
pub mod vault_reorg;

// These modules now live in `hq-tool-api` (they depend only on `hq-core`). Re-exported under the
// same names so `hq_tools::registry::...` and `crate::registry::...` keep resolving.
pub use hq_tool_api::{family_confirm, family_guest, registry, util};
// `web`, `remote_mcp` and `imagegen` now live in `hq-tools-web`; same names here.
pub use hq_tools_web::{imagegen, remote_mcp, web};

pub use external_runner::run_external_cli_harness_strict;
pub use registry::{HqTool, ToolRegistry, ToolSummary};
