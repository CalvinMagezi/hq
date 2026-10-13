//! `hq notify-restart` — best-effort pre/post-restart broadcast for CI-triggered deploys.
//!
//! Invoked by the deploy tooling around the binary swap + `systemctl restart hq`,
//! so a live session gets a heads-up before the interruption and a confirmation after,
//! rather than a silent restart. Sends a plain notice straight to Telegram and Discord and
//! makes a loopback call into the running daemon's `hq-web` broadcast so an open web UI
//! tab sees it too. It is not a value-bus item: a restart notice has nothing to approve,
//! and the value bus would repeat it minutes later. Every delivery path is independently
//! best-effort: nothing here may fail the deploy.

use anyhow::Result;
use hq_core::config::HqConfig;
use std::time::Duration;

const WEB_BROADCAST_TIMEOUT: Duration = Duration::from_secs(2);
/// Lifetime of the device token minted for one broadcast, so a crash cannot leave it live.
const BROADCAST_TOKEN_TTL_SECS: i64 = 60;
const BROADCAST_TOKEN_LABEL: &str = "Restart notice";

/// The bearer for the loopback broadcast. The configured token works until the owner signs it
/// out; after that, a device token that lives for one call and is deleted right after.
enum Bearer {
    Configured(String),
    Temporary {
        db: hq_db::Database,
        id: String,
        token: String,
    },
}

impl Bearer {
    fn token(&self) -> &str {
        match self {
            Bearer::Configured(t) => t,
            Bearer::Temporary { token, .. } => token,
        }
    }
}

impl Drop for Bearer {
    fn drop(&mut self) {
        if let Bearer::Temporary { db, id, .. } = self
            && let Err(e) = hq_db::web_auth::delete(db, id)
        {
            tracing::warn!(error = %e, "notify-restart: could not delete its broadcast token, it expires in a minute");
        }
    }
}

/// Never opens the database as a user who does not own it (the updater runs this as the
/// service user), so a manual root run cannot leave root-owned WAL files behind, and never
/// migrates it. Tables this binary knows but the database lacks read as "not signed out".
fn broadcast_bearer(config: &HqConfig) -> Option<Bearer> {
    let configured = config
        .web_auth_token
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty());
    let Ok(db) = super::web::open_existing_db_as_owner(config) else {
        return configured.map(|t| Bearer::Configured(t.to_string()));
    };
    if let Some(token) = configured
        && !hq_db::web_auth::is_revoked(&db, token).unwrap_or(false)
    {
        return Some(Bearer::Configured(token.to_string()));
    }
    if !hq_db::web_auth::sign_in_enabled(&db, configured).unwrap_or(false) {
        return None;
    }
    let now = chrono::Utc::now().timestamp();
    let expires = Some(now + BROADCAST_TOKEN_TTL_SECS);
    let minted = hq_db::web_auth::mint(&db, BROADCAST_TOKEN_LABEL, now, expires)
        .inspect_err(
            |e| tracing::warn!(error = %e, "notify-restart: could not mint a broadcast token"),
        )
        .ok()?;
    Some(Bearer::Temporary {
        db,
        id: minted.id,
        token: minted.token,
    })
}

fn message_for(phase: &str, reason: &str, sha: Option<&str>) -> (String, String) {
    let commit = sha.map(|s| format!(" ({s})")).unwrap_or_default();
    match phase {
        "pre" => (
            format!("HQ restarting for a {reason}{commit}"),
            "Brief interruption expected, usually under 30s. Long-running work resumes \
             automatically after restart; a short in-flight request may need retrying."
                .to_string(),
        ),
        "post-ok" => (
            "HQ back up".to_string(),
            format!("Restart for {reason}{commit} completed and passed its health check."),
        ),
        "post-rolled-back" => (
            "HQ deploy rolled back".to_string(),
            format!(
                "New build for {reason}{commit} failed its health check; restored the previous binary."
            ),
        ),
        "alert" => ("HQ update needs attention".to_string(), reason.to_string()),
        other => (
            format!("HQ notify-restart: unknown phase '{other}'"),
            reason.to_string(),
        ),
    }
}

async fn send_telegram(config: &HqConfig, text: &str) {
    #[cfg(feature = "telegram")]
    {
        let Some(token) = config
            .relay
            .notifications_token
            .as_deref()
            .or(config.relay.telegram_token.as_deref())
        else {
            return;
        };
        let Some(chat) =
            hq_core::telegram_access::notification_chat_id(&config.vault_path, &config.relay)
        else {
            return;
        };
        if let Err(e) = hq_relay::telegram::send_message(token, chat, text).await {
            tracing::warn!(error = %e, "notify-restart: telegram delivery failed");
        }
    }
    #[cfg(not(feature = "telegram"))]
    {
        let _ = (config, text);
    }
}

async fn send_discord(config: &HqConfig, text: &str) {
    let Some(token) = config.relay.discord_token.as_deref() else {
        return;
    };
    let channel =
        hq_core::discord_notify::resolve_discord_channel(&config.vault_path, "system-alerts")
            .or_else(|| hq_core::discord_notify::read_discord_presence_channel(&config.vault_path));
    let Some(channel) = channel else {
        return;
    };
    if let Err(e) = hq_core::discord_notify::send_discord_message(token, channel, text).await {
        tracing::warn!(error = %e, "notify-restart: discord delivery failed");
    }
}

/// Loopback call into the (still-running, pre-restart, or freshly-restarted) daemon's
/// hq-web WS broadcast. Silently a no-op if the daemon isn't reachable, e.g. right after
/// a restart that hasn't come back up yet — the `post-*` calls happen after the health
/// check already confirmed it's up.
async fn broadcast_web(config: &HqConfig, text: &str) {
    let Ok(client) = reqwest::Client::builder()
        .timeout(WEB_BROADCAST_TIMEOUT)
        .build()
    else {
        return;
    };
    let url = format!("http://127.0.0.1:{}/api/admin/broadcast", config.ws_port);
    let mut request = client
        .post(&url)
        .json(&serde_json::json!({ "message": text }));
    let bearer = broadcast_bearer(config);
    if let Some(bearer) = &bearer {
        request = request.bearer_auth(bearer.token());
    }
    match request.send().await {
        Ok(res) if !res.status().is_success() => {
            tracing::warn!(status = %res.status(), "notify-restart: web broadcast rejected");
        }
        Ok(_) => {}
        Err(e) => tracing::warn!(error = %e, "notify-restart: web broadcast unreachable"),
    }
}

pub async fn run(config: &HqConfig, phase: &str, reason: &str, sha: Option<&str>) -> Result<()> {
    let (title, body) = message_for(phase, reason, sha);
    let text = format!("{title}\n\n{body}");

    send_telegram(config, &text).await;
    send_discord(config, &text).await;
    broadcast_web(config, &text).await;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_in(vault: &std::path::Path, token: Option<&str>) -> HqConfig {
        std::fs::create_dir_all(vault.join("_data")).unwrap();
        HqConfig {
            vault_path: vault.to_path_buf(),
            web_auth_token: token.map(str::to_string),
            ..Default::default()
        }
    }

    #[test]
    fn the_broadcast_uses_the_configured_token_until_it_is_signed_out() {
        let vault = tempfile::TempDir::new().unwrap();
        let config = config_in(vault.path(), Some("operator-env-token"));
        let bearer = broadcast_bearer(&config).unwrap();
        assert_eq!(bearer.token(), "operator-env-token");
        drop(bearer);

        let db = hq_db::Database::open(&config.db_path()).unwrap();
        let kept =
            hq_db::web_auth::rotate(&db, Some("operator-env-token"), "Sign out everywhere", 1)
                .unwrap();
        let bearer = broadcast_bearer(&config).unwrap();
        let now = chrono::Utc::now().timestamp();
        assert!(
            bearer
                .token()
                .starts_with(hq_db::web_auth::DEVICE_TOKEN_PREFIX)
        );
        assert!(
            hq_db::web_auth::verify(&db, bearer.token(), now)
                .unwrap()
                .is_some()
        );
        let used = bearer.token().to_string();
        drop(bearer);
        assert_eq!(hq_db::web_auth::verify(&db, &used, now).unwrap(), None);
        let left = hq_db::web_auth::list(&db, now).unwrap();
        assert_eq!(left.len(), 1, "only the owner's device remains");
        assert_eq!(left[0].id, kept.id);
    }

    #[test]
    fn an_older_database_is_never_migrated_and_keeps_the_configured_token() {
        let vault = tempfile::TempDir::new().unwrap();
        let config = config_in(vault.path(), Some("operator-env-token"));
        hq_db::Connection::open(config.db_path())
            .unwrap()
            .execute_batch("CREATE TABLE unrelated (a INTEGER);")
            .unwrap();
        let bearer = broadcast_bearer(&config).unwrap();
        assert_eq!(bearer.token(), "operator-env-token");
        drop(bearer);
        let tables: i64 = hq_db::Connection::open(config.db_path())
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name IN ('schema_version', 'web_device_tokens')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tables, 0, "notify-restart ran no migration");
    }

    #[test]
    fn an_open_server_gets_no_bearer() {
        let vault = tempfile::TempDir::new().unwrap();
        assert!(broadcast_bearer(&config_in(vault.path(), None)).is_none());
    }
}
