//! `hq start` command — launches all HQ components (daemon, relays, websocket).

pub mod common;
pub mod daemon;
pub mod relay;

use anyhow::{Context, Result};
use hq_agent::shutdown::{CleanupRegistry, ShutdownSignal, wait_for_shutdown_signal};
use hq_core::config::HqConfig;
use hq_db::Database;
use hq_vault::VaultClient;
use std::sync::Arc;
use std::time::Duration;
use tracing::info;

pub async fn run(config: &HqConfig, component: &str) -> Result<()> {
    // The web port answers "Starting HQ" from the first moment, before the slow steps below.
    let early = if component == "all" {
        bind_web_early(config).await?
    } else {
        None
    };
    let vault =
        Arc::new(VaultClient::new(config.vault_path.clone()).context("failed to open vault")?);

    let db_path = config.db_path();
    let db = Arc::new(Database::open(&db_path).context("failed to open database")?);
    hq_agent::install_ledger(db.clone());

    info!(vault = %config.vault_path.display(), "HQ starting");
    super::skills::sync_builtin_at_start(config);
    if matches!(component, "all" | "daemon") && config.profile.expects_shell_sandbox() {
        notify_if_bash_refused(config, &db);
    }

    match component {
        "all" => {
            println!("Starting all HQ components...");
            start_all(config, vault, db, early).await
        }
        "daemon" => {
            println!("Starting daemon...");
            let shutdown = ShutdownSignal::new();
            let shutdown_trigger = shutdown.clone();
            tokio::spawn(async move {
                wait_for_shutdown_signal().await;
                shutdown_trigger.trigger();
            });
            daemon::run_daemon(config, vault, db, shutdown).await
        }
        "relay" | "discord" | "telegram" if config.profile.is_lite() => {
            anyhow::bail!("the chat relays are not part of HQ Lite (profile: lite)")
        }
        "relay" | "discord" => {
            println!("Starting Discord relay...");
            start_discord(config, vault, db).await
        }
        "telegram" => {
            println!("Starting Telegram relay...");
            start_telegram(config, vault, db).await
        }
        other => {
            println!("Unknown component: {other}");
            println!("Options: all, daemon, relay, discord, telegram");
            Ok(())
        }
    }
}

/// Error log plus an owner notification when the default `required` sandbox has
/// no backend, so an upgrade that silently disables agent bash is not silent.
fn notify_if_bash_refused(config: &HqConfig, db: &Database) {
    use hq_agent::bash_sandbox::{BashSettings, available_backend_for, report_refusal};

    let settings = BashSettings::from_config(&config.governance.bash, Vec::new());
    let boot_id = chrono::Utc::now().timestamp().to_string();
    let backend = available_backend_for(settings.network);
    if let Err(e) = report_refusal(&settings, backend, db, &boot_id) {
        tracing::warn!(error = %e, "could not file the bash sandbox notification");
    }
}

/// Cleanup handlers get this long before shutdown stops waiting on them.
const CLEANUP_FAILSAFE: Duration = Duration::from_secs(10);

/// Settle time for spawned tasks to finish their own shutdown sequences.
const SHUTDOWN_SETTLE: Duration = Duration::from_millis(500);

async fn start_all(
    config: &HqConfig,
    vault: Arc<VaultClient>,
    db: Arc<Database>,
    early: Option<hq_web::hello::StartingServer>,
) -> Result<()> {
    ensure_no_running_daemon(config)?;
    print_components(config);
    if let Some(server) = &early {
        server.progress.at(STEP_LOADING);
    }

    let shutdown = ShutdownSignal::new();
    let cleanup = CleanupRegistry::new();
    spawn_daemon(config, vault.clone(), db.clone(), shutdown.clone());
    // Lite has no chat relay, whatever tokens are configured.
    if !config.profile.is_lite() {
        spawn_discord(config, &vault, &db);
        spawn_telegram(config, &vault, &db);
    }
    spawn_web_server(config, &vault, &db, early);

    println!("All components running. Press Ctrl+C to stop.");
    println!();
    wait_for_shutdown_signal().await;
    println!("\nShutting down...");
    shutdown.trigger();
    cleanup.run_all(CLEANUP_FAILSAFE).await;
    tokio::time::sleep(SHUTDOWN_SETTLE).await;
    info!("all components stopped");
    Ok(())
}

/// Fail the whole process fast when another instance holds the daemon lock.
/// Otherwise `run_daemon`'s own acquire, inside a bare spawn that only logs,
/// would decline quietly while the web server and relays still started: a
/// second daemon with no scheduler racing the real one for the same port.
/// The lock is dropped at once; `run_daemon` holds its own for the process.
fn ensure_no_running_daemon(config: &HqConfig) -> Result<()> {
    match hq_daemon::instance_lock::DaemonLock::try_acquire(&config.vault_path) {
        Err(e) => Err(e).context("failed to check daemon lock"),
        Ok(None) => anyhow::bail!(
            "another hq daemon is already running (instance lock held) — stop it first"
        ),
        Ok(Some(_lock)) => Ok(()),
    }
}

fn print_components(config: &HqConfig) {
    println!("  Daemon scheduler");
    if config.relay.discord_enabled {
        println!("  Discord relay");
    }
    if config.relay.telegram_enabled {
        println!("  Telegram relay");
    }
    println!("  WebSocket server on port {}", config.ws_port);
    println!();
}

fn spawn_daemon(
    config: &HqConfig,
    vault: Arc<VaultClient>,
    db: Arc<Database>,
    shutdown: ShutdownSignal,
) {
    let config = config.clone();
    tokio::spawn(async move {
        info!("daemon: starting scheduler");
        if let Err(e) = daemon::run_daemon(&config, vault, db, shutdown).await {
            tracing::error!("daemon error: {e}");
        }
    });
}

#[cfg(feature = "discord")]
fn spawn_discord(config: &HqConfig, vault: &Arc<VaultClient>, db: &Arc<Database>) {
    if !config.relay.discord_enabled {
        return;
    }
    let Some(token) = config.relay.discord_token.clone() else {
        println!("  Discord enabled but no token configured — skipping");
        return;
    };
    let (vault, db) = (vault.clone(), db.clone());
    let allowed_user_ids = config.relay.discord_allowed_user_ids.clone();
    let family_channel_id = config.relay.discord_family_channel_id;
    let family_users = config.relay.discord_family_users.clone();
    tokio::spawn(async move {
        info!("discord: starting relay");
        let result = relay::run_discord_relay(
            &token,
            vault,
            db,
            allowed_user_ids,
            family_channel_id,
            family_users,
            None,
        )
        .await;
        if let Err(e) = result {
            tracing::error!("discord relay error: {e}");
        }
    });
}

#[cfg(not(feature = "discord"))]
fn spawn_discord(config: &HqConfig, _vault: &Arc<VaultClient>, _db: &Arc<Database>) {
    if config.relay.discord_enabled {
        println!(
            "  Discord enabled in config but this build was compiled without the \"discord\" feature — skipping"
        );
    }
}

#[cfg(feature = "telegram")]
fn spawn_telegram(config: &HqConfig, vault: &Arc<VaultClient>, db: &Arc<Database>) {
    if !config.relay.telegram_enabled {
        return;
    }
    let Some(token) = config.relay.telegram_token.clone() else {
        println!("  Telegram enabled but no token configured — skipping");
        return;
    };
    let (vault, db) = (vault.clone(), db.clone());
    tokio::spawn(async move {
        info!("telegram: starting relay");
        let result = relay::run_telegram_relay(&token, vault, db, None).await;
        if let Err(e) = result {
            tracing::error!("telegram relay error: {e}");
        }
    });
}

#[cfg(not(feature = "telegram"))]
fn spawn_telegram(config: &HqConfig, _vault: &Arc<VaultClient>, _db: &Arc<Database>) {
    if config.relay.telegram_enabled {
        println!(
            "  Telegram enabled in config but this build was compiled without the \"telegram\" feature — skipping"
        );
    }
}

/// Start steps the early web port reports (`hq_web::hello::START_STEPS`).
const STEP_LOADING: usize = 1;
const STEP_ALMOST_READY: usize = 2;

fn web_addr(bind: &str, port: u16) -> std::net::SocketAddr {
    format!("{bind}:{port}")
        .parse()
        .unwrap_or_else(|_| std::net::SocketAddr::from(([127, 0, 0, 1], port)))
}

/// Whether HQ is useless without its web port, so a port it cannot bind ends the process. On the
/// phone the app is the only way in, and another app may hold the port while HQ is down: running
/// on without a web server would leave that app answering in HQ's place, unnoticed. Ending makes
/// the app's supervisor see the failure, report it and try again.
fn web_port_required(config: &HqConfig) -> bool {
    cfg!(target_os = "android") || config.profile.is_mobile()
}

/// Exit status of a start that ended because the web port could not be bound.
const WEB_BIND_FAILED_EXIT: i32 = 3;

/// Binds the web port before the vault and database open, answering `/api/hello` with the start
/// step and every other route with "starting". `None` leaves the port to [`spawn_web_server`];
/// an error means the port is required ([`web_port_required`]) and could not be bound.
async fn bind_web_early(config: &HqConfig) -> Result<Option<hq_web::hello::StartingServer>> {
    if hq_web::auth::check_web_bind(&config.web_bind, config.web_auth_token.as_deref()).is_err() {
        return Ok(None);
    }
    let Some(server) = hq_web::hello::StartingServer::new(
        &config.vault_path,
        config.profile,
        config.web_auth_token.as_deref(),
    )
    .inspect_err(|e| tracing::warn!("ws: no early start page: {e:#}"))
    .ok() else {
        return Ok(None);
    };
    let listener =
        match tokio::net::TcpListener::bind(web_addr(&config.web_bind, config.ws_port)).await {
            Ok(listener) => listener,
            Err(e) if web_port_required(config) => {
                return Err(e).with_context(|| {
                    format!(
                        "could not listen on the web port {}; not starting without it",
                        config.ws_port
                    )
                });
            }
            Err(e) => {
                tracing::error!(port = config.ws_port, "ws: failed to bind port: {e}");
                return Ok(None);
            }
        };
    let app = server.router();
    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            tracing::error!("ws server error: {e}");
        }
    });
    Ok(Some(server))
}

/// The WebSocket and web UI server. An open non-loopback bind skips only this
/// server; the relays and the daemon still start. With an `early` port already
/// answering "starting", the full router takes it over.
fn spawn_web_server(
    config: &HqConfig,
    vault: &Arc<VaultClient>,
    db: &Arc<Database>,
    early: Option<hq_web::hello::StartingServer>,
) {
    let web_auth_token = config.web_auth_token.clone();
    if let Err(e) = hq_web::auth::check_web_bind(&config.web_bind, web_auth_token.as_deref()) {
        tracing::error!("ws: not starting the web server: {e}");
        eprintln!("Web server not started: {e}");
        return;
    }
    let state = build_web_state(
        config,
        vault,
        db,
        config.web_static_dir.clone(),
        web_auth_token,
        &config.web_bind,
    );
    if let Some(server) = early {
        server.progress.at(STEP_ALMOST_READY);
        server.ready(hq_web::create_router(state));
        info!(
            port = config.ws_port,
            "ws: serving on the port opened at start"
        );
        return;
    }
    let (bind, port) = (config.web_bind.clone(), config.ws_port);
    let required = web_port_required(config);
    tokio::spawn(async move {
        info!(port, "ws: starting server");
        serve_web(state, &bind, port, required).await;
    });
}

/// Shared state for the web server: tool registry, UI directory and token.
/// `static_dir` unset falls back to [`default_static_dir`]. Used by `hq start`
/// and `hq web`, so both serve the same thing.
pub(crate) fn build_web_state(
    config: &HqConfig,
    vault: &Arc<VaultClient>,
    db: &Arc<Database>,
    static_dir: Option<std::path::PathBuf>,
    web_auth_token: Option<String>,
    bind: &str,
) -> Arc<hq_web::WsState> {
    let registry = Arc::new(hq_mcp::registry::create_default_registry(
        vault.clone(),
        db.clone(),
        hq_core::skills_dir(&config.vault_path),
        config.vault_path.join("Agents"),
        Some(config),
    ));
    // Before any TLS client is built: the dep tree enables both aws-lc-rs
    // and ring, so rustls cannot auto-pick and would panic.
    hq_web::install_default_crypto_provider();
    let vault_path = config.vault_path.clone();
    let repo_root = vault_path.parent().unwrap_or(&vault_path);
    let static_dir = static_dir.unwrap_or_else(|| default_static_dir(repo_root));
    // Files are read per request, so a later web deploy is picked up
    // without restarting hq; a missing build is only a warning.
    if !static_dir.join("index.html").exists() {
        tracing::warn!(path = %static_dir.display(), "ws: no web UI build yet (index.html missing)");
    }
    info!(path = %static_dir.display(), "ws: serving web UI");
    let mut state = hq_web::WsState::new(vault_path, Some(static_dir))
        .with_profile(config.profile)
        .with_registry(registry);
    state.web_auth_token = web_auth_token;
    // `hq web --lan` overrides the configured bind, and /mcp's dev switch must follow the real one.
    state.web_bind_is_loopback = hq_web::auth::bind_is_loopback(bind);
    if let Err(e) = state.configure_connector(&config.mcp_connector) {
        tracing::error!("ws: claude.ai connector stays off: {e}");
        eprintln!("claude.ai connector not enabled: {e}");
    }
    if let Err(e) = state.configure_web_push(&config.web_push) {
        tracing::error!("ws: web push stays off: {e}");
        eprintln!("web push not enabled: {e}");
    }
    let state = Arc::new(state);
    hq_web::install_ask_runner(&state);
    state
}

async fn serve_web(state: Arc<hq_web::WsState>, bind: &str, port: u16, required: bool) {
    let app = hq_web::create_router(state);
    let listener = match tokio::net::TcpListener::bind(web_addr(bind, port)).await {
        Ok(listener) => listener,
        Err(e) => {
            tracing::error!(port, "ws: failed to bind port: {e}");
            if required {
                // See `web_port_required`: no HQ on the phone runs on without its web port.
                eprintln!("Could not listen on the web port {port}: {e}. Stopping.");
                std::process::exit(WEB_BIND_FAILED_EXIT);
            }
            return;
        }
    };
    if let Err(e) = axum::serve(listener, app).await {
        tracing::error!("ws server error: {e}");
    }
}

#[cfg(feature = "discord")]
async fn start_discord(
    config: &HqConfig,
    vault: Arc<VaultClient>,
    db: Arc<Database>,
) -> Result<()> {
    let token =
        config.relay.discord_token.as_ref().context(
            "Discord token not configured. Set relay.discord_token in ~/.hq/config.yaml",
        )?;
    relay::run_discord_relay(
        token,
        vault,
        db,
        config.relay.discord_allowed_user_ids.clone(),
        config.relay.discord_family_channel_id,
        config.relay.discord_family_users.clone(),
        None,
    )
    .await
}

#[cfg(not(feature = "discord"))]
async fn start_discord(
    _config: &HqConfig,
    _vault: Arc<VaultClient>,
    _db: Arc<Database>,
) -> Result<()> {
    anyhow::bail!("this build was compiled without the \"discord\" feature")
}

#[cfg(feature = "telegram")]
async fn start_telegram(
    config: &HqConfig,
    vault: Arc<VaultClient>,
    db: Arc<Database>,
) -> Result<()> {
    let token =
        config.relay.telegram_token.as_ref().context(
            "Telegram token not configured. Set relay.telegram_token in ~/.hq/config.yaml",
        )?;
    relay::run_telegram_relay(token, vault, db, None).await
}

#[cfg(not(feature = "telegram"))]
async fn start_telegram(
    _config: &HqConfig,
    _vault: Arc<VaultClient>,
    _db: Arc<Database>,
) -> Result<()> {
    anyhow::bail!("this build was compiled without the \"telegram\" feature")
}

/// Where the web UI is looked for when `web_static_dir` is unset. The first candidate with an
/// `index.html` wins; with none, the first is returned so the "no web UI build" warning names it.
pub(crate) fn default_static_dir(repo_root: &std::path::Path) -> std::path::PathBuf {
    let mut candidates = vec![
        repo_root.join("web").join("dist"),
        // A source checkout after `bun run build` (what `hq web --build` runs).
        repo_root
            .join("apps")
            .join("hq-web")
            .join("dist")
            .join("client"),
    ];
    // A portable install keeps the web files next to the program: `hq.exe` and `web\`. They are
    // plain files, so a policy that restricts which programs run does not touch them.
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(std::path::Path::to_path_buf))
    {
        candidates.push(dir.join("web"));
    }
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/share"))
        });
    if let Some(dir) = data_home {
        // Where the install script puts the web UI.
        candidates.push(dir.join("agent-hq").join("web").join("dist"));
    }
    candidates.push(std::path::PathBuf::from("/usr/local/share/hq/web/dist"));
    first_with_index(candidates)
}

fn first_with_index(candidates: Vec<std::path::PathBuf>) -> std::path::PathBuf {
    let first = candidates[0].clone();
    candidates
        .into_iter()
        .find(|c| c.join("index.html").exists())
        .unwrap_or(first)
}

#[cfg(test)]
mod web_port_tests {
    use super::*;

    fn config_on(
        vault: &std::path::Path,
        port: u16,
        profile: hq_core::config::Profile,
    ) -> HqConfig {
        HqConfig {
            vault_path: vault.to_path_buf(),
            web_auth_token: Some("example-web-token".into()),
            web_bind: "127.0.0.1".into(),
            ws_port: port,
            profile,
            ..Default::default()
        }
    }

    /// Another program on the port: the phone's HQ ends, so its supervisor sees it; elsewhere
    /// HQ starts without the early page and reports the port later.
    #[tokio::test]
    async fn a_taken_web_port_ends_the_phone_start_only() {
        let vault = tempfile::TempDir::new().unwrap();
        let squatter = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = squatter.local_addr().unwrap().port();

        let mobile = config_on(vault.path(), port, hq_core::config::Profile::Mobile);
        assert!(web_port_required(&mobile));
        let err = bind_web_early(&mobile).await.err().expect("refused");
        assert!(err.to_string().contains(&port.to_string()), "{err:#}");

        let full = config_on(vault.path(), port, hq_core::config::Profile::Full);
        assert_eq!(web_port_required(&full), cfg!(target_os = "android"));
        if !web_port_required(&full) {
            assert!(bind_web_early(&full).await.unwrap().is_none());
        }
        drop(squatter);
    }

    #[tokio::test]
    async fn a_free_web_port_is_held_from_the_start() {
        let vault = tempfile::TempDir::new().unwrap();
        let port = {
            let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            free.local_addr().unwrap().port()
        };
        let mobile = config_on(vault.path(), port, hq_core::config::Profile::Mobile);
        assert!(bind_web_early(&mobile).await.unwrap().is_some());
        assert!(
            std::net::TcpListener::bind(("127.0.0.1", port)).is_err(),
            "HQ holds the port"
        );
    }
}

#[cfg(test)]
mod static_dir_tests {
    use super::first_with_index;

    #[test]
    fn picks_the_first_candidate_that_has_an_index() {
        let tmp = std::env::temp_dir().join(format!("hq-static-{}", std::process::id()));
        let (a, b, c) = (tmp.join("a"), tmp.join("b"), tmp.join("c"));
        for d in [&a, &b, &c] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(b.join("index.html"), "x").unwrap();
        std::fs::write(c.join("index.html"), "x").unwrap();
        assert_eq!(first_with_index(vec![a.clone(), b.clone(), c]), b);
        assert_eq!(first_with_index(vec![a.clone(), tmp.join("nope")]), a);
        std::fs::remove_dir_all(&tmp).ok();
    }
}
