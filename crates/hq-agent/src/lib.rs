//! Agent execution engine — session loop, tool registry, coding tools, governance, and sub-agent dispatch.

pub mod adversarial;
pub mod agents;
pub(crate) mod backend;
pub mod bash_policy;
pub mod bash_sandbox;
pub mod builder;
pub(crate) mod callable_agents;
pub(crate) mod coding;
pub mod context;
pub mod followup;
pub mod governance;
pub(crate) mod lsp;
pub(crate) mod lsp_tools;
pub(crate) mod middleware_runtime;
pub mod native_hq;
pub(crate) mod outcome_sink;
pub mod session;
pub mod session_presets;
pub mod shutdown;
pub mod skill_review;
pub(crate) mod subagent;
pub mod threads;
pub(crate) mod tool_policy;
pub(crate) mod tools;
pub(crate) mod web;
