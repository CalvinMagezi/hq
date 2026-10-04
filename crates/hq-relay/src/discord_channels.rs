//! Discord channel category resolution: maps notification categories
//! (task-completions, daily-briefings, self-improvement, ...) to real
//! Discord channel IDs by name, and tracks which mailbox `from` producer
//! owns which category for the single-consumer mailbox split.

use std::collections::HashMap;
use std::path::Path;

/// Built-in category names this plan resolves against the guild's channel
/// list. `github-activity` is resolved (harmless,
/// future-proof) but have no producer wired yet — see the plan's Global
/// Constraints. An operator's own project/personal categories come from
/// `RelayConfig::discord_notification_categories` instead of being hardcoded
/// here — see `resolve_and_persist_channels`.
pub const DISCORD_CATEGORY_NAMES: &[&str] = &[
    "task-completions",
    "daily-briefings",
    "self-improvement",
    "github-activity",
    "agent-status",
    "claude-code-updates",
    "fyi",
];

/// Producer of email FYIs, routed to the `fyi` channel only when it exists (FR-026).
const EMAIL_FYI_PRODUCER: &str = "email-fyi";
const FYI_CATEGORY: &str = "fyi";

/// How long Telegram leaves a Discord-owned email FYI for Discord's 45s poller
/// before delivering it itself.
/// ponytail: a stale channel map or a Discord relay with no active channel
/// delays FYIs by this window; a Discord liveness heartbeat would remove it.
const EMAIL_FYI_DISCORD_GRACE_MINUTES: i64 = 10;

const ROUTING_FILE: &str = "_system/DISCORD-ROUTING.md";

/// Which mailbox `from` producer owns which Discord category, for the
/// single-consumer split in the mailbox pollers. Reads a vault-editable map
/// (`producer: category` lines, `#`-prefixed comments ignored) so new
/// producers can be routed without a rebuild. Falls back to the one verified
/// built-in default (`agent-worker` -> `task-completions`) when the routing
/// file is missing or doesn't list a given producer. Returns `None` for
/// anything unmapped: those messages stay on Telegram (value-bus and
/// email-triage are intentionally never migrated here).
pub fn owning_category(vault_path: &Path, from: &str) -> Option<String> {
    if from == EMAIL_FYI_PRODUCER {
        return hq_core::discord_notify::resolve_discord_channel(vault_path, FYI_CATEGORY)
            .map(|_| FYI_CATEGORY.to_string());
    }
    if let Some(category) = read_routing_file(vault_path, from) {
        return Some(category);
    }
    if from == "agent-worker" {
        return Some("task-completions".to_string());
    }
    None
}

/// Whether Telegram's poller should hand `msg` back to the mailbox for
/// Discord. An email FYI Discord has not taken within the grace window goes
/// to the Telegram digest instead, so a dead Discord relay cannot strand it.
pub fn telegram_defers_to_discord(
    vault_path: &Path,
    msg: &hq_core::types::MailboxMessage,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    if owning_category(vault_path, &msg.from).is_none() {
        return false;
    }
    msg.from != EMAIL_FYI_PRODUCER
        || now - msg.timestamp < chrono::Duration::minutes(EMAIL_FYI_DISCORD_GRACE_MINUTES)
}

fn read_routing_file(vault_path: &Path, from: &str) -> Option<String> {
    let content = std::fs::read_to_string(vault_path.join(ROUTING_FILE)).ok()?;
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((producer, category)) = line.split_once(':') else {
            continue;
        };
        if producer.trim() == from {
            return Some(category.trim().to_string());
        }
    }
    None
}

/// Resolve every known category against the guild's channel list by name and
/// persist the result. Called once from the Discord `ready()` handler. Logs and
/// continues on a per-category or API failure rather than blocking startup.
pub async fn resolve_and_persist_channels(
    http: &serenity::http::Http,
    guild_id: serenity::model::id::GuildId,
    vault_path: &Path,
    extra_categories: &[String],
) {
    use serenity::model::channel::ChannelType;

    let channels = match guild_id.channels(http).await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "discord_channels: failed to list guild channels");
            return;
        }
    };

    let by_name: HashMap<String, u64> = channels
        .into_iter()
        .filter(|(_, ch)| ch.kind == ChannelType::Text)
        .map(|(id, ch)| (ch.name, id.get()))
        .collect();

    let all_categories: Vec<&str> = DISCORD_CATEGORY_NAMES
        .iter()
        .copied()
        .chain(extra_categories.iter().map(String::as_str))
        .collect();

    let mut lines = Vec::new();
    for category in &all_categories {
        match by_name.get(*category) {
            Some(id) => lines.push(format!("{category}: {id}")),
            None => tracing::info!(category, "discord_channels: no matching channel in guild"),
        }
    }

    let sys_dir = vault_path.join("_system");
    if let Err(e) = std::fs::create_dir_all(&sys_dir) {
        tracing::warn!(error = %e, "discord_channels: failed to create _system dir");
        return;
    }
    if let Err(e) = std::fs::write(sys_dir.join("DISCORD-CHANNELS.md"), lines.join("\n") + "\n") {
        tracing::warn!(error = %e, "discord_channels: failed to persist channel map");
    } else {
        tracing::info!(
            resolved = lines.len(),
            total = all_categories.len(),
            "discord_channels: resolved category channels"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owning_category_maps_agent_worker_to_task_completions_by_default() {
        let dir = tempfile::tempdir().unwrap();
        // No routing file at all — the one built-in default still applies.
        assert_eq!(
            owning_category(dir.path(), "agent-worker"),
            Some("task-completions".to_string())
        );
    }

    #[test]
    fn owning_category_reads_from_routing_file() {
        let dir = tempfile::tempdir().unwrap();
        let sys_dir = dir.path().join("_system");
        std::fs::create_dir_all(&sys_dir).unwrap();
        std::fs::write(
            sys_dir.join("DISCORD-ROUTING.md"),
            "# comment line, ignored\nscheduled-prompts: daily-briefings\n",
        )
        .unwrap();
        assert_eq!(
            owning_category(dir.path(), "scheduled-prompts"),
            Some("daily-briefings".to_string())
        );
    }

    #[test]
    fn owning_category_returns_none_for_unmapped_producers() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(owning_category(dir.path(), "value-bus"), None);
        assert_eq!(owning_category(dir.path(), "email-triage"), None);
        assert_eq!(owning_category(dir.path(), "curiosity-engine"), None);
    }

    #[test]
    fn owning_category_routing_file_overrides_builtin_default() {
        let dir = tempfile::tempdir().unwrap();
        let sys_dir = dir.path().join("_system");
        std::fs::create_dir_all(&sys_dir).unwrap();
        std::fs::write(
            sys_dir.join("DISCORD-ROUTING.md"),
            "agent-worker: agent-status\n",
        )
        .unwrap();
        assert_eq!(
            owning_category(dir.path(), "agent-worker"),
            Some("agent-status".to_string())
        );
    }

    #[test]
    fn category_names_include_all_builtin_channels() {
        let expected = [
            "task-completions",
            "daily-briefings",
            "self-improvement",
            "github-activity",
            "agent-status",
            "claude-code-updates",
            "fyi",
        ];
        for name in expected {
            assert!(
                DISCORD_CATEGORY_NAMES.contains(&name),
                "missing category: {name}"
            );
        }
        assert_eq!(DISCORD_CATEGORY_NAMES.len(), expected.len());
    }

    fn email_fyi_at(
        minutes_ago: i64,
    ) -> (
        hq_core::types::MailboxMessage,
        chrono::DateTime<chrono::Utc>,
    ) {
        let mut msg = hq_core::mailbox::new_message(
            EMAIL_FYI_PRODUCER,
            "relay",
            hq_core::types::MailboxMessageType::Direct,
            Some("Email FYI"),
            "FYI",
            None,
        );
        let now = chrono::Utc::now();
        msg.timestamp = now - chrono::Duration::minutes(minutes_ago);
        (msg, now)
    }

    fn with_fyi_channel() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("_system")).unwrap();
        std::fs::write(dir.path().join("_system/DISCORD-CHANNELS.md"), "fyi: 123\n").unwrap();
        dir
    }

    #[test]
    fn email_fyi_stays_on_telegram_without_a_fyi_channel() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("_system")).unwrap();
        std::fs::write(
            dir.path().join("_system/DISCORD-ROUTING.md"),
            "email-fyi: fyi\n",
        )
        .unwrap();
        assert_eq!(owning_category(dir.path(), EMAIL_FYI_PRODUCER), None);
        let (msg, now) = email_fyi_at(0);
        assert!(!telegram_defers_to_discord(dir.path(), &msg, now));
    }

    #[test]
    fn email_fyi_goes_to_discord_when_the_fyi_channel_resolves() {
        let dir = with_fyi_channel();
        assert_eq!(
            owning_category(dir.path(), EMAIL_FYI_PRODUCER),
            Some("fyi".to_string())
        );
        let (fresh, now) = email_fyi_at(1);
        assert!(telegram_defers_to_discord(dir.path(), &fresh, now));
    }

    #[test]
    fn telegram_delivers_an_email_fyi_discord_never_took() {
        let dir = with_fyi_channel();
        let (stale, now) = email_fyi_at(EMAIL_FYI_DISCORD_GRACE_MINUTES + 1);
        assert!(!telegram_defers_to_discord(dir.path(), &stale, now));
    }
}
