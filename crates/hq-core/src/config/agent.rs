use super::default_true;
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

    /// Where note vectors come from. `local` needs no network once its model is on disk.
    #[serde(default)]
    pub embedding_provider: EmbeddingProvider,

    /// How a chat turn becomes a memory: `llm` asks a model, `local` asks none.
    #[serde(default)]
    pub memory_extraction: MemoryExtraction,

    /// A weekly inbox item bringing up one saved note that matches open tasks. No model is used.
    #[serde(default = "default_true")]
    pub collection_tip: bool,

    /// Summarise links the owner shares into a note. Page text goes to the configured model.
    #[serde(default = "default_true")]
    pub link_capture: bool,

    /// A weekly inbox digest of recurring tool failures. Reads local telemetry only.
    #[serde(default = "default_true")]
    pub tool_failure_digest: bool,

    /// A weekly inbox digest of how HQ was used: capabilities, corrections, return visits.
    /// Reads local chat and tool records only.
    #[serde(default = "default_true")]
    pub adoption_digest: bool,

    /// A weekly inbox reminder to review finance rows and import new messages, only for a person
    /// who has a ledger. Reads counts from the ledger; never a message, and never a model.
    #[serde(default = "default_true")]
    pub finance_nudge: bool,

    /// Offers, once, to set up the free coding helpers on a connected computer that has none
    /// (`docs/HELPER_SETUP.md`). One inbox item; nothing is installed until the owner approves it.
    #[serde(default = "default_true")]
    pub helper_setup_offer: bool,

    /// A daily tidy that moves stale helper requests out of Needs you (restorable for 30 days).
    #[serde(default = "default_true")]
    pub inbox_tidy: bool,

    /// One inbox card a day with the next step of each focus lane (`hq focus`). Silent when there
    /// are no lanes or nothing to say.
    #[serde(default = "default_true")]
    pub daily_brief: bool,

    /// Fires the reminders the owner set (`hq reminders`, the `remind_*` tools). No model is involved.
    #[serde(default = "default_true")]
    pub reminders: bool,

    /// Every few hours, check the notes for links that reach nothing, duplicates and unreadable
    /// properties, and keep the result for the Notes screen and `hq doctor`. Reads notes on this
    /// device only: no model, no network, and it changes no note.
    #[serde(default = "default_true")]
    pub vault_health: bool,
}

/// How chat turns are turned into memories.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryExtraction {
    /// One model call per turn writes the summary, names, topics and importance.
    #[default]
    Llm,
    /// No model: an extractive summary, recurring names, tags borrowed from the nearest tagged
    /// notes (with the local embedder), and a salience-based importance.
    Local,
}

/// The source of note embeddings.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EmbeddingProvider {
    /// A small static model on this machine, downloaded once and checked against pinned hashes.
    /// The default because note text then never leaves the device.
    #[default]
    Local,
    /// OpenRouter when an API key is configured, Ollama on this machine otherwise. Note text
    /// goes to that provider, so an owner has to ask for it.
    Remote,
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
            embedding_provider: EmbeddingProvider::default(),
            memory_extraction: MemoryExtraction::default(),
            collection_tip: true,
            link_capture: true,
            tool_failure_digest: true,
            adoption_digest: true,
            finance_nudge: true,
            helper_setup_offer: true,
            inbox_tidy: true,
            daily_brief: true,
            reminders: true,
            vault_health: true,
        }
    }
}
