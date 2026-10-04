//! The steps of one inbound Discord message and one button or slash interaction.

use std::ops::ControlFlow;

use serenity::builder::{CreateInteractionResponse, CreateInteractionResponseMessage};
use serenity::model::application::{CommandInteraction, ComponentInteraction};
use serenity::prelude::Context as Ctx;

use super::*;

const TYPING_REFRESH: std::time::Duration = std::time::Duration::from_secs(8);
const PLACEHOLDER_TEXT: &str = "Thinking\u{2026} \u{258D}";
const SKILL_HINT_BUDGET: usize = 2000;

impl Handler {
    /// While a turn runs, steer the message into it (or say it is busy) before
    /// any command runs, as Telegram does: a resume, watch or reset mid-turn
    /// would otherwise corrupt the running turn or start a second one.
    /// Cancel stays exempt so a stuck turn can always be stopped.
    pub(super) async fn reply_if_busy(
        &self,
        ctx: &Ctx,
        msg: &Message,
        effective_channel: ChannelId,
        content: &str,
        lower: &str,
    ) -> bool {
        if lower == "!cancel" || lower == "/cancel" {
            return false;
        }
        let busy = self
            .threads
            .lock()
            .await
            .get(&effective_channel.get())
            .and_then(|s| s.busy_reply(content));
        let Some(reply) = busy else {
            return false;
        };
        if effective_channel == msg.channel_id {
            let _ = msg.reply(&ctx.http, reply).await;
        } else {
            let _ = effective_channel.say(&ctx.http, reply).await;
        }
        true
    }

    /// Mention/DM gate plus the allowlist. Logs a rejection only for a sender
    /// that reached the check; an unmentioned bot or guild message stays silent.
    pub(super) async fn authorized_owner(&self, msg: &Message, bot_id: UserId) -> bool {
        let is_mention = msg.mentions.iter().any(|u| u.id == bot_id);
        let is_dm = msg.guild_id.is_none();
        // Webhook posts also carry `author.bot`, but any member with Manage
        // Webhooks can create one, so only a real bot account skips the allowlist.
        let is_real_bot = msg.author.bot && msg.webhook_id.is_none();
        let allowed = authorize_incoming(
            &self.vault_path,
            msg.author.id.get(),
            is_real_bot,
            is_mention,
            is_dm,
            &self.discord_allowed_user_ids,
        )
        .await;
        if !allowed && !is_real_bot && (is_mention || is_dm) {
            tracing::warn!(
                user_id = msg.author.id.get(),
                "discord: rejecting message from unauthorized user"
            );
        }
        allowed
    }

    /// True if authorized as owner (DM/mention) or authorized as a family guest.
    pub(super) async fn authorized(&self, msg: &Message, bot_id: UserId) -> bool {
        self.authorized_owner(msg, bot_id).await || self.family_authorized(msg).await
    }

    /// Persist the last active channel, read by the mailbox poller here and by
    /// `hq notify-restart` through `DISCORD-PRESENCE.md`.
    pub(super) async fn record_presence(&self, channel: u64) {
        *self.active_channel_id.lock().await = Some(channel);
        let presence_dir = self.vault_path.join("_system");
        let _ = std::fs::create_dir_all(&presence_dir);
        let content = format!(
            "platform: discord\nchannel_id: {channel}\nlast_active: {}",
            chrono::Utc::now().to_rfc3339()
        );
        let _ = std::fs::write(presence_dir.join("DISCORD-PRESENCE.md"), content);
    }

    /// `!reset`, `!focus`, `!model`, `!help`, `!status`, `!cancel` and `!permission`.
    /// True when one of them answered the message.
    pub(super) async fn bang_command(
        &self,
        ctx: &Ctx,
        msg: &Message,
        effective_channel: ChannelId,
        content: &str,
        lower: &str,
    ) -> bool {
        let channel_key = effective_channel.get();
        if self.family_authorized(msg).await {
            if lower == "!model"
                || lower.starts_with("!model ")
                || lower == "!backend"
                || lower.starts_with("!backend ")
            {
                let owner = self.owner_name();
                let reply = format!(
                    "!model is only available to {owner}. You're a guest — ask me questions and I'll loop {owner} in when needed."
                );
                let _ = effective_channel.say(&ctx.http, reply).await;
                return true;
            }
            if crate::chat_commands::is_permission_command(lower) {
                let owner = self.owner_name();
                let reply = format!(
                    "!permission is only available to {owner}. You're a guest — ask me questions and I'll loop {owner} in when needed."
                );
                let _ = effective_channel.say(&ctx.http, reply).await;
                return true;
            }
        }
        let reply = if lower == "!reset" || lower == "!new" {
            crate::chat_commands::reset(&self.threads, &channel_key).await
        } else if let Some(topic) = crate::chat_commands::focus_topic(lower, content) {
            crate::chat_commands::focus(&self.threads, channel_key, topic).await
        } else if lower == "!model" || lower.starts_with("!model ") {
            let name = content.split_whitespace().nth(1).unwrap_or("");
            crate::chat_commands::model(name, "!model")
        } else if lower == "!help" {
            Self::make_help_text()
        } else if lower == "!status" {
            let status = crate::chat_commands::status_fields(&self.threads, &channel_key).await;
            let embed = build_status_embed(&status.model, status.messages);
            let _ = effective_channel
                .send_message(&ctx.http, CreateMessage::new().add_embed(embed))
                .await;
            return true;
        } else if lower == "!cancel" || lower == "/cancel" {
            crate::chat_commands::cancel(&self.threads, channel_key)
                .await
                .to_string()
        } else if crate::chat_commands::is_permission_command(lower) {
            let arg = crate::chat_commands::permission_arg(content);
            crate::chat_commands::permission(&self.threads, channel_key, arg, "!permission").await
        } else {
            return false;
        };
        let _ = effective_channel.say(&ctx.http, reply).await;
        true
    }

    /// `resume`, `watch` and `unwatch`. `Break` when answered; `Continue(Some)`
    /// when a stored prompt should run as this turn.
    pub(super) async fn registry_step(
        &self,
        ctx: &Ctx,
        msg: &Message,
        effective_channel: ChannelId,
        content: &str,
        lower: &str,
    ) -> ControlFlow<(), Option<String>> {
        let reply = crate::chat_commands::registry_command(
            lower,
            content,
            "discord",
            effective_channel.get(),
            None,
            || {
                let config = hq_core::config::HqConfig::load().ok()?;
                Database::open(&config.db_path()).ok().map(Arc::new)
            },
        );
        match reply {
            Some(crate::chat_commands::CommandReply::Done(reply)) => {
                if effective_channel == msg.channel_id {
                    let _ = msg.reply(&ctx.http, reply).await;
                } else {
                    let _ = effective_channel.say(&ctx.http, reply).await;
                }
                ControlFlow::Break(())
            }
            Some(crate::chat_commands::CommandReply::Redispatch { ack, prompt }) => {
                if effective_channel == msg.channel_id {
                    let _ = msg.reply(&ctx.http, ack).await;
                } else {
                    let _ = effective_channel.say(&ctx.http, ack).await;
                }
                ControlFlow::Continue(Some(prompt))
            }
            None => ControlFlow::Continue(None),
        }
    }

    /// Placeholder, native HQ turn, persist, deliver.
    pub(super) async fn run_turn(
        &self,
        ctx: &Ctx,
        msg: &Message,
        effective_channel: ChannelId,
        content: String,
        images: Vec<hq_core::types::ImageAttachment>,
        title: String,
    ) {
        let channel_key = effective_channel.get();
        let typing = spawn_typing(ctx.clone(), effective_channel);
        let placeholder = effective_channel
            .send_message(&ctx.http, CreateMessage::new().content(PLACEHOLDER_TEXT))
            .await;
        let mut placeholder = match placeholder {
            Ok(m) => m,
            Err(e) => {
                tracing::error!("discord: failed to send placeholder: {e}");
                typing.abort();
                return;
            }
        };
        let (enriched_prompt, _matched_skills) = hq_tools::skills::enrich_system_prompt(
            &self.skill_index,
            &self.system_prompt,
            &content,
            None,
            Some(SKILL_HINT_BUDGET),
        );
        let mirror = crate::native_run::Mirror {
            platform: "discord",
            thread_sync: self.thread_sync.clone(),
            title: title.clone(),
        };
        let guest_name = self
            .family_user_name(msg.author.id.get())
            .map(|s| s.to_string());
        let scope = self.disclosure_scope(msg.author.id.get(), guest_name.is_some());
        let result = dispatch_hq(
            &content,
            &enriched_prompt,
            &self.threads,
            channel_key,
            ctx.clone(),
            placeholder.id,
            images,
            mirror,
            guest_name,
            scope,
        )
        .await;
        typing.abort();
        let result = match result {
            Ok(result) => result,
            Err(busy) => {
                let _ = placeholder.delete(&ctx.http).await;
                if effective_channel == msg.channel_id {
                    let _ = msg.reply(&ctx.http, busy).await;
                } else {
                    let _ = effective_channel.say(&ctx.http, busy).await;
                }
                return;
            }
        };
        if let Some(state) = self.threads.lock().await.get(&channel_key) {
            state.save(&self.vault_path, &format!("dc-{channel_key}"));
        }
        self.deliver_reply(
            ctx,
            msg,
            effective_channel,
            &mut placeholder,
            result,
            &title,
        )
        .await;
    }

    async fn deliver_reply(
        &self,
        ctx: &Ctx,
        msg: &Message,
        effective_channel: ChannelId,
        placeholder: &mut Message,
        result: Result<String>,
        title: &str,
    ) {
        let reply = match result {
            Ok(reply) if reply.is_empty() => {
                let _ = placeholder
                    .edit(
                        &ctx.http,
                        EditMessage::new().content("No response received."),
                    )
                    .await;
                return;
            }
            Ok(reply) => reply,
            Err(e) => {
                tracing::error!(error = %e, "hq harness error");
                let _ = placeholder
                    .edit(
                        &ctx.http,
                        EditMessage::new().content(format!("Error (hq): {e}")),
                    )
                    .await;
                return;
            }
        };
        let _ = placeholder.delete(&ctx.http).await;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        self.thread_sync.record(
            "discord",
            &effective_channel.get().to_string(),
            title,
            "assistant",
            &reply,
        );
        let mut last_sent: Option<Message> = None;
        for chunk in split_message(&reply, DC_MESSAGE_CHARS) {
            match effective_channel.say(&ctx.http, &chunk).await {
                Ok(m) => last_sent = Some(m),
                Err(e) => tracing::error!("discord send error: {e}"),
            }
        }
        if effective_channel != msg.channel_id {
            if let Some(sent) = last_sent {
                let _ = sent
                    .react(&ctx.http, ReactionType::Unicode("\u{2705}".to_string()))
                    .await;
            }
        } else {
            let _ = msg
                .react(&ctx.http, ReactionType::Unicode("\u{2705}".to_string()))
                .await;
        }
    }

    pub(super) async fn slash_command(&self, ctx: &Ctx, cmd: CommandInteraction) {
        if !is_message_author_allowed(
            &self.vault_path,
            cmd.user.id.get(),
            &self.discord_allowed_user_ids,
        )
        .await
        {
            tracing::warn!(
                user = cmd.user.id.get(),
                command = %cmd.data.name,
                "discord: rejected slash command from a non-allowlisted user"
            );
            let refusal = CreateInteractionResponseMessage::new()
                .content("Not authorized.")
                .ephemeral(true);
            let _ = cmd
                .create_response(&ctx.http, CreateInteractionResponse::Message(refusal))
                .await;
            return;
        }
        let channel_key = cmd.channel_id.get();
        let response_text = match cmd.data.name.as_str() {
            "reset" => crate::chat_commands::reset(&self.threads, &channel_key).await,
            "model" => {
                let name = cmd
                    .data
                    .options
                    .first()
                    .and_then(|o| match &o.value {
                        serenity::model::application::CommandDataOptionValue::String(s) => {
                            Some(s.as_str())
                        }
                        _ => None,
                    })
                    .unwrap_or_default();
                crate::chat_commands::model(name.trim(), "/model")
            }
            "status" => crate::chat_commands::status_fields(&self.threads, &channel_key)
                .await
                .text(),
            "help" => Self::make_help_text(),
            _ => "Unknown command.".to_string(),
        };
        let reply = CreateInteractionResponseMessage::new().content(response_text);
        let _ = cmd
            .create_response(&ctx.http, CreateInteractionResponse::Message(reply))
            .await;
    }

    /// Email send/skip and value-bus approve/dismiss buttons.
    pub(super) async fn button(&self, ctx: &Ctx, comp: ComponentInteraction) {
        let id = comp.data.custom_id.as_str();
        let reply = if let Some((action, token)) = parse_email_button_id(id) {
            if !self.accept_button(ctx, &comp, token, "email").await {
                return;
            }
            let action = if action == "send" {
                hq_daemon::EmailAction::Send
            } else {
                hq_daemon::EmailAction::Skip
            };
            hq_daemon::resolve_email_action(&self.vault_path, token, action, None)
                .await
                .unwrap_or_else(|| {
                    "This email approval has already been handled or expired.".to_string()
                })
        } else if let Some((action, token)) = parse_value_button_id(id) {
            if !self.accept_button(ctx, &comp, token, "value-bus").await {
                return;
            }
            let action = if action == "approve" {
                hq_daemon::ValueAction::Approve
            } else {
                hq_daemon::ValueAction::Dismiss
            };
            match hq_daemon::record_engagement(&self.vault_path, token, action) {
                Ok(true) => "Recorded.".to_string(),
                Ok(false) => "This item has already been handled or expired.".to_string(),
                Err(e) => {
                    tracing::warn!(error = %e, token, "discord: value-bus record_engagement failed");
                    format!("Failed to record: {e}")
                }
            }
        } else if let Some((action, token)) = parse_family_button_id(id) {
            if !self
                .accept_button(ctx, &comp, token, "family-confirm")
                .await
            {
                return;
            }
            let action = if action == "approve" {
                hq_daemon::FamilyAction::Approve
            } else {
                hq_daemon::FamilyAction::Deny
            };
            if let Some((guest_reply, origin_channel_id)) =
                hq_daemon::family_confirm::resolve_family_action(&self.vault_path, token, action)
                    .await
            {
                let origin_channel = ChannelId::new(origin_channel_id);
                for chunk in split_message(&guest_reply, DC_MESSAGE_CHARS) {
                    if let Err(e) = origin_channel.say(&ctx.http, &chunk).await {
                        tracing::warn!(
                            error = %e,
                            origin_channel_id,
                            "failed to deliver family confirm reply to guest thread"
                        );
                    }
                }
                "Done — relayed to family thread.".to_string()
            } else {
                "This family request has already been handled or expired.".to_string()
            }
        } else {
            return;
        };
        let _ = comp.channel_id.say(&ctx.http, &reply).await;
    }

    /// Validate the token, check the clicker against the allowlist, and acknowledge.
    async fn accept_button(
        &self,
        ctx: &Ctx,
        comp: &ComponentInteraction,
        token: &str,
        kind: &str,
    ) -> bool {
        if !is_valid_button_token(token) {
            tracing::warn!(custom_id = %comp.data.custom_id, "discord: rejected malformed {kind} token");
            return false;
        }
        if !is_message_author_allowed(
            &self.vault_path,
            comp.user.id.get(),
            &self.discord_allowed_user_ids,
        )
        .await
        {
            tracing::warn!(
                user = comp.user.id.get(),
                "discord: rejected {kind} button click from a non-allowlisted user"
            );
            return false;
        }
        if let Err(e) = comp
            .create_response(&ctx.http, CreateInteractionResponse::Acknowledge)
            .await
        {
            tracing::error!(error = %e, "discord: failed to acknowledge component interaction");
            return false;
        }
        true
    }
}

/// The text to act on: mentions stripped, reply context prepended. `None` when empty.
pub(super) fn inbound_text(msg: &Message, bot_id: UserId) -> Option<String> {
    let raw = msg
        .content
        .replace(&format!("<@{bot_id}>"), "")
        .replace(&format!("<@!{bot_id}>"), "")
        .trim()
        .to_string();
    if raw.is_empty() {
        return None;
    }
    Some(match &msg.referenced_message {
        Some(referenced) => {
            let who = if referenced.author.bot {
                "assistant"
            } else {
                &referenced.author.name
            };
            quote_reply(who, &referenced.content, &raw)
        }
        None => raw,
    })
}

fn spawn_typing(ctx: Ctx, channel: ChannelId) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let _ = channel.broadcast_typing(&ctx.http).await;
            tokio::time::sleep(TYPING_REFRESH).await;
        }
    })
}
