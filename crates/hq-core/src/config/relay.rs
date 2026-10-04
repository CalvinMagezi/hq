use serde::{Deserialize, Serialize};

/// Role for a Telegram user in the relay allowlist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TelegramUserRole {
    #[default]
    Guest,
    Owner,
}

/// Which surface HQ proactively reaches out on (digest composer + urgent lane).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum PrimarySurface {
    #[default]
    Telegram,
    Discord,
}

/// Named Telegram participant with display name and role.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramUserEntry {
    pub chat_id: i64,
    pub name: String,
    #[serde(default)]
    pub role: TelegramUserRole,
}

/// A named family member scoped to one Discord channel
/// (`discord_family_channel_id`) rather than the bot-wide
/// `discord_allowed_user_ids`. Full HQ tool access, but identity-tagged so
/// the model knows who it's talking to and the guest-specific gates
/// (remote-MCP confirm, agy-only Herdr) can key off it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscordFamilyUser {
    pub user_id: u64,
    pub name: String,
}

impl RelayConfig {
    /// Display name of the Telegram owner, or "the owner" when none is configured.
    pub fn owner_name(&self) -> String {
        self.telegram_users
            .iter()
            .find(|u| u.role == TelegramUserRole::Owner)
            .map(|u| u.name.clone())
            .unwrap_or_else(|| "the owner".to_string())
    }
}

impl std::fmt::Debug for RelayConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        crate::redact::fmt_redacted(self, "RelayConfig", f)
    }
}

fn default_discord_family_allowed_harnesses() -> Vec<String> {
    vec!["agy".to_string()]
}

#[derive(Clone, Serialize, Deserialize)]
pub struct RelayConfig {
    pub discord_token: Option<String>,
    pub telegram_token: Option<String>,
    /// Separate bot token for outbound notifications (proactive alerts, outreach).
    /// Falls back to `telegram_token` if not set.
    pub notifications_token: Option<String>,
    pub discord_enabled: bool,
    pub telegram_enabled: bool,
    /// Override the LLM model used by the interactive relay.
    /// Defaults to `default_model` if not set.
    /// Recommendation: use a model with native tool-calling (e.g. granite4.1:8b),
    /// NOT Gemma — Gemma uses a text shim that bloats the system prompt and
    /// overrides persona instructions with its base training.
    pub model: Option<String>,
    /// Authorized Telegram chat ID. Only messages from this chat are processed.
    /// When unset, the relay refuses everyone until an owner exists: set this,
    /// list an owner in `telegram_users`, or run `hq pair` and send `/pair <code>`.
    pub telegram_authorized_chat_id: Option<i64>,
    /// Explicit allowlist of additional Telegram chat IDs that may interact with HQ.
    /// Any ID in this list is accepted alongside `telegram_authorized_chat_id`.
    /// Use this to add co-owners without changing the primary owner.
    #[serde(default)]
    pub telegram_allowed_chat_ids: Vec<i64>,
    /// Named Telegram users (owner + guests). Preferred over bare allowlist IDs.
    #[serde(default)]
    pub telegram_users: Vec<TelegramUserEntry>,
    /// Surface HQ proactively reaches out on (digest + urgent lane). Default Telegram.
    #[serde(default)]
    pub primary_surface: PrimarySurface,

    /// Seconds a relay turn may run before it detaches into a background
    /// task and the chat receives an async result. Default 270 (legacy behavior).
    #[serde(default = "default_turn_ack_timeout_secs")]
    pub turn_ack_timeout_secs: u64,

    /// Maximum days a detached background turn may run before the supervisor
    /// fails it. Default 5.
    #[serde(default = "default_background_turn_max_days")]
    pub background_turn_max_days: u64,

    /// Seconds between progress heartbeats for a detached background turn.
    /// `None` = unset; consumers apply the effective default of 300 seconds.
    #[serde(default)]
    pub background_progress_secs: Option<u64>,

    /// Run a harness session's final pane output through the LLM before
    /// delivering it, so the operator gets a readable outcome instead of a
    /// terminal dump. Default true. Any failure falls back to the raw excerpt.
    #[serde(default = "default_summarize_session_exits")]
    pub summarize_session_exits: bool,

    /// Seconds a single session-exit summary may run before the supervisor
    /// gives up and delivers the raw excerpt. Default 90. The supervisor also
    /// budgets the whole sweep against the daemon's task timeout, so the
    /// effective ceiling is whichever of the two is smaller.
    #[serde(default = "default_session_exit_summary_timeout_secs")]
    pub session_exit_summary_timeout_secs: u64,

    /// Discord user IDs allowed to send the bot regular chat messages, act on
    /// approval buttons (email-triage Send/Skip, value-bus Approve/Dismiss), and click component interactions
    /// generally. When empty (the default), nobody is authorized until an owner is
    /// paired: run `hq pair --platform discord` and DM the bot `!pair <code>`
    /// (the owner is recorded in `_system/.discord-auth-user`). Set this
    /// explicitly to lock the bot down to specific Discord user IDs up
    /// front. Never applied to bot-authored messages from a genuine bot
    /// account (those are mention-gated separately in `hq-relay`), so other
    /// agents in a shared server can still talk to this bot and to each
    /// other freely — a webhook-posted message is not exempt, even though
    /// Discord also marks its author `bot: true`.
    #[serde(default)]
    pub discord_allowed_user_ids: Vec<u64>,

    /// Discord channel that family members (`discord_family_users`) are scoped
    /// to. Messages from a family user are only ever authorized inside this
    /// channel or a thread HQ created under it — never via DM, never via
    /// @-mention elsewhere. `None` disables family-guest handling entirely.
    #[serde(default)]
    pub discord_family_channel_id: Option<u64>,

    /// Family members scoped to `discord_family_channel_id`. Unlike
    /// `discord_allowed_user_ids`, these users get full HQ tool access (not
    /// gated by allowlist elsewhere) but are identity-tagged (see
    /// `hq_core::identity::FamilyGuestInfo`) so the model addresses them by
    /// name and the remote-MCP-confirm / agy-only-Herdr gates apply to them.
    #[serde(default)]
    pub discord_family_users: Vec<DiscordFamilyUser>,

    /// Herdr harnesses a family guest may start. Defaults to `["agy"]`;
    /// set `[]` to let guests use any harness.
    #[serde(default = "default_discord_family_allowed_harnesses")]
    pub discord_family_allowed_harnesses: Vec<String>,

    /// Extra Discord channel-category names to resolve and persist alongside
    /// the built-in notification categories (task-completions, etc.) — for
    /// an operator's own personal/project channels routed via
    /// `_system/DISCORD-ROUTING.md`. Empty by default; these are inherently
    /// per-operator, not something a fresh install can guess.
    #[serde(default)]
    pub discord_notification_categories: Vec<String>,
}

fn default_turn_ack_timeout_secs() -> u64 {
    270
}

fn default_background_turn_max_days() -> u64 {
    5
}

fn default_summarize_session_exits() -> bool {
    true
}

fn default_session_exit_summary_timeout_secs() -> u64 {
    90
}

/// Hand-written to match each field's own `#[serde(default = "fn")]`.
/// `#[derive(Default)]` would give every field its type's zero value instead,
/// and `HqConfig::default()` bakes this struct's `Default::default()` into the
/// Figment base layer `HqConfig::load()` merges under the real config file —
/// so a derived Default here silently wins over the intended values below for
/// any field the config file doesn't set.
impl Default for RelayConfig {
    fn default() -> Self {
        Self {
            discord_token: None,
            telegram_token: None,
            notifications_token: None,
            discord_enabled: false,
            telegram_enabled: false,
            model: None,
            telegram_authorized_chat_id: None,
            telegram_allowed_chat_ids: Vec::new(),
            telegram_users: Vec::new(),
            primary_surface: PrimarySurface::default(),
            turn_ack_timeout_secs: default_turn_ack_timeout_secs(),
            background_turn_max_days: default_background_turn_max_days(),
            background_progress_secs: None,
            summarize_session_exits: default_summarize_session_exits(),
            session_exit_summary_timeout_secs: default_session_exit_summary_timeout_secs(),
            discord_allowed_user_ids: Vec::new(),
            discord_family_channel_id: None,
            discord_family_users: Vec::new(),
            discord_family_allowed_harnesses: default_discord_family_allowed_harnesses(),
            discord_notification_categories: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relay_config_turn_fields_default_to_legacy_values() {
        let yaml = "discord_enabled: false\ntelegram_enabled: true\n";
        let config: RelayConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.turn_ack_timeout_secs, 270);
        assert_eq!(config.background_turn_max_days, 5);
    }

    #[test]
    fn relay_config_turn_fields_deserialize_explicit_values() {
        let yaml = "discord_enabled: false\ntelegram_enabled: true\nturn_ack_timeout_secs: 60\nbackground_turn_max_days: 2\n";
        let config: RelayConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.turn_ack_timeout_secs, 60);
        assert_eq!(config.background_turn_max_days, 2);
    }

    /// FR-056: `max_turns` was removed from `RelayConfig` (no config option
    /// may reinstate an application-level turn-count ceiling). A deployed
    /// `~/.hq/config.yaml` written before the removal that still sets
    /// `relay.max_turns` must keep loading cleanly, with the key ignored.
    #[test]
    fn legacy_max_turns_config_key_is_tolerated() {
        let yaml = "discord_enabled: false\ntelegram_enabled: true\nmax_turns: 500\n";
        let config: RelayConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(config.telegram_enabled);
    }

    #[test]
    fn relay_config_session_exit_summary_defaults_to_enabled() {
        let yaml = "discord_enabled: false\ntelegram_enabled: true\n";
        let config: RelayConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(config.summarize_session_exits);
        assert_eq!(config.session_exit_summary_timeout_secs, 90);
    }

    #[test]
    fn relay_config_session_exit_summary_can_be_disabled() {
        let yaml = "discord_enabled: false\ntelegram_enabled: true\nsummarize_session_exits: false\nsession_exit_summary_timeout_secs: 30\n";
        let config: RelayConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(!config.summarize_session_exits);
        assert_eq!(config.session_exit_summary_timeout_secs, 30);
    }

    #[test]
    fn relay_config_background_progress_secs_defaults_to_none() {
        let yaml = "discord_enabled: false\ntelegram_enabled: true\n";
        let config: RelayConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.background_progress_secs, None);
    }

    #[test]
    fn relay_config_background_progress_secs_round_trips() {
        let yaml =
            "discord_enabled: false\ntelegram_enabled: true\nbackground_progress_secs: 120\n";
        let config: RelayConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.background_progress_secs, Some(120));
        let serialized = serde_yaml::to_string(&config).unwrap();
        assert!(serialized.contains("background_progress_secs: 120"));
        let reparsed: RelayConfig = serde_yaml::from_str(&serialized).unwrap();
        assert_eq!(reparsed.background_progress_secs, Some(120));
    }

    #[test]
    fn relay_config_discord_family_fields_default() {
        let yaml = "discord_enabled: false\ntelegram_enabled: true\n";
        let config: RelayConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.discord_family_channel_id, None);
        assert!(config.discord_family_users.is_empty());
        assert_eq!(config.discord_family_allowed_harnesses, vec!["agy"]);
    }

    #[test]
    fn relay_config_discord_family_fields_round_trip() {
        let yaml = "discord_enabled: true\ntelegram_enabled: false\ndiscord_family_channel_id: 100200300400500600\ndiscord_family_users:\n  - user_id: 123456\n    name: Bob\n  - user_id: 789012\n    name: Carol\n";
        let config: RelayConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.discord_family_channel_id, Some(100200300400500600));
        assert_eq!(config.discord_family_users.len(), 2);
        assert_eq!(config.discord_family_users[0].name, "Bob");
        assert_eq!(config.discord_family_users[0].user_id, 123456);
        assert_eq!(config.discord_family_users[1].name, "Carol");
        assert_eq!(config.discord_family_users[1].user_id, 789012);

        let serialized = serde_yaml::to_string(&config).unwrap();
        assert!(serialized.contains("100200300400500600"));
        assert!(serialized.contains("Bob"));
        assert!(serialized.contains("Carol"));
        let reparsed: RelayConfig = serde_yaml::from_str(&serialized).unwrap();
        assert_eq!(
            reparsed.discord_family_channel_id,
            Some(100200300400500600)
        );
        assert_eq!(reparsed.discord_family_users.len(), 2);
        assert_eq!(reparsed.discord_family_users[0].name, "Bob");
        assert_eq!(reparsed.discord_family_users[1].name, "Carol");
    }
}
