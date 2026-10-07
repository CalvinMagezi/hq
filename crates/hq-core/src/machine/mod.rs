//! Machine capability profile — what is actually installed on this host.
//!
//! Agents had no way to answer "do I have GitHub access?" other than guessing,
//! so they guessed wrong. This module probes the host once, caches the result
//! to `_system/MACHINE.md`, and lets every system prompt carry the answer.
//!
//! The cached markdown is what goes in prompts. `system_info` re-probes live
//! when an agent needs to confirm something before relying on it.

mod cache;
mod probe;
mod render;
#[cfg(test)]
mod tests;

pub use cache::{load_cached, load_cached_profile, refresh};
pub use probe::{agent_hq_checkout_path, probe_machine, probe_machine_fast, which_binary};
pub use render::render_markdown;

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Binaries worth knowing about. Existence is probed for all of them;
/// [`VERSIONED_BINARIES`] additionally get a `--version` call.
pub const PROBED_BINARIES: &[&str] = &[
    "gh", "git", "docker", "gws", "cargo", "rustc", "node", "npm", "pnpm", "bun", "python3", "uv",
    "rg", "jq", "fd", "ollama", "sqlite3", "ffmpeg", "curl", "gcloud", "aws", "vercel",
    "psql",
];

/// Environment variables `system_info` may report. Everything else stays
/// hidden — API keys and tokens live in the environment.
pub const ENV_ALLOWLIST: &[&str] = &["PATH", "HOME", "SHELL", "LANG", "TERM"];

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BinaryStatus {
    pub name: String,
    pub path: Option<PathBuf>,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MachineProfile {
    pub generated_at: chrono::DateTime<chrono::Utc>,
    pub os: String,
    pub arch: String,
    pub hostname: String,
    pub in_container: bool,
    pub cpu_cores: usize,
    pub memory_gb: u64,
    pub home: PathBuf,
    pub vault_path: Option<PathBuf>,
    /// Present binaries, sorted by name.
    pub binaries: Vec<BinaryStatus>,
    /// Probed but absent, sorted.
    pub missing: Vec<String>,
    /// `gh auth status` summary, e.g. "alex (repo, workflow)".
    pub gh_auth: Option<String>,
    /// Whether auth and daemon state were actually probed. The fast startup
    /// probe skips them, and "not checked" must never render as "not
    /// authenticated" — a false negative here is the exact failure this
    /// profile exists to prevent.
    #[serde(default = "default_true")]
    pub deep_probed: bool,
    pub docker_running: bool,
    pub git_user: Option<String>,
    /// Whether this host can build/modify the agent-hq source itself:
    /// `cargo`/`git` on PATH and a checkout reachable per
    /// [`find_agent_hq_checkout`]. A prompt claiming "verify with cargo
    /// before shipping" is false on a host with only the installed binary
    /// (e.g. a bare relay VPS) — see FEATURE-REQUESTS.md FR-003.
    #[serde(default)]
    pub can_build_self: bool,
    /// Legacy summary: the first usable backend's detail, or `Some("brave")`, if one
    /// was reachable/configured at probe time, `None` if neither. Only
    /// meaningful when `deep_probed` is true, same rule as `gh_auth`.
    #[serde(default)]
    pub web_search_backend: Option<String>,
    /// Per-backend `web_search` health. Empty in profiles cached before it existed.
    #[serde(default)]
    pub web_search: Vec<WebSearchBackendStatus>,
}

/// One `web_search` backend, with "configured", "reachable" and "answered a
/// real query" kept apart so a set API key never reads as a working backend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebSearchBackendStatus {
    pub provider: String,
    pub configured: bool,
    /// `None` when this probe did not try to connect.
    pub reachable: Option<bool>,
    /// `None` when this probe sent no test query.
    pub answered: Option<bool>,
    /// Human-readable summary, never containing a credential.
    pub detail: String,
}

impl WebSearchBackendStatus {
    /// Whether `web_search` can plausibly use this backend right now.
    pub fn usable(&self) -> bool {
        self.configured && self.reachable != Some(false) && self.answered != Some(false)
    }
}
