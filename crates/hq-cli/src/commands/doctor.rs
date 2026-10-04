use anyhow::Result;
use hq_core::config::HqConfig;

/// Diagnose common issues — comprehensive health check.
pub async fn run(config: &HqConfig) -> Result<()> {
    // Doctor is essentially an alias for health with extra checks
    super::health::run(config).await?;
    report_backend(config).await;
    report_skill_issues(config);
    report_bash_sandbox(config);
    report_machine_profile(config);
    report_web_search(config).await;
    Ok(())
}

/// The machine profile only sees configured and reachable; this sends one
/// real query per configured backend (one request of Brave's quota).
async fn report_web_search(config: &HqConfig) {
    println!("\nWeb search (one test query per configured backend)");
    let statuses = hq_tools::web::probe_search_backends(
        config.searxng_url.as_deref(),
        config.brave_api_key.as_deref(),
    )
    .await;
    for status in &statuses {
        let label = match (status.configured, status.answered) {
            (false, _) => "--",
            (true, Some(true)) => "ok",
            (true, _) => "FAIL",
        };
        println!("  {label}  {}", status.detail);
    }
    if !statuses.iter().any(|s| s.answered == Some(true)) {
        println!(
            "  warn  no backend answered, so web_search will fail. Run \
             `scripts/setup-searxng.sh` or set `brave_api_key` (or HQ_BRAVE_API_KEY)."
        );
    }
}

/// Whether model-written shell commands are isolated, refused or running bare.
fn report_bash_sandbox(config: &HqConfig) {
    use hq_agent::bash_sandbox::{BashSettings, host_sandbox_status};

    let settings = BashSettings::from_config(&config.governance.bash, Vec::new());
    let (label, text) = host_sandbox_status(&settings).describe();
    println!("\nBash sandbox");
    println!("  {label}  {text}");
}

/// Surface skills that can never fire. Non-fatal — `hq skills validate` is the
/// command that exits non-zero.
fn report_skill_issues(config: &HqConfig) {
    use hq_tools::skills::Severity;

    let skills_dir = hq_core::skills_dir(&config.vault_path);
    let issues = hq_tools::skills::validate_skills(&skills_dir);
    let errors = issues
        .iter()
        .filter(|i| i.severity == Severity::Error)
        .count();

    println!("\nSkills");
    if issues.is_empty() {
        println!(
            "  ok  {} skills, no issues",
            hq_tools::skills::list_skills(&skills_dir).len()
        );
        return;
    }
    for issue in issues.iter().take(10) {
        let label = match issue.severity {
            Severity::Error => "FAIL",
            Severity::Warning => "warn",
        };
        println!("  {label}  {}: {}", issue.skill, issue.message);
    }
    if errors > 0 {
        println!("  run `hq skills validate` for the full report");
    }
}

/// A missing or stale machine profile means every agent prompt is guessing
/// about what is installed.
fn report_machine_profile(config: &HqConfig) {
    let path = config.vault_path.join("_system/MACHINE.md");
    println!("\nMachine profile");
    match std::fs::metadata(&path).and_then(|m| m.modified()) {
        Ok(modified) => {
            let age = modified.elapsed().unwrap_or_default();
            let mins = age.as_secs() / 60;
            if age > std::time::Duration::from_secs(6 * 3600) {
                println!(
                    "  warn  {} is {mins} minutes old — is the daemon running?",
                    path.display()
                );
            } else {
                println!("  ok  refreshed {mins} minutes ago");
            }
        }
        Err(_) => println!(
            "  warn  no _system/MACHINE.md — agents cannot tell which CLIs are installed. \
             Start the daemon, or it is written within 30s of `hq start`."
        ),
    }
}

const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// One line of the backend section: a label (`ok`, `warn`, `FAIL`, `--`) and text.
#[derive(Debug, PartialEq)]
struct Line(&'static str, String);

/// Where config and vault resolve to, and whether any LLM backend is configured.
/// Pure over its inputs so tests need no filesystem or environment.
fn describe_backend(
    config: &HqConfig,
    config_path: &std::path::Path,
    config_exists: bool,
    vault_exists: bool,
) -> Vec<Line> {
    let config_line = if config_exists {
        Line(
            "ok",
            format!("config loaded from {}", config_path.display()),
        )
    } else {
        Line(
            "warn",
            format!(
                "{} does not exist, running on defaults",
                config_path.display()
            ),
        )
    };
    let vault_line = if vault_exists {
        Line("ok", format!("vault {}", config.vault_path.display()))
    } else {
        Line(
            "FAIL",
            format!("vault {} does not exist", config.vault_path.display()),
        )
    };
    let has_key = |k: &Option<String>| k.as_deref().is_some_and(|k| !k.trim().is_empty());
    let legacy = has_key(&config.openrouter_api_key)
        || has_key(&config.anthropic_api_key)
        || has_key(&config.google_ai_api_key)
        || !config.providers.is_empty();
    let llm_line = if config.backends.is_configured() {
        Line(
            "ok",
            format!(
                "backend chain: {}",
                config.backends.chain_order().join(" -> ")
            ),
        )
    } else if legacy {
        Line(
            "ok",
            "LLM provider configured via legacy api keys / providers".to_string(),
        )
    } else if config.local_only {
        Line(
            "warn",
            "no cloud backend configured (local_only: Ollama only)".to_string(),
        )
    } else {
        Line(
            "FAIL",
            "no LLM backend configured: set `backends:` or an API key in the config".to_string(),
        )
    };
    vec![config_line, vault_line, llm_line]
}

/// The `host:port` a plain TCP probe should dial for an endpoint URL.
fn probe_target(endpoint: &str) -> Option<String> {
    let url = reqwest::Url::parse(endpoint).ok()?;
    let port = url.port_or_known_default()?;
    Some(format!("{}:{port}", url.host_str()?))
}

fn uses_copilot(config: &HqConfig) -> bool {
    use hq_core::config::BackendKind;
    config.backends.backends.iter().any(|b| {
        b.enabled
            && matches!(
                b.kind,
                BackendKind::GithubCopilotApi | BackendKind::GithubCopilotCli
            )
    })
}

/// Names the Copilot credential source without ever printing a token.
async fn copilot_credential_line(copilot_in_use: bool) -> Line {
    if let Some(var) = hq_llm::copilot::env_token_source() {
        return Line("ok", format!("Copilot credential: env var {var}"));
    }
    match hq_llm::CopilotProvider::resolve_raw_token().await {
        Ok(_) => Line("ok", "Copilot credential: `gh auth token`".to_string()),
        Err(_) if copilot_in_use => Line(
            "FAIL",
            "Copilot backend configured but no credential: set COPILOT_GITHUB_TOKEN, \
             GH_TOKEN or GITHUB_TOKEN, or run `gh auth login`"
                .to_string(),
        ),
        Err(_) => Line(
            "--",
            "Copilot credential: none (no Copilot backend configured)".to_string(),
        ),
    }
}

/// Dial the primary backend's endpoint (TCP only, no credentials sent).
async fn probe_primary(config: &HqConfig) -> Option<Line> {
    let entry = config.backends.backend(&config.backends.primary)?;
    let target = probe_target(&entry.resolved_endpoint()?)?;
    let dial = tokio::net::TcpStream::connect(&target);
    let name = &entry.name;
    Some(match tokio::time::timeout(PROBE_TIMEOUT, dial).await {
        Ok(Ok(_)) => Line(
            "ok",
            format!("primary backend `{name}` reachable at {target}"),
        ),
        Ok(Err(e)) => Line(
            "FAIL",
            format!("primary backend `{name}` unreachable at {target}: {e}"),
        ),
        Err(_) => Line(
            "FAIL",
            format!("primary backend `{name}` timed out at {target}"),
        ),
    })
}

async fn report_backend(config: &HqConfig) {
    println!("\nBackend");
    let config_path = HqConfig::config_read_path();
    let mut lines = describe_backend(
        config,
        &config_path,
        config_path.exists(),
        config.vault_path.exists(),
    );
    lines.push(copilot_credential_line(uses_copilot(config)).await);
    lines.extend(probe_primary(config).await);
    for Line(label, text) in lines {
        println!("  {label}  {text}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn labels(lines: &[Line]) -> Vec<&'static str> {
        lines.iter().map(|l| l.0).collect()
    }

    #[test]
    fn no_backend_is_a_failure_and_missing_config_a_warning() {
        let config = HqConfig::default();
        let lines = describe_backend(&config, Path::new("/nope/config.yaml"), false, true);
        assert_eq!(labels(&lines), ["warn", "ok", "FAIL"]);
    }

    #[test]
    fn legacy_api_key_counts_as_configured_and_is_never_printed() {
        let config = HqConfig {
            openrouter_api_key: Some("sk-test".into()),
            ..HqConfig::default()
        };
        let lines = describe_backend(&config, Path::new("/c.yaml"), true, false);
        assert_eq!(labels(&lines), ["ok", "FAIL", "ok"]);
        assert!(lines.iter().all(|l| !l.1.contains("sk-test")));
    }

    #[test]
    fn probe_target_uses_known_default_ports() {
        assert_eq!(
            probe_target("https://api.githubcopilot.com").as_deref(),
            Some("api.githubcopilot.com:443")
        );
        assert_eq!(
            probe_target("http://localhost:11434/v1").as_deref(),
            Some("localhost:11434")
        );
        assert_eq!(probe_target("not a url"), None);
    }
}
