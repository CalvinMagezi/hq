//! Runs a real ACP agent through the host, optionally under the open process sandbox. Needs the
//! agent installed and signed in, so it is ignored by default:
//!
//! `HQ_ACP_LIVE="hermes acp" cargo test -p hq-host --test acp_live -- --ignored --nocapture`
//!
//! Set `HQ_ACP_LIVE_SANDBOX=open` to confine it `HQ_ACP_LIVE_WRITABLE` (colon-separated extra paths) and `HQ_ACP_LIVE_MODEL` (an id the agent offers) and `HQ_ACP_LIVE_AGENT` to the agent kind the
//! sandbox tables know (`antigravity`, `copilot`, ...). The agent is asked to write `hello.txt` in a
//! scratch folder; the test answers its dialog if it asks, and checks the file exists.
#![cfg(unix)]

use hq_host::{AgentState, Host, ReadSource, SandboxMode, SandboxSpec, SpawnSpec, Transport};
use std::time::{Duration, Instant};

const WAIT: Duration = Duration::from_secs(180);
const POLL: Duration = Duration::from_millis(200);

#[test]
#[ignore = "needs an ACP agent that is installed and signed in"]
fn a_real_agent_writes_a_file_through_the_host() {
    let command = std::env::var("HQ_ACP_LIVE").expect("set HQ_ACP_LIVE to the agent command");
    let argv: Vec<String> = command.split_whitespace().map(str::to_owned).collect();
    let project = tempfile::tempdir().unwrap();
    let run = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    let host = Host::new();
    host.set_run_dir(run.path());
    let mut spec = SpawnSpec::new("live", argv, project.path());
    spec.transport = Transport::Acp;
    spec.agent = std::env::var("HQ_ACP_LIVE_AGENT").ok();
    spec.model = std::env::var("HQ_ACP_LIVE_MODEL").ok();
    spec.env = vec![("PATH".into(), std::env::var("PATH").unwrap())];
    if let Ok(home) = std::env::var("HOME") {
        spec.env.push(("HOME".into(), home));
    }
    if std::env::var("HQ_ACP_LIVE_SANDBOX").as_deref() == Ok("open") {
        let mut sandbox = SandboxSpec::none();
        sandbox.mode = SandboxMode::Process;
        if let Ok(paths) = std::env::var("HQ_ACP_LIVE_WRITABLE") {
            sandbox.writable = paths.split(':').map(Into::into).collect();
        }
        spec.sandbox = Some(sandbox);
    }
    let info = host
        .spawn(spec)
        .expect("the agent starts and opens a session");
    println!(
        "started: sandbox={} session={:?}",
        info.sandbox, info.agent_session_id
    );
    host.prompt(
        "live",
        "Create a file hello.txt containing the word hi, then reply DONE_LIVE.",
    )
    .unwrap();
    let deadline = Instant::now() + WAIT;
    loop {
        let info = host.info("live").unwrap();
        if let (Some(AgentState::Blocked), Some(seq)) = (info.state, info.pending_seq) {
            println!("dialog: {:?}", info.pending.as_deref().map(|d| &d.text));
            host.answer("live", seq, Some(1)).unwrap();
        }
        if info.done {
            break;
        }
        assert!(Instant::now() < deadline, "no result in time");
        std::thread::sleep(POLL);
    }
    println!("{}", host.read("live", ReadSource::Recent, 0).unwrap());
    assert!(
        project.path().join("hello.txt").exists(),
        "the agent did not write the file"
    );
    host.kill("live").unwrap();
}
