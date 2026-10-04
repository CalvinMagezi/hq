//! The unified child-execution runtime.
//!
//! [`AgentService`] is the single governed API for running child agents, with
//! four execution modes ([`ChildMode`]). The [`SpawnSubagentsTool`] exposes it
//! to models as the one native delegation tool.

mod child_context;
mod ledger;
pub mod model_select;
mod modes;
pub mod service;
pub mod tool;
pub mod types;

pub use model_select::{EffectiveModel, ModelSource, OnModelUnavailable};
pub use service::{AgentService, INPROCESS_BACKEND};
pub use tool::{ChildRunSummary, ReportProgressTool, SpawnSubagentsTool};
pub use types::{
    ChildCompletionEvent, ChildExecContext, ChildMode, ChildOutcome, ChildPlan, ChildRequest,
    ChildStatus, CompletionSink, EnvelopeSink, RunInfo,
};

#[cfg(test)]
mod tests;

#[cfg(test)]
mod model_select_tests;

#[cfg(test)]
mod supervision_tests;
