//! The sandbox request HQ sends with every launch on a built-in host.

use hq_core::config::{AgentHostConfig, SandboxMode};
use serde_json::{Value, json};

/// The API host Claude Code needs to work at all.
const ANTHROPIC_API_HOST: &str = "api.anthropic.com";
const HTTPS_PORT: u16 = 443;
/// Names a client uses for its own machine when it goes through the proxy.
const LOOPBACK_NAMES: &[&str] = &["127.0.0.1", "::1", "localhost"];

/// `name` or `name:port` from an allow list entry. A bare name means 443.
fn split_entry(entry: &str) -> Option<(String, u16)> {
    let entry = entry.trim();
    if entry.is_empty() {
        return None;
    }
    match entry.rsplit_once(':') {
        Some((host, port)) => Some((host.to_string(), port.parse().ok()?)),
        None => Some((entry.to_string(), HTTPS_PORT)),
    }
}

/// The host and port of the HQ MCP endpoint. It is an address the operator
/// chose, often on a tailnet, so its rule may resolve to a private address.
fn mcp_target(url: &str) -> Option<(String, u16)> {
    let parsed = url::Url::parse(url).ok()?;
    Some((parsed.host_str()?.to_string(), parsed.port_or_known_default()?))
}

const SECS_PER_HOUR: u64 = 3600;

/// Seconds of idleness after which the host stops a session; None when off.
pub(super) fn idle_ttl_secs(cfg: &AgentHostConfig) -> Option<u64> {
    (cfg.idle_reap_hours > 0).then(|| cfg.idle_reap_hours.saturating_mul(SECS_PER_HOUR))
}

pub(super) fn plan(cfg: &AgentHostConfig) -> Value {
    let sandbox = &cfg.sandbox;
    if sandbox.mode == SandboxMode::None {
        return json!({ "mode": "none" });
    }
    let mut allow = vec![json!({ "host": ANTHROPIC_API_HOST, "ports": [HTTPS_PORT] })];
    if let Some((host, port)) = cfg.agent_mcp_url.as_deref().and_then(mcp_target) {
        allow.push(json!({ "host": host, "ports": [port], "private": true }));
    }
    let ports = loopback_ports(&sandbox.loopback_ports);
    // Clients that honour HTTP_PROXY send loopback requests to the proxy, which
    // refuses local addresses unless a rule names them as private. They go
    // before `allow_domains` because the proxy takes the first rule that matches.
    if !ports.is_empty() {
        allow.extend(LOOPBACK_NAMES.iter().map(|host| json!({ "host": host, "ports": ports, "private": true })));
    }
    for entry in &sandbox.allow_domains {
        if entry.trim_start().starts_with("*.") {
            tracing::warn!(entry = %entry, "a wildcard in agent_host.sandbox.allow_domains also allows other sites that share the CDN address; prefer exact hostnames");
        }
        match split_entry(entry) {
            Some((host, port)) => allow.push(json!({ "host": host, "ports": [port] })),
            None => tracing::warn!(entry = %entry, "ignoring a malformed agent_host.sandbox.allow_domains entry"),
        }
    }
    json!({
        "mode": "process",
        "allow": allow,
        "writable": sandbox.writable,
        "readable": sandbox.readable,
        "agent_readable": sandbox.agent_readable,
        "loopback_ports": ports,
    })
}

/// The configured loopback ports without port 0 or repeats.
fn loopback_ports(configured: &[u16]) -> Vec<u16> {
    let mut ports: Vec<u16> = configured.iter().copied().filter(|p| *p != 0).collect();
    ports.sort_unstable();
    ports.dedup();
    ports
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> AgentHostConfig {
        AgentHostConfig::default()
    }

    #[test]
    fn process_is_the_default_and_allows_only_the_api() {
        let p = plan(&cfg());
        assert_eq!(p["mode"], "process");
        assert_eq!(p["allow"], json!([{ "host": "api.anthropic.com", "ports": [443] }]));
    }

    #[test]
    fn the_mcp_endpoint_is_allowed_on_its_own_port_and_may_be_private() {
        let mut c = cfg();
        c.agent_mcp_url = Some("https://hq.example.ts.net:8444/mcp".into());
        let p = plan(&c);
        assert_eq!(p["allow"][1], json!({ "host": "hq.example.ts.net", "ports": [8444], "private": true }));
    }

    #[test]
    fn extra_domains_take_an_optional_port_and_bad_entries_are_dropped() {
        let mut c = cfg();
        c.sandbox.allow_domains = vec!["github.com".into(), "*.npmjs.org:443".into(), "x:notaport".into(), " ".into(), "dev.test:8080".into()];
        let p = plan(&c);
        let hosts: Vec<&str> = p["allow"].as_array().unwrap().iter().map(|a| a["host"].as_str().unwrap()).collect();
        assert_eq!(hosts, ["api.anthropic.com", "github.com", "*.npmjs.org", "dev.test"]);
        assert_eq!(p["allow"][3]["ports"], json!([8080]));
        assert!(p["allow"][1].get("private").is_none(), "extra domains never get a private address");
    }

    #[test]
    fn loopback_ports_are_sent_and_reachable_through_the_proxy_as_private_loopback_names() {
        let mut c = cfg();
        c.sandbox.loopback_ports = vec![11434, 0, 5037, 11434];
        let p = plan(&c);
        assert_eq!(p["loopback_ports"], json!([5037, 11434]), "port 0 and repeats are dropped");
        let allow = p["allow"].as_array().unwrap();
        assert_eq!(allow[1], json!({ "host": "127.0.0.1", "ports": [5037, 11434], "private": true }));
        assert_eq!(allow[2], json!({ "host": "::1", "ports": [5037, 11434], "private": true }));
        assert_eq!(allow[3], json!({ "host": "localhost", "ports": [5037, 11434], "private": true }));
        assert_eq!(allow.len(), 4);
    }

    #[test]
    fn loopback_rules_come_before_a_plain_domain_entry_for_the_same_name() {
        let mut c = cfg();
        c.sandbox.allow_domains = vec!["localhost:11434".into()];
        c.sandbox.loopback_ports = vec![11434];
        let p = plan(&c);
        let first_localhost = p["allow"].as_array().unwrap().iter().find(|a| a["host"] == "localhost").unwrap();
        assert_eq!(first_localhost["private"], true, "the proxy takes the first match");
    }

    #[test]
    fn no_loopback_ports_adds_no_private_rule() {
        let p = plan(&cfg());
        assert_eq!(p["loopback_ports"], json!([]));
        assert!(p["allow"].as_array().unwrap().iter().all(|a| a.get("private").is_none()));
    }

    #[test]
    fn agent_readable_paths_are_passed_through_by_agent() {
        let mut c = cfg();
        c.sandbox.agent_readable.insert("claude".into(), vec!["/home/alice/.config/gh".into()]);
        let p = plan(&c);
        assert_eq!(p["agent_readable"], json!({ "claude": ["/home/alice/.config/gh"] }));
    }

    #[test]
    fn idle_reaping_defaults_to_a_day_and_zero_turns_it_off() {
        let mut c = cfg();
        assert_eq!(idle_ttl_secs(&c), Some(24 * 3600));
        c.idle_reap_hours = 0;
        assert_eq!(idle_ttl_secs(&c), None);
    }

    #[test]
    fn mode_none_sends_no_policy() {
        let mut c = cfg();
        c.sandbox.mode = SandboxMode::None;
        assert_eq!(plan(&c), json!({ "mode": "none" }));
    }
}
