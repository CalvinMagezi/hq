mod permission;
mod relay;
mod session;
pub mod value_bus;

pub use permission::*;
pub use relay::*;
pub use session::*;
pub use value_bus::*;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ─── Enumerations ──────────────���────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum SecurityProfile {
    Minimal,
    Standard,
    #[default]
    Guarded,
    Admin,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum NoteType {
    Note,
    Digest,
    #[serde(rename = "system-file")]
    SystemFile,
    Report,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum EmbeddingStatus {
    Pending,
    Processing,
    Embedded,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum HarnessType {
    Hq,
    /// Catch-all for legacy harness names in old vault data.
    /// `#[serde(other)]` ensures "claude-code", "opencode", etc. deserialize here.
    #[serde(other)]
    Any,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Blocked,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AgentVertical {
    Engineering,
    Qa,
    Research,
    Content,
    Ops,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AgentRole {
    Coder,
    Researcher,
    Reviewer,
    Planner,
    Devops,
    Workspace,
}

// ─── Subagent Types ────────────────────────────────────────────

/// Specialized subagent type that controls tool filtering, system prompt,
/// permission mode, and model alias. All sub-agents run in-process via
/// HQ's built-in LLM router.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SubagentType {
    /// Full tool access, standard system prompt.
    #[default]
    General,
    /// Fast read-only explorer: vault search and file reads only.
    Explorer,
    /// Software architect: read-only + plan file writes. Produces plans.
    Planner,
    /// Adversarial verification: read-only + test execution.
    Verifier,
    /// Code implementation: full write access, optional worktree isolation.
    Coder,
    /// Custom agent loaded from `.vault/Agents/{name}.md`.
    Custom(String),
}

impl SubagentType {
    /// Whether this agent type is read-only (cannot write files).
    pub fn is_read_only(&self) -> bool {
        matches!(self, Self::Explorer | Self::Planner | Self::Verifier)
    }
}

impl std::fmt::Display for SubagentType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::General => write!(f, "general"),
            Self::Explorer => write!(f, "explorer"),
            Self::Planner => write!(f, "planner"),
            Self::Verifier => write!(f, "verifier"),
            Self::Coder => write!(f, "coder"),
            Self::Custom(name) => write!(f, "custom:{name}"),
        }
    }
}

// ─── Note & Search Types ───────────────────────────────────────

/// A note in the vault.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Note {
    pub title: String,
    pub content: String,
    pub path: String,
    #[serde(default)]
    pub frontmatter: HashMap<String, serde_yaml::Value>,
    #[serde(default, rename = "noteType")]
    pub note_type: Option<NoteType>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default, rename = "embeddingStatus")]
    pub embedding_status: Option<EmbeddingStatus>,
    #[serde(default, rename = "createdAt")]
    pub created_at: Option<String>,
    #[serde(default, rename = "updatedAt")]
    pub updated_at: Option<String>,
    pub modified_at: DateTime<Utc>,
}

/// Match type for search results.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MatchType {
    Keyword,
    Semantic,
    Hybrid,
    Recent,
}

/// A search result from vault search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub note_path: String,
    pub title: String,
    pub notebook: String,
    pub snippet: String,
    pub tags: Vec<String>,
    pub relevance: f64,
    pub match_type: MatchType,
}

/// Search index statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchStats {
    pub fts_count: usize,
    pub embedding_count: usize,
}

/// System context (SOUL, MEMORY, PREFERENCES, HEARTBEAT).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SystemContext {
    pub soul: String,
    pub memory: String,
    pub preferences: String,
    pub heartbeat: String,
    #[serde(default)]
    pub config: HashMap<String, String>,
    #[serde(default)]
    pub pinned_notes: Vec<Note>,
    #[serde(default)]
    pub pinned_scan: PinnedScanReport,
}

/// Which directories the pinned-note scanner visited vs. skipped (missing),
/// so a caller can tell "no pinned notes here" apart from "this directory
/// isn't scanned at all".
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PinnedScanReport {
    pub scanned: Vec<String>,
    pub skipped: Vec<String>,
}

/// Agent definition (loaded from markdown frontmatter).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDefinition {
    #[serde(default)]
    pub name: String,
    #[serde(default, rename = "displayName")]
    pub display_name: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub vertical: Option<AgentVertical>,
    #[serde(default, rename = "baseRole")]
    pub base_role: Option<AgentRole>,
    #[serde(default, rename = "preferredHarness")]
    pub preferred_harness: Option<HarnessType>,
    #[serde(default, rename = "preferredModel")]
    pub preferred_model: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default, rename = "autoLoad")]
    pub auto_load: bool,
    /// `#[serde(default)]` deliberately: this struct also deserializes
    /// *just the YAML frontmatter* of an agent file (see
    /// `hq-tools/src/agents.rs::parse_agent_file`), which never contains
    /// `instruction` — that field comes from the markdown body afterward.
    /// Without this, that frontmatter deserialization always fails (missing
    /// required field) and silently discards every other frontmatter value
    /// (displayName, tags, baseRole, ...) via the caller's fallback path.
    #[serde(default)]
    pub instruction: String,
    #[serde(default, rename = "fallbackChain")]
    pub fallback_chain: Vec<HarnessType>,
}

// ─── LLM Chat Types ───────────────���─────────────────────────────

/// LLM chat message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: MessageRole,
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// DeepSeek thinking scratchpad. Must be replayed verbatim in history for
    /// assistant turns that had tool_calls — omitting it causes a 400 error on
    /// the next request. Safe to omit (and saves tokens) for non-tool turns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    /// Images attached to this (user) turn, forwarded to vision-capable
    /// models alongside `content` (FR-017). Additive by design: `content`
    /// stays a plain `String` rather than becoming an enum, because it's
    /// constructed via struct literal at ~140 call sites across the
    /// workspace — this field defaults to empty so every existing call site
    /// keeps compiling and serializing unchanged.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub image_parts: Vec<ImageAttachment>,
}

/// A single image attached to a `ChatMessage`. Carries a local file path,
/// not pre-encoded bytes — `ChatMessage` is cloned repeatedly into
/// conversation history, and keeping base64 blobs off the struct avoids
/// bloating every in-memory history and persisted transcript. Encoding
/// happens once, lazily, at request-build time via `to_data_url`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageAttachment {
    /// Local path to the downloaded file. Never sent upstream directly —
    /// only read to build a data URL — so this is not a credential/URL leak
    /// vector in logged request payloads.
    pub path: std::path::PathBuf,
    /// e.g. "image/jpeg", "image/png".
    pub mime_type: String,
}

impl ImageAttachment {
    /// Reads the file and returns a `data:<mime>;base64,<...>` URL suitable
    /// for an OpenAI-compatible vision request's `image_url.url` field.
    pub fn to_data_url(&self) -> std::io::Result<String> {
        use base64::Engine;
        let bytes = std::fs::read(&self.path)?;
        Ok(format!(
            "data:{};base64,{}",
            self.mime_type,
            base64::engine::general_purpose::STANDARD.encode(bytes)
        ))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
}

/// A tool call from the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

/// Tool definition for LLM function calling.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// Tool execution result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub content: Vec<ToolResultContent>,
    #[serde(default)]
    pub details: Option<serde_json::Value>,
    /// Optional context modifier applied after tool execution.
    /// Injected as a compact steering annotation before the next LLM call,
    /// saving ~200 tokens vs. a full message slot.
    #[serde(skip)]
    pub context_modifier: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResultContent {
    pub r#type: String,
    pub text: String,
}
