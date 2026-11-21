//! Embedded web UI server with API endpoints and WebSocket.

mod api;
pub mod hello;
mod status_ai;
pub use api::HEALTH_SERVICE;
pub use router::create_router;
pub use vault_api::{lite_hides, lite_hides_resolved};
mod adoption_api;
pub mod auth;
mod auto_title;
mod budgets_api;
mod capabilities_api;
mod chat_apps;
mod chat_folders_api;
mod chat_uploads;
mod coder_api;
mod copilot_usage_api;
mod device_actions_api;
mod device_auth;
mod devices_door;
mod dialog_answer;
mod edge_worker_api;
mod error;
mod fetch_sites_api;
pub mod github_connect_api;
pub mod gmail_ingest;
mod helper_runs_api;
mod helper_setup_api;
mod lane_quotas_api;
mod mcp_http;
mod note_ids;
mod note_import;
mod note_trash;
mod note_undo;
mod notifications_api;
mod notifications_watch;
mod oauth;
mod offers_api;
mod openrouter_usage_api;
mod origin;
mod prefs_api;
mod profile_gate;
mod provider_keys;
mod restart_api;
mod router;
mod sandbox_grants_api;
mod security_headers;
mod session_driver;
mod session_folders_api;
mod sessions_api;
mod settings_api;
mod setup_api;
mod subagent_followup;
#[cfg(feature = "suite")]
mod suite_api;
mod tasks_api;
mod tasks_watch;
mod threads_api;
pub mod tls;
mod usage_forecast_api;
mod usage_providers_api;
mod vault_api;
mod verification_api;
mod web_push;
mod workbench_api;
mod ws;

#[cfg(test)]
use axum::Router;
use hq_tool_api::registry::ToolRegistry;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::broadcast;

/// Install the rustls process-level CryptoProvider. Required because the
/// dependency tree enables BOTH `aws-lc-rs` and `ring`, so rustls 0.23 cannot
/// auto-select one and panics on first TLS use. Idempotent; call once at process
/// startup, before any TLS client is built.
pub fn install_default_crypto_provider() {
    let _ = tokio_rustls::rustls::crypto::aws_lc_rs::default_provider().install_default();
}

/// Shared server state.
pub struct WsState {
    pub(crate) tx: broadcast::Sender<String>,
    pub(crate) vault_path: PathBuf,
    /// Opened once at server construction and reused wherever request handlers
    /// need DB access (e.g. MCP-over-HTTP gateway telemetry), instead of
    /// opening a fresh connection pool per request.
    pub(crate) db: hq_db::Database,
    pub(crate) static_dir: Option<PathBuf>,
    pub(crate) registry: Option<Arc<ToolRegistry>>,
    pub(crate) hq_config: Option<Arc<hq_core::config::HqConfig>>,
    /// The configured web token for /ws and /api (see `auth::web_auth_middleware`). Set by the
    /// caller from the same config its bind check used. Device tokens live in the database;
    /// read sign-in state through `web_auth_enabled` and `web_credential`, never this field.
    pub web_auth_token: Option<String>,
    /// Browser origins allowed besides loopback and same-origin pages, from
    /// `web_allowed_origins` (see `origin::origin_guard`).
    pub(crate) allowed_origins: Vec<String>,
    /// Whether the server listens on loopback only, which is the one place the
    /// `/mcp` development switch may apply.
    pub web_bind_is_loopback: bool,
    /// Single-use tickets for opening `/ws` when a web token is set.
    pub(crate) ws_tickets: Arc<auth::WsTickets>,
    /// The claude.ai connector's origins, when `mcp_connector` is on and valid.
    pub(crate) connector: Option<hq_core::config::ConnectorSettings>,
    /// The connector's configuration as it was when the connector was configured, kept for the
    /// TLS door (`tls::spawn_door`), which needs its `tls` section.
    pub(crate) connector_cfg: Option<hq_core::config::McpConnectorConfig>,
    /// Abort handles for in-flight chat turns, keyed by thread id. A "stop"
    /// WS message aborts the turn's driver task, which drops the harness
    /// stream and finalizes the turn.
    pub(crate) active_chat_turns: ws::ChatTurnMap,
    /// Requests sent to a phone and not yet settled (device tools).
    pub(crate) device_actions: Arc<hq_tools::device_actions::DeviceActionStore>,
    pub(crate) device_action_rate: device_actions_api::RateWindow,
    /// Web Push, when `web_push` is on and its key could be loaded.
    pub(crate) web_push: Option<Arc<web_push::WebPush>>,
    /// The one-time code a signed-in device hands to a new one.
    pub(crate) device_codes: device_auth::DeviceCodes,
    /// Bumped on every sign-out, which closes the chat sockets opened before it.
    pub(crate) auth_epoch: tokio::sync::watch::Sender<u64>,
    /// Held by a code exchange and by sign-out everywhere, so a code redeemed during a rotate
    /// cannot mint a token after the rotate deleted them all, and two rotates run one at a time.
    pub(crate) auth_lock: tokio::sync::Mutex<()>,
    /// Whether the configured token was signed out (`auth::CONFIGURED_*`), cached.
    pub(crate) configured_state: std::sync::atomic::AtomicU8,
    /// When `configured_state` was last read from the database, unix seconds.
    pub(crate) configured_checked_at: std::sync::atomic::AtomicI64,
}

impl WsState {
    /// Runs `f` with a pooled database connection on a blocking thread, so a handler never holds
    /// an async worker while SQLite or the pool checkout waits. The error is the closure's own
    /// (`ApiError::from` classifies it); a panic or cancelled task becomes an internal error.
    pub(crate) async fn db_blocking<F, T>(&self, f: F) -> anyhow::Result<T>
    where
        F: FnOnce(&rusqlite::Connection) -> anyhow::Result<T> + Send + 'static,
        T: Send + 'static,
    {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || db.with_conn(f))
            .await
            .map_err(|e| anyhow::Error::from(crate::error::ApiError::internal(e)))?
    }

    pub fn new(vault_path: PathBuf, static_dir: Option<PathBuf>) -> Self {
        let (tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        let db = hq_db::Database::open(&vault_path.join("_data").join("vault.db"))
            .expect("failed to open vault database");
        let hq_config = hq_core::config::HqConfig::load().ok().map(Arc::new);
        let allowed_origins = hq_config
            .as_ref()
            .map(|c| c.web_allowed_origins.clone())
            .unwrap_or_default();
        let web_bind_is_loopback = hq_config
            .as_ref()
            .is_none_or(|c| auth::bind_is_loopback(&c.web_bind));
        Self {
            tx,
            vault_path,
            db,
            static_dir,
            registry: None,
            hq_config,
            web_auth_token: None,
            allowed_origins,
            web_bind_is_loopback,
            ws_tickets: Arc::new(auth::WsTickets::default()),
            connector: None,
            connector_cfg: None,
            active_chat_turns: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
            device_actions: Arc::default(),
            device_action_rate: Default::default(),
            web_push: None,
            device_codes: device_auth::DeviceCodes::default(),
            auth_epoch: tokio::sync::watch::channel(0).0,
            auth_lock: tokio::sync::Mutex::new(()),
            configured_state: std::sync::atomic::AtomicU8::new(auth::CONFIGURED_UNKNOWN),
            configured_checked_at: std::sync::atomic::AtomicI64::new(0),
        }
    }

    /// Puts an event the owner should see in their inbox (web, web push and the app).
    pub(crate) fn notify_owner(&self, source: &str, title: &str, body: &str) {
        use hq_core::types::{ValueItem, ValueKind};
        let item = ValueItem::new(source, ValueKind::Fyi, title, body);
        if let Err(e) = hq_db::value_items::insert(&self.db, &item) {
            tracing::warn!("web: a notice was not recorded: {title}: {e:#}");
        }
    }

    /// Tells the owner a sign-in changed who controls HQ or what it pays with (a chat app's bot
    /// or owner, a model provider's key), naming the device that did it. A stolen device token
    /// can make these changes; the notice is how the owner hears of them.
    pub(crate) fn notify_owner_of_change(
        &self,
        credential: Option<&auth::WebCredential>,
        title: &str,
        what: &str,
    ) {
        let who = match credential {
            Some(auth::WebCredential::Device(id)) => {
                hq_db::web_auth::list(&self.db, chrono::Utc::now().timestamp())
                    .ok()
                    .and_then(|all| all.into_iter().find(|d| &d.id == id))
                    .map(|d| format!("the device \"{}\"", d.label))
                    .unwrap_or_else(|| "a signed-in device".to_string())
            }
            Some(auth::WebCredential::Configured) => "the original login link".to_string(),
            None => "this server".to_string(),
        };
        let body = format!(
            "{what} This was done from {who} at {} UTC. If it was not you, sign that device out \
             in Settings, Devices (or sign out everywhere) and undo the change.",
            chrono::Utc::now().format("%Y-%m-%d %H:%M")
        );
        self.notify_owner(SETTINGS_CHANGE_SOURCE, title, &body);
    }

    /// The edition this server runs as. Lite is decided by the config the server was started
    /// with; a server with no readable config is the full profile.
    pub fn profile(&self) -> hq_core::config::Profile {
        self.hq_config
            .as_ref()
            .map(|c| c.profile)
            .unwrap_or_default()
    }

    /// Whether a client-supplied vault path may be read or written. Everything outside the
    /// vault is refused elsewhere; under Lite, HQ's own and hidden folders are refused too.
    pub(crate) fn path_allowed(&self, rel: &str) -> bool {
        !self.profile().is_lite() || !vault_api::lite_hides(rel)
    }

    /// Like [`Self::path_allowed`], for a path already resolved inside the vault: under Lite it
    /// follows symlinks, so a link in a normal folder cannot lead into a hidden one.
    pub(crate) fn resolved_allowed(&self, abs: &std::path::Path) -> bool {
        !self.profile().is_lite() || !vault_api::lite_hides_resolved(&self.vault_path, abs)
    }

    /// Run as `profile`, whatever the config file loaded at construction says.
    pub fn with_profile(mut self, profile: hq_core::config::Profile) -> Self {
        let mut cfg = self.hq_config.as_deref().cloned().unwrap_or_default();
        cfg.profile = profile;
        self.hq_config = Some(Arc::new(cfg));
        self
    }

    pub fn with_registry(mut self, registry: Arc<ToolRegistry>) -> Self {
        self.registry = Some(registry);
        self
    }

    /// Turns the claude.ai connector on when `cfg` asks for it and is valid. Call after
    /// `web_auth_token` is set: the consent page signs the owner in with it.
    pub fn configure_connector(
        &mut self,
        cfg: &hq_core::config::McpConnectorConfig,
    ) -> Result<(), String> {
        self.connector = None;
        self.connector_cfg = None;
        if cfg.enabled && self.profile().is_lite() {
            return Err("mcp_connector is not available under profile: lite".into());
        }
        self.connector = cfg.settings(self.web_auth_token.as_deref())?;
        self.connector_cfg = self.connector.is_some().then(|| cfg.clone());
        Ok(())
    }

    /// Turns Web Push on when `cfg` asks for it, creating the VAPID key on first use.
    pub fn configure_web_push(
        &mut self,
        cfg: &hq_core::config::WebPushConfig,
    ) -> Result<(), String> {
        self.web_push = None;
        if !cfg.enabled {
            return Ok(());
        }
        let push = web_push::WebPush::new(&self.vault_path, cfg, &self.allowed_origins)
            .map_err(|e| format!("{e:#}"))?;
        self.web_push = Some(Arc::new(push));
        Ok(())
    }

    /// Tells open pages that `place` changed (`inbox:change`, `notes:change` and the like), so they
    /// refetch it instead of polling (web wave 2, rule 5).
    pub(crate) fn announce_change(&self, place: &str) {
        self.broadcast(&serde_json::json!({ "type": format!("{place}:change") }).to_string());
    }

    pub(crate) fn broadcast(&self, msg: &str) {
        let _ = self.tx.send(msg.to_string());
    }
}

/// Makes `hq_ask` usable on this server: fails the asks a previous process left
/// pending, then installs the runner that starts replies in this server's chat.
/// Call once, after the state is shared.
pub fn install_ask_runner(state: &Arc<WsState>) {
    let failed = ws::reconcile_asks_after_restart(&state.db);
    if failed > 0 {
        tracing::info!(
            failed,
            "failed hq_ask questions whose reply died with the last process"
        );
    }
    hq_tools::github::dispatch::spawn_kick_listener(Arc::new(state.db.clone()));
    if failed > 0 {
        hq_tools::github::dispatch::kick();
    }
    if !hq_tools::ask::install_runner(Arc::new(ws::WebAskRunner::new(state))) {
        tracing::warn!("an hq_ask runner was already installed; keeping it");
    }
}

/// Events buffered per client before a slow one starts missing them; every streamed token is one.
const BROADCAST_CAPACITY: usize = 4096;

/// The places a page listens to on the socket instead of polling (`<place>:change`).
pub(crate) const CHANGE_INBOX: &str = "inbox";
pub(crate) const CHANGE_NOTES: &str = "notes";
pub(crate) const CHANGE_HELPERS: &str = "helpers";
pub(crate) const CHANGE_BUDGET: &str = "budget";
pub(crate) const CHANGE_SETTINGS: &str = "settings";

/// Inbox source of the notices [`WsState::notify_owner_of_change`] puts there.
pub(crate) const SETTINGS_CHANGE_SOURCE: &str = "settings-change";

#[cfg(test)]
mod web_auth_router_tests;

#[cfg(test)]
mod security_header_tests;
