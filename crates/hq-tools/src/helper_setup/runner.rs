//! Runs one program on a host in a pane, follows what it prints, and removes the pane. The one
//! primitive under installing, signing in and checking a helper. Blocking: callers use
//! `agent_host::blocking`.

use std::time::{Duration, Instant};

use crate::agent_host::{AgentHostError, AgentInfo, CommandRun, HostBackend};

/// How often a running pane is looked at.
const POLL: Duration = if cfg!(test) {
    Duration::from_millis(10)
} else {
    Duration::from_secs(2)
};
/// Pane lines kept as the tail.
const TAIL_LINES: usize = 40;
/// Links kept from what a program printed.
const MAX_URLS: usize = 4;
const MAX_TAIL_CHARS: usize = 4000;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Outcome {
    /// `None` when the program did not end in time.
    pub exit_code: Option<u32>,
    pub timed_out: bool,
    pub tail: String,
    pub urls: Vec<String>,
}

impl Outcome {
    pub fn ok(&self) -> bool {
        self.exit_code == Some(0)
    }
}

/// The small slice of a host the runner needs, so a test can stand in for one.
pub trait RunHost {
    fn start(&self, run: &CommandRun) -> Result<(), AgentHostError>;
    fn info(&self, name: &str) -> Result<Option<AgentInfo>, AgentHostError>;
    fn screen(&self, name: &str, lines: usize) -> Result<String, AgentHostError>;
    fn remove(&self, name: &str) -> Result<(), AgentHostError>;
}

impl RunHost for dyn HostBackend + '_ {
    fn start(&self, run: &CommandRun) -> Result<(), AgentHostError> {
        self.run_command(run)
    }
    fn info(&self, name: &str) -> Result<Option<AgentInfo>, AgentHostError> {
        self.agent(name)
    }
    fn screen(&self, name: &str, lines: usize) -> Result<String, AgentHostError> {
        self.read(name, lines)
    }
    fn remove(&self, name: &str) -> Result<(), AgentHostError> {
        self.close_workspace(name)
    }
}

/// Runs `run` and waits up to `timeout` for it to end. `progress` hears the tail and the links
/// each time they change, so an app can show a sign-in link while the program waits for the
/// person. A pane that vanished before it said how it ended counts as a failure.
pub fn run_to_end(
    host: &(impl RunHost + ?Sized),
    run: &CommandRun,
    timeout: Duration,
    mut progress: impl FnMut(&str, &[String]),
) -> Result<Outcome, AgentHostError> {
    host.start(run)?;
    let deadline = Instant::now() + timeout;
    let mut last = (String::new(), Vec::new());
    let outcome = loop {
        std::thread::sleep(POLL.min(timeout));
        let info = host.info(&run.name)?;
        let ended = info.as_ref().is_none_or(|i| i.exited);
        let (tail, urls) = look(host, &run.name);
        if (tail.as_str(), urls.as_slice()) != (last.0.as_str(), last.1.as_slice()) {
            progress(&tail, &urls);
            last = (tail.clone(), urls.clone());
        }
        if ended {
            break Outcome {
                exit_code: info.and_then(|i| i.exit_code),
                timed_out: false,
                tail,
                urls,
            };
        }
        if Instant::now() >= deadline {
            break Outcome {
                exit_code: None,
                timed_out: true,
                tail,
                urls,
            };
        }
    };
    if let Err(e) = host.remove(&run.name) {
        tracing::warn!(pane = %run.name, error = %e, "could not remove a setup pane");
    }
    Ok(outcome)
}

fn look(host: &(impl RunHost + ?Sized), name: &str) -> (String, Vec<String>) {
    let screen = host.screen(name, TAIL_LINES).unwrap_or_default();
    (clean(&screen), urls_in(&screen))
}

/// The screen as text to show: secrets-shaped strings removed, then cut to the end.
fn clean(screen: &str) -> String {
    let text = hq_core::redact::redact_secrets(screen.trim_end());
    let skip = text.chars().count().saturating_sub(MAX_TAIL_CHARS);
    text.chars().skip(skip).collect()
}

/// Web links in `text`, the way a sign-in page is shown by a program with no browser to open.
pub fn urls_in(text: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for word in text.split(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>')) {
        let Some(start) = word.find("https://") else {
            continue;
        };
        let url = word[start..].trim_end_matches(['.', ',', ')', ']', ';']);
        if url.len() > "https://".len() && !found.iter().any(|u| u == url) {
            found.push(url.to_string());
        }
        if found.len() == MAX_URLS {
            break;
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn links_are_found_and_trimmed_and_repeats_dropped() {
        let text = "Open https://auth.example.com/login?code=ab12. then\n(see https://example.com/help) https://auth.example.com/login?code=ab12";
        assert_eq!(
            urls_in(text),
            [
                "https://auth.example.com/login?code=ab12",
                "https://example.com/help"
            ]
        );
        assert!(urls_in("no links, http://insecure.example.com only").is_empty());
    }

    /// A pane that prints, then ends after `ends_after` looks.
    struct Fake {
        looks: RefCell<u32>,
        ends_after: u32,
        exit: Option<u32>,
        removed: RefCell<bool>,
    }

    fn info(exited: bool, exit_code: Option<u32>) -> AgentInfo {
        AgentInfo {
            name: Some("hqsetup-1".into()),
            kind: String::new(),
            status: crate::agent_host::AgentStatus::Idle,
            pane_id: "p".into(),
            workspace_id: "w".into(),
            cwd: "/".into(),
            title: None,
            state_change_seq: 0,
            launch_pending: false,
            agent_session_id: None,
            exited,
            exit_code,
            rule: None,
            pending: None,
            pending_seq: None,
            acp: None,
        }
    }

    impl RunHost for Fake {
        fn start(&self, _: &CommandRun) -> Result<(), AgentHostError> {
            Ok(())
        }
        fn info(&self, _: &str) -> Result<Option<AgentInfo>, AgentHostError> {
            *self.looks.borrow_mut() += 1;
            let done = *self.looks.borrow() > self.ends_after;
            Ok(Some(info(done, done.then_some(self.exit).flatten())))
        }
        fn screen(&self, _: &str, _: usize) -> Result<String, AgentHostError> {
            Ok(format!(
                "Sign in at https://auth.example.com/x\nkey {}",
                fake_key()
            ))
        }
        fn remove(&self, _: &str) -> Result<(), AgentHostError> {
            *self.removed.borrow_mut() = true;
            Ok(())
        }
    }

    /// A key-shaped string, built here so no secret-looking literal sits in the source.
    fn fake_key() -> String {
        format!("sk-{}", "a1b2c3d4e5".repeat(4))
    }

    fn run() -> CommandRun {
        CommandRun {
            name: "hqsetup-1".into(),
            argv: vec!["true".into()],
            env: vec![],
        }
    }

    #[test]
    fn a_run_that_ends_reports_its_exit_its_link_and_is_cleaned_up() {
        let host = Fake {
            looks: 0.into(),
            ends_after: 0,
            exit: Some(0),
            removed: false.into(),
        };
        let mut heard = Vec::new();
        let out = run_to_end(&host, &run(), Duration::from_secs(30), |_, urls| {
            heard.push(urls.to_vec())
        })
        .unwrap();
        assert!(out.ok());
        assert_eq!(out.urls, ["https://auth.example.com/x"]);
        assert!(!out.tail.contains(&fake_key()), "{}", out.tail);
        assert_eq!(heard.len(), 1);
        assert!(*host.removed.borrow());
    }

    #[test]
    fn a_run_that_never_ends_times_out_and_is_removed() {
        let host = Fake {
            looks: 0.into(),
            ends_after: u32::MAX,
            exit: None,
            removed: false.into(),
        };
        let out = run_to_end(&host, &run(), Duration::from_millis(1), |_, _| {}).unwrap();
        assert!(out.timed_out && !out.ok());
        assert!(*host.removed.borrow());
    }

    #[test]
    fn a_pane_that_ended_without_saying_how_is_not_a_success() {
        let host = Fake {
            looks: 0.into(),
            ends_after: 0,
            exit: None,
            removed: false.into(),
        };
        let out = run_to_end(&host, &run(), Duration::from_secs(30), |_, _| {}).unwrap();
        assert!(!out.ok() && !out.timed_out);
    }
}
