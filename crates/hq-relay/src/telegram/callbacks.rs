//! Inline keyboard taps: value-bus Approve/Dismiss and notification keep/mute.

use teloxide::prelude::*;

/// Single callback_query endpoint for every inline keyboard this relay
/// sends: value-bus Approve/Dismiss (`mailbox.rs::build_value_action_keyboard`)
/// and notif keep/mute (`reaction_keyboard`).
pub(super) async fn value_bus_callback_handler(
    bot: Bot,
    callback: teloxide::types::CallbackQuery,
) -> ResponseResult<()> {
    use teloxide::prelude::Requester;

    let Some(data) = callback.data.as_deref() else {
        let _ = bot.answer_callback_query(callback.id).await;
        return Ok(());
    };

    let config = hq_core::config::HqConfig::load().unwrap_or_default();
    let vault_path = config.vault_path.clone();
    let sender = callback.from.id.0 as i64;
    if !super::access::callback_sender_allowed(&vault_path, &config.relay, sender) {
        tracing::warn!(sender, "telegram: ignoring button tap from a non-owner");
        let _ = bot.answer_callback_query(callback.id).await;
        return Ok(());
    }
    let ack_text =
        if let Some((trait_key, value)) = super::mailbox::parse_reaction_callback_data(data) {
            let _ = hq_daemon::user_model::apply_user_reply(&vault_path, &trait_key, value);
            let verb = if value == "noise" { "muted" } else { "kept" };
            format!("Got it — {verb} {trait_key}.")
        } else if let Some((action, token)) = super::mailbox::parse_value_callback_data(data) {
            let value_action = match action {
                "approve" => hq_daemon::ValueAction::Approve,
                _ => hq_daemon::ValueAction::Dismiss,
            };
            match hq_daemon::record_engagement(&vault_path, token, value_action) {
                Ok(true) => "Recorded.".to_string(),
                _ => "Already handled.".to_string(),
            }
        } else {
            // Still answer an unknown tap so the client stops its loading spinner.
            let _ = bot.answer_callback_query(callback.id).await;
            return Ok(());
        };
    acknowledge(&bot, callback, &ack_text).await;
    Ok(())
}

/// Answer the tap, then append the outcome to the message, which also drops
/// its inline keyboard so the buttons can't be tapped again.
async fn acknowledge(bot: &Bot, callback: teloxide::types::CallbackQuery, ack_text: &str) {
    let regular_message = callback.regular_message().cloned();
    let _ = bot.answer_callback_query(callback.id).text(ack_text).await;
    if let Some(message) = regular_message {
        let new_text = format!("{}\n\n[{ack_text}]", message.text().unwrap_or_default());
        let _ = bot
            .edit_message_text(message.chat.id, message.id, new_text)
            .await;
    }
}
