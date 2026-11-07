//! Files the config names that an agent's tools must not read or replace.
//!
//! The bash sandbox runs per command, far from the
//! loaded config, so `HqConfig::load` registers these here once and they read
//! the registered set. A process that never loads the config (most tests)
//! registers nothing, and the callers fall back to their built-in lists.

use std::path::PathBuf;
use std::sync::RwLock;

use super::HqConfig;

/// Paths taken from the config, as written there.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProtectedFiles {
    /// Readable but never writable from the bash sandbox: the self-update install
    /// paths, where a write would replace the binary the daemon runs.
    pub readonly: Vec<PathBuf>,
}

impl ProtectedFiles {
    pub fn from_config(config: &HqConfig) -> Self {
        Self {
            readonly: config.self_update.install_paths.clone(),
        }
    }
}

static REGISTERED: RwLock<Option<ProtectedFiles>> = RwLock::new(None);

/// Replace the registered set (the newest loaded config wins).
pub fn register(files: ProtectedFiles) {
    let mut slot = REGISTERED.write().unwrap_or_else(|e| e.into_inner());
    *slot = Some(files);
}

/// The registered set, empty when no config was loaded in this process.
pub fn registered() -> ProtectedFiles {
    REGISTERED
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_install_paths_are_taken_from_the_config() {
        let mut config = HqConfig::default();
        config.self_update.install_paths = vec![PathBuf::from("/opt/bin/hq")];
        let files = ProtectedFiles::from_config(&config);
        assert_eq!(files.readonly, vec![PathBuf::from("/opt/bin/hq")]);
    }
}
