//! Runs the real Claude Code CLI under the sandbox. Needs `claude` on PATH and a
//! logged-in account, so it is ignored by default:
//! `cargo test -p hq-host --test sandbox_live -- --ignored --nocapture`.
#![cfg(unix)]

use hq_host::{Allow, Host, ReadSource, SandboxMode, SandboxSpec, SpawnSpec};
use std::time::{Duration, Instant};

const WAIT: Duration = Duration::from_secs(120);

fn screen(host: &Host) -> String {
    host.read("live", ReadSource::Visible, 0).unwrap_or_default()
}

#[test]
#[ignore = "needs a logged-in Claude Code and network access"]
fn claude_answers_a_prompt_under_the_sandbox() {
    let project = tempfile::tempdir().unwrap();
    let run = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    let host = Host::new();
    host.set_run_dir(run.path());
    let mut spec = SpawnSpec::new("live", vec!["claude".into(), "--version".into()], project.path());
    spec.argv = vec!["claude".into()];
    spec.agent = Some("claude".into());
    spec.env = vec![("PATH".into(), std::env::var("PATH").unwrap())];
    // Set to run under a named account instead of the default one.
    if let Ok(dir) = std::env::var("HQ_LIVE_CLAUDE_CONFIG_DIR") {
        spec.env.push(("CLAUDE_CONFIG_DIR".into(), dir));
    }
    spec.sandbox = Some(SandboxSpec {
        mode: SandboxMode::Process,
        allow: vec![Allow { host: "api.anthropic.com".into(), ports: vec![443], private: false }],
        writable: Vec::new(),
        readable: Vec::new(),
    });
    host.spawn(spec).unwrap();
    let deadline = Instant::now() + WAIT;
    let mut asked = false;
    loop {
        let text = screen(&host);
        assert!(host.info("live").unwrap().status == hq_host::PaneStatus::Running, "claude exited; screen:\n{}", host.read("live", ReadSource::Recent, 0).unwrap_or_default());
        if asked && text.contains("node v") && text.contains("· done") {
            break;
        }
        if text.contains("trust") && text.contains("Enter") && !asked {
            host.send_keys("live", &["down".into()]).unwrap();
            std::thread::sleep(Duration::from_millis(300));
            host.send_keys("live", &["enter".into()]).unwrap();
        } else if text.contains("Claude Max") && text.contains("❯") && !asked {
            host.prompt("live", "Run the shell command `git --version` and `node --version`, then print the string made by joining PO and NG on its own line.").unwrap();
            asked = true;
        }
        assert!(Instant::now() < deadline, "no answer; screen:\n{text}");
        std::thread::sleep(Duration::from_millis(500));
    }
    for d in host.egress_decisions("live") {
        println!("egress {} {}:{} {}", if d.allowed { "ALLOW" } else { "DENY " }, d.host, d.port, d.reason);
    }
    host.kill("live").unwrap();
}
