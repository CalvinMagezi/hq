//! Core types and configuration for Agent-HQ.

pub mod approval_key;
pub mod build_info;
pub mod config;
pub mod critic;
pub mod daemon_restart;
pub mod discord_notify;
pub mod frontmatter_utils;
pub mod fs_private;
pub mod hardware;
pub mod heartbeat;
pub mod identity;
pub mod machine;
pub mod mailbox;
pub mod microcompact;
pub mod middleware;
pub mod pairing;
pub mod paths;
pub mod privacy;
pub mod prose_quality;
pub mod redact;
pub mod setup_provider;
pub mod telegram_access;
pub mod text;
#[cfg(any(test, feature = "test-util"))]
pub mod test_util;
pub mod tokens;
pub mod types;

pub use config::HqConfig;
pub use identity::{RequestIdentity, RequestSource};

/// Resolve the vault's skills directory.
///
/// Three spellings accumulated across the codebase — `skills`, `Skills`, and
/// `_skills` — which macOS's case-insensitive filesystem hid. On Linux (this
/// repo ships a Dockerfile) they are three different directories, so a skill
/// written through one path is invisible through another. `skills` is
/// canonical; the others resolve only when they already exist on disk.
pub fn skills_dir(vault_path: &std::path::Path) -> std::path::PathBuf {
    let canonical = vault_path.join("skills");
    if canonical.is_dir() {
        return canonical;
    }
    for legacy in ["Skills", "_skills"] {
        let candidate = vault_path.join(legacy);
        if candidate.is_dir() {
            tracing::warn!(
                path = %candidate.display(),
                "using legacy skills directory; rename it to `skills`"
            );
            return candidate;
        }
    }
    canonical
}

/// Get a stable device identifier based on hostname (first 16 hex chars of SHA-256).
/// Used for presence federation to distinguish local vs remote heartbeats.
pub fn device_id() -> String {
    use sha2::{Digest, Sha256};
    let host = hostname::get()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    let hash = Sha256::digest(host.as_bytes());
    hex::encode(&hash[..8])
}

#[cfg(test)]
mod skills_dir_tests {
    use super::skills_dir;

    #[test]
    fn defaults_to_lowercase_when_nothing_exists() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(skills_dir(tmp.path()), tmp.path().join("skills"));
    }

    #[test]
    fn prefers_the_canonical_directory_when_it_exists() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("skills")).unwrap();
        assert_eq!(skills_dir(tmp.path()), tmp.path().join("skills"));
    }

    /// Only meaningful on a case-sensitive filesystem. On macOS `skills` and
    /// `Skills` are the same inode, so the canonical branch matches first and
    /// there is nothing to fall back to — which is the correct answer there.
    #[test]
    fn falls_back_to_a_legacy_directory_when_canonical_is_absent() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("_skills")).unwrap();
        let resolved = skills_dir(tmp.path());
        assert_eq!(resolved, tmp.path().join("_skills"));
    }
}
