//! Stateless Discord/Telegram-adjacent notification helpers used by daemon
//! tasks that cannot depend on `hq-relay` (which itself depends on
//! `hq-daemon`). Mirrors the rationale of `crate::mailbox`: lives here so
//! `hq-daemon`, `hq-relay`, and `hq-web` can all reach it without a cycle.

use std::path::Path;
use std::time::Duration;

const CHANNELS_FILE: &str = "_system/DISCORD-CHANNELS.md";
const PRESENCE_FILE: &str = "_system/DISCORD-PRESENCE.md";
const SEND_TIMEOUT: Duration = Duration::from_secs(10);

/// Truncates to at most `max_bytes`, backing off to the nearest earlier char
/// boundary rather than slicing mid-character. Plain byte-index slicing
/// (`&s[..n]`) panics whenever a multi-byte UTF-8 character straddles that
/// index — a real risk here since these bodies are API error text that can
/// contain a non-ASCII (e.g. localized) description.
fn truncate_utf8(s: &str, max_bytes: usize) -> &str {
    &s[..s.floor_char_boundary(max_bytes)]
}

fn parse_channels_file(vault_path: &Path, category: &str) -> Option<u64> {
    let content = std::fs::read_to_string(vault_path.join(CHANNELS_FILE)).ok()?;
    for line in content.lines() {
        if let Some((name, id)) = line.split_once(':')
            && name.trim() == category
        {
            return id.trim().parse().ok();
        }
    }
    None
}

/// Resolve a notification category (e.g. "task-completions") to a Discord
/// channel ID, from the map the Discord relay persists at startup.
pub fn resolve_discord_channel(vault_path: &Path, category: &str) -> Option<u64> {
    parse_channels_file(vault_path, category)
}

/// Read the Discord channel_id from DISCORD-PRESENCE.md (last-active channel,
/// used as a fallback when no category channel is resolved).
pub fn read_discord_presence_channel(vault_path: &Path) -> Option<u64> {
    let content = std::fs::read_to_string(vault_path.join(PRESENCE_FILE)).ok()?;
    for line in content.lines() {
        if let Some(val) = line.strip_prefix("channel_id:") {
            return val.trim().parse().ok();
        }
    }
    None
}

/// Send a plain text message to a Discord channel via REST API.
pub async fn send_discord_message(token: &str, channel_id: u64, text: &str) -> anyhow::Result<()> {
    let client = reqwest::Client::builder().timeout(SEND_TIMEOUT).build()?;
    let resp = client
        .post(format!(
            "https://discord.com/api/v10/channels/{channel_id}/messages"
        ))
        .header("Authorization", format!("Bot {token}"))
        .json(&serde_json::json!({ "content": text }))
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Discord API error {status}: {}", truncate_utf8(&body, 200));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_utf8_never_panics_on_a_multibyte_boundary() {
        // A run of 3-byte '€' characters puts byte offset 200 squarely
        // inside a character, not on a boundary.
        let body = "€".repeat(100);
        let truncated = truncate_utf8(&body, 200);
        assert!(truncated.len() <= 200);
        assert!(body.starts_with(truncated));
    }

    #[test]
    fn truncate_utf8_is_a_no_op_under_the_limit() {
        assert_eq!(truncate_utf8("short", 200), "short");
    }

    #[test]
    fn resolve_discord_channel_reads_known_category() {
        let dir = tempfile::tempdir().unwrap();
        let sys = dir.path().join("_system");
        std::fs::create_dir_all(&sys).unwrap();
        std::fs::write(
            sys.join("DISCORD-CHANNELS.md"),
            "task-completions: 111111111111111111\ndaily-briefings: 222222222222222222\n",
        )
        .unwrap();

        assert_eq!(
            resolve_discord_channel(dir.path(), "task-completions"),
            Some(111111111111111111)
        );
        assert_eq!(
            resolve_discord_channel(dir.path(), "daily-briefings"),
            Some(222222222222222222)
        );
        assert_eq!(resolve_discord_channel(dir.path(), "github-activity"), None);
    }

    #[test]
    fn resolve_discord_channel_missing_file_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            resolve_discord_channel(dir.path(), "task-completions"),
            None
        );
    }

    #[test]
    fn read_discord_presence_channel_reads_channel_id_line() {
        let dir = tempfile::tempdir().unwrap();
        let sys = dir.path().join("_system");
        std::fs::create_dir_all(&sys).unwrap();
        std::fs::write(
            sys.join("DISCORD-PRESENCE.md"),
            "platform: discord\nchannel_id: 333333333333333333\nlast_active: 2026-07-02T00:00:00Z",
        )
        .unwrap();
        assert_eq!(
            read_discord_presence_channel(dir.path()),
            Some(333333333333333333)
        );
    }
}
