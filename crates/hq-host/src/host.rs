//! The set of agents this host runs, and the operations on them.

use crate::emu::{Row, VtEmulator};
use crate::env::pane_env;
use crate::error::HostError;
use crate::keys::encode_key;
pub use crate::pane::PaneStatus;
use crate::pane::{LaunchArgs, Pane};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

const MAX_NAME_LEN: usize = 32;
const DEFAULT_ROWS: u16 = 40;
const DEFAULT_COLS: u16 = 120;
const DEFAULT_SCROLLBACK_ROWS: usize = 10_000;
/// Pause between pasting a prompt and pressing Enter, so a TUI that handles the
/// paste first does not swallow the Enter.
const PROMPT_ENTER_DELAY: Duration = Duration::from_millis(150);
const BRACKETED_PASTE_START: &[u8] = b"\x1b[200~";
const BRACKETED_PASTE_END: &[u8] = b"\x1b[201~";

/// Agent names: `[a-z][a-z0-9_-]{0,31}`.
pub fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '-'))
        && name.len() <= MAX_NAME_LEN
}

#[derive(Debug, Clone)]
pub struct SpawnSpec {
    pub name: String,
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    /// Variables the process gets in addition to the allowlisted ones.
    pub env: Vec<(String, String)>,
    pub rows: u16,
    pub cols: u16,
    pub scrollback_rows: usize,
}

impl SpawnSpec {
    pub fn new(name: impl Into<String>, argv: Vec<String>, cwd: impl Into<PathBuf>) -> Self {
        Self {
            name: name.into(),
            argv,
            cwd: cwd.into(),
            env: Vec::new(),
            rows: DEFAULT_ROWS,
            cols: DEFAULT_COLS,
            scrollback_rows: DEFAULT_SCROLLBACK_ROWS,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadSource {
    /// Rows on screen now.
    Visible,
    /// Scrollback and screen, rows as displayed.
    Recent,
    /// Scrollback and screen, wrapped rows joined back into logical lines.
    RecentUnwrapped,
}

#[derive(Debug, Clone)]
pub struct PaneInfo {
    pub name: String,
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    pub pid: Option<u32>,
    pub status: PaneStatus,
    pub rows: u16,
    pub cols: u16,
    pub bytes_seen: u64,
    pub quiet_for: Duration,
    pub age: Duration,
}

#[derive(Default)]
pub struct Host {
    panes: Mutex<BTreeMap<String, Arc<Pane>>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn unwrap_rows(rows: Vec<Row>) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for row in rows {
        current.push_str(&row.text);
        if !row.wrapped {
            lines.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

impl Host {
    pub fn new() -> Self {
        Self::default()
    }

    fn pane(&self, name: &str) -> Result<Arc<Pane>, HostError> {
        lock(&self.panes)
            .get(name)
            .cloned()
            .ok_or_else(|| HostError::NotFound(name.to_string()))
    }

    pub fn spawn(&self, spec: SpawnSpec) -> Result<PaneInfo, HostError> {
        if !valid_name(&spec.name) {
            return Err(HostError::InvalidName(spec.name));
        }
        let mut panes = lock(&self.panes);
        if panes.contains_key(&spec.name) {
            return Err(HostError::NameTaken(spec.name));
        }
        let pane = Pane::spawn(
            LaunchArgs {
                argv: spec.argv,
                cwd: spec.cwd,
                env: pane_env(&spec.env),
                rows: spec.rows,
                cols: spec.cols,
            },
            Box::new(VtEmulator::new(spec.rows, spec.cols, spec.scrollback_rows)),
        )?;
        let pane = Arc::new(pane);
        panes.insert(spec.name.clone(), pane.clone());
        Ok(info_of(&spec.name, pane.as_ref()))
    }

    pub fn info(&self, name: &str) -> Result<PaneInfo, HostError> {
        Ok(info_of(name, self.pane(name)?.as_ref()))
    }

    pub fn list(&self) -> Vec<PaneInfo> {
        lock(&self.panes)
            .iter()
            .map(|(n, p)| info_of(n, p))
            .collect()
    }

    /// Forgets an agent. A running process is killed.
    pub fn remove(&self, name: &str) -> Result<(), HostError> {
        lock(&self.panes)
            .remove(name)
            .map(|_| ())
            .ok_or_else(|| HostError::NotFound(name.to_string()))
    }

    /// The last `lines` lines of the chosen view (all of them when `lines` is 0).
    pub fn read(&self, name: &str, source: ReadSource, lines: usize) -> Result<String, HostError> {
        let pane = self.pane(name)?;
        let mut out: Vec<String> = match source {
            ReadSource::Visible => pane.rows(false).into_iter().map(|r| r.text).collect(),
            ReadSource::Recent => pane.rows(true).into_iter().map(|r| r.text).collect(),
            ReadSource::RecentUnwrapped => unwrap_rows(pane.rows(true)),
        };
        if lines > 0 && out.len() > lines {
            out.drain(..out.len() - lines);
        }
        Ok(out.join("\n"))
    }

    fn live_pane(&self, name: &str) -> Result<Arc<Pane>, HostError> {
        let pane = self.pane(name)?;
        if pane.status() != PaneStatus::Running {
            return Err(HostError::Exited(name.to_string()));
        }
        Ok(pane)
    }

    /// Raw bytes, exactly as given.
    pub fn send_text(&self, name: &str, text: &str) -> Result<(), HostError> {
        self.live_pane(name)?.write(text.as_bytes())
    }

    /// Text as a paste: bracketed when the program asked for it, so embedded
    /// newlines are not taken for Enter.
    pub fn paste(&self, name: &str, text: &str) -> Result<(), HostError> {
        let pane = self.live_pane(name)?;
        if pane.with_emu(|e| e.bracketed_paste()) {
            let mut bytes = BRACKETED_PASTE_START.to_vec();
            bytes.extend_from_slice(text.as_bytes());
            bytes.extend_from_slice(BRACKETED_PASTE_END);
            pane.write(&bytes)
        } else {
            pane.write(text.as_bytes())
        }
    }

    /// Logical key names (`enter`, `esc`, `ctrl+c`, ...), validated before anything is sent.
    pub fn send_keys(&self, name: &str, keys: &[String]) -> Result<(), HostError> {
        let mut bytes = Vec::new();
        for key in keys {
            bytes.extend(encode_key(key)?);
        }
        self.live_pane(name)?.write(&bytes)
    }

    /// Pastes `text` and presses Enter.
    pub fn prompt(&self, name: &str, text: &str) -> Result<(), HostError> {
        self.paste(name, text)?;
        std::thread::sleep(PROMPT_ENTER_DELAY);
        self.send_keys(name, &["enter".to_string()])
    }

    pub fn resize(&self, name: &str, rows: u16, cols: u16) -> Result<(), HostError> {
        self.live_pane(name)?.resize(rows, cols)
    }

    pub fn wait_exit(&self, name: &str, timeout: Duration) -> Result<u32, HostError> {
        self.pane(name)?.wait_exit(timeout)
    }

    pub fn wait_quiet(
        &self,
        name: &str,
        quiet: Duration,
        timeout: Duration,
    ) -> Result<(), HostError> {
        self.pane(name)?.wait_quiet(quiet, timeout)
    }

    pub fn kill(&self, name: &str) -> Result<(), HostError> {
        self.pane(name)?.kill();
        Ok(())
    }
}

fn info_of(name: &str, pane: &Pane) -> PaneInfo {
    let (rows, cols) = pane.size();
    PaneInfo {
        name: name.to_string(),
        argv: pane.argv.clone(),
        cwd: pane.cwd.clone(),
        pid: pane.pid,
        status: pane.status(),
        rows,
        cols,
        bytes_seen: pane.bytes_seen(),
        quiet_for: pane.quiet_for(),
        age: pane.started.elapsed(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_the_agent_name_rule() {
        for ok in ["a", "hs-claude-abc123", "x_1"] {
            assert!(valid_name(ok), "{ok}");
        }
        for bad in ["", "A", "1a", "-a", "a b", "a.b", &"a".repeat(33)] {
            assert!(!valid_name(bad), "{bad:?}");
        }
    }
}
