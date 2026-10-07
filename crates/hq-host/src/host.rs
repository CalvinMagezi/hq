//! The set of agents this host runs, and the operations on them.

use crate::detect::{AgentState, Detector, Input};
use crate::emu::{Row, VtEmulator};
use crate::env::pane_env;
use crate::error::HostError;
use crate::keys::encode_key;
use crate::report;
use crate::token;
pub use crate::pane::PaneStatus;
use crate::pane::{ExitHook, LaunchArgs, Pane, Resume};
use crate::state::{PaneRecord, StateFile};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

const MAX_NAME_LEN: usize = 32;
const DEFAULT_ROWS: u16 = 40;
const DEFAULT_COLS: u16 = 120;
const DEFAULT_SCROLLBACK_ROWS: usize = 10_000;
/// Rows and columns each must be in `1..=MAX_DIMENSION`; the screen grid is
/// allocated up front and a zero size panics inside the emulator.
const MAX_DIMENSION: u16 = 1000;
const MAX_SCROLLBACK_ROWS: usize = 100_000;
/// Pause between pasting a prompt and pressing Enter, so a TUI that handles the
/// paste first does not swallow the Enter.
const PROMPT_ENTER_DELAY: Duration = Duration::from_millis(150);
/// How often `wait_state` looks at the screen again.
const STATE_POLL: Duration = Duration::from_millis(100);
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
    /// Which agent this is (`claude`, `codex`, ...), so its state can be
    /// detected from the screen. None for a plain process.
    pub agent: Option<String>,
    /// How to start this agent again after a host restart (for example
    /// `claude --continue`). Without it the agent is not brought back.
    pub resume_argv: Option<Vec<String>>,
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
            agent: None,
            resume_argv: None,
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
    pub agent: Option<String>,
    /// Will be started again after a host restart.
    pub resumable: bool,
    /// The agent's own id for its conversation, once its hooks have reported
    /// it. This is what `--resume` takes for agents that have one.
    pub agent_session_id: Option<String>,
    /// The terminal title the program set, or empty.
    pub title: String,
    /// Detected state; None when the agent kind has no rule file.
    pub state: Option<AgentState>,
    /// The rule that decided the state; None when the default applied.
    pub rule: Option<String>,
    pub cwd: PathBuf,
    pub pid: Option<u32>,
    pub status: PaneStatus,
    pub rows: u16,
    pub cols: u16,
    pub bytes_seen: u64,
    pub quiet_for: Duration,
    pub age: Duration,
}

/// Environment variables a pane gets so its hooks can reach the host.
pub const PANE_TOKEN_ENV: &str = "HQ_HOST_TOKEN";
pub const RUN_DIR_ENV: &str = "HQ_HOST_DIR";

type Registry = Arc<Mutex<BTreeMap<String, Arc<Pane>>>>;
/// Agents restored from the state file that cannot start until their
/// environment is supplied again.
type Awaiting = Arc<Mutex<BTreeMap<String, PaneRecord>>>;

/// An agent that is waiting for its environment before it can resume.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AwaitingInfo {
    pub name: String,
    pub agent: Option<String>,
    pub cwd: PathBuf,
    /// The variables `Host::resume` must supply.
    pub env_keys: Vec<String>,
}

pub struct Host {
    panes: Registry,
    awaiting: Awaiting,
    /// Per-agent secret each pane gets so its hooks can report on itself and
    /// nothing else. Kept in memory only.
    pane_tokens: Mutex<BTreeMap<String, String>>,
    /// Where the control socket lives, passed to panes so hooks can find it.
    run_dir: Mutex<Option<PathBuf>>,
    detector: Detector,
    state: Option<Arc<StateFile>>,
}

/// What `Host::restore` did.
#[derive(Debug, Default)]
pub struct RestoreReport {
    pub restored: Vec<String>,
    /// `(name, reason)`; the name is `session.json` when the file itself was refused.
    pub skipped: Vec<(String, String)>,
}

impl Default for Host {
    fn default() -> Self {
        Self::new()
    }
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
        Self::with_detector(Detector::builtin())
    }

    /// A host that detects agent state with `detector` (for example one with
    /// local rule-file overrides loaded).
    pub fn with_detector(detector: Detector) -> Self {
        Self {
            panes: Arc::new(Mutex::new(BTreeMap::new())),
            awaiting: Arc::new(Mutex::new(BTreeMap::new())),
            pane_tokens: Mutex::new(BTreeMap::new()),
            run_dir: Mutex::new(None),
            detector,
            state: None,
        }
    }

    /// A host that remembers its resumable agents in `<dir>/session.json`.
    pub fn with_state_dir(mut self, dir: &Path) -> Self {
        self.state = Some(Arc::new(StateFile::new(dir)));
        self
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
        check_size(spec.rows, spec.cols)?;
        if lock(&self.panes).contains_key(&spec.name)
            || lock(&self.awaiting).contains_key(&spec.name)
        {
            return Err(HostError::NameTaken(spec.name));
        }
        // Start the process without holding the registry lock: looking up the
        // program and forking can be slow and must not stall every other call.
        let scrollback_rows = spec.scrollback_rows.min(MAX_SCROLLBACK_ROWS);
        let resume = match spec.resume_argv {
            Some(argv) if argv.is_empty() => return Err(HostError::InvalidResume),
            Some(argv) => Some(Resume {
                argv,
                env: spec.env.clone(),
                scrollback_rows,
            }),
            None => None,
        };
        let token = token::random_hex().map_err(|e| HostError::Io(e.to_string()))?;
        let mut env = pane_env(&spec.env);
        env.push((PANE_TOKEN_ENV.to_string(), token.clone()));
        if let Some(dir) = lock(&self.run_dir).as_ref() {
            env.push((RUN_DIR_ENV.to_string(), dir.to_string_lossy().into_owned()));
        }
        let pane = Pane::spawn(
            LaunchArgs {
                argv: spec.argv,
                resume,
                on_exit: self.exit_hook(),
                agent: spec.agent,
                cwd: spec.cwd,
                env,
                rows: spec.rows,
                cols: spec.cols,
            },
            Box::new(VtEmulator::new(spec.rows, spec.cols, scrollback_rows)),
        )?;
        let pane = Arc::new(pane);
        let mut panes = lock(&self.panes);
        if panes.contains_key(&spec.name) {
            pane.kill();
            return Err(HostError::NameTaken(spec.name));
        }
        panes.insert(spec.name.clone(), pane.clone());
        drop(panes);
        lock(&self.pane_tokens).insert(spec.name.clone(), token);
        self.save();
        Ok(info_of(&spec.name, pane.as_ref(), &self.detector))
    }

    /// Rewrites the state file with the agents that are running and can be
    /// resumed. A failure is reported on stderr and never fails the caller: an
    /// agent that cannot be remembered still runs.
    fn save(&self) {
        save_state(&self.panes, &self.awaiting, self.state.as_deref());
    }

    fn exit_hook(&self) -> Option<ExitHook> {
        let state = self.state.clone()?;
        let panes = self.panes.clone();
        let awaiting = self.awaiting.clone();
        Some(Arc::new(move || {
            save_state(&panes, &awaiting, Some(&state))
        }))
    }

    /// Starts again every agent the state file lists, using its resume
    /// command. Call once, before serving.
    pub fn restore(&self) -> RestoreReport {
        let mut report = RestoreReport::default();
        let Some(state) = &self.state else {
            return report;
        };
        let records = match state.read() {
            Ok(records) => records,
            Err(e) => {
                report
                    .skipped
                    .push(("session.json".to_string(), e.to_string()));
                return report;
            }
        };
        for rec in records {
            if !rec.env_keys.is_empty() {
                lock(&self.awaiting).insert(rec.name.clone(), rec);
                continue;
            }
            let name = rec.name.clone();
            match self.spawn(spec_of(rec, Vec::new())) {
                Ok(_) => report.restored.push(name),
                Err(e) => report.skipped.push((name, e.to_string())),
            }
        }
        report
    }

    /// Agents restored from the state file that wait for their environment.
    pub fn awaiting(&self) -> Vec<AwaitingInfo> {
        lock(&self.awaiting)
            .values()
            .map(|r| AwaitingInfo {
                name: r.name.clone(),
                agent: r.agent.clone(),
                cwd: r.cwd.clone(),
                env_keys: r.env_keys.clone(),
            })
            .collect()
    }

    /// Starts a waiting agent with the environment its starter supplies again.
    /// It stays waiting if a variable is missing or the start fails.
    pub fn resume(&self, name: &str, env: Vec<(String, String)>) -> Result<PaneInfo, HostError> {
        let rec = lock(&self.awaiting)
            .get(name)
            .cloned()
            .ok_or_else(|| HostError::NotFound(name.to_string()))?;
        let missing: Vec<&str> = rec
            .env_keys
            .iter()
            .filter(|k| !env.iter().any(|(have, _)| have == *k))
            .map(String::as_str)
            .collect();
        if !missing.is_empty() {
            return Err(HostError::MissingEnv {
                name: name.to_string(),
                missing: missing.join(", "),
            });
        }
        // Spawn sees the name as taken while it is waiting, so release it first
        // and put it back if the start fails.
        lock(&self.awaiting).remove(name);
        match self.spawn(spec_of(rec.clone(), env)) {
            Ok(info) => Ok(info),
            Err(e) => {
                lock(&self.awaiting).insert(rec.name.clone(), rec);
                Err(e)
            }
        }
    }

    /// Keeps the state file as it is, then stops every agent. Call when the
    /// host is going away: the agents are stopped but still listed, so the next
    /// start brings them back.
    pub fn shutdown(&self) {
        self.save();
        if let Some(state) = &self.state {
            state.freeze();
        }
        let panes: Vec<Arc<Pane>> = lock(&self.panes).values().cloned().collect();
        for pane in panes {
            pane.kill();
        }
    }

    /// Tells the host where its control socket is, so panes can be given the
    /// way to reach it. Called by the server when it binds.
    pub fn set_run_dir(&self, dir: &Path) {
        *lock(&self.run_dir) = Some(dir.to_path_buf());
    }

    /// The agent a pane token belongs to, if that agent still exists.
    pub fn agent_for_token(&self, given: &str) -> Option<String> {
        let tokens = lock(&self.pane_tokens);
        let name = tokens
            .iter()
            .find(|(_, t)| token::matches(t, given))
            .map(|(n, _)| n.clone())?;
        drop(tokens);
        lock(&self.panes).contains_key(&name).then_some(name)
    }

    /// Replaces the command that brings a resumable agent back after a host
    /// restart, for example once its conversation id is known.
    pub fn set_resume(&self, name: &str, argv: Vec<String>) -> Result<(), HostError> {
        let pane = self.pane(name)?;
        if argv.is_empty() || pane.resume.is_none() {
            return Err(HostError::InvalidResume);
        }
        pane.set_resume_argv(argv);
        self.save();
        Ok(())
    }

    /// Records what an agent's hook said: the state it implies, and the id of
    /// the agent's own conversation. An event that says nothing is ignored.
    pub fn report(
        &self,
        name: &str,
        event: &str,
        notification_type: Option<&str>,
        session_id: Option<String>,
    ) -> Result<(), HostError> {
        let pane = self.pane(name)?;
        pane.record_report(report::state_for(event, notification_type), event, session_id);
        Ok(())
    }

    pub fn info(&self, name: &str) -> Result<PaneInfo, HostError> {
        Ok(info_of(name, self.pane(name)?.as_ref(), &self.detector))
    }

    pub fn list(&self) -> Vec<PaneInfo> {
        lock(&self.panes)
            .iter()
            .map(|(n, p)| info_of(n, p, &self.detector))
            .collect()
    }

    /// Forgets an agent and stops its process and the processes it started.
    pub fn remove(&self, name: &str) -> Result<(), HostError> {
        let Some(pane) = lock(&self.panes).remove(name) else {
            if lock(&self.awaiting).remove(name).is_some() {
                self.save();
                return Ok(());
            }
            return Err(HostError::NotFound(name.to_string()));
        };
        self.save();
        // Other threads may still hold the pane (a caller waiting on it), so
        // stopping it cannot be left to the last reference being dropped.
        pane.kill();
        Ok(())
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
        check_size(rows, cols)?;
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

    /// Waits until the detected state is one of `states` and has held for
    /// `stable`, so a flicker between two screens is not taken for a change.
    /// An agent kind without a rule file never matches and times out.
    pub fn wait_state(
        &self,
        name: &str,
        states: &[AgentState],
        stable: Duration,
        timeout: Duration,
    ) -> Result<PaneInfo, HostError> {
        let deadline = Instant::now() + timeout;
        let mut since: Option<(AgentState, Instant)> = None;
        loop {
            let info = self.info(name)?;
            match info.state {
                Some(state) if states.contains(&state) => {
                    let held = match since {
                        Some((s, at)) if s == state => at,
                        _ => Instant::now(),
                    };
                    since = Some((state, held));
                    if held.elapsed() >= stable {
                        return Ok(info);
                    }
                }
                _ => since = None,
            }
            if info.status != PaneStatus::Running && info.state.is_none() {
                return Err(HostError::Exited(name.to_string()));
            }
            if Instant::now() >= deadline {
                return Err(HostError::Timeout(timeout));
            }
            std::thread::sleep(STATE_POLL);
        }
    }

    pub fn kill(&self, name: &str) -> Result<(), HostError> {
        self.pane(name)?.kill();
        // Do not wait for the exit: a host that dies now must not bring it back.
        self.save();
        Ok(())
    }
}

/// Writes the resumable running agents to the state file, if there is one.
fn save_state(panes: &Registry, awaiting: &Awaiting, state: Option<&StateFile>) {
    let Some(state) = state else { return };
    let waiting: Vec<PaneRecord> = lock(awaiting).values().cloned().collect();
    let records: Vec<PaneRecord> = lock(panes)
        .iter()
        .filter(|(_, pane)| pane.status() == PaneStatus::Running && !pane.is_stopping())
        .filter_map(|(name, pane)| {
            let resume = pane.resume.as_ref()?;
            let resume_argv = pane.resume_argv()?;
            let (rows, cols) = pane.size();
            Some(PaneRecord {
                name: name.clone(),
                resume_argv,
                agent: pane.agent.clone(),
                cwd: pane.cwd.clone(),
                env_keys: resume.env.iter().map(|(k, _)| k.clone()).collect(),
                rows,
                cols,
                scrollback_rows: resume.scrollback_rows,
            })
        })
        .chain(waiting)
        .collect();
    if let Err(e) = state.write(&records) {
        eprintln!("hq host: could not save session.json: {e}");
    }
}

fn spec_of(rec: PaneRecord, env: Vec<(String, String)>) -> SpawnSpec {
    let mut spec = SpawnSpec::new(rec.name, rec.resume_argv.clone(), rec.cwd);
    spec.agent = rec.agent;
    spec.env = env;
    spec.resume_argv = Some(rec.resume_argv);
    spec.rows = rec.rows;
    spec.cols = rec.cols;
    spec.scrollback_rows = rec.scrollback_rows;
    spec
}

fn check_size(rows: u16, cols: u16) -> Result<(), HostError> {
    let ok = |n: u16| (1..=MAX_DIMENSION).contains(&n);
    if ok(rows) && ok(cols) {
        Ok(())
    } else {
        Err(HostError::InvalidSize {
            rows,
            cols,
            max: MAX_DIMENSION,
        })
    }
}

/// The screen as detection sees it: visible rows as text, plus the title.
fn screen_of(pane: &Pane) -> (String, String) {
    pane.with_emu(|e| {
        let rows: Vec<String> = e.visible().into_iter().map(|r| r.text).collect();
        (rows.join("\n"), e.title())
    })
}

fn info_of(name: &str, pane: &Pane, detector: &Detector) -> PaneInfo {
    let (rows, cols) = pane.size();
    let detection = pane.agent.as_deref().and_then(|agent| {
        let (screen, title) = screen_of(pane);
        detector.detect(
            agent,
            Input {
                screen: &screen,
                osc_title: &title,
            },
        )
    });
    let screen = detection.as_ref().map(|d| d.state);
    let reported = pane.reported();
    let (state, from_hook) = report::combine(
        reported.as_ref().map(|r| r.state),
        screen,
        pane.quiet_for(),
    );
    let rule = match (&reported, from_hook) {
        (Some(r), true) => Some(format!("hook:{}", r.event)),
        _ => detection.and_then(|d| d.rule),
    };
    PaneInfo {
        name: name.to_string(),
        argv: pane.argv.clone(),
        agent: pane.agent.clone(),
        resumable: pane.resume.is_some(),
        agent_session_id: pane.agent_session_id(),
        title: pane.with_emu(|e| e.title()),
        state,
        rule,
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
