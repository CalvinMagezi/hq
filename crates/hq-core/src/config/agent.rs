use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Agent name (used in heartbeat, traces)
    #[serde(default = "default_agent_name")]
    pub name: String,
}

fn default_agent_name() -> String {
    hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "hq-agent".to_string())
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            name: default_agent_name(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonConfig {
    /// Embedding batch size
    #[serde(default = "default_embed_batch")]
    pub embedding_batch_size: usize,

    /// Embedding interval in seconds
    #[serde(default = "default_embed_interval")]
    pub embedding_interval_secs: u64,
}

fn default_embed_batch() -> usize {
    10
}

fn default_embed_interval() -> u64 {
    600
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            embedding_batch_size: default_embed_batch(),
            embedding_interval_secs: default_embed_interval(),
        }
    }
}
