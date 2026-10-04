//! Discord relay: serenity bot with slash commands and native HQ dispatch.

use anyhow::{Context, Result};
use hq_db::Database;
use hq_vault::VaultClient;
use serenity::async_trait;
use serenity::builder::{
    CreateCommand, CreateCommandOption, CreateEmbed, CreateMessage, EditMessage,
};
use serenity::model::application::{Command, CommandOptionType, Interaction};
use serenity::model::prelude::*;
use serenity::prelude::*;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex as TokioMutex;
use tracing::info;

use crate::relay_common::*;

mod auth;
mod buttons;
mod dispatch;
mod family;
mod media;
mod message;
mod poller;

use crate::session_runner::strip_loaded_soul;
use auth::*;
pub use buttons::build_status_embed;
use buttons::*;
use dispatch::*;
use family::*;
pub use media::format_attachment_descriptor;
use media::*;
use message::*;
use poller::*;

// ─── Handler ─────────────────────────────────────────────────

struct Handler {
    system_prompt: String,
    threads: Arc<TokioMutex<HashMap<u64, ChannelState>>>,
    skill_index: Arc<hq_tools::skills::SkillHintIndex>,
    vault_path: std::path::PathBuf,
    active_channel_id: Arc<TokioMutex<Option<u64>>>,
    /// Gates both human chat messages and approval-button clicks. Empty means
    /// unrestricted — see `RelayConfig::discord_allowed_user_ids`. Never applied
    /// to bot-authored messages (those are mention-gated separately).
    discord_allowed_user_ids: Vec<u64>,
    /// Extra channel-category names to resolve on top of the built-in ones
    /// — see `RelayConfig::discord_notification_categories`.
    discord_notification_categories: Vec<String>,
    /// Discord channel that family members are scoped to.
    discord_family_channel_id: Option<u64>,
    /// Family members scoped to `discord_family_channel_id`.
    discord_family_users: Vec<hq_core::config::DiscordFamilyUser>,
    /// Discord thread ids HQ created under `discord_family_channel_id`.
    /// Persisted so a restart doesn't orphan an in-flight thread.
    family_threads: Arc<TokioMutex<std::collections::HashSet<u64>>>,
    /// Shared thread-store recorder: mirrors every inbound/outbound message
    /// into `chat_threads`/`chat_messages` and (if wired) broadcasts a live
    /// event so the Web UI's unified inbox can follow along.
    thread_sync: crate::thread_sync::ThreadSync,
}

/// A short human-readable label for a Discord conversation, used as the Web
/// UI thread title. No extra network/cache lookups — just what's already on
/// the message.
fn discord_thread_title(msg: &Message, is_dm: bool) -> String {
    if is_dm {
        format!("Discord DM: {}", msg.author.name)
    } else {
        format!("Discord: #{}", msg.channel_id.get())
    }
}

impl Handler {
    fn make_help_text() -> String {
        [
            "**HQ Bot Commands**",
            "",
            "`!reset` / `!new` — Clear conversation history",
            "`!focus [topic]`: pivot focus, clearing history but keeping the model and permission pin",
            "`!model` — List the backend chain; `!model <name>` switches primary (backend name or model id)",
            "`!status` — Show the current model and message count",
            "`resume <id>` — Rerun an interrupted background turn (bare `resume` lists them)",
            "`/watch <minutes> [for <hours>h] <prompt>` — Recurring check that keeps you posted",
            "`/unwatch <id>` — Stop a running watch",
            "`!help` — Show this help",
            "",
            "**Slash commands:** `/reset`, `/model`, `/status`, `/help`",
        ]
        .join("\n")
    }
}

// ─── Event handler ───────────────────────────────────────────

#[async_trait]
impl EventHandler for Handler {
    async fn message(&self, ctx: serenity::prelude::Context, msg: Message) {
        let bot_id = ctx.cache.current_user().id;
        if try_pair_dm(
            &self.vault_path,
            msg.author.id.get(),
            msg.guild_id.is_none(),
            !msg.author.bot,
            &self.discord_allowed_user_ids,
            &msg.content,
            hq_core::pairing::now_secs(),
        ) {
            let _ = msg.reply(&ctx.http, "Paired. You are now the owner.").await;
            return;
        }
        if !self.authorized(&msg, bot_id).await {
            return;
        }
        let Some(content) = inbound_text(&msg, bot_id) else {
            return;
        };
        let (descriptors, images) = collect_attachments(&msg, &self.vault_path).await;
        let content = if descriptors.is_empty() {
            content
        } else {
            format!("{content}\n\n{}", descriptors.join("\n"))
        };
        let effective_channel_id = if self.is_family_channel_toplevel(msg.channel_id.get()) {
            self.spawn_family_thread(&ctx, &msg)
                .await
                .unwrap_or(msg.channel_id.get())
        } else {
            msg.channel_id.get()
        };
        let effective_channel = ChannelId::new(effective_channel_id);

        let title = discord_thread_title(&msg, msg.guild_id.is_none());
        self.thread_sync.record(
            "discord",
            &effective_channel_id.to_string(),
            &title,
            "user",
            &content,
        );
        if !self.is_family_channel_or_thread(effective_channel_id).await {
            self.record_presence(effective_channel_id).await;
        }

        let lower = content.to_lowercase();
        if self
            .reply_if_busy(&ctx, &msg, effective_channel, &content, &lower)
            .await
        {
            return;
        }
        if self
            .bang_command(&ctx, &msg, effective_channel, &content, &lower)
            .await
        {
            return;
        }
        // A resume or first watch firing replaces the command text for the rest of the turn.
        let content = match self
            .registry_step(&ctx, &msg, effective_channel, &content, &lower)
            .await
        {
            std::ops::ControlFlow::Break(()) => return,
            std::ops::ControlFlow::Continue(prompt) => prompt.unwrap_or(content),
        };
        self.run_turn(&ctx, &msg, effective_channel, content, images, title)
            .await;
    }

    async fn interaction_create(&self, ctx: serenity::prelude::Context, interaction: Interaction) {
        match interaction {
            Interaction::Command(cmd) => self.slash_command(&ctx, cmd).await,
            Interaction::Component(comp) => self.button(&ctx, comp).await,
            _ => {}
        }
    }

    async fn ready(&self, ctx: serenity::prelude::Context, ready: Ready) {
        info!(
            user = %ready.user.name,
            guilds = ready.guilds.len(),
            "discord: connected"
        );

        // FR-006b: the allowlisted id is otherwise just a number — resolve
        // and log its username at startup so the operator can eyeball whether it's
        // actually his own account without extra tooling. Confirming that is
        // still a manual step; this only makes it cheap to do.
        for uid in &self.discord_allowed_user_ids {
            match serenity::model::id::UserId::new(*uid)
                .to_user(&ctx.http)
                .await
            {
                Ok(user) => {
                    info!(user_id = uid, username = %user.name, "discord: authorized owner")
                }
                Err(e) => tracing::warn!(
                    user_id = uid,
                    error = %e,
                    "discord: could not resolve authorized owner id — is it valid?"
                ),
            }
        }
        write_relay_status(
            &self.vault_path,
            "discord",
            "connected",
            Some(&format!(
                "{} guild{}",
                ready.guilds.len(),
                if ready.guilds.len() == 1 { "" } else { "s" }
            )),
            None,
        );

        if let Some(guild) = ready.guilds.first() {
            if ready.guilds.len() > 1 {
                tracing::warn!(
                    count = ready.guilds.len(),
                    "discord: bot is in multiple guilds; category channel resolution only checks the first"
                );
            }
            crate::discord_channels::resolve_and_persist_channels(
                &ctx.http,
                guild.id,
                &self.vault_path,
                &self.discord_notification_categories,
            )
            .await;
        }

        let commands = vec![
            CreateCommand::new("reset").description("Clear conversation history"),
            CreateCommand::new("model")
                .description("Show or switch the backend/model chain (bare lists choices)")
                .add_option(CreateCommandOption::new(
                    CommandOptionType::String,
                    "name",
                    "Backend name or model id — omit to list available choices",
                )),
            CreateCommand::new("status").description("Show the current model and message count"),
            CreateCommand::new("help").description("Show available commands"),
        ];

        match Command::set_global_commands(&ctx.http, commands).await {
            Ok(cmds) => info!(count = cmds.len(), "discord: registered slash commands"),
            Err(e) => tracing::error!("discord: failed to register slash commands: {e}"),
        }
    }
}

// ─── Public entry point ──────────────────────────────────────

// Relay wiring takes each shared handle separately; a params struct is tracked in TECHDEBT.md.
#[allow(clippy::too_many_arguments)]
pub async fn run_discord_relay(
    token: &str,
    vault: Arc<VaultClient>,
    db: Arc<Database>,
    discord_allowed_user_ids: Vec<u64>,
    discord_notification_categories: Vec<String>,
    discord_family_channel_id: Option<u64>,
    discord_family_users: Vec<hq_core::config::DiscordFamilyUser>,
    thread_events: Option<tokio::sync::broadcast::Sender<String>>,
) -> Result<()> {
    let system_prompt = load_system_prompt_with_env(&vault);
    info!(
        prompt_len = system_prompt.len(),
        "discord: loaded system prompt"
    );

    let skills_dir = hq_core::skills_dir(vault.vault_path());
    let skill_index = Arc::new(hq_tools::skills::SkillHintIndex::build(&skills_dir));

    let intents = GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::DIRECT_MESSAGES
        | GatewayIntents::MESSAGE_CONTENT;

    // Restore persisted channel states from disk
    let restored = ChannelState::restore_all::<u64>(vault.vault_path(), "dc-");
    if !restored.is_empty() {
        info!(count = restored.len(), "discord: restored channel states");
    }

    let restored_family_threads = load_family_threads(vault.vault_path());

    // Seed active channel from persisted state so the poller works immediately
    // on restart. Never seed from the family channel or a family thread —
    // The owner's own proactive notifications must never default into
    // #la-familia before he sends his own first message.
    let seed_channel: Option<u64> = restored
        .keys()
        .find(|id| {
            discord_family_channel_id != Some(**id) && !restored_family_threads.contains(*id)
        })
        .copied();
    let family_threads = Arc::new(TokioMutex::new(restored_family_threads));
    let active_channel_id: Arc<TokioMutex<Option<u64>>> = Arc::new(TokioMutex::new(seed_channel));
    let active_channel_id_for_poller = active_channel_id.clone();

    let threads: Arc<TokioMutex<HashMap<u64, ChannelState>>> = Arc::new(TokioMutex::new(restored));

    if discord_allowed_user_ids.is_empty() && !discord_auth_owner_path(vault.vault_path()).exists() {
        tracing::warn!(
            "discord: no owner configured, so every message is refused. Set \
             relay.discord_allowed_user_ids, or run `hq pair --platform discord` and DM the \
             bot `!pair <code>`."
        );
    }

    let handler = Handler {
        system_prompt: system_prompt.clone(),
        threads: Arc::clone(&threads),
        vault_path: vault.vault_path().to_path_buf(),
        skill_index,
        active_channel_id,
        discord_allowed_user_ids,
        discord_notification_categories,
        discord_family_channel_id,
        discord_family_users,
        family_threads,
        thread_sync: crate::thread_sync::ThreadSync::new(db.clone(), thread_events.clone()),
    };

    let mut client = Client::builder(token, intents)
        .event_handler(handler)
        .await
        .context("failed to create Discord client")?;

    // Share the client's Http so rate-limit state is not split across two instances.
    let vault_arc = Arc::new(vault.vault_path().to_path_buf());
    let poller_http = Arc::clone(&client.http);
    let _ = std::fs::create_dir_all(vault.vault_path().join("_mailboxes/relay"));

    // Watch scheduler: re-dispatches due recurring watch turns (kind='watch')
    // through the shared native-hq path so each firing detaches and delivers
    // like a normal turn. Restart-safe by construction: due watches fire on
    // the first poll after startup.
    {
        let sched_db = db.clone();
        let sender_http = Arc::clone(&client.http);
        let surface = crate::watch_scheduler::WatchSurface {
            platform: "discord",
            threads: Arc::clone(&threads),
            system_prompt: Arc::new(system_prompt.clone()),
            progress_chars: DC_PROGRESS_CHARS,
            identity: hq_core::identity::RequestIdentity::from_discord,
            sender: Arc::new(move |channel: u64| {
                dc_sender(sender_http.clone(), ChannelId::new(channel))
            }),
        };
        tokio::spawn(crate::subagent_followup::run_subagent_followups(
            surface.clone(),
            sched_db.clone(),
        ));
        let (dispatch, notify) = crate::watch_scheduler::watch_callbacks(surface);
        tokio::spawn(crate::watch_scheduler::run_watch_scheduler(
            sched_db,
            "discord",
            std::time::Duration::from_secs(crate::watch_scheduler::DEFAULT_POLL_SECS),
            dispatch,
            notify,
        ));
        info!("discord: watch scheduler spawned");
    }

    // Wrap poller in a restart loop — tokio::spawn drops the task on panic without restarting.
    tokio::spawn(async move {
        loop {
            run_discord_mailbox_poller(
                vault_arc.clone(),
                poller_http.clone(),
                active_channel_id_for_poller.clone(),
            )
            .await;
            tracing::warn!("discord-mailbox-poller: exited unexpectedly, restarting in 5s");
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    });

    info!("discord: connecting to gateway...");
    let result = client.start().await.context("Discord client error");
    if let Err(ref e) = result {
        write_relay_status(
            vault.vault_path(),
            "discord",
            "error",
            None,
            Some(&e.to_string()),
        );
    }
    result
}
