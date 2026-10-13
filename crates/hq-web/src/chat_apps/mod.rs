//! Chat apps in Settings: connect a Telegram or Discord bot and pair its owner from the web.
//!
//! `GET /api/chat-apps` reports each relay (never a token, only whether one is set).
//! `POST /api/chat-apps/{platform}/verify` checks a token with the platform.
//! `POST /api/chat-apps/{platform}` saves a checked token and turns the relay on;
//! `DELETE` turns it off and clears the token. `POST .../pair` issues an owner pairing code.
//! Relays read their config once at start, so a save is applied with `POST /api/system/restart`.
//!
//! Every write needs a configured web token, as first-run setup does: behind `tailscale serve`
//! every request arrives from loopback, and a pairing code makes its holder the agent's owner.

mod records;
#[cfg(test)]
mod tests;
mod verify;

use std::path::Path;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Path as UrlPath, State};
use axum::response::IntoResponse;
use hq_core::config::{HqConfig, RelayConfig};
use hq_core::pairing::{self, PairPlatform};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::WsState;
use crate::error::ApiError;
use verify::Verified;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Platform {
    Telegram,
    Discord,
}

impl Platform {
    /// The `{platform}` path segment; anything else is a JSON 404 like an unknown route.
    fn from_path(raw: &str) -> Result<Self, ApiError> {
        PLATFORMS
            .into_iter()
            .find(|p| p.as_str() == raw)
            .ok_or_else(ApiError::not_found)
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Platform::Telegram => "telegram",
            Platform::Discord => "discord",
        }
    }

    fn pair_platform(self) -> PairPlatform {
        match self {
            Platform::Telegram => PairPlatform::Telegram,
            Platform::Discord => PairPlatform::Discord,
        }
    }

    /// The `HQ_` variables that override this relay's token and switch.
    fn env_names(self) -> [&'static str; 2] {
        match self {
            Platform::Telegram => ["HQ_RELAY__TELEGRAM_TOKEN", "HQ_RELAY__TELEGRAM_ENABLED"],
            Platform::Discord => ["HQ_RELAY__DISCORD_TOKEN", "HQ_RELAY__DISCORD_ENABLED"],
        }
    }

    fn token(self, relay: &RelayConfig) -> Option<&str> {
        let token = match self {
            Platform::Telegram => relay.telegram_token.as_deref(),
            Platform::Discord => relay.discord_token.as_deref(),
        };
        token.filter(|t| !t.trim().is_empty())
    }

    fn enabled(self, relay: &RelayConfig) -> bool {
        match self {
            Platform::Telegram => relay.telegram_enabled,
            Platform::Discord => relay.discord_enabled,
        }
    }

    fn set(self, relay: &mut RelayConfig, token: Option<String>, enabled: bool) {
        match self {
            Platform::Telegram => (relay.telegram_token, relay.telegram_enabled) = (token, enabled),
            Platform::Discord => (relay.discord_token, relay.discord_enabled) = (token, enabled),
        }
    }

    fn pair_command(self, code: &str) -> String {
        match self {
            Platform::Telegram => format!("/pair {code}"),
            Platform::Discord => format!("!pair {code}"),
        }
    }
}

const PLATFORMS: [Platform; 2] = [Platform::Telegram, Platform::Discord];

const NO_WEB_TOKEN: &str = "Managing chat apps from the web needs a web token (web_auth_token). Edit the relay section of the config on the server instead.";
const TELEGRAM_OWNER_IN_CONFIG: &str = "The Telegram owner is set in the config file (relay.telegram_authorized_chat_id or a telegram_users owner), so it cannot be paired from here.";
const DISCORD_ALLOWLIST_SET: &str = "Discord access is set by relay.discord_allowed_user_ids in the config file, so pairing is off.";
const ALREADY_PAIRED: &str =
    "An owner is already paired. Choose Pair a different account to replace them.";
const NOT_CONNECTED: &str = "Connect the bot first, then pair.";

/// Refuses writes on an instance without a web token.
pub(crate) fn require_web_token(state: &WsState) -> Result<(), ApiError> {
    if crate::setup_api::token_configured(state) {
        return Ok(());
    }
    Err(ApiError::Forbidden(NO_WEB_TOKEN.into()))
}

/// Whether a process variable named `name` exists, ignoring case as figment does.
fn real_env_set(name: &str) -> bool {
    std::env::vars_os().any(|(k, _)| k.to_str().is_some_and(|k| k.eq_ignore_ascii_case(name)))
}

/// The variable that pins this relay from the environment, which a config write cannot change.
fn env_lock(platform: Platform, env_set: &dyn Fn(&str) -> bool) -> Option<&'static str> {
    platform.env_names().into_iter().find(|n| env_set(n))
}

fn refuse_if_env_locked(
    platform: Platform,
    env_set: &dyn Fn(&str) -> bool,
) -> Result<(), ApiError> {
    match env_lock(platform, env_set) {
        Some(name) => Err(ApiError::Conflict(format!(
            "This bot is set by the server's environment ({name}). Change it there."
        ))),
        None => Ok(()),
    }
}

/// Saves a checked token and turns the relay on, in the config file at `config_path`.
fn save_token_at(config_path: &Path, platform: Platform, token: String) -> anyhow::Result<()> {
    HqConfig::save_patch_to_path(config_path, |c| {
        platform.set(&mut c.relay, Some(token), true)
    })?;
    Ok(())
}

/// Turns the relay off and clears its token, in the config file at `config_path`.
fn disconnect_at(config_path: &Path, platform: Platform) -> anyhow::Result<()> {
    HqConfig::save_patch_to_path(config_path, |c| platform.set(&mut c.relay, None, false))?;
    Ok(())
}

fn owner_json(platform: Platform, relay: &RelayConfig, vault: &Path) -> Value {
    match platform {
        Platform::Telegram => {
            let owner = hq_core::telegram_access::owner_chat_id(vault, relay);
            json!({
                "paired": owner.is_some(),
                "id": owner.map(|id| id.to_string()),
                "from_config": hq_core::telegram_access::owner_set_in_config(relay),
            })
        }
        Platform::Discord => {
            let owner = hq_core::discord_access::paired_owner(vault);
            let allowlist = !relay.discord_allowed_user_ids.is_empty();
            json!({
                "paired": owner.is_some() || allowlist,
                "id": owner.map(|id| id.to_string()),
                "from_config": allowlist,
            })
        }
    }
}

/// Everything the running process knows for one relay, as of `loaded` (the config on disk
/// plus environment). `running` is the config this process started with.
fn platform_status(
    platform: Platform,
    loaded: &RelayConfig,
    running: Option<&RelayConfig>,
    vault: &Path,
    env_set: &dyn Fn(&str) -> bool,
) -> Value {
    let [token_env, enabled_env] = platform.env_names();
    let token = platform.token(loaded);
    let needs_restart = running.is_some_and(|r| {
        platform.token(r) != token || platform.enabled(r) != platform.enabled(loaded)
    });
    let bot = token.and_then(|t| records::load_identity(vault, platform, t));
    let started_at = hq_core::daemon_restart::process_started_at();
    json!({
        "token_set": token.is_some(),
        "token_from_env": env_set(token_env),
        "enabled": platform.enabled(loaded),
        "enabled_from_env": env_set(enabled_env),
        "needs_restart": needs_restart,
        "owner": owner_json(platform, loaded, vault),
        "pending_code": pairing::has_pending_code(vault, platform.pair_platform(), pairing::now_secs()),
        "bot": bot.map(|b| json!({
            "username": b.username,
            "application_id": b.application_id,
            "message_content": b.message_content,
        })),
        "relay": records::relay_status(vault, platform, started_at),
    })
}

async fn load_config() -> Result<HqConfig, ApiError> {
    tokio::task::spawn_blocking(HqConfig::load)
        .await
        .map_err(ApiError::internal)?
        .map_err(|e| ApiError::internal(format!("could not read the HQ config: {e}")))
}

fn restart_json(state: &WsState) -> (bool, Option<&'static str>) {
    let can = crate::restart_api::supervisor(state).is_some();
    (
        can,
        (!can).then_some(crate::restart_api::MANUAL_RESTART_HINT),
    )
}

pub(crate) async fn status_handler(
    State(state): State<Arc<WsState>>,
) -> Result<impl IntoResponse, ApiError> {
    let loaded = load_config().await?;
    let running = state.hq_config.as_deref().map(|c| &c.relay);
    let (can_restart, restart_hint) = restart_json(&state);
    let mut body = json!({
        "writable": crate::setup_api::token_configured(&state),
        "can_restart": can_restart,
        "restart_hint": restart_hint,
        "started_at": hq_core::daemon_restart::process_started_at(),
    });
    for platform in PLATFORMS {
        body[platform.as_str()] = platform_status(
            platform,
            &loaded.relay,
            running,
            &state.vault_path,
            &real_env_set,
        );
    }
    Ok(Json(body))
}

#[derive(Deserialize)]
pub(crate) struct VerifyBody {
    /// Absent to re-check the token already saved.
    #[serde(default)]
    token: Option<String>,
}

#[derive(Deserialize)]
pub(crate) struct TokenBody {
    token: String,
}

fn verified_json(v: &Verified) -> Value {
    json!({
        "ok": true,
        "username": v.username,
        "application_id": v.application_id,
        "message_content": v.message_content,
    })
}

/// Trims a pasted token and checks its shape, so junk never leaves the server.
fn clean_token(platform: Platform, raw: &str) -> Result<String, ApiError> {
    let token = raw.trim();
    verify::check_shape(platform, token).map_err(ApiError::bad_request)?;
    Ok(token.to_string())
}

pub(crate) async fn verify_handler(
    State(state): State<Arc<WsState>>,
    UrlPath(raw): UrlPath<String>,
    Json(body): Json<VerifyBody>,
) -> Result<impl IntoResponse, ApiError> {
    let platform = Platform::from_path(&raw)?;
    require_web_token(&state)?;
    let (raw, saved) = match body.token {
        Some(t) => (t, false),
        None => {
            let loaded = load_config().await?;
            let t = platform.token(&loaded.relay).map(str::to_string);
            (t.ok_or_else(|| ApiError::bad_request(NOT_CONNECTED))?, true)
        }
    };
    let token = clean_token(platform, &raw)?;
    Ok(Json(
        match verify::verify(platform, verify::api_base(platform), &token).await {
            Ok(v) => {
                if saved {
                    records::store_identity(&state.vault_path, platform, &token, &v);
                }
                verified_json(&v)
            }
            Err(error) => json!({ "ok": false, "error": error }),
        },
    ))
}

pub(crate) async fn save_handler(
    State(state): State<Arc<WsState>>,
    UrlPath(raw): UrlPath<String>,
    Json(body): Json<TokenBody>,
) -> Result<impl IntoResponse, ApiError> {
    let platform = Platform::from_path(&raw)?;
    require_web_token(&state)?;
    refuse_if_env_locked(platform, &real_env_set)?;
    let token = clean_token(platform, &body.token)?;
    let v = verify::verify(platform, verify::api_base(platform), &token)
        .await
        .map_err(ApiError::BadRequest)?;
    let path = HqConfig::config_file_path();
    let saved = token.clone();
    tokio::task::spawn_blocking(move || save_token_at(&path, platform, saved))
        .await
        .map_err(ApiError::internal)?
        .map_err(|e| ApiError::internal(format!("could not save the config: {e}")))?;
    records::store_identity(&state.vault_path, platform, &token, &v);
    let mut reply = verified_json(&v);
    reply["can_restart"] = json!(restart_json(&state).0);
    Ok(Json(reply))
}

pub(crate) async fn delete_handler(
    State(state): State<Arc<WsState>>,
    UrlPath(raw): UrlPath<String>,
) -> Result<impl IntoResponse, ApiError> {
    let platform = Platform::from_path(&raw)?;
    require_web_token(&state)?;
    refuse_if_env_locked(platform, &real_env_set)?;
    let path = HqConfig::config_file_path();
    tokio::task::spawn_blocking(move || disconnect_at(&path, platform))
        .await
        .map_err(ApiError::internal)?
        .map_err(|e| ApiError::internal(format!("could not save the config: {e}")))?;
    records::forget(&state.vault_path, platform);
    Ok(Json(
        json!({ "ok": true, "can_restart": restart_json(&state).0 }),
    ))
}

#[derive(Deserialize, Default)]
pub(crate) struct PairBody {
    /// Remove the current owner first: the owner is deliberately re-pairing.
    #[serde(default)]
    replace: bool,
}

fn owner_file_exists(platform: Platform, vault: &Path) -> bool {
    match platform {
        Platform::Telegram => hq_core::telegram_access::paired_owner(vault).is_some(),
        Platform::Discord => hq_core::discord_access::paired_owner(vault).is_some(),
    }
}

fn remove_owner_file(platform: Platform, vault: &Path) -> std::io::Result<()> {
    let path = match platform {
        Platform::Telegram => hq_core::telegram_access::owner_file(vault),
        Platform::Discord => hq_core::discord_access::owner_file(vault),
    };
    std::fs::remove_file(path)
}

/// Issues a pairing code, removing the current paired owner when `replace` asks to.
/// The code is created first, so a failure never leaves the bot with no owner and no code.
fn issue_pairing_code(
    platform: Platform,
    relay: &RelayConfig,
    vault: &Path,
    replace: bool,
    now: i64,
) -> Result<String, ApiError> {
    match platform {
        Platform::Telegram if hq_core::telegram_access::owner_set_in_config(relay) => {
            return Err(ApiError::Conflict(TELEGRAM_OWNER_IN_CONFIG.into()));
        }
        Platform::Discord if !relay.discord_allowed_user_ids.is_empty() => {
            return Err(ApiError::Conflict(DISCORD_ALLOWLIST_SET.into()));
        }
        _ => {}
    }
    let paired = owner_file_exists(platform, vault);
    if paired && !replace {
        return Err(ApiError::Conflict(ALREADY_PAIRED.into()));
    }
    let code = pairing::create_pairing_code(vault, platform.pair_platform(), now)
        .map_err(|e| ApiError::internal(format!("could not create a pairing code: {e:?}")))?;
    if paired {
        remove_owner_file(platform, vault)
            .map_err(|e| ApiError::internal(format!("could not remove the old owner: {e}")))?;
    }
    Ok(code)
}

/// The bot's Telegram username for the `t.me` link: remembered, or asked for once.
async fn telegram_username(vault: &Path, token: &str) -> Option<String> {
    if let Some(id) = records::load_identity(vault, Platform::Telegram, token) {
        return Some(id.username);
    }
    let v = verify::verify(Platform::Telegram, verify::TELEGRAM_API, token)
        .await
        .ok()?;
    records::store_identity(vault, Platform::Telegram, token, &v);
    Some(v.username)
}

pub(crate) async fn pair_handler(
    State(state): State<Arc<WsState>>,
    UrlPath(raw): UrlPath<String>,
    Json(body): Json<PairBody>,
) -> Result<impl IntoResponse, ApiError> {
    let platform = Platform::from_path(&raw)?;
    require_web_token(&state)?;
    let loaded = load_config().await?;
    let Some(token) = platform.token(&loaded.relay).map(str::to_string) else {
        return Err(ApiError::Conflict(NOT_CONNECTED.into()));
    };
    let now = pairing::now_secs();
    let code = issue_pairing_code(
        platform,
        &loaded.relay,
        &state.vault_path,
        body.replace,
        now,
    )?;
    let deep_link = match platform {
        Platform::Telegram => telegram_username(&state.vault_path, &token)
            .await
            .map(|bot| format!("https://t.me/{bot}?start={code}")),
        Platform::Discord => None,
    };
    Ok(Json(json!({
        "code": code,
        "command": platform.pair_command(&code),
        "expires_at": now + pairing::PAIRING_TTL_SECS,
        "deep_link": deep_link,
    })))
}
