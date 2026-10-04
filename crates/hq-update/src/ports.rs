//! The effects the update engine needs from its environment. Production
//! implementations live in `real.rs`; tests use in-memory fakes.

use crate::error::Result;
use async_trait::async_trait;
use std::path::Path;

#[async_trait]
pub trait Http: Send + Sync {
    /// Fetches a small document, failing past `max_bytes`.
    async fn get(&self, url: &str, max_bytes: u64) -> Result<Vec<u8>>;
    /// Streams a large artifact to `dest`, failing past `max_bytes`.
    async fn download(&self, url: &str, dest: &Path, max_bytes: u64) -> Result<()>;
}

#[async_trait]
pub trait Restarter: Send + Sync {
    async fn restart(&self) -> Result<()>;
    async fn stop(&self) -> Result<()>;
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HealthInfo {
    pub git_sha: Option<String>,
    pub version: Option<String>,
}

#[async_trait]
pub trait Health: Send + Sync {
    /// One probe of `/health`; `None` when the service does not answer OK.
    async fn probe(&self) -> Option<HealthInfo>;
}

/// Everything that touches the host beyond plain file moves: running
/// binaries (unprivileged), the vault database and notifications.
#[async_trait]
pub trait Host: Send + Sync {
    /// Runs `<bin> --version` as the unprivileged service user.
    async fn staged_version(&self, bin: &Path) -> Result<String>;
    /// Runs `hq install --upgrade` from the new binary, as the service user.
    async fn post_upgrade(&self) -> Result<()>;
    /// Best effort: phases are `pre`, `post-ok` and `post-rolled-back`.
    async fn notify(&self, phase: &str, sha: &str);
    /// A loud message for an operator (automatic rollback, skipped snapshot).
    async fn alert(&self, message: &str);
    /// Deletes old snapshots as the service user, keeping `keep` newest and `protected`.
    async fn db_prune(
        &self,
        dir: &Path,
        keep: usize,
        protected: &[std::path::PathBuf],
    ) -> Result<()>;
    /// Writes a consistent copy of the vault DB; `false` when there is none.
    async fn db_snapshot(&self, dest: &Path) -> Result<bool>;
    async fn db_restore(&self, snapshot: &Path) -> Result<()>;
    /// Number of applied schema migrations, `None` when there is no DB.
    async fn db_migrations(&self) -> Result<Option<u64>>;
}
