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
    let mut request = client.post(&url).json(&serde_json::json!({ "message": text }));
    if let Some(token) = config.web_auth_token.as_deref() {
        request = request.bearer_auth(token);
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
