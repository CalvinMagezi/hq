//! Discord channel category resolution: maps category names (task-completions,
//! daily-briefings, ...) to the guild's channel IDs at startup. Nothing posts
//! notifications to them any more: Discord carries chat only.

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

/// Resolves the category channels by name and records them in the vault.
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
}
