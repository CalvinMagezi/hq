//! One agent process in a pseudo-terminal, with its emulated screen.

use crate::emu::{Emulator, Row};
use crate::error::HostError;
use nix::sys::signal::{Signal, killpg};
use nix::unistd::Pid;
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

const READ_CHUNK: usize = 16 * 1024;
/// After SIGHUP, how long a process group gets to exit before SIGKILL.
const KILL_GRACE: Duration = Duration::from_secs(2);
/// After the process exits, how long to wait for the reader to drain what it
/// printed last. Descendants that keep the terminal open must not block this.
const DRAIN_GRACE: Duration = Duration::from_millis(300);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneStatus {
    Running,
    Exited { code: u32 },
}

struct State {
    last_output: Instant,
    bytes: u64,
    exit: Option<u32>,
    reader_done: bool,
}

struct Shared {
    emu: Mutex<Box<dyn Emulator>>,
    state: Mutex<State>,
    changed: Condvar,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Shared {
    /// Blocks until the process exits, returning its exit code.
    fn wait_exit(&self, timeout: Duration) -> Result<u32, HostError> {
        let deadline = Instant::now() + timeout;
        let mut state = lock(&self.state);
        loop {
            if let Some(code) = state.exit {
                return Ok(code);
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(HostError::Timeout(timeout));
            }
            state = self
                .changed
                .wait_timeout(state, left)
                .unwrap_or_else(|p| p.into_inner())
                .0;
        }
    }
}

/// Called once a pane's process has exited and its output has been read.
pub(crate) type ExitHook = Arc<dyn Fn() + Send + Sync>;

/// How to start a pane again after a host restart.
#[derive(Clone)]
pub(crate) struct Resume {
    pub argv: Vec<String>,
    pub env: Vec<(String, String)>,
    pub scrollback_rows: usize,
}

/// A state an agent reported through a hook, and the event that said so.
#[derive(Debug, Clone)]
pub(crate) struct Reported {
    pub state: crate::detect::AgentState,
    pub event: String,
}

pub(crate) struct Pane {
    pub(crate) argv: Vec<String>,
    pub(crate) resume: Option<Resume>,
    pub(crate) agent: Option<String>,
    pub(crate) cwd: PathBuf,
    pub(crate) pid: Option<u32>,
    pub(crate) started: Instant,
    shared: Arc<Shared>,
    /// Someone asked this process to stop; it may take a moment to exit.
    stopping: AtomicBool,
    /// Sequence number of the last state-change event for this agent: a counter
    /// that only goes up, for clients that alert once per change.
    state_seq: AtomicU64,
    /// A turn finished and the agent has not worked since.
    done: AtomicBool,
    /// The last state an event was sent for; None before the first one. Held
    /// while a change is announced, so the watcher and a hook report cannot
    /// announce the same change twice.
    announced: Mutex<Option<crate::detect::AgentState>>,
    /// A later command to resume with, replacing `resume.argv`.
    resume_argv: Mutex<Option<Vec<String>>>,
    /// What the agent last said about itself through a hook.
    reported: Mutex<Option<Reported>>,
    /// The agent's own id for its conversation, as its hooks reported it.
    session_id: Mutex<Option<String>>,
    writer: Mutex<Box<dyn Write + Send>>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
}

pub(crate) struct LaunchArgs {
    pub argv: Vec<String>,
    pub resume: Option<Resume>,
    pub on_exit: Option<ExitHook>,
    pub agent: Option<String>,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
    pub rows: u16,
    pub cols: u16,
}

impl Pane {
    pub(crate) fn spawn(args: LaunchArgs, emu: Box<dyn Emulator>) -> Result<Self, HostError> {
        let command = args.argv.join(" ");
        let spawn_err = |detail: String| HostError::Spawn {
            command: command.clone(),
            detail,
        };
        let (program, rest) = args
            .argv
            .split_first()
            .ok_or_else(|| spawn_err("empty command".into()))?;
        // The pty library quietly starts the process in the home directory when
        // the working directory is missing, so check it here.
        if !args.cwd.is_absolute() || !args.cwd.is_dir() {
            return Err(spawn_err(format!(
                "working directory {} is not an existing absolute directory",
                args.cwd.display()
            )));
        }

        let pair = native_pty_system()
            .openpty(PtySize {
                rows: args.rows,
                cols: args.cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| spawn_err(e.to_string()))?;
        // Take both ends before starting the process, so a failure here cannot
        // leave an untracked child behind.
        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| spawn_err(e.to_string()))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| spawn_err(e.to_string()))?;
        let mut builder = CommandBuilder::new(program);
        builder.args(rest);
        builder.cwd(&args.cwd);
        builder.env_clear();
        for (k, v) in &args.env {
            builder.env(k, v);
        }
        let child = pair
            .slave
            .spawn_command(builder)
            .map_err(|e| spawn_err(e.to_string()))?;
        drop(pair.slave);
        let pid = child.process_id();
        let killer = child.clone_killer();

        let shared = Arc::new(Shared {
            emu: Mutex::new(emu),
            state: Mutex::new(State {
                last_output: Instant::now(),
                bytes: 0,
                exit: None,
                reader_done: false,
            }),
            changed: Condvar::new(),
        });
        spawn_reader(reader, shared.clone());
        spawn_waiter(child, shared.clone(), args.on_exit);

        Ok(Self {
            argv: args.argv,
            resume: args.resume,
            agent: args.agent,
            cwd: args.cwd,
            pid,
            started: Instant::now(),
            shared,
            stopping: AtomicBool::new(false),
            state_seq: AtomicU64::new(0),
            done: AtomicBool::new(false),
            announced: Mutex::new(None),
            resume_argv: Mutex::new(None),
            reported: Mutex::new(None),
            session_id: Mutex::new(None),
            writer: Mutex::new(writer),
            master: Mutex::new(pair.master),
            killer: Mutex::new(killer),
        })
    }

    pub(crate) fn status(&self) -> PaneStatus {
        match lock(&self.shared.state).exit {
            Some(code) => PaneStatus::Exited { code },
            None => PaneStatus::Running,
        }
    }

    pub(crate) fn bytes_seen(&self) -> u64 {
        lock(&self.shared.state).bytes
    }

    pub(crate) fn quiet_for(&self) -> Duration {
        lock(&self.shared.state).last_output.elapsed()
    }

    pub(crate) fn size(&self) -> (u16, u16) {
        lock(&self.shared.emu).size()
    }

    pub(crate) fn with_emu<T>(&self, f: impl FnOnce(&mut dyn Emulator) -> T) -> T {
        f(lock(&self.shared.emu).as_mut())
    }

    pub(crate) fn rows(&self, whole_history: bool) -> Vec<Row> {
        self.with_emu(|e| {
            if whole_history {
                e.history()
            } else {
                e.visible()
            }
        })
    }

    pub(crate) fn write(&self, bytes: &[u8]) -> Result<(), HostError> {
        let mut w = lock(&self.writer);
        w.write_all(bytes)
            .and_then(|()| w.flush())
            .map_err(|e| HostError::Io(e.to_string()))
    }

    pub(crate) fn resize(&self, rows: u16, cols: u16) -> Result<(), HostError> {
        lock(&self.master)
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| HostError::Io(e.to_string()))?;
        lock(&self.shared.emu).resize(rows, cols);
        Ok(())
    }

    /// Stops the process and everything it started: SIGHUP to the process
    /// group (the agent is its own session leader), then SIGKILL to the group
    /// if the leader is still alive after a grace period. Does nothing once
    /// the process has exited, so a recycled pid is never signalled.
    pub(crate) fn record_report(
        &self,
        state: Option<crate::detect::AgentState>,
        event: &str,
        session_id: Option<String>,
    ) {
        if let Some(state) = state {
            *lock(&self.reported) = Some(Reported {
                state,
                event: event.to_string(),
            });
        }
        if let Some(id) = session_id.filter(|id| !id.is_empty()) {
            *lock(&self.session_id) = Some(id);
        }
    }

    pub(crate) fn set_state_seq(&self, seq: u64) {
        self.state_seq.store(seq, Ordering::SeqCst);
    }

    pub(crate) fn announced(&self) -> MutexGuard<'_, Option<crate::detect::AgentState>> {
        lock(&self.announced)
    }

    pub(crate) fn set_done(&self, done: bool) {
        self.done.store(done, Ordering::SeqCst);
    }

    pub(crate) fn is_done(&self) -> bool {
        self.done.load(Ordering::SeqCst)
    }

    pub(crate) fn state_seq(&self) -> u64 {
        self.state_seq.load(Ordering::SeqCst)
    }

    pub(crate) fn set_resume_argv(&self, argv: Vec<String>) {
        *lock(&self.resume_argv) = Some(argv);
    }

    /// The command that brings this agent back, if it can be brought back.
    pub(crate) fn resume_argv(&self) -> Option<Vec<String>> {
        let own = self.resume.as_ref()?;
        Some(lock(&self.resume_argv).clone().unwrap_or_else(|| own.argv.clone()))
    }

    pub(crate) fn reported(&self) -> Option<Reported> {
        lock(&self.reported).clone()
    }

    pub(crate) fn agent_session_id(&self) -> Option<String> {
        lock(&self.session_id).clone()
    }

    pub(crate) fn is_stopping(&self) -> bool {
        self.stopping.load(Ordering::SeqCst)
    }

    pub(crate) fn kill(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        if lock(&self.shared.state).exit.is_some() {
            return;
        }
        let Some(pid) = self.pid.and_then(|p| i32::try_from(p).ok()) else {
            let _ = lock(&self.killer).kill();
            return;
        };
        let group = Pid::from_raw(pid);
        let _ = killpg(group, Signal::SIGHUP);
        let shared = self.shared.clone();
        std::thread::spawn(move || {
            if shared.wait_exit(KILL_GRACE).is_err() {
                let _ = killpg(group, Signal::SIGKILL);
            }
        });
    }

    pub(crate) fn wait_exit(&self, timeout: Duration) -> Result<u32, HostError> {
        self.shared.wait_exit(timeout)
    }

    /// Blocks until the pane has printed nothing for `quiet`, or exits.
    pub(crate) fn wait_quiet(&self, quiet: Duration, timeout: Duration) -> Result<(), HostError> {
        let deadline = Instant::now() + timeout;
        let mut state = lock(&self.shared.state);
        loop {
            if state.exit.is_some() || state.last_output.elapsed() >= quiet {
                return Ok(());
            }
            let until_quiet = quiet.saturating_sub(state.last_output.elapsed());
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(HostError::Timeout(timeout));
            }
            state = self
                .shared
                .changed
                .wait_timeout(state, until_quiet.min(left))
                .unwrap_or_else(|p| p.into_inner())
                .0;
        }
    }
}

impl Drop for Pane {
    fn drop(&mut self) {
        self.kill();
    }
}

fn spawn_reader(mut reader: Box<dyn Read + Send>, shared: Arc<Shared>) {
    std::thread::spawn(move || {
        let mut buf = vec![0u8; READ_CHUNK];
        while let Ok(n) = reader.read(&mut buf) {
            if n == 0 {
                break;
            }
            lock(&shared.emu).process(&buf[..n]);
            let mut state = lock(&shared.state);
            state.last_output = Instant::now();
            state.bytes += n as u64;
            drop(state);
            shared.changed.notify_all();
        }
        lock(&shared.state).reader_done = true;
        shared.changed.notify_all();
    });
}

fn spawn_waiter(
    mut child: Box<dyn portable_pty::Child + Send + Sync>,
    shared: Arc<Shared>,
    on_exit: Option<ExitHook>,
) {
    std::thread::spawn(move || {
        let code = child.wait().map(|s| s.exit_code()).unwrap_or(u32::MAX);
        // Publish the exit only after the reader has taken the last output, so
        // a caller that sees the exit also sees everything the process printed.
        let deadline = Instant::now() + DRAIN_GRACE;
        let mut state = lock(&shared.state);
        while !state.reader_done {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            state = shared
                .changed
                .wait_timeout(state, left)
                .unwrap_or_else(|p| p.into_inner())
                .0;
        }
        state.exit = Some(code);
        drop(state);
        shared.changed.notify_all();
        if let Some(hook) = on_exit {
            hook();
        }
    });
}
