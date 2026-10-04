//! Persistent task state — tracks last-run times across daemon restarts.
//!
//! Enables missed-task detection: on startup, compare persisted last-run times
//! against task intervals to identify tasks that should have fired while the
//! daemon was down. Inspired by Claude Code's Kairos `findMissedTasks()`.

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::{debug, warn};

/// State file location relative to vault root.
const STATE_FILE: &str = "_system/.daemon-task-state.json";

#[derive(Serialize, Deserialize, Default)]
pub struct TaskState {
    /// Task name -> last successful run time (UTC).
    pub last_run: HashMap<String, DateTime<Utc>>,
    /// When the daemon last started.
    pub last_startup: Option<DateTime<Utc>>,
    /// When the daemon last shut down cleanly.
    pub last_shutdown: Option<DateTime<Utc>>,
}

impl TaskState {
    /// Load persisted state from disk. Returns default if file doesn't exist or is corrupt.
    pub fn load(vault_path: &Path) -> Self {
        let path = Self::path(vault_path);
        match std::fs::read_to_string(&path) {
            Ok(contents) => serde_json::from_str(&contents).unwrap_or_else(|e| {
                warn!("daemon task state corrupt, starting fresh: {e}");
                Self::default()
            }),
            Err(_) => {
                debug!("no daemon task state found, starting fresh");
                Self::default()
            }
        }
    }

    /// Save state to disk (atomic: write to temp, then rename).
    pub fn save(&self, vault_path: &Path) -> Result<()> {
        let path = Self::path(vault_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("json.tmp");
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    /// Record a successful task run.
    pub fn mark_run(&mut self, task_name: &str) {
        self.last_run.insert(task_name.to_string(), Utc::now());
    }

    /// Record daemon startup.
    pub fn mark_startup(&mut self) {
        self.last_startup = Some(Utc::now());
    }

    /// Record clean daemon shutdown.
    pub fn mark_shutdown(&mut self) {
        self.last_shutdown = Some(Utc::now());
    }

    /// Find tasks that missed at least one full cycle while the daemon was down.
    /// Returns Vec of (task_name, seconds_overdue).
    pub fn find_missed_tasks(&self, tasks: &[(&str, std::time::Duration)]) -> Vec<(String, i64)> {
        let now = Utc::now();
        let mut missed = Vec::new();

        for &(name, interval) in tasks {
            if let Some(last) = self.last_run.get(name) {
                let elapsed = (now - *last).num_seconds();
                let interval_secs = interval.as_secs() as i64;
                // Missed if more than 2x the interval has passed
                if elapsed > interval_secs * 2 {
                    let overdue = elapsed - interval_secs;
                    missed.push((name.to_string(), overdue));
                }
            }
            // Tasks with no recorded last_run are not considered "missed" —
            // they simply fire on the first tick as usual.
        }

        missed
    }

    fn path(vault_path: &Path) -> PathBuf {
        vault_path.join(STATE_FILE)
    }
}
