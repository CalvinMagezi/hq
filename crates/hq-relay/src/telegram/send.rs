//! Outbound message helpers: Telegram delivery utilities.
//!
//! These are thin wrappers used by daemon notifications and other callers
//! that need to push messages without holding relay session state.

/// Send a single message to a Telegram chat.
pub async fn send_message(token: &str, chat_id: i64, text: &str) -> anyhow::Result<()> {
    use teloxide::prelude::Requester;
    let bot = teloxide::Bot::new(token);
    let chat = teloxide::types::ChatId(chat_id);
    bot.send_message(chat, text).await?;
    Ok(())
}

