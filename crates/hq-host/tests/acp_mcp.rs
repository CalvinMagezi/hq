//! HQ's MCP connection reaches an ACP agent in its session, only when asked for and only when the
//! agent says it can use an HTTP MCP server.
#![cfg(unix)]

use hq_host::{Host, HostError, McpEndpoint, ReadSource, SpawnSpec, Transport};
use std::time::{Duration, Instant};

const WAIT: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(25);
const URL: &str = "http://127.0.0.1:47480/mcp";
const TOKEN: &str = "session-token-123";

fn spec(name: &str, scenario: &str, mcp: bool) -> SpawnSpec {
    let bin = env!("CARGO_BIN_EXE_hq-host-fake-acp-agent");
    let mut spec = SpawnSpec::new(
        name,
        vec![bin.into(), scenario.into()],
        std::env::temp_dir(),
    );
    spec.transport = Transport::Acp;
    spec.mcp = mcp.then(|| McpEndpoint {
        url: URL.into(),
        token: TOKEN.into(),
    });
    spec
}

fn what_the_agent_was_given(host: &Host, name: &str) -> String {
    host.prompt(name, "say").unwrap();
    let deadline = Instant::now() + WAIT;
    while !host.info(name).unwrap().done {
        assert!(Instant::now() < deadline, "no answer in time");
        std::thread::sleep(POLL);
    }
    host.read(name, ReadSource::Recent, 0).unwrap()
}

#[test]
fn the_session_gets_hqs_server_with_a_bearer_header() {
    let host = Host::new();
    host.spawn(spec("mcp1", "mcp", true)).unwrap();
    let said = what_the_agent_was_given(&host, "mcp1");
    assert!(said.contains(URL) && said.contains("agent-hq"), "{said}");
    assert!(said.contains(&format!("Bearer {TOKEN}")), "{said}");
    host.kill("mcp1").unwrap();
}

#[test]
fn nothing_is_given_when_not_asked_for_or_when_the_agent_cannot_use_http() {
    let host = Host::new();
    host.spawn(spec("mcp2", "mcp", false)).unwrap();
    assert!(what_the_agent_was_given(&host, "mcp2").contains("MCP:[]"));
    host.kill("mcp2").unwrap();
    host.spawn(spec("mcp3", "mcp-no-http", true)).unwrap();
    let said = what_the_agent_was_given(&host, "mcp3");
    assert!(said.contains("MCP:[]") && !said.contains(TOKEN), "{said}");
    host.kill("mcp3").unwrap();
}

#[test]
fn the_token_is_not_in_debug_output_and_a_terminal_agent_is_refused_it() {
    let s = spec("mcp4", "mcp", true);
    assert!(!format!("{s:?}").contains(TOKEN));
    let mut pty = SpawnSpec::new(
        "mcp5",
        vec!["sh".into(), "-c".into(), "sleep 5".into()],
        std::env::temp_dir(),
    );
    pty.mcp = s.mcp;
    let refused = Host::new().spawn(pty);
    assert!(
        matches!(refused, Err(HostError::Unsupported(_))),
        "{refused:?}"
    );
}

#[test]
fn an_oversized_address_or_token_is_refused() {
    let mut s = spec("mcp6", "mcp", true);
    s.mcp = Some(McpEndpoint {
        url: URL.into(),
        token: "t".repeat(1000),
    });
    assert!(matches!(Host::new().spawn(s), Err(HostError::TooLarge(_))));
}
