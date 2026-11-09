//! `github_write:` — the repositories the `github_write` tool may push branches to.

use serde::{Deserialize, Serialize};

/// With a connected GitHub App the repositories are the ones the person picked on GitHub, and
/// `repos` only narrows them. Without an App, an empty list refuses every call. The list is the
/// owner's alone; `config_manage` refuses this key and agents cannot write the config file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GithubWriteConfig {
    /// Repositories as `owner/name`.
    #[serde(default)]
    pub repos: Vec<String>,
    /// Let the daemon pick up open tasks tagged `edge-task` by itself and work them with the
    /// `edge-coding-worker` skill (`edge-worker-dispatch`). On by default: it does nothing until
    /// GitHub is connected or `repos` is listed.
    #[serde(default = "default_dispatch_tasks")]
    pub dispatch_tasks: bool,
    /// The backend (or the model one serves) that drives each dispatched task, for example a
    /// cheaper model than the chat's own chain. Unset, the chat's chain drives it.
    #[serde(default)]
    pub dispatch_model: Option<String>,
    /// The most one queued task may spend on models, in US dollars. A task that runs out is
    /// parked. It can only lower `budget.session_cap_usd`.
    #[serde(default = "default_task_budget_usd")]
    pub task_budget_usd: f64,
}

fn default_dispatch_tasks() -> bool {
    true
}

fn default_task_budget_usd() -> f64 {
    0.50
}

impl Default for GithubWriteConfig {
    fn default() -> Self {
        Self {
            repos: Vec::new(),
            dispatch_tasks: default_dispatch_tasks(),
            dispatch_model: None,
            task_budget_usd: default_task_budget_usd(),
        }
    }
}
