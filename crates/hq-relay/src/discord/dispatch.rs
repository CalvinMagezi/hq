//! Native HQ dispatch for one Discord message: placeholder ticker, detach, progress.

use super::*;
use crate::native_run::{AbortOnDrop, Mirror, TurnRow};

/// Discord's per-message character limit.
pub(super) const DC_MESSAGE_CHARS: usize = 2000;
/// Progress notes are cut to fit one message with room to spare.
pub(super) const DC_PROGRESS_CHARS: usize = 1900;
const TICKER_PERIOD: std::time::Duration = std::time::Duration::from_secs(30);

/// Send text to a channel, split at Discord's message limit.
pub(super) fn dc_sender(
    http: Arc<serenity::http::Http>,
    channel: ChannelId,
) -> crate::relay_common::ChatSender {
    Arc::new(move |text: String| {
        let http = http.clone();
        tokio::spawn(async move {
            for chunk in split_message(&text, DC_MESSAGE_CHARS) {
                if let Err(e) = channel.say(&http, &chunk).await {
                    tracing::warn!(error = %e, "discord delivery failed");
                    break;
                }
            }
        });
    })
}

/// Run a Discord message through the shared `run_native_hq` path, with a
/// 30-second ticker that edits the placeholder with turn and tool info.
// Per-message Discord context is threaded as flat parameters; a params struct is tracked in TECHDEBT.md.
#[allow(clippy::too_many_arguments)]
async fn dispatch_hq_native(
    content: &str,
    system_prompt: &str,
    threads: &Arc<TokioMutex<HashMap<u64, ChannelState>>>,
    channel_key: u64,
    discord_ctx: serenity::prelude::Context,
    placeholder_id: serenity::model::prelude::MessageId,
    mirror: Mirror,
    guest_name: Option<String>,
    scope: hq_core::privacy::DisclosureScope,
) -> Result<String> {
    let config = crate::native_run::load_config("discord dispatch");
    // A Discord message was just received; a live person is driving this turn.
    let (session, repo_root) = crate::native_run::relay_session(&config, true);
    let mut instructions = strip_loaded_soul(system_prompt, &config.vault_path);
    let owner = config.relay.owner_name();
    let allowed = config.relay.discord_family_allowed_harnesses.clone();
    if let Some(ref name) = guest_name {
        let harness_rule = if allowed.is_empty() {
            String::new()
        } else {
            format!(
                " If {name} asks for development/coding work through the host, you may only \
launch these harnesses: {}. Tell {name} plainly if that limits what you can do.",
                allowed.join(", ")
            )
        };
        let block = format!(
            "You're talking with {name}, a trusted family member of {owner}'s (not {owner} \
themselves) with full access to your tools. Before doing anything genuinely \
risky, irreversible, or externally-visible on {owner}'s behalf (spending \
money, sending something in their name, deleting or changing real data, \
infrastructure changes), stop and check with {owner} first rather than \
proceeding. Any remote MCP tool call is handled automatically: it will pause \
and ask {owner} for you, so just tell {name} you're checking with {owner} \
rather than trying to work around it.{harness_rule}\n\n"
        );
        instructions = format!("{block}{instructions}");
    }
    if let Some(notice) = scope.prompt_notice() {
        instructions = format!("{notice}{instructions}");
    }
    let inputs = crate::native_run::turn_inputs(threads, channel_key).await;

    let mut identity = hq_core::identity::RequestIdentity::from_discord(channel_key);
    identity.scope = scope;
    if let Some(ref name) = guest_name {
        identity.user_name = name.clone();
        identity.family_guest = Some(hq_core::identity::FamilyGuestInfo {
            name: name.clone(),
            origin_channel_id: channel_key,
            owner_name: owner,
            allowed_harnesses: allowed,
        });
    }
    let channel = ChannelId::new(channel_key);
    let send = dc_sender(discord_ctx.http.clone(), channel);
    let row = TurnRow::register(
        &config,
        "discord",
        &channel_key.to_string(),
        Some(&identity.user_id),
        content,
    );
    let on_detached = crate::native_run::detached_sink(
        threads,
        channel_key,
        format!("dc-{channel_key}"),
        &row,
        send.clone(),
        mirror,
        config.vault_path.clone(),
    );

    let feed = crate::heartbeat::new_activity_feed();
    let on_event = feed_hook(feed.clone());
    let _ticker = AbortOnDrop(spawn_ticker(discord_ctx, channel, placeholder_id, feed));

    let run = hq_agent::native_hq::run_native_hq(
        &config,
        content,
        instructions,
        repo_root,
        session,
        hq_agent::native_hq::NativeHqHooks {
            history: inputs.history,
            image_parts: inputs.images,
            on_event: Some(on_event),
            on_cancel: Some(crate::native_run::cancel_hook(threads, channel_key)),
            on_steer: Some(crate::native_run::steer_hook(threads, channel_key)),
            on_detached: Some(on_detached),
            on_child_completion: Some(row.child_sink(send.clone(), DC_PROGRESS_CHARS)),
            permission_preset: inputs.permission_preset,
            ..row.hooks(&config, identity, send, DC_PROGRESS_CHARS)
        },
    )
    .await;
    drop(_ticker);
    crate::native_run::clear_cancel(threads, channel_key).await;
    row.close(&run);
    run.map(|r| r.text)
}

/// Turn count and the last tool started, for the placeholder ticker.
fn feed_hook(
    feed: crate::heartbeat::SharedActivityFeed,
) -> Box<dyn Fn(hq_core::types::SessionEvent) + Send + Sync> {
    use hq_core::types::SessionEvent;
    Box::new(move |event| match &event {
        SessionEvent::TurnEnd { turn } => {
            if let Ok(mut f) = feed.lock() {
                f.on_turn(*turn);
            }
        }
        SessionEvent::ToolStart { tool_name, .. } => {
            if let Ok(mut f) = feed.lock() {
                f.on_tool_start(tool_name);
            }
        }
        _ => {}
    })
}

fn spawn_ticker(
    ctx: serenity::prelude::Context,
    channel: ChannelId,
    placeholder_id: serenity::model::prelude::MessageId,
    feed: crate::heartbeat::SharedActivityFeed,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let start = std::time::Instant::now();
        let mut tick = tokio::time::interval(TICKER_PERIOD);
        tick.tick().await;
        loop {
            tick.tick().await;
            let text = ticker_line(&feed.lock().unwrap(), start.elapsed().as_secs());
            let edit = EditMessage::new().content(text);
            let _ = channel.edit_message(&ctx.http, placeholder_id, edit).await;
        }
    })
}

fn ticker_line(hb: &crate::heartbeat::ActivityFeed, elapsed_secs: u64) -> String {
    let turn = if hb.turn > 0 {
        format!(" | Turn {}", hb.turn)
    } else {
        String::new()
    };
    let tool = if hb.last_tool.is_empty() {
        String::new()
    } else {
        format!(" | {}", hb.last_tool)
    };
    let (mins, secs) = (elapsed_secs / 60, elapsed_secs % 60);
    let time = if mins > 0 {
        format!("{mins}m {secs}s")
    } else {
        format!("{secs}s")
    };
    format!("\u{21BA}{turn}{tool} | {time}")
}

/// Run one Discord message. The outer `Err` is the busy reply when a turn is
/// already running in this channel; the message was steered or refused.
// Per-message Discord context is threaded as flat parameters; a params struct is tracked in TECHDEBT.md.
#[allow(clippy::too_many_arguments)]
pub(super) async fn dispatch_hq(
    content: &str,
    system_prompt: &str,
    threads: &Arc<TokioMutex<HashMap<u64, ChannelState>>>,
    channel_key: u64,
    discord_ctx: serenity::prelude::Context,
    placeholder_id: serenity::model::prelude::MessageId,
    images: Vec<hq_core::types::ImageAttachment>,
    mirror: Mirror,
    guest_name: Option<String>,
    scope: hq_core::privacy::DisclosureScope,
) -> Result<Result<String>, &'static str> {
    {
        let mut t = threads.lock().await;
        let state = t
            .entry(channel_key)
            .or_insert_with(ChannelState::new_default);
        state.claim_turn(content)?;
        state.stage_turn(system_prompt, content, images);
    }

    let dispatch_result: Result<String> = dispatch_hq_native(
        content,
        system_prompt,
        threads,
        channel_key,
        discord_ctx,
        placeholder_id,
        mirror,
        guest_name,
        scope,
    )
    .await;

    // On success, the assistant's reply is appended so this channel's resubmitted
    // history stays a faithful record; on failure, `record_turn_outcome` appends a
    // marker instead of leaving a dangling unanswered user turn.
    {
        let mut t = threads.lock().await;
        if let Some(state) = t.get_mut(&channel_key) {
            state.record_turn_outcome(&dispatch_result);
            state.turn_in_flight = false;
            state.pending_steer = None;
        }
    }

    Ok(dispatch_result)
}

#[cfg(test)]
mod identity_tests {
    use hq_core::identity::RequestIdentity;

    #[test]
    fn discord_identity_from_channel_id() {
        let id = RequestIdentity::from_discord(123456789);
        assert_eq!(id.user_id, "discord:123456789");
        assert_eq!(id.session_key, "hq-dc-123456789"); // gitleaks:allow
        assert_eq!(id.source.label(), "discord");
    }
}
