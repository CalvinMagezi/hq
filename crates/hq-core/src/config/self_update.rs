use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Configuration for HQ's checkpointed self-update lifecycle: editing its own
/// source, testing, rebuilding, and reinstalling the live binary.
///
/// Off by default everywhere, not just on cloud instances: self-modifying a
/// running agent process is risky enough to warrant an explicit opt-in
/// regardless of deployment target (`self_update: { enabled: true }` in
/// config.yaml). A production/shared instance should update via a real
/// deploy script or `hq update` (signed releases) run by a deploy-capable
/// operator, not via in-process self-rewrite. When enabled, `install` still
/// waits for the owner to approve the exact tree (docs/security/SELF_UPDATE.md).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelfUpdateConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// Path to the agent-hq repository checkout HQ modifies.
    /// Defaults to the parent of the vault (the repo contains `.vault/`).
    #[serde(default)]
    pub repo_path: Option<PathBuf>,

    /// Binaries replaced on install.
    #[serde(default = "default_install_paths")]
    pub install_paths: Vec<PathBuf>,

    /// launchd label kickstarted after install (empty disables).
    #[serde(default = "default_launchd_label")]
    pub launchd_label: String,

    /// Refuse install unless cargo check + tests passed for this run.
    #[serde(default = "default_require_tests")]
    pub require_tests: bool,
}

fn default_enabled() -> bool {
    false
}

fn default_install_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(home) = dirs::home_dir() {
        paths.push(home.join("bin/hq"));
    }
    paths.push(PathBuf::from("/usr/local/bin/hq"));
    paths
}

fn default_launchd_label() -> String {
    "com.agent-hq.hq-all".to_string()
}

fn default_require_tests() -> bool {
    true
}

impl Default for SelfUpdateConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            repo_path: None,
            install_paths: default_install_paths(),
            launchd_label: default_launchd_label(),
            require_tests: default_require_tests(),
        }
    }
}

impl SelfUpdateConfig {
    /// Resolve the repo path: explicit config or the vault's parent directory.
    pub fn resolve_repo_path(&self, vault_path: &std::path::Path) -> PathBuf {
        self.repo_path
            .clone()
            .unwrap_or_else(|| vault_path.parent().unwrap_or(vault_path).to_path_buf())
    }
}
