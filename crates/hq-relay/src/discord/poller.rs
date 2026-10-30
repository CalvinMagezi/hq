//! Empties `_mailboxes/relay/`: Discord carries chat only, so notices are dropped. The one chat
//! flow it still carries is a family member's request, posted to the owner's active channel with
//! Approve and Deny buttons.

use super::*;

const POLL_EVERY: std::time::Duration = std::time::Duration::from_secs(45);

pub(super) async fn run_discord_mailbox_poller(
    vault_path: Arc<std::path::PathBuf>,
    http: Arc<serenity::http::Http>,
    active_channel_id: Arc<TokioMutex<Option<u64>>>,
) {
    let mut interval = tokio::time::interval(POLL_EVERY);
    loop {
        interval.tick().await;
        let flows = crate::relay_common::take_chat_flows(&vault_path);
        let Some(cid) = *active_channel_id.lock().await else {
            // No channel to answer in yet: keep the requests for a later tick.
            for msg in flows {
                let _ = hq_core::mailbox::send_message(&vault_path, &msg);
            }
            continue;
        };
        for msg in flows {
            post_family_request(&http, cid, &msg).await;
        }
    }
}

async fn post_family_request(
    http: &serenity::http::Http,
    channel: u64,
    msg: &hq_core::types::MailboxMessage,
) {
    let ch = serenity::model::prelude::ChannelId::new(channel);
    let chunks = split_message(&msg.content, DC_MESSAGE_CHARS);
    let buttons = hq_daemon::family_confirm::parse_family_command(&msg.content)
        .map(|(_, token)| build_family_action_message(&msg.content, &token));
    if let (1, Some(out)) = (chunks.len(), buttons) {
        let _ = ch.send_message(http, out).await;
        return;
    }
    for chunk in chunks {
        let _ = ch.say(http, &chunk).await;
    }
}
