use anyhow::Result;
use hq_core::config::HqConfig;

/// Diagnose common issues — comprehensive health check.
pub async fn run(config: &HqConfig) -> Result<()> {
    // Doctor is health plus extra sections; the summary counts failures from all of them.
    let mut issues = super::health::check(config).await;
    issues += report_backend(config).await;
    issues += report_skill_issues(config);
    report_bash_sandbox(config);
    report_machine_profile(config);
    report_search_index(config);
    report_web_search(config).await;
    report_import_tools();
    report_code_signature();
    report_spend_ledger(config);
    super::health::print_summary(issues, "hq doctor");
    Ok(())
}

/// `hq doctor --egress`: every outbound destination this config can reach. Under `profile:
/// lite` it fails (non-zero exit) when any of them is not allowed.
pub fn run_egress(config: &HqConfig) -> Result<()> {
    run_egress_with(config, &hq_core::config::process_env)
}

fn run_egress_with(config: &HqConfig, env: hq_core::config::EnvLookup<'_>) -> Result<()> {
    use hq_core::config::{egress_report, lite_violations};
    let report = egress_report(config, env);
    let lite = config.profile.is_lite();
    println!("Outbound destinations (profile: {})", config.profile.name());
    if report.is_empty() {
        println!("  none: nothing configured sends data to another service");
    }
    let refused: Vec<String> = lite_violations(config, env)
        .into_iter()
        .map(|v| v.id)
        .collect();
    for e in &report {
        let verdict = if !lite {
            ""
        } else if e.lite_ok {
            "  [allowed in lite]"
        } else if refused.contains(&e.id) {
            "  [REFUSED in lite]"
        } else {
            "  [allowed by lite.allow_egress]"
        };
        println!("  {}  ({}){verdict}", e.label, e.id);
        println!("      hosts:   {}", e.hosts.join(", "));
        println!("      carries: {}", e.carries);
        if let Some(why) = e.refusal.filter(|_| refused.contains(&e.id)) {
            println!("      note:    {why}");
        }
    }
    println!(
        "\nNot listed: web search and fetch, image generation, email and the coding-agent host \
         have no tools in the lite profile; the full profile reaches them when asked."
    );
    hq_core::config::enforce_lite(config, env)
}

/// Whether the spend ledger can be trusted: priced models and attributed calls.
fn report_spend_ledger(config: &HqConfig) {
    use hq_db::usage_ledger::{GroupBy, grouped_usage, sum_rows, unpriced_models};
    const WINDOW_DAYS: i64 = 7;
    const UNATTRIBUTED_WARN_PCT: f64 = 5.0;
    println!("\nSpend ledger (last {WINDOW_DAYS} days)");
    let Ok(db) = hq_db::Database::open(&config.db_path()) else {
        println!("  --  no database yet");
        return;
    };
    let since = chrono::Utc::now().timestamp() - WINDOW_DAYS * 86_400;
    let Ok(origins) = db.with_conn(|c| grouped_usage(c, since, GroupBy::Origin)) else {
        println!("  warn  the ledger could not be read");
        return;
    };
    let total = sum_rows(&origins);
    if total.calls == 0 {
        println!("  --  no calls recorded");
        return;
    }
    let unknown = origins
        .iter()
        .find(|r| r.key == "unknown")
        .map_or(0, |r| r.calls);
    let unknown_pct = unknown as f64 / total.calls as f64 * 100.0;
    if unknown_pct > UNATTRIBUTED_WARN_PCT {
        println!(
            "  warn  {unknown} of {} calls have no origin, so spend cannot be attributed",
            total.calls
        );
    } else {
        println!("  ok  {} calls, {unknown} without an origin", total.calls);
    }
    let unpriced = db
        .with_conn(|c| unpriced_models(c, since))
        .unwrap_or_default();
    if unpriced.is_empty() {
        println!("  ok  every model that ran has a known price");
    }
    for (model, calls) in unpriced {
        println!(
            "  warn  {model} ran {calls} times with no known price, so its cost is not counted"
        );
    }
}

/// What this machine can do when importing files (`hq vault import`, `convert_to_markdown`):
/// OCR for scans and photos, tables in PDFs, and old Office files. The built-in readers need
/// none of it; each missing tool narrows only the files that need it, and the inventory marks
/// those files `blocked`.
fn report_import_tools() {
    println!("\nImporting files as notes");
    let caps = hq_convert::ingest::capabilities();
    for Line(label, text) in
        import_tool_lines(&caps, on_path("pdftoppm"), cfg!(target_os = "macos"))
    {
        println!("  {label}  {text}");
    }
}

fn import_tool_lines(
    caps: &hq_convert::ingest::Capabilities,
    pdftoppm: bool,
    macos: bool,
) -> Vec<Line> {
    let mut lines = Vec::new();
    let install = |apt: &str, brew: &str| {
        if macos {
            format!("Install with `brew install {brew}`.")
        } else {
            format!("Install with `apt install {apt}`.")
        }
    };
    lines.push(match (caps.ocr, pdftoppm) {
        (true, true) => Line("ok", "OCR for scans and photos".into()),
        (true, false) => Line(
            "warn",
            format!(
                "OCR works for pictures, but scanned PDFs need pdftoppm. {}",
                install("poppler-utils", "poppler")
            ),
        ),
        (false, _) => Line(
            "warn",
            format!(
                "no OCR, so scans and photos cannot be imported (they show as blocked, and a PDF with no text layer fails). {}",
                install("tesseract-ocr poppler-utils", "tesseract poppler")
            ),
        ),
    });
    lines.push(if caps.pdf_tables {
        Line("ok", "tables in PDFs are rebuilt from the page layout (pdftotext)".into())
    } else {
        Line(
            "--",
            format!(
                "pdftotext not found: PDFs still import, but their tables come out as loose text. {}",
                install("poppler-utils", "poppler")
            ),
        )
    });
    lines.push(if caps.legacy_office {
        Line("ok", "old .doc, .ppt and .xls files open through LibreOffice".into())
    } else {
        Line(
            "--",
            format!(
                "LibreOffice not found: old .doc and .ppt files show as blocked, and an old .xls imports without its number formats. {}",
                install(
                    "--no-install-recommends libreoffice-writer libreoffice-impress libreoffice-calc",
                    "--cask libreoffice"
                )
            ),
        )
    });
    lines
}

/// macOS ties folder-access approvals to the code signature, so an ad-hoc `hq` asks again
/// after every update. Shown only where the host is set up.
#[cfg(unix)]
fn report_code_signature() {
    let Some((healthy, text)) = super::host_install::signature_report() else {
        return;
    };
    println!("\nCode signature (macOS folder access)");
    println!("  {}  {text}", if healthy { "ok" } else { "warn" });
}

/// There is no coding-agent host, and so no host install module, off Unix.
#[cfg(not(unix))]
fn report_code_signature() {}

fn on_path(tool: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(tool).is_file()))
}

/// The machine profile only sees configured and reachable; this sends one
/// real query per configured backend (one request of Brave's quota).
async fn report_web_search(config: &HqConfig) {
    println!("\nWeb search (one test query per configured backend)");
    let statuses = hq_tools::web::probe_search_backends(
        config.searxng_url.as_deref(),
        config.brave_api_key.as_deref(),
        config.web_search_native,
    )
    .await;
    for status in &statuses {
        let label = match (status.configured, status.answered) {
            (false, _) => "--",
            (true, Some(true)) => "ok",
            // Web search is optional, so an unreachable backend is a warning.
            (true, _) => "warn",
        };
        println!("  {label}  {}", status.detail);
    }
    if !statuses.iter().any(|s| s.answered == Some(true)) {
        println!(
            "  warn  no backend answered, so web_search will fail. Check the network, set \
             `web_search_native: true`, point `searxng_url` at a SearxNG instance, or set \
             `brave_api_key` (or HQ_BRAVE_API_KEY)."
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
fn report_skill_issues(config: &HqConfig) -> usize {
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
        return 0;
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
    errors
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

/// The daemon refreshes its report every tick, a few seconds apart; this much silence means it is
/// not running.
const WATCH_REPORT_MAX_AGE_SECS: i64 = 120;

/// The watcher that keeps search current, and how current each index actually is.
fn report_search_index(config: &HqConfig) {
    use hq_daemon::vault_watch::status;

    let report = status::read(&config.vault_path);
    let worker = hq_daemon::embedding_worker::read(&config.vault_path);
    let freshness = hq_db::Database::open_existing(&config.db_path())
        .ok()
        .and_then(|db| db.with_conn(hq_db::search::index_freshness).ok());
    println!("\nSearch index");
    let lines = search_index_lines(
        config.vault_watch.enabled,
        report.as_ref(),
        freshness.as_ref(),
        chrono::Utc::now(),
    );
    for Line(label, text) in lines {
        println!("  {label}  {text}");
    }
    // Lite and a mobile instance with its data saver on run no embedding worker, so there is
    // nothing to report and "no report yet" would never clear.
    if config.embeds_notes() {
        for Line(label, text) in embedding_worker_lines(worker.as_ref(), chrono::Utc::now()) {
            println!("  {label}  {text}");
        }
    }
}

/// What the background embedder is doing. It holds off for longer after each provider failure,
/// which is worth saying: until it recovers, semantic search falls behind keyword search.
fn embedding_worker_lines(
    worker: Option<&hq_daemon::embedding_worker::EmbeddingWorkerStatus>,
    now: chrono::DateTime<chrono::Utc>,
) -> Vec<Line> {
    let Some(worker) = worker else {
        return vec![Line(
            "--",
            "no embedding report yet; the daemon writes one after its first embedding pass".into(),
        )];
    };
    let last_pass = worker
        .last_pass_at
        .as_deref()
        .and_then(parse_time)
        .map_or("not yet".to_string(), |t| ago(t, now));
    let mut lines = vec![Line(
        "--",
        format!(
            "embedding worker: {} embedded since the daemon started, last pass {last_pass}",
            worker.embedded
        ),
    )];
    if worker.backoff_secs > 0 {
        lines.push(Line(
            "warn",
            format!(
                "embedding is holding off for {} s after a provider failure: {}",
                worker.backoff_secs,
                worker.last_error.as_deref().unwrap_or("no reason recorded")
            ),
        ));
    }
    if worker.unreadable > 0 {
        lines.push(Line(
            "--",
            format!(
                "{} note(s) could not be read for embedding",
                worker.unreadable
            ),
        ));
    }
    if !worker.unreadable_paths.is_empty() {
        lines.push(Line(
            "--",
            format!("skipping now: {}", worker.unreadable_paths.join(", ")),
        ));
    }
    lines
}

fn ago(since: chrono::DateTime<chrono::Utc>, now: chrono::DateTime<chrono::Utc>) -> String {
    let secs = (now - since).num_seconds().max(0);
    match secs {
        0..=89 => format!("{secs}s ago"),
        90..=5399 => format!("{} min ago", secs / 60),
        _ => format!("{} h ago", secs / 3600),
    }
}

fn parse_time(text: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|t| t.with_timezone(&chrono::Utc))
}

fn search_index_lines(
    watcher_enabled: bool,
    report: Option<&hq_daemon::vault_watch::VaultWatchStatus>,
    freshness: Option<&hq_db::search::IndexFreshness>,
    now: chrono::DateTime<chrono::Utc>,
) -> Vec<Line> {
    let mut lines = Vec::new();
    match (watcher_enabled, report) {
        (false, _) => lines.push(Line(
            "--",
            "watcher off (vault_watch.enabled is false); search refreshes on the scheduled scan, \
             every 30 minutes"
                .into(),
        )),
        (true, None) => lines.push(Line(
            "warn",
            "no watcher report yet. The daemon writes it within seconds of `hq start`; if it is \
             running, check its log"
                .into(),
        )),
        (true, Some(report)) => lines.extend(watcher_lines(report, now)),
    }
    if let Some(freshness) = freshness {
        lines.extend(freshness_lines(freshness, now));
    }
    lines
}

fn watcher_lines(
    report: &hq_daemon::vault_watch::VaultWatchStatus,
    now: chrono::DateTime<chrono::Utc>,
) -> Vec<Line> {
    use hq_daemon::vault_watch::WatchState;

    let mut lines = Vec::new();
    let reported = parse_time(&report.updated_at);
    if let Some(reported) = reported
        && (now - reported).num_seconds() > WATCH_REPORT_MAX_AGE_SECS
    {
        lines.push(Line(
            "warn",
            format!(
                "the watcher report is {}; is the daemon running?",
                ago(reported, now).replace(" ago", " old")
            ),
        ));
    }

    let roots = report.roots.join(", ");
    let last_event = report
        .last_notification_at
        .as_deref()
        .and_then(parse_time)
        .map_or("none yet".to_string(), |t| ago(t, now));
    lines.push(match report.state {
        WatchState::Watching => Line(
            "ok",
            format!(
                "watching {roots} with file events; {} waiting; last event {last_event}",
                report.queue_depth
            ),
        ),
        WatchState::Polling => Line(
            "ok",
            format!(
                "polling {roots} for changes (file events are not available here); {} waiting",
                report.queue_depth
            ),
        ),
        WatchState::CatchingUp => Line(
            "warn",
            "watcher started but its first scan has not finished; search may be behind until it has"
                .into(),
        ),
        WatchState::NotWatching => Line(
            "warn",
            format!(
                "not watching: {}. The scheduled scan keeps search current, every 30 minutes",
                report.detail.as_deref().unwrap_or("no reason recorded")
            ),
        ),
    });

    if let Some(finished) = report.last_scan_finished_at.as_deref().and_then(parse_time) {
        lines.push(Line(
            "--",
            format!(
                "last full scan finished {} ({} since the daemon started)",
                ago(finished, now),
                report.full_scans
            ),
        ));
    }
    let lost = report.queue_overflows + report.watcher_overflows + report.dropped_events;
    if lost > 0 {
        lines.push(Line(
            "--",
            format!(
                "events were lost {} time(s) and recovered with a full scan",
                report.queue_overflows
                    + report.watcher_overflows
                    + u64::from(report.dropped_events > 0)
            ),
        ));
    }
    if report.abandoned > 0 {
        lines.push(Line(
            "warn",
            format!(
                "{} update(s) failed after retries; the scheduled scan will pick them up",
                report.abandoned
            ),
        ));
    }
    if report.files_failed > 0 {
        lines.push(Line(
            "warn",
            format!(
                "{} note(s) could not be read and were left as the index had them",
                report.files_failed
            ),
        ));
    }
    lines
}

fn freshness_lines(
    f: &hq_db::search::IndexFreshness,
    now: chrono::DateTime<chrono::Utc>,
) -> Vec<Line> {
    let mut lines = Vec::new();
    let synced = f
        .last_synced_at
        .and_then(|t| chrono::DateTime::from_timestamp(t, 0))
        .map_or(String::new(), |t| {
            format!("; last note indexed {}", ago(t, now))
        });
    lines.push(Line(
        "ok",
        format!("{} notes in the keyword index{synced}", f.keyword_indexed),
    ));
    if f.awaiting_embedding == 0 {
        lines.push(Line(
            "ok",
            format!("all {} notes have an embedding", f.embedded),
        ));
    } else {
        lines.push(Line(
            "warn",
            format!(
                "{} of {} notes await an embedding ({} embedded). Embedding runs apart from \
                 search, so keyword results are current while semantic results lag",
                f.awaiting_embedding, f.keyword_indexed, f.embedded
            ),
        ));
    }
    if f.stale_embeddings > 0 {
        lines.push(Line(
            "--",
            format!(
                "{} embedding(s) were made from an older version of their note and are queued to be redone",
                f.stale_embeddings
            ),
        ));
    }
    if f.orphan_embeddings > 0 {
        lines.push(Line(
            "warn",
            format!(
                "{} embedding(s) belong to notes that are gone; the next scan removes them",
                f.orphan_embeddings
            ),
        ));
    }
    lines
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
    let legacy = config.has_llm_key();
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

/// A default model nobody chose whose provider has no key, with the model `hq env` would move it to.
fn keyless_default_line(config: &HqConfig) -> Option<Line> {
    let keyed = super::env::keyed_providers(config, &[]);
    let mut moved = config.clone();
    let target = hq_core::setup_provider::follow_keys(&mut moved, keyed)?;
    Some(Line(
        "warn",
        format!(
            "default model `{}` belongs to a provider with no key; set `default_model: {target}` \
             (what `hq env set` picks for the key you have) or add that provider's key",
            config.default_model
        ),
    ))
}

async fn report_backend(config: &HqConfig) -> usize {
    println!("\nBackend");
    let config_path = HqConfig::config_read_path();
    let mut lines = describe_backend(
        config,
        &config_path,
        config_path.exists(),
        config.vault_path.exists(),
    );
    let has_backend = config.backends.is_configured() || config.has_llm_key() || config.local_only;
    lines.push(copilot_credential_line(uses_copilot(config)).await);
    lines.extend(probe_primary(config).await);
    if has_backend {
        let result = super::model_probe::probe_default_model(config).await;
        let (label, text) = super::model_probe::describe(&config.default_model, &result);
        lines.push(Line(label, text));
        lines.extend(keyless_default_line(config));
    }
    let fails = lines.iter().filter(|l| l.0 == "FAIL").count();
    for Line(label, text) in lines {
        println!("  {label}  {text}");
    }
    fails
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
    #[test]
    fn egress_report_fails_only_for_a_lite_config_with_something_refused() {
        let none = |_: &str| None;
        let mut cfg = HqConfig {
            openrouter_api_key: Some("k".into()),
            ..HqConfig::default()
        };
        assert!(
            run_egress_with(&cfg, &none).is_ok(),
            "the full profile only reports"
        );

        cfg.profile = hq_core::config::Profile::Lite;
        assert!(
            run_egress_with(&cfg, &none).is_err(),
            "lite refuses a configured OpenRouter key"
        );

        cfg.lite.allow_egress = vec!["openrouter".into()];
        assert!(
            run_egress_with(&cfg, &none).is_ok(),
            "listing the item allows it"
        );

        // The environment counts too: a key exported in the shell is refused.
        let env = |n: &str| (n == "ANTHROPIC_API_KEY").then(|| "k".to_string());
        assert!(run_egress_with(&cfg, &env).is_err());
    }

    mod search_index {
        use super::*;
        use chrono::{Duration, TimeZone, Utc};
        use hq_daemon::vault_watch::{PipelineStats, VaultWatchStatus, WatchState};
        use hq_db::search::IndexFreshness;

        fn now() -> chrono::DateTime<Utc> {
            Utc.with_ymd_and_hms(2026, 10, 15, 12, 0, 0).unwrap()
        }

        fn report(state: WatchState) -> VaultWatchStatus {
            let mut r = PipelineStats::default().snapshot();
            r.state = state;
            r.updated_at = now().to_rfc3339();
            r
        }

        fn texts(lines: &[Line]) -> String {
            lines
                .iter()
                .map(|l| format!("{} {}", l.0, l.1))
                .collect::<Vec<_>>()
                .join("\n")
        }

        #[test]
        fn a_watcher_that_is_switched_off_says_so_and_names_the_fallback() {
            let lines = search_index_lines(false, None, None, now());
            assert_eq!(labels(&lines), vec!["--"]);
            assert!(texts(&lines).contains("30 minutes"));
        }

        #[test]
        fn no_report_yet_is_a_warning_not_silence() {
            let lines = search_index_lines(true, None, None, now());
            assert_eq!(labels(&lines), vec!["warn"]);
        }

        #[test]
        fn a_running_watcher_shows_its_roots_queue_and_last_event() {
            let mut r = report(WatchState::Watching);
            r.queue_depth = 3;
            r.last_notification_at = Some((now() - Duration::seconds(42)).to_rfc3339());
            let lines = search_index_lines(true, Some(&r), None, now());
            let text = texts(&lines);
            assert_eq!(labels(&lines)[0], "ok");
            assert!(text.contains("Notebooks"), "{text}");
            assert!(text.contains("3 waiting"), "{text}");
            assert!(text.contains("42s ago"), "{text}");
        }

        #[test]
        fn a_watcher_that_is_down_is_degraded_with_its_reason() {
            let mut r = report(WatchState::NotWatching);
            r.detail = Some("Notebooks does not exist yet".into());
            let text = texts(&search_index_lines(true, Some(&r), None, now()));
            assert!(
                text.starts_with("warn not watching: Notebooks does not exist yet"),
                "{text}"
            );
            assert!(text.contains("scheduled scan"), "{text}");
        }

        #[test]
        fn search_is_not_called_current_before_the_first_scan_finishes() {
            let lines =
                search_index_lines(true, Some(&report(WatchState::CatchingUp)), None, now());
            assert_eq!(labels(&lines), vec!["warn"]);
            assert!(texts(&lines).contains("first scan has not finished"));
        }

        #[test]
        fn a_polling_watcher_is_ok_but_says_events_are_unavailable() {
            let text = texts(&search_index_lines(
                true,
                Some(&report(WatchState::Polling)),
                None,
                now(),
            ));
            assert!(text.starts_with("ok polling"), "{text}");
            assert!(text.contains("not available"), "{text}");
        }

        #[test]
        fn a_report_the_daemon_stopped_updating_is_called_stale() {
            let mut r = report(WatchState::Watching);
            r.updated_at = (now() - Duration::minutes(10)).to_rfc3339();
            let lines = search_index_lines(true, Some(&r), None, now());
            assert_eq!(labels(&lines)[0], "warn");
            assert!(texts(&lines).contains("10 min old"), "{}", texts(&lines));
        }

        #[test]
        fn lost_events_and_failed_updates_are_surfaced() {
            let mut r = report(WatchState::Watching);
            r.watcher_overflows = 2;
            r.dropped_events = 9;
            r.abandoned = 1;
            r.files_failed = 4;
            r.full_scans = 3;
            r.last_scan_finished_at = Some((now() - Duration::minutes(5)).to_rfc3339());
            let text = texts(&search_index_lines(true, Some(&r), None, now()));
            assert!(text.contains("events were lost 3 time(s)"), "{text}");
            assert!(text.contains("warn 1 update(s) failed"), "{text}");
            assert!(text.contains("warn 4 note(s) could not be read"), "{text}");
            assert!(
                text.contains("last full scan finished 5 min ago (3 since"),
                "{text}"
            );
        }

        #[test]
        fn the_embedding_worker_reports_its_back_off_and_its_reason() {
            use hq_daemon::embedding_worker::EmbedStats;

            assert_eq!(labels(&embedding_worker_lines(None, now())), vec!["--"]);

            let stats = EmbedStats::default();
            stats
                .embedded
                .store(12, std::sync::atomic::Ordering::Relaxed);
            let healthy = embedding_worker_lines(Some(&stats.snapshot()), now());
            assert_eq!(labels(&healthy), vec!["--"]);
            assert!(texts(&healthy).contains("12 embedded since the daemon started"));

            stats
                .backoff_secs
                .store(120, std::sync::atomic::Ordering::Relaxed);
            *stats.last_error.lock().unwrap() = Some("rate limited".into());
            let backing_off = embedding_worker_lines(Some(&stats.snapshot()), now());
            assert_eq!(labels(&backing_off), vec!["--", "warn"]);
            assert!(texts(&backing_off).contains("holding off for 120 s"));
            assert!(texts(&backing_off).contains("rate limited"));
        }

        #[test]
        fn notes_the_embedder_is_skipping_are_named() {
            use hq_daemon::embedding_worker::EmbedStats;

            let stats = EmbedStats::default();
            stats
                .unreadable
                .store(2, std::sync::atomic::Ordering::Relaxed);
            *stats.unreadable_paths.lock().unwrap() = vec![
                "Notebooks/a-broken.md".to_string(),
                "skills/old/SKILL.md".to_string(),
            ];

            let text = texts(&embedding_worker_lines(Some(&stats.snapshot()), now()));

            assert!(text.contains("2 note(s) could not be read for embedding"));
            assert!(
                text.contains("skipping now: Notebooks/a-broken.md, skills/old/SKILL.md"),
                "{text}"
            );
        }

        #[test]
        fn edited_notes_awaiting_a_new_embedding_are_mentioned() {
            let f = IndexFreshness {
                keyword_indexed: 10,
                embedded: 10,
                stale_embeddings: 4,
                ..IndexFreshness::default()
            };
            let text = texts(&search_index_lines(false, None, Some(&f), now()));
            assert!(
                text.contains("4 embedding(s) were made from an older version"),
                "{text}"
            );
        }

        #[test]
        fn keyword_and_embedding_freshness_are_reported_separately() {
            let current = IndexFreshness {
                keyword_indexed: 120,
                embedded: 120,
                ..IndexFreshness::default()
            };
            assert_eq!(
                labels(&search_index_lines(false, None, Some(&current), now())),
                vec!["--", "ok", "ok"]
            );

            let lagging = IndexFreshness {
                keyword_indexed: 120,
                embedded: 100,
                awaiting_embedding: 20,
                orphan_embeddings: 3,
                stale_embeddings: 0,
                last_synced_at: Some((now() - Duration::minutes(2)).timestamp()),
            };
            let lines = search_index_lines(false, None, Some(&lagging), now());
            let text = texts(&lines);
            assert!(
                text.contains("ok 120 notes in the keyword index; last note indexed 2 min ago"),
                "{text}"
            );
            assert!(
                text.contains("warn 20 of 120 notes await an embedding (100 embedded)"),
                "{text}"
            );
            assert!(
                text.contains("warn 3 embedding(s) belong to notes that are gone"),
                "{text}"
            );
        }
    }
}

#[cfg(test)]
mod import_tool_tests {
    use super::*;
    use hq_convert::ingest::Capabilities;

    fn caps(ocr: bool, pdf_tables: bool, legacy_office: bool) -> Capabilities {
        Capabilities {
            ocr,
            pdf_tables,
            legacy_office,
        }
    }

    #[test]
    fn a_machine_with_everything_says_so_in_three_lines() {
        let lines = import_tool_lines(&caps(true, true, true), true, false);
        assert_eq!(
            lines.iter().map(|l| l.0).collect::<Vec<_>>(),
            ["ok", "ok", "ok"]
        );
    }

    #[test]
    fn a_bare_server_is_told_what_each_tool_buys_and_how_to_get_it() {
        let lines = import_tool_lines(&caps(false, false, false), false, false);
        assert_eq!(
            lines.iter().map(|l| l.0).collect::<Vec<_>>(),
            ["warn", "--", "--"]
        );
        assert!(
            lines[0]
                .1
                .contains("apt install tesseract-ocr poppler-utils"),
            "{}",
            lines[0].1
        );
        assert!(lines[0].1.contains("blocked"), "{}", lines[0].1);
        assert!(
            lines[1].1.contains("loose text") && lines[1].1.contains("apt install poppler-utils")
        );
        assert!(lines[2].1.contains("libreoffice-writer") && lines[2].1.contains(".xls"));
    }

    #[test]
    fn ocr_without_pdftoppm_works_for_pictures_only() {
        let lines = import_tool_lines(&caps(true, true, true), false, false);
        assert_eq!(lines[0].0, "warn");
        assert!(
            lines[0].1.contains("scanned PDFs need pdftoppm"),
            "{}",
            lines[0].1
        );
    }

    #[test]
    fn macos_is_pointed_at_brew() {
        let lines = import_tool_lines(&caps(false, false, false), false, true);
        assert!(
            lines[0].1.contains("brew install tesseract poppler"),
            "{}",
            lines[0].1
        );
        assert!(
            lines[2].1.contains("brew install --cask libreoffice"),
            "{}",
            lines[2].1
        );
        assert!(lines.iter().all(|l| !l.1.contains("apt ")));
    }
}
