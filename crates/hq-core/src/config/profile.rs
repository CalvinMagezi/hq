//! The `lite` profile and the egress report behind it.
//!
//! HQ Lite is for a machine the owner does not fully control, such as a work laptop whose
//! only approved model path is the company's Copilot seat. In that profile the server serves
//! the web app, tasks and vault from a narrowed tool registry, refuses chat turns and session
//! control, and refuses to start while anything that sends vault text to another service is
//! configured, unless the owner lists it in `lite.allow_egress`.
//!
//! [`egress_report`] lists every outbound destination a config can reach, and
//! [`lite_violations`] is the part of it the profile does not allow. `hq doctor --egress`
//! prints the first; the server's startup check enforces the second.

use serde::{Deserialize, Serialize};

use super::{BackendKind, HqConfig};

/// Which edition of HQ this instance runs as.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Profile {
    /// Everything: coding-agent host, sandboxed shell, relays, every tool.
    #[default]
    Full,
    /// The web app, tasks and vault only, with no outbound traffic you did not list.
    Lite,
}

impl Profile {
    pub fn is_lite(self) -> bool {
        self == Self::Lite
    }
}

/// Settings that only apply under `profile: lite`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LiteConfig {
    /// Allow the `github-copilot-api` backend. It signs in through Copilot's internal token
    /// endpoint and presents itself as VS Code, which GitHub does not document as a supported
    /// way for other software to use a seat, so Lite refuses it unless you opt in.
    #[serde(default)]
    pub allow_unofficial_copilot: bool,
    /// Destinations Lite may use anyway: an item id from `hq doctor --egress`
    /// (`openrouter`, `telegram`, `backend:work`) or a host name (`api.example.com`, which
    /// also allows its subdomains).
    #[serde(default)]
    pub allow_egress: Vec<String>,
}

/// One outbound destination a config can reach.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EgressItem {
    /// Stable id, used in `lite.allow_egress`.
    pub id: String,
    pub label: String,
    pub hosts: Vec<String>,
    /// What can travel there.
    pub carries: &'static str,
    /// The profile accepts this without a listing (the supported Copilot route).
    pub lite_ok: bool,
    /// Why Lite refuses it even though the host might be fine, if it does.
    pub refusal: Option<&'static str>,
}

/// Reads a setting from the environment; a parameter so tests need no process state.
pub type EnvLookup<'a> = &'a dyn Fn(&str) -> Option<String>;

/// The process environment, for production callers.
pub fn process_env(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

fn present(value: &Option<String>, env: EnvLookup<'_>, vars: &[&str]) -> bool {
    let set = |s: &str| !s.trim().is_empty();
    value.as_deref().is_some_and(set) || vars.iter().any(|v| env(v).is_some_and(|s| set(&s)))
}

/// The host of a URL, lowercased, without credentials or port.
fn host_of(url: &str) -> Option<String> {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    // WHATWG parsers (and so most HTTP clients) read a backslash as a slash in http(s) URLs.
    let authority = rest.split(['/', '\\', '?', '#']).next()?;
    let authority = authority.rsplit('@').next()?;
    let host = if let Some(v6) = authority.strip_prefix('[') {
        v6.split(']').next()?
    } else {
        authority.split(':').next()?
    };
    (!host.is_empty()).then(|| host.trim_end_matches('.').to_ascii_lowercase())
}

fn is_loopback(host: &str) -> bool {
    host == "localhost"
        || host == "::1"
        || host == "0:0:0:0:0:0:0:1"
        || host
            .parse::<std::net::Ipv4Addr>()
            .is_ok_and(|ip| ip.is_loopback())
}

fn item(id: &str, label: &str, hosts: &[&str], carries: &'static str) -> EgressItem {
    EgressItem {
        id: id.to_string(),
        label: label.to_string(),
        hosts: hosts.iter().map(|h| h.to_string()).collect(),
        carries,
        lite_ok: false,
        refusal: None,
    }
}

/// Every outbound destination `cfg` can reach, with `env` standing in for the environment.
/// Destinations on this machine are not listed. Web search and fetch, image generation and the
/// coding-agent host are not either: the Lite registry has none of those tools, and under the
/// full profile the report is informational.
pub fn egress_report(cfg: &HqConfig, env: EnvLookup<'_>) -> Vec<EgressItem> {
    let mut items = Vec::new();
    // Providers that are keyed only by an environment variable.
    let cfg_none: Option<String> = None;

    for (id, label, value, vars, hosts) in [
        (
            "openrouter",
            "OpenRouter",
            &cfg.openrouter_api_key,
            &["OPENROUTER_API_KEY"][..],
            &["openrouter.ai"][..],
        ),
        (
            "anthropic",
            "Anthropic API",
            &cfg.anthropic_api_key,
            &["ANTHROPIC_API_KEY"][..],
            &["api.anthropic.com"][..],
        ),
        (
            "google",
            "Google AI",
            &cfg.google_ai_api_key,
            &["GOOGLE_AI_API_KEY", "GEMINI_API_KEY"][..],
            &["generativelanguage.googleapis.com"][..],
        ),
        (
            "cerebras",
            "Cerebras",
            &cfg.cerebras_api_key,
            &["CEREBRAS_API_KEY"][..],
            &["api.cerebras.ai"][..],
        ),
        (
            "groq",
            "Groq",
            &cfg.groq_api_key,
            &["GROQ_API_KEY"][..],
            &["api.groq.com"][..],
        ),
        (
            "deepseek",
            "DeepSeek",
            &cfg.deepseek_api_key,
            &["DEEPSEEK_API_KEY"][..],
            &["api.deepseek.com"][..],
        ),
        (
            "openai",
            "OpenAI API",
            &cfg.openai_api_key,
            &["OPENAI_API_KEY"][..],
            &["api.openai.com"][..],
        ),
        (
            "siliconflow",
            "SiliconFlow",
            &cfg_none,
            &["SILICONFLOW_API_KEY"][..],
            &["api.siliconflow.cn"][..],
        ),
        (
            "novita",
            "Novita",
            &cfg_none,
            &["NOVITA_API_KEY"][..],
            &["api.novita.ai"][..],
        ),
        (
            "kimi",
            "Kimi Code",
            &cfg.kimi_code_api_key,
            &["KIMI_CODE_API_KEY", "KIMI_API_KEY"][..],
            &["api.kimi.com"][..],
        ),
    ] {
        if present(value, env, vars) {
            let carries = if id == "openrouter" {
                "prompts, and note titles and excerpts sent for semantic search"
            } else {
                "prompts"
            };
            let mut e = item(id, label, hosts, carries);
            // OPENROUTER_BASE_URL sends the same traffic somewhere else.
            if id == "openrouter"
                && let Some(host) = env("OPENROUTER_BASE_URL").as_deref().and_then(host_of)
                && !is_loopback(&host)
                && !e.hosts.contains(&host)
            {
                e.hosts.push(host);
            }
            items.push(e);
        }
    }

    for p in cfg.providers.iter().filter(|p| p.enabled) {
        let host = host_of(&p.api_base).unwrap_or_default();
        if !is_loopback(&host) {
            items.push(item(
                &format!("provider:{}", p.name),
                &format!("LLM provider {}", p.name),
                &[host.as_str()],
                "prompts",
            ));
        }
    }

    for b in cfg.backends.backends.iter().filter(|b| b.enabled) {
        match b.kind {
            BackendKind::GithubCopilotCli => {
                let mut e = item(
                    "github-copilot-cli",
                    &format!("GitHub Copilot CLI ({})", b.name),
                    &["api.githubcopilot.com"],
                    "prompts, through your Copilot seat",
                );
                e.lite_ok = true;
                items.push(e);
            }
            BackendKind::GithubCopilotApi => {
                let mut e = item(
                    "github-copilot-api",
                    &format!("GitHub Copilot internal API ({})", b.name),
                    &["api.githubcopilot.com", "api.github.com"],
                    "prompts, through Copilot's internal token endpoint",
                );
                e.refusal = Some(
                    "not a documented way for other software to use a Copilot seat; set \
                     lite.allow_unofficial_copilot to use it anyway",
                );
                items.push(e);
            }
            kind => {
                let endpoint = b.resolved_endpoint();
                let host = endpoint.as_deref().and_then(host_of).unwrap_or_else(|| match kind {
                    BackendKind::Openrouter => "openrouter.ai".into(),
                    BackendKind::KimiCode => "api.kimi.com".into(),
                    BackendKind::AnthropicCompatible => "api.anthropic.com".into(),
                    _ => String::new(),
                });
                if !is_loopback(&host) {
                    items.push(item(
                        &format!("backend:{}", b.name),
                        &format!("LLM backend {}", b.name),
                        &[host.as_str()],
                        "prompts",
                    ));
                }
            }
        }
    }

    let relay = &cfg.relay;
    // A token is enough: the disk watchdog and restart notices post with it whether or not
    // the relay itself is enabled.
    if present(&relay.telegram_token, env, &["TELEGRAM_BOT_TOKEN"])
        || present(&relay.notifications_token, env, &[])
    {
        items.push(item(
            "telegram",
            "Telegram relay",
            &["api.telegram.org"],
            "chat messages and replies",
        ));
    }
    if present(&relay.discord_token, env, &["DISCORD_BOT_TOKEN"]) {
        items.push(item(
            "discord",
            "Discord relay",
            &["discord.com", "gateway.discord.gg"],
            "chat messages and replies",
        ));
    }

    for m in &cfg.remote_mcp {
        if let Some(host) = host_of(&m.url).filter(|h| !is_loopback(h)) {
            items.push(item(
                &format!("remote-mcp:{}", m.name),
                &format!("Remote MCP server {}", m.name),
                &[host.as_str()],
                "tool calls and their arguments",
            ));
        }
    }

    if cfg.decisions.enabled {
        for (i, route) in cfg.decisions.routes.iter().enumerate() {
            if let Some(host) = host_of(&route.endpoint).filter(|h| !is_loopback(h)) {
                items.push(item(
                    &format!("decisions:{i}"),
                    &format!("Decision route {i}"),
                    &[host.as_str()],
                    "note titles and excerpts, when a note is placed in a task list",
                ));
            }
        }
    }

    let reads_gmail = cfg.companies.iter().any(|c| {
        c.listeners.iter().any(|l| l.kind == "email") || c.connectors.iter().any(|b| b.kind == "gws")
    });
    if reads_gmail {
        items.push(item(
            "gmail",
            "Gmail through the gws CLI",
            &["gmail.googleapis.com"],
            "email metadata and message text",
        ));
    }

    for (name, host) in &cfg.agent_host.hosts {
        let target = host.ssh.rsplit('@').next().unwrap_or(&host.ssh);
        items.push(item(
            &format!("agent-host:{name}"),
            &format!("Remote coding-agent host {name}"),
            &[target],
            "agent commands, files and screens, over ssh",
        ));
    }
    if let Some(host) = cfg.agent_host.agent_mcp_url.as_deref().and_then(host_of)
        && !is_loopback(&host)
    {
        items.push(item(
            "agent-mcp-url",
            "Agent MCP endpoint",
            &[host.as_str()],
            "tool calls from launched agents",
        ));
    }

    if let Some(host) = env("OLLAMA_HOST").as_deref().and_then(host_of)
        && !is_loopback(&host)
    {
        items.push(item(
            "ollama",
            "Ollama on another machine (OLLAMA_HOST)",
            &[host.as_str()],
            "note titles and excerpts embedded for semantic search",
        ));
    }
    if let Some(host) = env("TURBOQUANT_BASE_URL").as_deref().and_then(host_of)
        && !is_loopback(&host)
    {
        items.push(item(
            "turboquant",
            "TurboQuant endpoint (TURBOQUANT_BASE_URL)",
            &[host.as_str()],
            "prompts",
        ));
    }

    if cfg.self_update.enabled {
        items.push(item(
            "self-update",
            "Self-update (repository checkout)",
            &["github.com"],
            "update checks and downloads; no vault text",
        ));
    }

    if cfg.copilot_usage.enabled && super::copilot_active(cfg) {
        let mut e = item(
            "copilot-usage",
            "Copilot credit meter",
            &["api.github.com"],
            "your GitHub token; no vault text",
        );
        // The meter reads Copilot's internal user endpoint, like the unofficial backend.
        e.refusal = Some(
            "reads Copilot's internal user endpoint; set copilot_usage.enabled: false or \
             lite.allow_unofficial_copilot",
        );
        items.push(e);
    }

    items
}

fn host_allowed(host: &str, listed: &str) -> bool {
    let listed = listed.trim().trim_end_matches('.').to_ascii_lowercase();
    // A host needs a dot: a bare label such as `ai` is an id, and as a suffix it would allow
    // every host under that top-level domain.
    listed.contains('.') && (host == listed || host.ends_with(&format!(".{listed}")))
}

/// Whether the owner's `lite.allow_egress` lets `e` through: its id, or a listing for every one
/// of its hosts (an item that reaches two hosts is not allowed by naming only one).
fn listed_in_allow_egress(cfg: &HqConfig, e: &EgressItem) -> bool {
    let by_id = cfg.lite.allow_egress.iter().any(|a| a.eq_ignore_ascii_case(&e.id));
    let by_hosts = !e.hosts.is_empty()
        && e.hosts
            .iter()
            .all(|h| cfg.lite.allow_egress.iter().any(|a| host_allowed(h, a)));
    by_id || by_hosts
}

/// The items `profile: lite` does not allow, with `env` standing in for the environment. Empty
/// under the full profile.
pub fn lite_violations(cfg: &HqConfig, env: EnvLookup<'_>) -> Vec<EgressItem> {
    if !cfg.profile.is_lite() {
        return Vec::new();
    }
    egress_report(cfg, env)
        .into_iter()
        .filter(|e| {
            if e.lite_ok {
                return false;
            }
            // An item with a refusal answers to its own switch alone, so listing a parent
            // domain (`github.com` for self-update) cannot lift it by the back door.
            if e.refusal.is_some() {
                return !(cfg.lite.allow_unofficial_copilot
                    && matches!(e.id.as_str(), "github-copilot-api" | "copilot-usage"));
            }
            !listed_in_allow_egress(cfg, e)
        })
        .collect()
}

/// A startup error naming each violation and how to clear it, or `Ok` when there is none.
pub fn enforce_lite(cfg: &HqConfig, env: EnvLookup<'_>) -> anyhow::Result<()> {
    let violations = lite_violations(cfg, env);
    if violations.is_empty() {
        return Ok(());
    }
    let mut msg = String::from(
        "profile: lite refuses to start while these can send data to other services:\n",
    );
    for v in &violations {
        msg.push_str(&format!(
            "  - {} ({}): {}{}\n",
            v.label,
            v.hosts.join(", "),
            v.carries,
            v.refusal.map(|r| format!(". {r}")).unwrap_or_default()
        ));
    }
    msg.push_str(
        "Remove the key or setting, or list the item id or host in lite.allow_egress \
         (`hq doctor --egress` shows the ids).",
    );
    anyhow::bail!(msg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{BackendEntry, WireApi};
    use std::collections::HashMap;

    fn no_env(_: &str) -> Option<String> {
        None
    }

    fn lite() -> HqConfig {
        HqConfig {
            profile: Profile::Lite,
            ..HqConfig::default()
        }
    }

    fn backend(name: &str, kind: BackendKind, endpoint: Option<&str>) -> BackendEntry {
        BackendEntry {
            name: name.into(),
            kind,
            endpoint: endpoint.map(String::from),
            credential_env: None,
            model: None,
            effort: None,
            wire: WireApi::default(),
            enabled: true,
        }
    }

    fn ids(items: &[EgressItem]) -> Vec<&str> {
        items.iter().map(|e| e.id.as_str()).collect()
    }

    #[test]
    fn a_bare_config_reaches_nothing() {
        assert!(egress_report(&lite(), &no_env).is_empty());
        assert!(lite_violations(&lite(), &no_env).is_empty());
        assert!(enforce_lite(&lite(), &no_env).is_ok());
    }

    #[test]
    fn keys_in_the_config_or_the_environment_both_count() {
        let mut cfg = lite();
        cfg.openrouter_api_key = Some("sk-or-test".into());
        assert_eq!(ids(&egress_report(&cfg, &no_env)), ["openrouter"]);

        let env: HashMap<&str, &str> = HashMap::from([("ANTHROPIC_API_KEY", "k"), ("GROQ_API_KEY", "  ")]);
        let lookup = |n: &str| env.get(n).map(|v| v.to_string());
        assert_eq!(ids(&egress_report(&lite(), &lookup)), ["anthropic"], "a blank value is not a key");
    }

    #[test]
    fn openrouter_is_named_as_carrying_note_excerpts() {
        let mut cfg = lite();
        cfg.openrouter_api_key = Some("k".into());
        let report = egress_report(&cfg, &no_env);
        assert!(report[0].carries.contains("note titles"), "{:?}", report[0]);
    }

    #[test]
    fn the_full_profile_never_has_violations() {
        let cfg = HqConfig {
            openrouter_api_key: Some("k".into()),
            ..HqConfig::default()
        };
        assert_eq!(egress_report(&cfg, &no_env).len(), 1);
        assert!(lite_violations(&cfg, &no_env).is_empty());
        assert!(enforce_lite(&cfg, &no_env).is_ok());
    }

    #[test]
    fn lite_refuses_a_key_until_it_is_listed_by_id_or_host() {
        let mut cfg = lite();
        cfg.openrouter_api_key = Some("k".into());
        assert_eq!(ids(&lite_violations(&cfg, &no_env)), ["openrouter"]);
        let err = enforce_lite(&cfg, &no_env).unwrap_err().to_string();
        assert!(err.contains("OpenRouter") && err.contains("lite.allow_egress"), "{err}");

        cfg.lite.allow_egress = vec!["openrouter".into()];
        assert!(lite_violations(&cfg, &no_env).is_empty());
        cfg.lite.allow_egress = vec!["OpenRouter.AI".into()];
        assert!(lite_violations(&cfg, &no_env).is_empty(), "host match ignores case");
        cfg.lite.allow_egress = vec!["ai".into()];
        assert!(!lite_violations(&cfg, &no_env).is_empty(), "a suffix is not a subdomain");
        cfg.lite.allow_egress = vec!["notopenrouter.ai".into()];
        assert!(!lite_violations(&cfg, &no_env).is_empty());
    }

    #[test]
    fn the_supported_copilot_route_is_allowed_and_the_internal_one_is_not() {
        let mut cfg = lite();
        cfg.backends.backends = vec![backend("work", BackendKind::GithubCopilotCli, None)];
        assert_eq!(ids(&egress_report(&cfg, &no_env)), ["github-copilot-cli"]);
        assert!(lite_violations(&cfg, &no_env).is_empty());

        cfg.backends.backends = vec![backend("work", BackendKind::GithubCopilotApi, None)];
        let v = lite_violations(&cfg, &no_env);
        assert_eq!(ids(&v), ["github-copilot-api", "copilot-usage"]);
        assert!(v[0].refusal.is_some());

        cfg.lite.allow_unofficial_copilot = true;
        assert!(lite_violations(&cfg, &no_env).is_empty(), "the switch lifts both");

        cfg.lite.allow_unofficial_copilot = false;
        cfg.copilot_usage.enabled = false;
        assert_eq!(ids(&lite_violations(&cfg, &no_env)), ["github-copilot-api"]);
    }

    #[test]
    fn a_backend_on_this_machine_is_not_egress() {
        let mut cfg = lite();
        cfg.backends.backends = vec![
            backend("local", BackendKind::OpenaiCompatible, Some("http://localhost:11434/v1")),
            backend("loop", BackendKind::OpenaiCompatible, Some("http://127.0.0.1:8080/v1")),
            backend("lan", BackendKind::OpenaiCompatible, Some("http://192.168.1.5:8080/v1")),
        ];
        assert_eq!(ids(&egress_report(&cfg, &no_env)), ["backend:lan"]);
    }

    #[test]
    fn a_disabled_backend_or_provider_is_not_egress() {
        let mut cfg = lite();
        let mut off = backend("off", BackendKind::Openrouter, None);
        off.enabled = false;
        cfg.backends.backends = vec![off];
        assert!(egress_report(&cfg, &no_env).is_empty());
    }

    #[test]
    fn relays_remote_mcp_and_self_update_are_listed() {
        let mut cfg = lite();
        cfg.relay.telegram_enabled = true;
        cfg.relay.telegram_token = Some("123:abc".into());
        cfg.relay.discord_enabled = true;
        cfg.relay.discord_token = None;
        cfg.remote_mcp = vec![crate::config::RemoteMcpServer {
            name: "diagrams".into(),
            url: "https://user:pw@mcp.example.com:8443/api".into(),
            api_key: None,
            live_user_turn_only: true,
        }];
        cfg.self_update.enabled = true;
        let report = egress_report(&cfg, &no_env);
        assert_eq!(ids(&report), ["telegram", "remote-mcp:diagrams", "self-update"]);
        assert_eq!(report[1].hosts, ["mcp.example.com"], "no credentials or port in the host");
    }

    #[test]
    fn host_parsing_handles_the_awkward_shapes() {
        assert_eq!(host_of("https://API.Example.com./v1").as_deref(), Some("api.example.com"));
        assert_eq!(host_of("http://[::1]:8080/x").as_deref(), Some("::1"));
        assert_eq!(host_of("http://a@b.example.com").as_deref(), Some("b.example.com"));
        assert_eq!(host_of("https://").as_deref(), None);
        assert!(is_loopback("127.0.0.7") && is_loopback("localhost") && !is_loopback("localhost.evil.example"));
    }

    #[test]
    fn the_profile_and_its_settings_load_from_the_config_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        std::fs::write(
            &path,
            format!(
                "vault_path: {}\nprofile: lite\nlite:\n  allow_unofficial_copilot: true\n  allow_egress: [telegram, api.example.com]\n",
                dir.path().display()
            ),
        )
        .unwrap();
        let cfg = HqConfig::load_from_path(&path).unwrap();
        assert_eq!(cfg.profile, Profile::Lite);
        assert!(cfg.lite.allow_unofficial_copilot);
        assert_eq!(cfg.lite.allow_egress, ["telegram", "api.example.com"]);

        std::fs::write(&path, format!("vault_path: {}\n", dir.path().display())).unwrap();
        let plain = HqConfig::load_from_path(&path).unwrap();
        assert_eq!(plain.profile, Profile::Full, "an existing config stays on the full profile");
        assert!(plain.lite.allow_egress.is_empty());
    }
    #[test]
    fn env_only_providers_and_remote_endpoints_are_listed() {
        let env: HashMap<&str, &str> = HashMap::from([
            ("SILICONFLOW_API_KEY", "k"),
            ("NOVITA_API_KEY", "k"),
            ("OLLAMA_HOST", "http://gpu.example.com:11434"),
            ("TURBOQUANT_BASE_URL", "http://tq.example.com/v1"),
            ("OPENROUTER_API_KEY", "k"),
            ("OPENROUTER_BASE_URL", "https://gateway.example.com/api/v1"),
        ]);
        let lookup = |n: &str| env.get(n).map(|v| v.to_string());
        let report = egress_report(&lite(), &lookup);
        let got = ids(&report);
        for id in ["siliconflow", "novita", "ollama", "turboquant", "openrouter"] {
            assert!(got.contains(&id), "{id} missing from {got:?}");
        }
        let openrouter = report.iter().find(|e| e.id == "openrouter").unwrap();
        assert!(openrouter.hosts.contains(&"gateway.example.com".to_string()), "{openrouter:?}");

        let local: HashMap<&str, &str> = HashMap::from([("OLLAMA_HOST", "127.0.0.1:11434"), ("TURBOQUANT_BASE_URL", "http://localhost:9/v1")]);
        let lookup = |n: &str| local.get(n).map(|v| v.to_string());
        assert!(egress_report(&lite(), &lookup).is_empty(), "a local Ollama is not egress");
    }

    #[test]
    fn a_relay_token_counts_even_when_the_relay_is_switched_off() {
        let mut cfg = lite();
        cfg.relay.telegram_enabled = false;
        cfg.relay.telegram_token = Some("123:abc".into());
        cfg.relay.discord_enabled = false;
        cfg.relay.discord_token = Some("tok".into());
        assert_eq!(ids(&egress_report(&cfg, &no_env)), ["telegram", "discord"]);
    }

    #[test]
    fn decisions_gmail_and_agent_hosts_are_listed() {
        let mut cfg = lite();
        cfg.decisions.enabled = true;
        cfg.companies = vec![crate::config::CompanyConfig {
            id: "acme".into(),
            name: "Acme".into(),
            identity: crate::config::CompanyIdentity { contact_name: "A".into(), role: "owner".into() },
            vault_prefix: "Notebooks/Companies/acme".into(),
            listeners: vec![crate::config::ListenerDef {
                id: "inbox".into(),
                kind: "email".into(),
                path: "/".into(),
                secret_ref: None,
            }],
            connectors: Vec::new(),
        }];
        cfg.agent_host.hosts.insert(
            "build".into(),
            crate::config::RemoteHostConfig {
                ssh: "ci@build.example.com".into(),
                port: None,
                identity_file: None,
                gate_command: "hq host gate".into(),
            },
        );
        cfg.agent_host.agent_mcp_url = Some("https://hq.example.com:8444/mcp".into());
        let got = ids(&egress_report(&cfg, &no_env)).join(",");
        for id in ["decisions:0", "gmail", "agent-host:build", "agent-mcp-url"] {
            assert!(got.contains(id), "{id} missing from {got}");
        }
        let hosts: Vec<_> = egress_report(&cfg, &no_env)
            .into_iter()
            .find(|e| e.id == "agent-host:build")
            .unwrap()
            .hosts;
        assert_eq!(hosts, ["build.example.com"], "the ssh user is not part of the host");
    }

    /// The dedicated switch is the only way past a refusal; a parent-domain listing is not.
    #[test]
    fn allow_egress_cannot_lift_a_refusal_by_the_back_door() {
        let mut cfg = lite();
        cfg.backends.backends = vec![backend("work", BackendKind::GithubCopilotApi, None)];
        cfg.lite.allow_egress = vec!["github.com".into(), "githubcopilot.com".into(), "github-copilot-api".into(), "copilot-usage".into()];
        assert_eq!(ids(&lite_violations(&cfg, &no_env)), ["github-copilot-api", "copilot-usage"]);
        cfg.lite.allow_unofficial_copilot = true;
        assert!(lite_violations(&cfg, &no_env).is_empty());
    }

    #[test]
    fn an_item_with_two_hosts_needs_both_listed() {
        let mut cfg = lite();
        cfg.relay.discord_token = Some("tok".into());
        cfg.lite.allow_egress = vec!["discord.com".into()];
        assert_eq!(ids(&lite_violations(&cfg, &no_env)), ["discord"], "gateway.discord.gg is still unlisted");
        cfg.lite.allow_egress = vec!["discord.com".into(), "gateway.discord.gg".into()];
        assert!(lite_violations(&cfg, &no_env).is_empty());
        cfg.lite.allow_egress = vec!["discord".into()];
        assert!(lite_violations(&cfg, &no_env).is_empty(), "the id covers every host");
    }

    #[test]
    fn a_backslash_ends_the_host_like_a_slash() {
        assert_eq!(host_of("https://a.example.com\\@b.example.com").as_deref(), Some("a.example.com"));
    }
}
