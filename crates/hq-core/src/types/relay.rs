use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ─── Mailbox Types ─────────────────────────────────────────────

/// Type of inter-agent mailbox message.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum MailboxMessageType {
    /// Task completion notification with results
    TaskResult,
    /// Context handoff (pass data to another agent)
    ContextHandoff,
    /// Idle notification (agent has no work)
    Idle,
    /// Plan approval request (agent -> lead)
    PlanApproval,
    /// Plan approval response (lead -> agent): approved/rejected with feedback
    PlanApprovalResponse,
    /// Progress update from a running subagent
    Progress,
    /// Generic direct message
    Direct,
    /// Low-priority attention request for stalled or pending plans
    Nudge,
    /// A newly-minted skill proposal awaiting user approval (move from _proposed/ to apply).
    SkillProposal,
    /// Targeted question to refine the structured user model (USER.md).
    UserModelQuestion,
    /// HQ proposes a code/skill/vault change for user approval before dispatching Claude Code.
    CodeProposal,
}

/// A point-to-point message between agents/harnesses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MailboxMessage {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub from: String,
    pub to: String,
    pub msg_type: MailboxMessageType,
    pub subject: Option<String>,
    pub content: String,
    /// Related job ID, if any
    #[serde(default)]
    pub job_id: Option<String>,
    /// Arbitrary key-value metadata
    #[serde(default)]
    pub meta: HashMap<String, String>,
}

/// Heartbeat record for an active harness.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarnessHeartbeat {
    pub harness_id: String,
    pub pid: u32,
    pub last_heartbeat: DateTime<Utc>,
    pub current_job_id: Option<String>,
    pub status: String, // "working", "idle", "connected"
    #[serde(default)]
    pub harness_type: Option<String>, // "worker", "mcp", "relay-discord", "relay-telegram", "subagent"
    #[serde(default)]
    pub working_directory: Option<String>,
    #[serde(default)]
    pub task_summary: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub parent_session_id: Option<String>,
    #[serde(default)]
    pub started_at: Option<DateTime<Utc>>,
    /// Device ID for presence federation. When set and differs from local,
    /// PID liveness checks are skipped (remote heartbeats use staleness only).
    #[serde(default)]
    pub device_id: Option<String>,
}
