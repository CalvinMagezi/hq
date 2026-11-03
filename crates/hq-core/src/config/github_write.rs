//! `github_write:` — the repositories the `github_write` tool may push branches to.

use serde::{Deserialize, Serialize};

/// Off by default: with no repositories listed the tool refuses every call. The list is the
/// owner's alone; `config_manage` refuses this key and agents cannot write the config file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubWriteConfig {
    /// Repositories as `owner/name`.
    #[serde(default)]
    pub repos: Vec<String>,
    /// Let the daemon pick up open tasks tagged `edge-task` by itself and work them with the
    /// `edge-coding-worker` skill (`edge-worker-dispatch`). Off unless set.
    #[serde(default)]
    pub dispatch_tasks: bool,
    /// The backend (or the model one serves) that drives each dispatched task, for example a
    /// cheaper model than the chat's own chain. Unset, the chat's chain drives it.
    #[serde(default)]
    pub dispatch_model: Option<String>,
}
