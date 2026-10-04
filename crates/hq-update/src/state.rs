//! On-disk layout and persistent state of the updater.

use crate::config::UpdateConfig;
use crate::error::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Identity of an installed build.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Installed {
    pub version: String,
    pub git_sha: String,
}

/// One kept previous build. Index 0 of `State::history` is `hq.1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Slot {
    pub installed: Installed,
    /// Vault DB snapshot taken just before this build was replaced.
    pub db_snapshot: Option<PathBuf>,
    /// Whether `dist.prev` holds this build's web files.
    pub has_web: bool,
}

/// A release the timer will not retry until `until` (unix seconds).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Block {
    pub version: String,
    pub until: i64,
}

/// Journal entry written before the first swap and cleared once the update
/// is healthy or rolled back; a leftover one means the updater died midway.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InProgress {
    pub target: Installed,
    pub from: Installed,
    pub db_snapshot: Option<PathBuf>,
    pub migrations_before: Option<u64>,
    pub phase: String,
}

/// Highest channel pointer accepted so far, to refuse a replayed older one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeenPointer {
    pub channel: String,
    pub seq: u64,
    pub manifest_sha256: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    #[serde(default)]
    pub seen_pointers: Vec<SeenPointer>,
    pub history: Vec<Slot>,
    /// Releases that failed on this host, skipped until their block expires.
    /// Expiry matters: the service user can cause failures on purpose.
    pub blocked: Vec<Block>,
    #[serde(default)]
    pub in_progress: Option<InProgress>,
}

fn now_secs() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Flushes a directory entry change (a rename) to disk.
pub fn sync_dir(path: &Path) {
    if let Some(parent) = path.parent()
        && let Ok(dir) = fs::File::open(parent)
    {
        let _ = dir.sync_all();
    }
}

#[derive(Debug, Clone)]
pub struct Layout {
    pub bin_path: PathBuf,
    pub web_dist: PathBuf,
    pub state_dir: PathBuf,
    pub snapshots_dir: PathBuf,
}

impl Layout {
    pub fn from_config(cfg: &UpdateConfig) -> Self {
        Self {
            bin_path: cfg.bin_path.clone(),
            web_dist: cfg.web_dist.clone(),
            state_dir: cfg.state_dir.clone(),
            snapshots_dir: cfg.snapshots_dir.clone(),
        }
    }

    pub fn state_file(&self) -> PathBuf {
        self.state_dir.join("state.json")
    }

    pub fn lock_file(&self) -> PathBuf {
        self.state_dir.join("update.lock")
    }

    pub fn backups_dir(&self) -> PathBuf {
        self.state_dir.join("bin")
    }

    pub fn backup(&self, n: usize) -> PathBuf {
        self.backups_dir().join(format!("hq.{n}"))
    }

    pub fn snapshots_dir(&self) -> PathBuf {
        self.snapshots_dir.clone()
    }

    pub fn work_dir(&self) -> PathBuf {
        self.state_dir.join("work")
    }

    /// Next to the live binary so the final rename never crosses a filesystem.
    pub fn staged_binary(&self) -> PathBuf {
        sibling(&self.bin_path, ".staged")
    }

    pub fn rollback_binary(&self) -> PathBuf {
        sibling(&self.bin_path, ".rollback")
    }

    /// Next to the live web dir for the same reason.
    pub fn staged_web(&self) -> PathBuf {
        sibling(&self.web_dist, ".new")
    }

    pub fn prev_web(&self) -> PathBuf {
        sibling(&self.web_dist, ".prev")
    }

    pub fn failed_web(&self) -> PathBuf {
        sibling(&self.web_dist, ".failed")
    }
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    path.with_file_name(format!(".{name}{suffix}"))
}

impl State {
    /// A corrupt or empty file is moved aside and treated as empty state:
    /// a permanent hard error would stop every future update.
    pub fn load(layout: &Layout) -> Result<Self> {
        let path = layout.state_file();
        let bytes = match fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(e.into()),
        };
        match serde_json::from_slice(&bytes) {
            Ok(state) => Ok(state),
            Err(e) => {
                let aside = layout
                    .state_dir
                    .join(format!("state.json.corrupt-{}", now_secs()));
                tracing::error!(error = %e, moved_to = %aside.display(), "state.json is corrupt, starting empty");
                fs::rename(&path, &aside)?;
                Ok(Self::default())
            }
        }
    }

    pub fn save(&self, layout: &Layout) -> Result<()> {
        use std::io::Write;
        fs::create_dir_all(&layout.state_dir)?;
        let tmp = layout.state_dir.join("state.json.tmp");
        let mut file = fs::File::create(&tmp)?;
        file.write_all(&serde_json::to_vec_pretty(self).map_err(anyhow::Error::from)?)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, layout.state_file())?;
        sync_dir(&layout.state_file());
        Ok(())
    }

    pub fn is_blocked(&self, version: &str) -> bool {
        let now = now_secs();
        self.blocked
            .iter()
            .any(|b| b.version == version && b.until > now)
    }

    pub fn block(&mut self, version: &str, ttl_secs: u64) {
        let now = now_secs();
        self.blocked
            .retain(|b| b.until > now && b.version != version);
        self.blocked.push(Block {
            version: version.to_string(),
            until: now.saturating_add(ttl_secs as i64),
        });
    }

    pub fn seen_pointer(&self, channel: &str) -> Option<&SeenPointer> {
        self.seen_pointers.iter().find(|p| p.channel == channel)
    }

    /// Records a pointer if it is newer than the one on file.
    pub fn record_pointer(&mut self, channel: &str, seq: u64, manifest_sha256: &str) -> bool {
        if self.seen_pointer(channel).is_some_and(|p| p.seq >= seq) {
            return false;
        }
        self.seen_pointers.retain(|p| p.channel != channel);
        self.seen_pointers.push(SeenPointer {
            channel: channel.to_string(),
            seq,
            manifest_sha256: manifest_sha256.to_string(),
        });
        true
    }

    pub fn unblock(&mut self, version: &str) {
        self.blocked.retain(|b| b.version != version);
    }
}
