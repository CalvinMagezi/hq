use serde::{Deserialize, Serialize};

/// GitHub Copilot CLI (`gh copilot`) headless harness configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubCopilotConfig {
    /// Override AI model (e.g. `"claude-sonnet-4.6"`, `"gpt-5.2"`). None = Copilot default.
    #[serde(default)]
    pub model: Option<String>,

    /// Pass `--allow-all` to grant all tool permissions. Use only in isolated envs.
    #[serde(default)]
    pub allow_all_tools: bool,

    /// Working directory for the gh process. Defaults to vault parent (repo root).
    #[serde(default)]
    pub cwd: Option<String>,

    /// Headless run timeout in seconds.
    #[serde(default = "default_github_copilot_timeout_secs")]
    pub timeout_secs: u64,
}

fn default_github_copilot_timeout_secs() -> u64 {
    120
}

impl Default for GitHubCopilotConfig {
    fn default() -> Self {
        Self {
            model: None,
            allow_all_tools: false,
            cwd: None,
            timeout_secs: default_github_copilot_timeout_secs(),
        }
    }
}

