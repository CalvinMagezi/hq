use serde::{Deserialize, Serialize};

use super::default_true;

// ─── LLM Provider Configuration ─────────────────────────────

/// A configured LLM provider (loaded from config.yaml).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    /// Provider name (used for logging and routing).
    pub name: String,
    /// OpenAI-compatible API base URL.
    pub api_base: String,
    /// Environment variable that holds the API key.
    pub api_key_env: String,
    /// Models available from this provider.
    #[serde(default)]
    pub models: Vec<String>,
    /// Tier: 0=local (free), 1=free cloud, 2=paid.
    #[serde(default)]
    pub tier: u8,
    /// Whether this provider is active.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// Budget configuration for LLM spending.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetConfig {
    /// Per-session spending cap in USD (prevents runaway sessions).
    #[serde(default = "default_session_cap")]
    pub session_cap_usd: f64,
}

fn default_session_cap() -> f64 {
    0.50
}

impl Default for BudgetConfig {
    fn default() -> Self {
        Self {
            session_cap_usd: default_session_cap(),
        }
    }
}
