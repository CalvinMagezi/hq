//! Helper setup against a real in-process host and real processes: a program run in a pane is
//! followed to its end, a missing program is found missing, and a present one is found.
#![cfg(unix)]

use hq_host::{Host, Server, StopHandle};
use hq_tools::agent_host::{CommandRun, HostBackend, NativeBackend};
use hq_tools::helper_setup::runner::run_to_end;
use hq_tools::helper_setup::service::detect;
use std::sync::Arc;
use std::time::Duration;

struct Running {
    dir: tempfile::TempDir,
    stop: StopHandle,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Running {
    fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let server = Server::bind(&dir.path().join("run"), Arc::new(Host::new())).unwrap();
        let stop = server.stop_handle();
        let thread = std::thread::spawn(move || server.serve());
        Self {
            dir,
            stop,
            thread: Some(thread),
        }
    }

    fn backend(&self) -> NativeBackend {
        NativeBackend::new(self.dir.path().join("run"))
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.stop.stop();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn sh(name: &str, script: &str) -> CommandRun {
    CommandRun {
        name: name.into(),
        argv: vec!["sh".into(), "-c".into(), script.into()],
        env: vec![],
    }
}

const WAIT: Duration = Duration::from_secs(20);

#[test]
fn a_program_run_on_the_host_reports_what_it_printed_and_how_it_ended() {
    let host = Running::start();
    let backend = host.backend();
    let host: &dyn HostBackend = &backend;
    let out = run_to_end(
        host,
        &sh(
            "hqsetup-ok",
            "echo 'sign in at https://auth.example.com/go'; exit 0",
        ),
        WAIT,
        |_, _| {},
    )
    .unwrap();
    assert!(out.ok(), "{out:?}");
    assert_eq!(out.urls, ["https://auth.example.com/go"]);
    assert!(out.tail.contains("sign in at"), "{}", out.tail);
    // The pane is gone afterwards.
    assert!(host.agent("hqsetup-ok").unwrap().is_none());

    let failed = run_to_end(
        host,
        &sh("hqsetup-bad", "echo nope; exit 3"),
        WAIT,
        |_, _| {},
    )
    .unwrap();
    assert_eq!(failed.exit_code, Some(3));
    assert!(!failed.ok());
}

#[test]
fn a_program_that_never_ends_is_stopped_at_the_time_limit() {
    let host = Running::start();
    let backend = host.backend();
    let host: &dyn HostBackend = &backend;
    let out = run_to_end(
        host,
        &sh("hqsetup-slow", "sleep 60"),
        Duration::from_secs(3),
        |_, _| {},
    )
    .unwrap();
    assert!(out.timed_out && !out.ok(), "{out:?}");
    assert!(host.agent("hqsetup-slow").unwrap().is_none());
}

#[test]
fn programs_are_found_on_the_computer_and_missing_ones_are_not() {
    let host = Running::start();
    let backend = host.backend();
    let host: &dyn HostBackend = &backend;
    let found = detect(host, &["sh", "definitely-not-installed-xyz"]).unwrap();
    assert!(found.contains("sh"), "{found:?}");
    assert!(!found.contains("definitely-not-installed-xyz"));
}
