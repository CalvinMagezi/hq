//! Keeping the machine awake while agents run (`agent_host.keep_awake: while_sessions`).
//!
//! A laptop that sleeps stops every agent on it, and nobody is told until HQ's unreachable
//! alert fires. The host holds a sleep assertion while at least one agent's process is
//! running and lets go when the last one ends. The assertion is a child process the OS knows
//! (`caffeinate` on macOS, `systemd-inhibit` on Linux) that also watches the host's own pid,
//! so a host that dies without cleaning up does not keep the machine awake for good.

use std::process::{Child, Command, Stdio};

/// The command that holds this platform's sleep assertion until the host (`pid`) exits, or
/// `None` where HQ knows none.
pub fn assertion_command(pid: u32) -> Option<Vec<String>> {
    let pid = pid.to_string();
    if cfg!(target_os = "macos") {
        // -i: no idle sleep. -w: release when that process exits.
        Some(vec!["caffeinate".into(), "-i".into(), "-w".into(), pid])
    } else if cfg!(target_os = "linux") {
        Some(vec![
            "systemd-inhibit".into(),
            "--what=sleep:idle".into(),
            "--who=hq host".into(),
            "--why=Agent sessions are running".into(),
            "--mode=block".into(),
            "tail".into(),
            format!("--pid={pid}"),
            "-f".into(),
            "/dev/null".into(),
        ])
    } else {
        None
    }
}

/// One sleep assertion, held or not.
pub struct KeepAwake {
    argv: Vec<String>,
    child: Option<Child>,
    /// The command could not start; said once, then left alone.
    failed: bool,
}

impl KeepAwake {
    pub fn new(argv: Vec<String>) -> Self {
        KeepAwake {
            argv,
            child: None,
            failed: false,
        }
    }

    /// Holds the assertion while `busy`, lets go of it otherwise. True while it is held.
    pub fn update(&mut self, busy: bool) -> bool {
        // An assertion that ended by itself is gone. One the OS refused (no permission to
        // inhibit sleep on this machine) fails the same way every time, so it is not retried.
        if let Some(child) = &mut self.child {
            match child.try_wait() {
                Ok(None) => {}
                Ok(Some(status)) => {
                    self.child = None;
                    if !status.success() {
                        self.give_up(&format!("it exited with {status}"));
                    }
                }
                Err(_) => self.child = None,
            }
        }
        if !busy {
            self.release();
            return false;
        }
        if self.child.is_none() && !self.failed {
            match self.spawn() {
                Ok(child) => self.child = Some(child),
                Err(e) => self.give_up(&e.to_string()),
            }
        }
        self.child.is_some()
    }

    fn give_up(&mut self, why: &str) {
        self.failed = true;
        eprintln!(
            "hq host could not keep this machine awake with {}: {why}",
            self.argv.first().map_or("", String::as_str)
        );
    }

    pub fn held(&self) -> bool {
        self.child.is_some()
    }

    fn spawn(&self) -> std::io::Result<Child> {
        let (program, args) = self
            .argv
            .split_first()
            .ok_or_else(|| std::io::Error::other("no command"))?;
        Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
    }

    fn release(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for KeepAwake {
    fn drop(&mut self) {
        self.release();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alive(pid: u32) -> bool {
        std::path::Path::new(&format!("/proc/{pid}")).exists()
            || Command::new("kill")
                .args(["-0", &pid.to_string()])
                .status()
                .is_ok_and(|s| s.success())
    }

    #[test]
    fn held_while_busy_and_let_go_after() {
        let mut awake = KeepAwake::new(vec!["sleep".into(), "30".into()]);
        assert!(!awake.update(false));
        assert!(awake.update(true));
        let pid = awake.child.as_ref().unwrap().id();
        assert!(awake.update(true), "held, not started twice");
        assert_eq!(awake.child.as_ref().unwrap().id(), pid);
        assert!(!awake.update(false));
        assert!(!alive(pid), "the assertion is gone once nothing runs");
        assert!(awake.update(true), "and comes back with the next agent");
        let pid = awake.child.as_ref().unwrap().id();
        drop(awake);
        assert!(!alive(pid), "a stopped host lets go");
    }

    #[test]
    fn a_missing_command_is_tried_once() {
        let mut awake = KeepAwake::new(vec!["hq-no-such-sleep-command".into()]);
        assert!(!awake.update(true));
        assert!(awake.failed);
        assert!(!awake.update(true));
    }

    #[test]
    fn an_assertion_that_ended_by_itself_is_started_again() {
        let mut awake = KeepAwake::new(vec!["true".into()]);
        assert!(awake.update(true));
        std::thread::sleep(std::time::Duration::from_millis(200));
        let first = awake.child.as_ref().unwrap().id();
        awake.update(true);
        assert_ne!(awake.child.as_ref().map(Child::id), Some(first));
    }

    #[test]
    fn an_assertion_the_os_refused_is_not_retried() {
        let mut awake = KeepAwake::new(vec!["false".into()]);
        assert!(awake.update(true));
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(!awake.update(true));
        assert!(awake.failed);
    }

    #[test]
    fn the_platform_command_watches_the_host() {
        if let Some(argv) = assertion_command(4242) {
            assert!(argv.iter().any(|a| a.contains("4242")), "{argv:?}");
        }
    }
}
