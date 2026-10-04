//! Forwards `_mailboxes/relay/` messages to the last active Discord channel.

use super::*;

const POLL_EVERY: std::time::Duration = std::time::Duration::from_secs(45);

/// Polls `_mailboxes/relay/` every 45s and forwards messages to the most
/// recently active Discord channel. Mirrors the Telegram mailbox poller.
pub(super) async fn run_discord_mailbox_poller(
    vault_path: Arc<std::path::PathBuf>,
    http: Arc<serenity::http::Http>,
    active_channel_id: Arc<TokioMutex<Option<u64>>>,
) {
    let mut interval = tokio::time::interval(POLL_EVERY);
    // Value-bus urgent items are owned by the primary surface (it renders the
    // approve/dismiss controls). Loaded once; a primary_surface change needs a
    // daemon restart, like other relay config.
    let discord_is_primary = hq_core::config::HqConfig::load()
        .map(|c| c.relay.primary_surface == hq_core::config::PrimarySurface::Discord)
        .unwrap_or(false);
    loop {
        interval.tick().await;
        let Some(cid) = *active_channel_id.lock().await else {
            continue;
        };
        let messages = match hq_core::mailbox::receive_messages(&vault_path, "relay") {
            Ok(messages) => messages,
            Err(e) => {
                tracing::debug!(error = %e, "discord-mailbox-poller: failed to read mailbox");
                continue;
            }
        };
        for msg in messages {
            route_one(&vault_path, &http, cid, discord_is_primary, msg).await;
        }
    }
}

/// Deliver one mailbox message to its category channel (or the active one),
/// or re-enqueue it for Telegram's poller.
async fn route_one(
    vault_path: &Path,
    http: &serenity::http::Http,
    active_channel: u64,
    discord_is_primary: bool,
    msg: hq_core::types::MailboxMessage,
) {
    let requeue = || {
        let _ = hq_core::mailbox::send_message(vault_path, &msg);
    };
    // Value-bus items belong to the primary surface, which renders the buttons.
    if msg.from == "value-bus" && !discord_is_primary {
        return requeue();
    }
    // Family confirmation prompts go directly to the owner's active channel with Approve/Deny buttons.
    if msg.from == "family-confirm" {
        let ch = serenity::model::prelude::ChannelId::new(active_channel);
        let chunks = split_message(&msg.content, DC_MESSAGE_CHARS);
        if chunks.len() == 1
            && let Some(out) = button_message(&msg.from, &msg.content)
        {
            let _ = ch.send_message(http, out).await;
            return;
        }
        for chunk in chunks {
            let _ = ch.say(http, &chunk).await;
        }
        return;
    }
    // Discord claims only categories it has real routing for; everything else,
    // email-triage included, stays with Telegram's poller. Without the requeue
    // both pollers race on the same destructive `receive_messages` call.
    let Some(category) = crate::discord_channels::owning_category(vault_path, &msg.from) else {
        return requeue();
    };
    // Only the gate's Drop matters here: Discord has no urgent-versus-digest
    // timing. A Nudge tagged interrupt=true is never dropped (FR-001b).
    if hq_daemon::notif_gate::gate_decision(vault_path, &msg)
        == hq_daemon::notif_gate::GateDecision::Drop
    {
        return;
    }
    let target = hq_core::discord_notify::resolve_discord_channel(vault_path, &category)
        .unwrap_or(active_channel);
    let ch = serenity::model::prelude::ChannelId::new(target);
    let chunks = split_message(&msg.content, DC_MESSAGE_CHARS);
    if chunks.len() == 1
        && let Some(out) = button_message(&msg.from, &msg.content)
    {
        let _ = ch.send_message(http, out).await;
        return;
    }
    for chunk in chunks {
        let _ = ch.say(http, &chunk).await;
    }
}

/// Real Send/Skip or Approve/Dismiss buttons for the two interactive types,
/// when the text carries a token; `None` means plain text is right.
fn button_message(from: &str, text: &str) -> Option<CreateMessage> {
    match from {
        "email-triage" => hq_daemon::parse_email_command(text)
            .map(|(_, token, _)| build_email_action_message(text, &token)),
        "value-bus" => hq_daemon::parse_value_command(text)
            .map(|(_, token)| build_value_action_message(text, &token)),
        "family-confirm" => hq_daemon::family_confirm::parse_family_command(text)
            .map(|(_, token)| build_family_action_message(text, &token)),
        _ => None,
    }
}
