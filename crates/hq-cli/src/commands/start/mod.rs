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

fn unlock_signing_keychain() {
    let home = std::env::var("HOME").unwrap_or_default();
    let keychain = format!("{}/.hq/signing.keychain", home);
    if !std::path::Path::new(&keychain).exists() {
        return;
    }
    let hostname = std::process::Command::new("hostname")
        .arg("-s")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_default();
    let pass = format!("hq-{}-signing", hostname.trim());
    let _ = std::process::Command::new("security")
        .args(["unlock-keychain", "-p", &pass, &keychain])
        .output();
}

pub async fn run(config: &HqConfig, component: &str) -> Result<()> {
    // Unlock signing keychain at startup so codesign works during self-update
    unlock_signing_keychain();

    let vault =
        Arc::new(VaultClient::new(config.vault_path.clone()).context("failed to open vault")?);

    let db_path = config.db_path();
    let db = Arc::new(Database::open(&db_path).context("failed to open database")?);

    info!(vault = %config.vault_path.display(), "HQ starting");
    if matches!(component, "all" | "daemon") {
        notify_if_bash_refused(config, &db);
    }

    match component {
        "all" => {
            println!("Starting all HQ components...");
            start_all(config, vault, db).await
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

async fn start_all(config: &HqConfig, vault: Arc<VaultClient>, db: Arc<Database>) -> Result<()> {
    ensure_no_running_daemon(config)?;
    print_components(config);

    let shutdown = ShutdownSignal::new();
    let cleanup = CleanupRegistry::new();
    spawn_daemon(config, vault.clone(), db.clone(), shutdown.clone());
    spawn_discord(config, &vault, &db);
    spawn_telegram(config, &vault, &db);
    spawn_web_server(config, &vault, &db);

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
    let notification_categories = config.relay.discord_notification_categories.clone();
    let family_channel_id = config.relay.discord_family_channel_id;
    let family_users = config.relay.discord_family_users.clone();
    tokio::spawn(async move {
        info!("discord: starting relay");
        let result = relay::run_discord_relay(
            &token,
            vault,
            db,
            allowed_user_ids,
            notification_categories,
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
    let notifications_token = config.relay.notifications_token.clone();
    tokio::spawn(async move {
        info!("telegram: starting relay");
        let result = relay::run_telegram_relay(&token, vault, db, notifications_token, None).await;
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

/// The WebSocket and web UI server. An open non-loopback bind skips only this
/// server; the relays and the daemon still start.
fn spawn_web_server(config: &HqConfig, vault: &Arc<VaultClient>, db: &Arc<Database>) {
    let registry = Arc::new(hq_mcp::registry::create_default_registry(
        vault.clone(),
        db.clone(),
        hq_core::skills_dir(&config.vault_path),
        config.vault_path.join("Agents"),
        Some(config),
    ));
    let web_auth_token = config.web_auth_token.clone();
    if let Err(e) = hq_web::auth::check_web_bind(&config.web_bind, web_auth_token.as_deref()) {
        tracing::error!("ws: not starting the web server: {e}");
        eprintln!("Web server not started: {e}");
        return;
    }
    let vault_path = config.vault_path.clone();
    let static_dir = config.web_static_dir.clone();
    let (bind, port) = (config.web_bind.clone(), config.ws_port);
    tokio::spawn(async move {
        info!(port, "ws: starting server");
        // Before any TLS client is built: the dep tree enables both aws-lc-rs
        // and ring, so rustls cannot auto-pick and would panic.
        hq_web::install_default_crypto_provider();
        let repo_root = vault_path.parent().unwrap_or(&vault_path);
        let static_dir = static_dir.unwrap_or_else(|| repo_root.join("web").join("dist"));
        // Files are read per request, so a later web deploy is picked up
        // without restarting hq; a missing build is only a warning.
        if !static_dir.join("index.html").exists() {
            tracing::warn!(path = %static_dir.display(), "ws: no web UI build yet (index.html missing)");
        }
        info!(path = %static_dir.display(), "ws: serving web UI");
        let mut state = hq_web::WsState::new(vault_path, Some(static_dir)).with_registry(registry);
        state.web_auth_token = web_auth_token;
        let state = Arc::new(state);
        hq_web::install_ask_runner(&state);
        serve_web(state, &bind, port).await;
    });
}

async fn serve_web(state: Arc<hq_web::WsState>, bind: &str, port: u16) {
    let app = hq_web::create_router(state);
    let addr: std::net::SocketAddr = format!("{bind}:{port}")
        .parse()
        .unwrap_or_else(|_| std::net::SocketAddr::from(([127, 0, 0, 1], port)));
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(listener) => listener,
        Err(e) => {
            tracing::error!(port, "ws: failed to bind port: {e}");
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
        config.relay.discord_notification_categories.clone(),
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
    relay::run_telegram_relay(
        token,
        vault,
        db,
        config.relay.notifications_token.clone(),
        None,
    )
    .await
}

#[cfg(not(feature = "telegram"))]
async fn start_telegram(
    _config: &HqConfig,
    _vault: Arc<VaultClient>,
    _db: Arc<Database>,
) -> Result<()> {
    anyhow::bail!("this build was compiled without the \"telegram\" feature")
}
