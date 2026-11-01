//! `github_write:` — the repositories the `github_write` tool may push branches to.

use serde::{Deserialize, Serialize};

/// Off by default: with no repositories listed the tool refuses every call. The list is the
/// owner's alone; `config_manage` refuses this key and agents cannot write the config file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubWriteConfig {
    /// Repositories as `owner/name`.
    #[serde(default)]
    pub repos: Vec<String>,
}
