//! Discord "family guest" scoping: one channel, real per-message threads,
//! full tool access with two specific hard gates (see remote_mcp.rs and
//! harness_session/tools.rs).

use std::collections::HashSet;
use std::path::Path;

use serenity::model::channel::Message;
use serenity::prelude::Context as Ctx;

use super::Handler;
use hq_core::config::DiscordFamilyUser;
use hq_core::privacy::{DisclosureScope, IdentityClaim, PersonRegistry, resolve};

/// Determine whether a user in a given channel and guild is authorized as a family guest.
/// Pure decision function for testability without Serenity structs.
pub(super) fn is_family_authorized(
    user_id: u64,
    channel_id: u64,
    guild_id: Option<u64>,
    family_channel_id: Option<u64>,
    family_users: &[DiscordFamilyUser],
    family_threads: &HashSet<u64>,
) -> bool {
    if guild_id.is_none() {
        return false; // never via DM
    }
    if !family_users.iter().any(|u| u.user_id == user_id) {
        return false;
    }
    let Some(family_channel) = family_channel_id else {
        return false;
    };
    channel_id == family_channel || family_threads.contains(&channel_id)
}

/// Path to the persisted family thread ids file.
pub(super) fn family_threads_path(vault_path: &Path) -> std::path::PathBuf {
    vault_path
        .join("_system")
        .join("discord-family-threads.txt")
}

/// Load previously created family thread ids from disk.
pub(super) fn load_family_threads(vault_path: &Path) -> HashSet<u64> {
    let path = family_threads_path(vault_path);
    let mut set = HashSet::new();
    if let Ok(content) = std::fs::read_to_string(&path) {
        for line in content.lines() {
            if let Ok(id) = line.trim().parse::<u64>() {
                set.insert(id);
            }
        }
    }
    set
}

/// Append a newly created family thread id to disk.
pub(super) fn record_family_thread(vault_path: &Path, thread_id: u64) {
    let path = family_threads_path(vault_path);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    use std::io::Write;
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = writeln!(file, "{thread_id}");
    }
}

impl Handler {
    /// `Some(name)` if `user_id` is a configured family member; the message
    /// must still be channel/thread-scoped separately (see `family_authorized`).
    pub(super) fn family_user_name(&self, user_id: u64) -> Option<&str> {
        self.discord_family_users
            .iter()
            .find(|u| u.user_id == user_id)
            .map(|u| u.name.as_str())
    }

    /// True when `msg` is from a configured family member AND posted in the
    /// family channel itself or a thread HQ created under it. Never true for
    /// a DM or a mention in any other channel — that's the whole point of
    /// the scoping. Does not require @mention inside the family channel.
    pub(super) async fn family_authorized(&self, msg: &Message) -> bool {
        let threads = self.family_threads.lock().await;
        is_family_authorized(
            msg.author.id.get(),
            msg.channel_id.get(),
            msg.guild_id.map(|g| g.get()),
            self.discord_family_channel_id,
            &self.discord_family_users,
            &threads,
        )
    }

    /// True for any message (guest or the owner) posted directly in the family
    /// channel, i.e. one that should spawn a new thread rather than reuse one.
    pub(super) fn is_family_channel_toplevel(&self, channel_id: u64) -> bool {
        self.discord_family_channel_id == Some(channel_id)
    }

    /// True if `channel_id` is either the family channel itself or a thread HQ created under it.
    pub(super) async fn is_family_channel_or_thread(&self, channel_id: u64) -> bool {
        let Some(family_channel) = self.discord_family_channel_id else {
            return false;
        };
        channel_id == family_channel || self.family_threads.lock().await.contains(&channel_id)
    }

    /// Who this Discord author is and what they may see. An author who is not a
    /// family guest reached this point through the owner gate (allowlist or
    /// paired owner), which is why they count as an owner binding.
    pub(super) fn disclosure_scope(&self, author_id: u64, is_guest: bool) -> DisclosureScope {
        let mut owners = self.discord_allowed_user_ids.clone();
        if !is_guest {
            owners.push(author_id);
        }
        let registry = PersonRegistry::from_discord(&owners, &self.discord_family_users);
        let id = author_id.to_string();
        DisclosureScope::for_person(&resolve(&registry, &IdentityClaim::discord(&id)))
    }

    /// Resolved display name for the bot owner, falling back to "the owner".
    pub(super) fn owner_name(&self) -> String {
        hq_core::config::HqConfig::load()
            .map(|c| c.relay.owner_name())
            .unwrap_or_else(|_| "the owner".to_string())
    }

    /// Create a Discord thread from `msg`, record it (in-memory + persisted
    /// file), and return its id as the `channel_key` the rest of the turn
    /// should use.
    pub(super) async fn spawn_family_thread(&self, ctx: &Ctx, msg: &Message) -> Option<u64> {
        let user_name = self
            .family_user_name(msg.author.id.get())
            .unwrap_or(&msg.author.name);
        let first_line = msg.content.lines().next().unwrap_or("Chat").trim();
        let preview: String = first_line.chars().take(60).collect();
        let mut title = if preview.is_empty() {
            format!("{user_name}'s conversation")
        } else {
            format!("{user_name}: {preview}")
        };
        if title.chars().count() > 100 {
            title = title.chars().take(99).collect();
        } else if title.chars().count() < 2 {
            title = format!("thread-{}", msg.id.get());
        }

        let builder = serenity::builder::CreateThread::new(title);
        match msg
            .channel_id
            .create_thread_from_message(&ctx.http, msg.id, builder)
            .await
        {
            Ok(thread) => {
                let thread_id = thread.id.get();
                self.family_threads.lock().await.insert(thread_id);
                record_family_thread(&self.vault_path, thread_id);
                Some(thread_id)
            }
            Err(e) => {
                tracing::warn!(error = %e, "discord: failed to create family thread from message");
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn family_auth_matrix() {
        let family_users = vec![
            DiscordFamilyUser {
                user_id: 101,
                name: "Bob".to_string(),
            },
            DiscordFamilyUser {
                user_id: 102,
                name: "Carol".to_string(),
            },
        ];
        let family_channel_id = Some(100200300400500600);
        let mut threads = HashSet::new();
        threads.insert(999001);

        let guild_id = Some(55555);

        // 1. Family user in family channel is allowed
        assert!(is_family_authorized(
            101,
            100200300400500600,
            guild_id,
            family_channel_id,
            &family_users,
            &threads
        ));
        assert!(is_family_authorized(
            102,
            100200300400500600,
            guild_id,
            family_channel_id,
            &family_users,
            &threads
        ));

        // 2. Family user in tracked thread under family channel is allowed
        assert!(is_family_authorized(
            101,
            999001,
            guild_id,
            family_channel_id,
            &family_users,
            &threads
        ));

        // 3. Family user via DM is rejected
        assert!(!is_family_authorized(
            101,
            100200300400500600,
            None,
            family_channel_id,
            &family_users,
            &threads
        ));
        assert!(!is_family_authorized(
            101,
            999001,
            None,
            family_channel_id,
            &family_users,
            &threads
        ));

        // 4. Family user in another channel is rejected
        assert!(!is_family_authorized(
            101,
            888888,
            guild_id,
            family_channel_id,
            &family_users,
            &threads
        ));

        // 5. Non-family user is not authorized as family
        assert!(!is_family_authorized(
            777,
            100200300400500600,
            guild_id,
            family_channel_id,
            &family_users,
            &threads
        ));

        // 6. When family_channel_id is None, no family auth
        assert!(!is_family_authorized(
            101,
            100200300400500600,
            guild_id,
            None,
            &family_users,
            &threads
        ));
    }

    #[test]
    fn family_threads_persistence_round_trip() {
        let temp_dir = tempfile::tempdir().unwrap();
        let vault_path = temp_dir.path();

        let initial = load_family_threads(vault_path);
        assert!(initial.is_empty());

        record_family_thread(vault_path, 123456789);
        record_family_thread(vault_path, 987654321);

        let loaded = load_family_threads(vault_path);
        assert_eq!(loaded.len(), 2);
        assert!(loaded.contains(&123456789));
        assert!(loaded.contains(&987654321));
    }
}
