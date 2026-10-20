use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Scheduled encrypted backups of the vault and its database (`hq backup`, `docs/BACKUP.md`).
/// Off by default. A scheduled backup uses a key file, never a passphrase, so no secret sits in
/// this file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupConfig {
    #[serde(default)]
    pub enabled: bool,

    /// Where backups are written: a folder outside the vault. Needed when enabled.
    #[serde(default)]
    pub dir: Option<PathBuf>,

    /// The key file made by `hq backup keygen`. Needed when enabled; only its owner may be able
    /// to read it. Also what `hq backup create` uses when no key is named.
    #[serde(default)]
    pub key_file: Option<PathBuf>,

    /// Hours between scheduled backups.
    #[serde(default = "default_every_hours")]
    pub every_hours: u64,

    /// How many backups to keep. Older ones are deleted after each new one; at least one.
    #[serde(default = "default_keep")]
    pub keep: usize,

    /// Also back up `config.yaml`, which holds API keys and tokens. Off by default.
    #[serde(default)]
    pub include_config: bool,
}

fn default_every_hours() -> u64 {
    24
}

fn default_keep() -> usize {
    7
}

impl Default for BackupConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            dir: None,
            key_file: None,
            every_hours: default_every_hours(),
            keep: default_keep(),
            include_config: false,
        }
    }
}
