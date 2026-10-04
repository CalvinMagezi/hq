//! Telegram slash command processing for the HQ relay.
//!
//! Each function handles one command group and returns Ok(true) when it consumed
//! the message (caller should return early), Ok(false) when the message should
//! continue to normal agent dispatch.

use std::collections::HashMap;
use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::{BotCommand, BotCommandScope};
use tokio::sync::Mutex as TokioMutex;

use super::access::TelegramRole;
use super::caller_context::CallerContext;
use crate::relay_common::ChannelState;

/// Publish HQ's slash-command list to Telegram so autocomplete reflects what
/// the relay actually handles. Scoped to private chats; mirrors `/help`.
pub async fn register_hq_commands(bot: &Bot) -> anyhow::Result<()> {
    use teloxide::prelude::Requester;
    let commands: Vec<BotCommand> = [
        ("new", "Start a fresh conversation (alias of /reset)"),
        ("reset", "Clear history"),
        ("focus", "Pivot focus: clear history, keep model and permission pin"),
        ("model", "List the backend chain or switch its primary"),
        (
            "permission",
            "Pin a permission preset for this chat: read-only, workspace-write, danger-full-access",
        ),
        (
            "status",
            "Show model and message count",
        ),
        ("backend", "Show or switch the LLM backend"),
        ("cancel", "Stop the current running task gracefully"),
        (
            "resume",
            "Rerun an interrupted background turn: /resume <id>",
        ),
        (
            "watch",
            "Recurring check: /watch <minutes> [for <hours>h] <prompt>",
        ),
        ("unwatch", "Stop a watch: /unwatch <id>"),
        ("help", "Show available commands"),
    ]
    .into_iter()
    .map(|(c, d)| BotCommand::new(c, d))
    .collect();

    bot.set_my_commands(commands)
        .scope(BotCommandScope::AllPrivateChats)
        .await?;
    Ok(())
}

/// Strip the @BotName suffix from a command (e.g. `/start@mybot` -> `/start`).
pub fn normalize_command(text: &str) -> String {
    if text.starts_with('/') {
        let parts: Vec<&str> = text.splitn(2, ' ').collect();
        let cmd = parts[0].split('@').next().unwrap_or(parts[0]);
        if parts.len() > 1 {
            format!("{} {}", cmd, parts[1])
        } else {
            cmd.to_string()
        }
    } else {
        text.to_string()
    }
}

pub struct CommandContext<'a> {
    pub bot: &'a Bot,
    pub msg: &'a teloxide::types::Message,
    pub chat_key: i64,
    pub text_clean: &'a str,
    pub text_lower: &'a str,
    pub threads: Arc<TokioMutex<HashMap<i64, ChannelState>>>,
    pub caller: &'a CallerContext,
}

/// Guest-only commands return true when handled (caller should return early).
pub async fn require_owner(ctx: &CommandContext<'_>, feature: &str) -> anyhow::Result<bool> {
    use teloxide::prelude::Requester;
    if ctx.caller.role == TelegramRole::Owner {
        return Ok(false);
    }
    ctx.bot
        .send_message(
            ctx.msg.chat.id,
            format!(
                "{feature} is only available to {}. You're a guest — ask me questions and I'll loop {} in when needed.",
                ctx.caller.owner_name, ctx.caller.owner_name
            ),
        )
        .await?;
    Ok(true)
}

/// Returns true if the message was a /reset or /new command.
pub async fn handle_reset(ctx: &CommandContext<'_>) -> anyhow::Result<bool> {
    use teloxide::prelude::Requester;
    let lower = ctx.text_lower;
    if lower == "/reset" || lower == "!reset" || lower == "/new" || lower == "!new" {
        let banner = crate::chat_commands::reset(&ctx.threads, &ctx.chat_key).await;
        // Clean-slate semantics: also drop any reply-keyboard bar (old
        // approve/skip or keep/mute pills) still docked at the bottom of the
        // chat from a prior notification.
        ctx.bot
            .send_message(ctx.msg.chat.id, banner)
            .reply_markup(teloxide::types::ReplyMarkup::kb_remove())
            .await?;
        return Ok(true);
    }
    Ok(false)
}

/// Returns true if the message was a /focus command.
pub async fn handle_focus(ctx: &CommandContext<'_>) -> anyhow::Result<bool> {
    use teloxide::prelude::Requester;
    let Some(topic) = crate::chat_commands::focus_topic(ctx.text_lower, ctx.text_clean) else {
        return Ok(false);
    };
    let reply = crate::chat_commands::focus(&ctx.threads, ctx.chat_key, topic).await;
    ctx.bot.send_message(ctx.msg.chat.id, reply).await?;
    Ok(true)
}

/// Returns true if the message was a /model command.
///
/// Bare `/model` lists the real backend chain (what will actually run);
/// `/model <name>` matches against a configured backend's name or model
/// string and, on a match, promotes it to primary the same way `/backend
/// <name>` does (same underlying `set_primary_backend`, so this is a global
/// switch, not scoped to this chat). A name that matches nothing is rejected.
pub async fn handle_model(ctx: &CommandContext<'_>) -> anyhow::Result<bool> {
    use teloxide::prelude::Requester;
    let trimmed = ctx.text_lower.trim();
    let is_model_cmd = trimmed == "/model"
        || trimmed == "!model"
        || trimmed.starts_with("/model ")
        || trimmed.starts_with("!model ");
    if !is_model_cmd {
        return Ok(false);
    }
    if require_owner(ctx, "Model override").await? {
        return Ok(true);
    }

    let name = ctx.text_clean.split_whitespace().nth(1).unwrap_or("");
    ctx.bot
        .send_message(ctx.msg.chat.id, crate::chat_commands::model(name, "/model"))
        .await?;
    Ok(true)
}

/// Returns true if the message was a /backend command.
///
/// `/backend` (no arg) lists the chain with capabilities; `/backend <name>`
/// promotes that backend to primary (config edit, takes effect next turn —
/// the relay reloads HqConfig per message). Owner-only, like /model.
pub async fn handle_backend(ctx: &CommandContext<'_>) -> anyhow::Result<bool> {
    use teloxide::prelude::Requester;
    let lower = ctx.text_lower;
    let is_bare = lower == "/backend" || lower == "!backend";
    let is_switch = lower.starts_with("/backend ") || lower.starts_with("!backend ");
    if !is_bare && !is_switch {
        return Ok(false);
    }
    if require_owner(ctx, "Backend switch").await? {
        return Ok(true);
    }

    let config = hq_core::config::HqConfig::load().unwrap_or_default();

    if is_bare {
        // List the chain with per-backend capabilities so the tradeoff of each
        // choice is visible (buffered CLI vs full API).
        let mut lines =
            vec!["**Backend chain** (primary drives session capabilities):".to_string()];
        for info in crate::session_info::chain_listing(&config) {
            let marker = if info.is_primary { "◆" } else { "◦" };
            let caps = match info.kind.as_str() {
                "GithubCopilotCli" => "buffered CLI — no streaming/tools",
                _ => "full API — streaming + tools",
            };
            let model = info.model.unwrap_or_else(|| "(default)".to_string());
            lines.push(format!("{marker} `{}` — {model} · {caps}", info.name));
        }
        lines.push(String::new());
        lines.push("`/backend <name>` to switch primary. Takes effect next turn.".to_string());
        ctx.bot
            .send_message(ctx.msg.chat.id, lines.join("\n"))
            .await?;
        return Ok(true);
    }

    let name = ctx
        .text_clean
        .split_whitespace()
        .nth(1)
        .unwrap_or("")
        .trim();
    // Convenience aliases.
    let name = match name {
        "kimi" => "kimi-k3-256k",
        other => other,
    };
    if name.is_empty() {
        ctx.bot
            .send_message(
                ctx.msg.chat.id,
                "Usage: `/backend <name>` (or bare `/backend` to list)",
            )
            .await?;
        return Ok(true);
    }
    if name == config.backends.primary {
        ctx.bot
            .send_message(
                ctx.msg.chat.id,
                format!("`{name}` is already the primary backend."),
            )
            .await?;
        return Ok(true);
    }

    match crate::session_info::set_primary_backend(name) {
        Ok((model, realigned)) => {
            let model_line = model
                .map(|m| format!("\n◆ Model: `{m}`"))
                .unwrap_or_default();
            let sync_line = if realigned {
                "\n◆ default_model realigned — no drift"
            } else {
                ""
            };
            ctx.bot
                .send_message(
                    ctx.msg.chat.id,
                    format!(
                        "✅ Primary backend switched to `{name}`.{model_line}{sync_line}\n\nTakes effect on your next message. `/backend` shows the new chain."
                    ),
                )
                .await?;
        }
        Err(e) => {
            ctx.bot
                .send_message(ctx.msg.chat.id, format!("Failed to switch backend: {e}"))
                .await?;
        }
    }
    Ok(true)
}

/// Returns true if the message was a /help command.
pub async fn handle_help(ctx: &CommandContext<'_>) -> anyhow::Result<bool> {
    use teloxide::prelude::Requester;
    if ctx.text_lower == "/help" || ctx.text_lower == "!help" {
        let help = [
            "HQ Bot Commands",
            "",
            "/new — Start a fresh conversation (alias of /reset)",
            "/reset — Clear history",
            "/focus [topic] — Pivot focus: clear history, keep model and permission pin",
            "/model [name] — List the backend chain or switch its primary (global)",
            "/backend [name] — Show backend chain / switch primary",
            "/status — Show model and session info",
            "/resume <id> — Rerun an interrupted background turn (bare /resume lists them)",
            "/watch <minutes> [for <hours>h] <prompt> — Recurring check that keeps you posted",
            "/unwatch <id> — Stop a running watch",
            "/help — Show this help",
        ]
        .join("\n");
        ctx.bot.send_message(ctx.msg.chat.id, help).await?;
        return Ok(true);
    }
    Ok(false)
}

/// Returns true if the message was a /status command.
pub async fn handle_status(ctx: &CommandContext<'_>) -> anyhow::Result<bool> {
    use teloxide::prelude::Requester;
    if ctx.text_lower == "/status" || ctx.text_lower == "!status" {
        let status =
            crate::chat_commands::status_fields(&ctx.threads, &ctx.chat_key).await;
        ctx.bot.send_message(ctx.msg.chat.id, status.text()).await?;
        return Ok(true);
    }
    Ok(false)
}

/// `/cancel`: stop the running turn and free the chat. Always consumed.
pub async fn handle_cancel(ctx: &CommandContext<'_>) -> anyhow::Result<bool> {
    use teloxide::prelude::Requester;
    if ctx.text_lower != "/cancel" && ctx.text_lower != "!cancel" {
        return Ok(false);
    }
    let reply = crate::chat_commands::cancel(&ctx.threads, ctx.chat_key).await;
    ctx.bot.send_message(ctx.msg.chat.id, reply).await?;
    Ok(true)
}

/// `/permission [preset]`: owner-only pin of this chat's permission preset.
pub async fn handle_permission_pin(ctx: &CommandContext<'_>) -> anyhow::Result<bool> {
    use teloxide::prelude::Requester;
    if !crate::chat_commands::is_permission_command(ctx.text_lower) {
        return Ok(false);
    }
    if require_owner(ctx, "Permission preset pinning").await? {
        return Ok(true);
    }
    let arg = crate::chat_commands::permission_arg(ctx.text_clean);
    let reply =
        crate::chat_commands::permission(&ctx.threads, ctx.chat_key, arg, "/permission").await;
    ctx.bot.send_message(ctx.msg.chat.id, reply).await?;
    Ok(true)
}
