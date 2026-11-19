//! Outbound message helpers: Telegram delivery utilities.
//!
//! These are thin wrappers used by daemon notifications and other callers
//! that need to push messages without holding relay session state.

/// Send a single message to a Telegram chat.
pub async fn send_message(token: &str, chat_id: i64, text: &str) -> anyhow::Result<()> {
    use teloxide::payloads::SendMessageSetters;
    use teloxide::prelude::Requester;
    let bot = teloxide::Bot::new(token);
    let chat = teloxide::types::ChatId(chat_id);
    bot.send_message(chat, text)
        .link_preview_options(no_link_preview())
        .await?;
    Ok(())
}

/// Link previews off. Telegram's servers fetch a preview from the link's host, so a link the
/// model wrote would send that host a request the owner never made.
pub(crate) fn no_link_preview() -> teloxide::types::LinkPreviewOptions {
    teloxide::types::LinkPreviewOptions {
        is_disabled: true,
        url: None,
        prefer_small_media: false,
        prefer_large_media: false,
        show_above_text: false,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn previews_are_off_for_model_written_links() {
        assert!(super::no_link_preview().is_disabled);
    }
}
