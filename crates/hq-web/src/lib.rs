//! Embedded web UI server with API endpoints and WebSocket.

mod api;
pub mod hello;
mod status_ai;
pub use api::HEALTH_SERVICE;
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

use axum::{
    Router,
    routing::{get, post},
};
use hq_tools::registry::ToolRegistry;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::broadcast;
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};

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

/// Create the web router — vault-focused endpoints only.
/// The HQ Suite's routes, or none in a build without the `suite` feature.
fn suite_routes() -> Router<Arc<WsState>> {
    #[cfg(feature = "suite")]
    return suite_api::routes();
    #[cfg(not(feature = "suite"))]
    return Router::new();
}

pub fn create_router(state: Arc<WsState>) -> Router {
    state.load_configured_state();
    let static_dir = state
        .static_dir
        .as_deref()
        .unwrap_or(std::path::Path::new("web/dist"))
        .to_path_buf();

    notifications_watch::spawn_notifications_watcher(state.clone());
    web_push::spawn_dispatcher(state.clone());
    tasks_watch::spawn_tasks_watcher(state.clone());
    // Both act on coding-agent sessions and sub-agent runs, which Lite does not have.
    if !state.profile().is_lite() {
        session_driver::spawn_session_driver(state.clone());
        subagent_followup::spawn_subagent_followup(state.clone());
    }

    let router = device_actions_api::routes(oauth::routes(Router::new(), &state))
        .route("/ws", get(ws::ws_handler))
        .route("/health", get(api::health_handler))
        .route(hello::HELLO_PATH, get(hello::hello_handler))
        .route("/api/status/ai", get(status_ai::status_ai_handler))
        .route("/api/status/busy", get(status_ai::status_busy_handler))
        .route("/api/backup/status", get(status_ai::backup_status_handler))
        .route("/api/vault-status", get(api::vault_status_handler))
        .route("/api/ws-ticket", post(auth::ws_ticket_handler))
        .route(
            device_auth::DEVICE_CODE_PATH,
            post(device_auth::device_code_handler),
        )
        .route(
            device_auth::EXCHANGE_PATH,
            post(device_auth::exchange_handler),
        )
        .route(
            device_auth::devices::DEVICES_PATH,
            get(device_auth::devices::list_handler),
        )
        .route(
            device_auth::devices::CURRENT_DEVICE_PATH,
            axum::routing::delete(device_auth::devices::delete_current_handler),
        )
        .route(
            device_auth::devices::DEVICE_PATH,
            axum::routing::delete(device_auth::devices::delete_handler),
        )
        .route(
            device_auth::devices::KEEP_PATH,
            post(device_auth::devices::keep_handler),
        )
        .route(
            device_auth::devices::ROTATE_PATH,
            post(device_auth::devices::rotate_handler),
        )
        .route(
            "/api/admin/broadcast",
            axum::routing::post(api::admin_broadcast_handler),
        )
        .route("/api/search", get(api::search_handler))
        .route(
            "/api/note",
            get(api::note_read_handler)
                .put(vault_api::note_update_handler)
                .delete(note_trash::note_delete_handler),
        )
        .route("/api/note/trash", get(note_trash::trash_list_handler))
        .route("/api/note/trash/read", get(note_trash::trash_read_handler))
        .route(
            "/api/note/trash/restore",
            post(note_trash::trash_restore_handler),
        )
        .route(
            "/api/note/trash/purge",
            post(note_trash::trash_purge_handler),
        )
        .route(
            "/api/note/trash/empty",
            post(note_trash::trash_empty_handler),
        )
        .route("/api/note/import", note_import::route())
        .route("/api/note/version", get(note_undo::note_version_handler))
        .route(
            "/api/note/restore-version",
            post(note_undo::restore_version_handler),
        )
        .route("/api/note/create", post(vault_api::note_create_handler))
        .route("/api/note/move", post(vault_api::note_move_handler))
        .route("/api/folder/move", post(vault_api::folder_move_handler))
        .route(
            "/api/prefs",
            get(prefs_api::prefs_get_handler).put(prefs_api::prefs_put_handler),
        )
        .route("/api/note/versions", get(vault_api::note_versions_handler))
        .route("/api/note/link", post(note_ids::note_link_handler))
        .route("/api/note/by-id/{id}", get(note_ids::note_by_id_handler))
        .route("/api/note/review", post(vault_api::note_review_handler))
        .route("/api/note/seen", post(vault_api::note_seen_handler))
        .route("/api/note/pdf", get(vault_api::note_pdf_handler))
        .route("/api/note/export", get(vault_api::note_export_handler))
        .route("/api/vault/folders", get(vault_api::folders_handler))
        .route("/api/vault-signals", get(vault_api::signals_handler))
        .route("/api/tree", get(api::tree_handler))
        .route("/api/settings", get(settings_api::settings_handler))
        .route(
            "/api/settings/provider-key",
            post(provider_keys::replace_handler),
        )
        .route(
            "/api/capabilities",
            get(capabilities_api::capabilities_handler),
        )
        .merge(suite_routes())
        .route("/api/adoption", get(adoption_api::adoption_handler))
        .route("/api/offers/replay", get(adoption_api::replay_handler))
        .route("/api/offers/answer", post(offers_api::answer_handler))
        .route(
            "/api/offers/later/open",
            post(offers_api::later_open_handler),
        )
        .route("/api/offers/never", post(offers_api::never_handler))
        .route("/api/offers/fewer", post(offers_api::fewer_handler))
        .route("/api/offers/summary", get(offers_api::summary_handler))
        .route("/api/offers/clear", post(offers_api::clear_handler))
        .route("/api/setup/status", get(setup_api::status_handler))
        .route("/api/setup/test", post(setup_api::test_handler))
        .route("/api/setup/provider", post(setup_api::provider_handler))
        .route("/api/chat-apps", get(chat_apps::status_handler))
        .route(
            "/api/chat-apps/{platform}",
            post(chat_apps::save_handler).delete(chat_apps::delete_handler),
        )
        .route(
            "/api/chat-apps/{platform}/verify",
            post(chat_apps::verify_handler),
        )
        .route(
            "/api/chat-apps/{platform}/pair",
            post(chat_apps::pair_handler),
        )
        .route("/api/system/restart", post(restart_api::restart_handler))
        .route(
            "/api/copilot-usage",
            get(copilot_usage_api::copilot_usage_handler),
        )
        .route(
            "/api/lane-quotas",
            get(lane_quotas_api::lane_quotas_handler),
        )
        .route(
            "/api/openrouter-usage",
            get(openrouter_usage_api::openrouter_usage_handler),
        )
        .route(
            "/api/budgets",
            get(budgets_api::budgets_handler).put(budgets_api::put_budgets_handler),
        )
        .route(
            "/api/usage/forecast",
            get(usage_forecast_api::usage_forecast_handler),
        )
        .route(
            "/api/usage/providers",
            get(usage_providers_api::usage_providers_handler),
        )
        .route("/api/pinned", get(api::pinned_handler))
        .route("/api/pin", axum::routing::post(api::pin_toggle_handler))
        .route("/api/vault-asset", get(api::vault_asset_handler))
        .route("/api/chat/uploads", chat_uploads::route())
        .route(
            "/api/threads",
            get(threads_api::list_threads_handler).post(threads_api::create_thread_handler),
        )
        .route(
            "/api/threads/{thread_id}/messages",
            get(threads_api::get_thread_messages_handler),
        )
        .route(
            "/api/threads/{thread_id}/rename",
            axum::routing::post(threads_api::rename_thread_handler),
        )
        .route(
            "/api/threads/{thread_id}/archive",
            axum::routing::post(threads_api::archive_thread_handler),
        )
        .route(
            "/api/threads/{thread_id}/unarchive",
            axum::routing::post(threads_api::unarchive_thread_handler),
        )
        .route(
            "/api/threads/{thread_id}/stop",
            axum::routing::post(threads_api::stop_thread_handler),
        )
        .route(
            "/api/coder/{action}",
            axum::routing::post(coder_api::control_handler),
        )
        .route(
            "/api/chat-folders/{thread}",
            get(chat_folders_api::list_handler),
        )
        .route(
            "/api/chat-folders/{thread}/{id}",
            axum::routing::delete(chat_folders_api::revoke_handler),
        )
        .route("/api/helper-setup", get(helper_setup_api::list_handler))
        .route(
            "/api/helper-setup/{host}/{action}",
            axum::routing::post(helper_setup_api::action_handler),
        )
        .route(
            "/api/fetch-sites",
            get(fetch_sites_api::list_handler).post(fetch_sites_api::add_domain_handler),
        )
        .route(
            "/api/fetch-sites/{host}",
            axum::routing::delete(fetch_sites_api::forget_handler),
        )
        .route(
            "/api/notifications",
            get(notifications_api::list_notifications_handler),
        )
        .route(
            "/api/notifications/{id}/action",
            axum::routing::post(notifications_api::notification_action_handler),
        )
        .route(
            "/api/notifications/{id}/restore",
            axum::routing::post(notifications_api::restore_notification_handler),
        )
        .route(
            "/api/notifications/{id}/answer",
            post(dialog_answer::answer_handler),
        )
        .route(
            "/api/notifications/mark-all-read",
            axum::routing::post(notifications_api::mark_all_read_handler),
        )
        .route(
            "/api/notifications/archive-all",
            post(notifications_api::archive_all_handler),
        )
        .route(
            "/api/notifications/read-all",
            post(notifications_api::read_all_handler),
        )
        .route(
            "/api/notifications/clear-done",
            post(notifications_api::clear_done_handler),
        )
        .route(
            "/api/notifications/restore",
            post(notifications_api::restore_many_handler),
        )
        .route(
            "/api/notifications/restore-cleared",
            post(notifications_api::restore_cleared_handler),
        )
        .route(
            "/api/notifications/{id}/read",
            post(notifications_api::read_handler),
        )
        .route(
            "/api/notifications/silences",
            get(notifications_api::silences_handler).post(notifications_api::silence_handler),
        )
        .route(
            "/api/notifications/silences/{topic}",
            axum::routing::delete(notifications_api::unsilence_handler),
        )
        .route(
            "/api/notifications/push",
            get(web_push::api::status_handler),
        )
        .route(
            "/api/notifications/push/subscribe",
            post(web_push::api::subscribe_handler),
        )
        .route(
            "/api/notifications/push/unsubscribe",
            post(web_push::api::unsubscribe_handler),
        )
        .route(
            "/api/notifications/push/test",
            post(web_push::api::test_handler),
        )
        .route(
            "/api/spaces",
            get(tasks_api::list_spaces_handler).post(tasks_api::create_space_handler),
        )
        .route(
            "/api/folders",
            get(tasks_api::list_folders_handler).post(tasks_api::create_folder_handler),
        )
        .route(
            "/api/initiatives",
            get(tasks_api::list_initiatives_handler).post(tasks_api::create_initiative_handler),
        )
        .route(
            "/api/tasks",
            get(tasks_api::list_tasks_handler).post(tasks_api::create_task_handler),
        )
        .route(
            "/api/tasks/{id}",
            get(tasks_api::get_task_handler)
                .patch(tasks_api::update_task_handler)
                .delete(tasks_api::delete_task_handler),
        )
        .route(
            "/api/tasks/{id}/related",
            get(tasks_api::related_tasks_handler),
        )
        .route(
            "/api/tasks/shelve",
            axum::routing::post(tasks_api::shelve_tasks_handler),
        )
        .route(
            "/api/tasks/unshelve",
            axum::routing::post(tasks_api::unshelve_tasks_handler),
        )
        .route(
            "/api/tasks/shelve-completed",
            axum::routing::post(tasks_api::shelve_completed_handler),
        )
        .route(
            "/api/initiatives/{id}/shelf",
            axum::routing::post(tasks_api::shelve_initiative_handler),
        )
        .route(
            "/api/tasks/{id}/comments",
            get(tasks_api::list_comments_handler).post(tasks_api::add_comment_handler),
        )
        .route(
            "/api/tasks/{id}/events",
            get(tasks_api::list_task_events_handler),
        )
        .route(
            "/api/tasks/{id}/work-sessions",
            get(tasks_api::list_work_sessions_handler),
        )
        .route("/api/tasks/{id}/time", get(tasks_api::task_time_handler))
        .route(
            "/api/tasks/{id}/links",
            get(tasks_api::list_task_links_handler),
        )
        .route(
            "/api/tasks/{id}/restore",
            axum::routing::post(tasks_api::restore_task_handler),
        )
        .route("/api/tasks/linked", get(tasks_api::tasks_linked_to_handler))
        // Folders the owner approved for connector and handoff-key sessions. Owner only.
        .route(
            "/api/session-folders",
            get(session_folders_api::list_handler),
        )
        .route(
            "/api/session-folders/{id}",
            axum::routing::delete(session_folders_api::revoke_handler),
        )
        // Task verification: the owner's side. The web credential is the owner's; /mcp keys never
        // reach these routes.
        .route(
            "/api/verification/status",
            get(verification_api::status_handler),
        )
        .route(
            "/api/verification/queue",
            get(verification_api::queue_handler),
        )
        .route(
            "/api/verification/briefing",
            get(verification_api::briefing_handler),
        )
        .route(
            "/api/verification/sweeps",
            get(verification_api::list_sweeps_handler).post(verification_api::create_sweep_handler),
        )
        .route(
            "/api/verification/sweeps/{id}",
            get(verification_api::get_sweep_handler),
        )
        .route(
            "/api/verification/sweeps/{id}/run",
            post(verification_api::run_sweep_handler),
        )
        .route(
            "/api/verification/sweeps/{id}/pause",
            post(verification_api::pause_sweep_handler),
        )
        .route(
            "/api/verification/sweeps/{id}/cancel",
            post(verification_api::cancel_sweep_handler),
        )
        .route(
            "/api/verification/sweeps/{id}/apply",
            post(verification_api::apply_sweep_handler),
        )
        .route(
            "/api/verification/sweeps/{id}/undo",
            post(verification_api::undo_sweep_handler),
        )
        .route(
            "/api/verification/audits",
            get(verification_api::audits_handler),
        )
        .route(
            "/api/verification/audits/{id}",
            post(verification_api::answer_audit_handler),
        )
        .route(
            "/api/verification/holds",
            get(verification_api::holds_handler),
        )
        .route(
            "/api/verification/holds/{id}/veto",
            post(verification_api::veto_handler),
        )
        .route(
            "/api/tasks/{id}/verification",
            get(verification_api::task_verification_handler),
        )
        .route(
            "/api/tasks/{id}/reverify",
            post(verification_api::reverify_handler),
        )
        .route("/api/tasks/stale", get(tasks_api::stale_tasks_handler))
        .route(
            "/api/tasks/{id}/checkpoint",
            get(tasks_api::task_checkpoint_handler),
        )
        .route(
            "/api/tasks/{id}/activity",
            get(tasks_api::task_activity_handler),
        )
        .route(
            "/api/tasks/{id}/decisions",
            get(tasks_api::task_decisions_handler),
        )
        .route(
            "/api/initiatives/{id}/progress",
            get(tasks_api::initiative_progress_handler),
        )
        .route(
            "/api/initiatives/{id}/activity",
            get(tasks_api::initiative_activity_handler),
        )
        .route("/api/task-time-report", get(tasks_api::time_report_handler))
        .route(
            "/api/work-sessions",
            get(tasks_api::recent_work_sessions_handler),
        )
        .route(
            "/api/edge-worker",
            get(edge_worker_api::edge_worker_handler),
        )
        .route(
            "/api/threads/{thread_id}/sessions",
            get(sessions_api::list_thread_sessions_handler),
        )
        .route(
            "/api/harness-sessions",
            get(sessions_api::list_all_handler).post(workbench_api::spawn_handler),
        )
        .route("/api/workbench/hosts", get(workbench_api::hosts_handler))
        .route(
            "/api/workbench/lockdown",
            post(workbench_api::lockdown_handler),
        )
        .route(
            "/api/workbench/hosts/{host}/dirs",
            get(workbench_api::list_dirs_handler).post(workbench_api::make_dir_handler),
        )
        .route(
            "/api/harness-sessions/{id}/stop",
            post(workbench_api::stop_handler),
        )
        .route(
            "/api/harness-sessions/{id}/resume",
            post(workbench_api::resume_handler),
        )
        .route(
            "/api/harness-sessions/{id}/rename",
            post(workbench_api::rename_handler),
        )
        .route(
            "/api/harness-sessions/{id}/archive",
            post(workbench_api::archive_handler),
        )
        .route(
            "/api/harness-sessions/{id}",
            get(sessions_api::get_session_handler),
        )
        .route(
            "/api/harness-sessions/{id}/screen",
            get(sessions_api::screen_handler),
        )
        .route(
            "/api/harness-sessions/{id}/screen/stream",
            get(sessions_api::screen_stream_handler),
        )
        .route(
            "/api/harness-sessions/{id}/send",
            post(sessions_api::send_handler),
        )
        .route(
            "/api/harness-sessions/{id}/answers",
            get(sessions_api::answers_handler),
        )
        .route(
            "/api/harness-sessions/{id}/pending",
            get(sessions_api::pending_handler),
        )
        .route(
            "/api/harness-sessions/{id}/answer",
            post(sessions_api::answer_handler),
        )
        .route(
            "/api/harness-sessions/{id}/adopt",
            post(sessions_api::adopt_handler),
        )
        .route(
            "/api/harness-sessions/{id}/drive",
            post(sessions_api::set_drive_handler),
        )
        .route(
            "/api/harness-sessions/{id}/goal",
            post(sessions_api::set_goal_handler),
        )
        .route(
            "/api/harness-sessions/{id}/unwatch",
            post(sessions_api::unwatch_handler),
        )
        .route(
            "/hooks/gmail",
            axum::routing::post(gmail_ingest::gmail_hook_handler),
        )
        // Unknown server paths 404 as JSON instead of falling through to the PWA shell.
        .route("/api/{*rest}", axum::routing::any(not_found_json))
        .route("/ws/{*rest}", axum::routing::any(not_found_json))
        .route("/hooks/{*rest}", axum::routing::any(not_found_json))
        .route("/mcp/{*rest}", axum::routing::any(not_found_json))
        // OAuth clients probe here after a 401; a disabled connector must not answer with the shell.
        .route("/.well-known/{*rest}", axum::routing::any(not_found_json))
        .route("/mcp", axum::routing::post(mcp_http::mcp_handler));
    let router = match github_connect_api::flow_for(&state) {
        Some(flow) => router
            .merge(github_connect_api::browser_routes(flow.clone()))
            .merge(github_connect_api::owner_routes(flow).route_layer(
                axum::middleware::from_fn_with_state(
                    state.clone(),
                    github_connect_api::require_sign_in,
                ),
            )),
        None => router,
    };

    let app = router
        // Hashed build chunks: a missing one must 404, not come back as the HTML shell.
        .nest_service("/assets", ServeDir::new(static_dir.join("assets")))
        // The PWA: static files, and index.html for every client-side route.
        // Added before the layers so they wrap it too.
        .fallback_service(
            ServeDir::new(&static_dir).fallback(ServeFile::new(static_dir.join("index.html"))),
        )
        .layer(axum::middleware::from_fn(no_cache_shell))
        // Slow mobile links: gzip/brotli the PWA's JS and CSS and every JSON response.
        .layer(tower_http::compression::CompressionLayer::new())
        // Inside auth: a request that is not allowed in at all is refused first, and one that is
        // reaches a route Lite serves only if the profile lists it.
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            profile_gate::lite_gate,
        ))
        // Inside CORS, so preflight requests are answered before auth runs.
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::web_auth_middleware,
        ))
        // Before auth, so a hostile page learns nothing from a 401.
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            origin::origin_guard,
        ))
        .layer(cors_layer(state.allowed_origins.clone()))
        // Outermost, so CORS preflights and guard rejections carry the headers too.
        .layer(axum::middleware::from_fn_with_state(
            security_headers::content_security_policy(&static_dir),
            security_headers::set_security_headers,
        ))
        .with_state(state.clone());
    // The connector's TLS door serves this same router (a second one would start the watchers
    // above twice). It is a no-op unless `mcp_connector.tls` is on.
    tls::spawn_door(&state, app.clone());
    devices_door::spawn(&state, app.clone());
    app
}

/// CORS grants read access only to origins the origin guard would let through.
fn cors_layer(allowed: Vec<String>) -> CorsLayer {
    let predicate = move |origin: &axum::http::HeaderValue, parts: &axum::http::request::Parts| {
        let host = parts
            .headers
            .get(axum::http::header::HOST)
            .and_then(|v| v.to_str().ok());
        origin
            .to_str()
            .is_ok_and(|o| origin::origin_allowed(o, host, &allowed))
    };
    CorsLayer::new()
        .allow_origin(tower_http::cors::AllowOrigin::predicate(predicate))
        .allow_methods([
            axum::http::Method::GET,
            axum::http::Method::POST,
            axum::http::Method::PATCH,
            axum::http::Method::PUT,
            axum::http::Method::DELETE,
            axum::http::Method::OPTIONS,
        ])
        .allow_headers([
            axum::http::header::CONTENT_TYPE,
            axum::http::header::AUTHORIZATION,
            axum::http::header::HeaderName::from_static("x-api-key"),
        ])
}

async fn not_found_json() -> axum::response::Response {
    axum::response::IntoResponse::into_response(error::ApiError::not_found())
}

/// The PWA shell and its service worker must revalidate on every load, or an
/// installed app keeps running an old build after a deploy.
async fn no_cache_shell(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let is_sw = req.uri().path() == "/sw.js";
    let mut res = next.run(req).await;
    let is_html = res
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("text/html"));
    if is_sw || is_html {
        res.headers_mut().insert(
            axum::http::header::CACHE_CONTROL,
            axum::http::HeaderValue::from_static("no-cache"),
        );
    }
    res
}

#[cfg(test)]
mod web_auth_router_tests;

#[cfg(test)]
mod security_header_tests;
