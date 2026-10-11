//! HQ's kill switch reaches the built-in host and stops what it runs.
#![cfg(unix)]

use hq_host::{Host, Server, SpawnSpec};
use hq_tools::agent_host::{Host as HostHandle, HostBackend, NativeBackend};
use std::sync::Arc;

fn sleeper(name: &str) -> SpawnSpec {
    SpawnSpec::new(
        name,
        vec!["sh".into(), "-c".into(), "sleep 60".into()],
        std::env::temp_dir(),
    )
}

#[test]
fn hq_can_lock_a_host_and_see_it_locked() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("run");
    let host = Arc::new(Host::new().with_state_dir(&dir));
    let server = Server::bind(&dir, host.clone()).unwrap();
    let stop = server.stop_handle();
    let thread = std::thread::spawn(move || server.serve());
    host.spawn(sleeper("running")).unwrap();

    // Through the shared handle every caller holds, so a method it fails to forward shows here.
    let backend: HostHandle = Arc::new(NativeBackend::new(&dir));
    assert!(backend.lockdown_status().unwrap().is_null());
    let held = backend.lockdown("agents nobody asked for").unwrap();
    assert_eq!(held["reason"], "agents nobody asked for");
    assert_eq!(backend.lockdown_status().unwrap()["source"], "operator");
    assert!(
        host.spawn(sleeper("later")).is_err(),
        "a locked host starts nothing"
    );

    stop.stop();
    let _ = thread.join();
    host.shutdown();
}
