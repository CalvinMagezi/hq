//! The contract every session-facing context adapter shares: who the caller is, what they
//! may read, and what comes back. Retrieval, MCP tools and adapters build on it later; this
//! module only decides identity and scope. See `docs/plans/context-gateway.md`.

mod resolver;
mod retrieval;
mod scope;

use chrono::{DateTime, Utc};
use hq_memory::context_packet::{EntryProvenance, Freshness, PacketGap};
use serde::{Deserialize, Serialize};

pub use resolver::DbGrantResolver;
pub use retrieval::{RateLimiter, Retriever};
pub use scope::{PrefixScope, SourceId};

pub const DEFAULT_MAX_GRAPH_DEPTH: u8 = 2;
pub const DEFAULT_MAX_RESULTS: usize = 20;
pub const DEFAULT_MAX_CHARS: usize = 12_000;

/// What a session may do, as a bit set. Reading is the only cap a fresh grant holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caps(u8);

impl Caps {
    pub const NONE: Caps = Caps(0);
    pub const READ: Caps = Caps(1);
    pub const TASK_COMMENT: Caps = Caps(2);
    pub const TASK_UPDATE: Caps = Caps(4);
    pub const ASK_HQ: Caps = Caps(8);

    pub fn contains(self, other: Caps) -> bool {
        self.0 & other.0 == other.0
    }
}

/// Which vault folders, and how much of them, one task opens to its sessions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceScope {
    pub allow_prefixes: Vec<SourceId>,
    pub max_graph_depth: u8,
    pub max_results: usize,
    pub max_chars: usize,
}

/// Built by the gateway from the proven session, never from call arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionGrant {
    pub session_id: String,
    pub task_id: String,
    pub mission_id: Option<String>,
    pub scope: SourceScope,
    pub caps: Caps,
    /// `None` means the grant lives as long as the session runs.
    pub expires_at: Option<DateTime<Utc>>,
}

impl SessionGrant {
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        self.expires_at.is_some_and(|at| at <= now)
    }
}

/// Why no grant exists. Every variant means "no access": callers never fall back to a wider
/// tool, and `Backend` (the database could not answer) counts as a refusal too.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unavailable {
    NoIdentity,
    Revoked,
    Expired,
    NotRunning,
    NoTask,
    Unmarked,
    Backend,
}

impl std::fmt::Display for Unavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NoIdentity => "no session identity",
            Self::Revoked => "the session credential was revoked",
            Self::Expired => "the session credential expired",
            Self::NotRunning => "the session is not running",
            Self::NoTask => "the session holds no task",
            Self::Unmarked => "the task has no readable folders",
            Self::Backend => "the grant could not be checked",
        })
    }
}

impl std::error::Error for Unavailable {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextOp {
    Orient,
    Search {
        query: String,
        limit: usize,
    },
    Expand {
        from: Vec<SourceId>,
        depth: u8,
    },
    Read {
        id: SourceId,
        section: Option<String>,
        max_chars: usize,
    },
    Related {
        id: SourceId,
    },
    AskHq {
        question: String,
    },
}

impl ContextOp {
    /// The cap an operation needs. Every retrieval op needs READ; asking HQ needs its own.
    pub fn required_cap(&self) -> Caps {
        match self {
            Self::AskHq { .. } => Caps::ASK_HQ,
            _ => Caps::READ,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextEntry {
    pub id: String,
    pub title: String,
    pub excerpt: String,
    /// Why retrieval returned this entry.
    pub why: String,
    pub graph_path: Option<String>,
    pub freshness: Freshness,
    pub provenance: EntryProvenance,
    pub content_hash: String,
    pub retrieved_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextResult {
    pub entries: Vec<ContextEntry>,
    pub gaps: Vec<PacketGap>,
    pub budget_chars: usize,
}

/// The single choke point. Search, graph, read and chat refs all go through it.
pub trait ScopeCheck {
    fn permits(&self, grant: &SessionGrant, id: &SourceId) -> bool;

    /// Splits candidates into the readable ones and a gap that counts the rest. The gap
    /// carries no source id or name, so a denied source leaks nothing but its existence.
    fn partition(
        &self,
        grant: &SessionGrant,
        candidates: Vec<SourceId>,
    ) -> (Vec<SourceId>, Option<PacketGap>) {
        let total = candidates.len();
        let permitted: Vec<SourceId> = candidates
            .into_iter()
            .filter(|id| self.permits(grant, id))
            .collect();
        let count = total - permitted.len();
        let gap = (count > 0).then(|| PacketGap {
            kind: hq_memory::context_packet::GapKind::Denied { count },
            detail: "sources outside this task's scope were left out".to_string(),
        });
        (permitted, gap)
    }
}

/// Turns a proven session id into a grant, or says why it cannot.
pub trait GrantResolver {
    fn resolve(&self, session_id: &str) -> Result<SessionGrant, Unavailable>;
}
