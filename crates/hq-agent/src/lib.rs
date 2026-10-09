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
pub mod budget_gate;
pub(crate) mod outcome_sink;
pub use outcome_sink::dropped_outcomes;
pub mod session;
pub mod session_presets;
pub mod shutdown;
pub mod skill_review;
pub(crate) mod subagent;
pub mod threads;
pub(crate) mod tool_policy;
pub(crate) mod tools;
pub(crate) mod web;

/// Record every LLM call to `db` and enforce the configured budgets, process-wide. Safe to call
/// more than once; the latest database wins.
pub fn install_ledger(db: std::sync::Arc<hq_db::Database>) {
    let instruments = hq_llm::Instruments::global();
    instruments.set_sink(outcome_sink::DbOutcomeSink::new(db.clone()));
    instruments.set_gate(budget_gate::LedgerBudgetGate::new(
        db,
        std::sync::Arc::new(|| {
            hq_core::config::HqConfig::load().ok().map(|c| c.budgets)
        }),
    ));
}
