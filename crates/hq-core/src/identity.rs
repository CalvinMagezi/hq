use crate::privacy::DisclosureScope;

/// Present on `RequestIdentity` when the caller is a Discord family guest
/// (see `RelayConfig::discord_family_users`), not the owner. Read by the
/// remote-MCP confirm gate and the agy-only Herdr gate.
#[derive(Debug, Clone)]
pub struct FamilyGuestInfo {
    pub name: String,
    /// The Discord channel/thread id the guest's turn is running in, so a
    /// deferred/confirmed action can be delivered back to the right place.
    pub origin_channel_id: u64,
    /// Display name of the bot owner who approves the guest's risky actions.
    pub owner_name: String,
    /// Herdr harnesses the guest may start; empty means unrestricted.
    pub allowed_harnesses: Vec<String>,
}

/// Resolved identity for every request entering HQ.
#[derive(Debug, Clone)]
pub struct RequestIdentity {
    pub user_id: String,
    pub user_name: String,
    pub source: RequestSource,
    pub session_key: String,
    pub vault_paths: Vec<String>,
    pub preferred_model: Option<String>,
    pub family_guest: Option<FamilyGuestInfo>,
    /// What context this request may read or disclose. Surfaces with their own
    /// authenticated owner gate default to unrestricted; Discord defaults to deny.
    pub scope: DisclosureScope,
}

#[derive(Debug, Clone)]
pub enum RequestSource {
    Telegram {
        chat_id: i64,
    },
    Discord {
        channel_id: u64,
    },
    ProxyApi {
        user_name: String,
    },
    /// A web chat thread, so tools can tie what they start to that chat.
    /// `driver_turn` marks a reply HQ started rather than one the user typed (a session driver
    /// turn or a sub-agent follow-up); `session_driver` narrows that to the session driver.
    Web {
        thread_id: String,
        driver_turn: bool,
        session_driver: bool,
    },
    LocalCli,
}

impl RequestSource {
    pub fn label(&self) -> &'static str {
        match self {
            RequestSource::Telegram { .. } => "telegram",
            RequestSource::Discord { .. } => "discord",
            RequestSource::ProxyApi { .. } => "proxy",
            RequestSource::Web { .. } => "web",
            RequestSource::LocalCli => "cli",
        }
    }
}

impl RequestIdentity {
    pub fn local() -> Self {
        let user_name = std::env::var("USER")
            .or_else(|_| std::env::var("USERNAME"))
            .unwrap_or_else(|_| "local".into());
        Self {
            user_id: "local".into(),
            user_name,
            source: RequestSource::LocalCli,
            session_key: "hq-cli".into(),
            vault_paths: vec!["*".into()],
            preferred_model: None,
            family_guest: None,
            scope: DisclosureScope::Unrestricted,
        }
    }

    pub fn from_telegram(chat_id: i64) -> Self {
        Self {
            user_id: format!("telegram:{chat_id}"),
            user_name: format!("tg-{chat_id}"),
            source: RequestSource::Telegram { chat_id },
            session_key: format!("hq-tg-{chat_id}"),
            vault_paths: vec!["*".into()],
            preferred_model: None,
            family_guest: None,
            scope: DisclosureScope::Unrestricted,
        }
    }

    pub fn from_discord(channel_id: u64) -> Self {
        Self {
            user_id: format!("discord:{channel_id}"),
            user_name: format!("dc-{channel_id}"),
            source: RequestSource::Discord { channel_id },
            session_key: format!("hq-dc-{channel_id}"),
            vault_paths: vec!["*".into()],
            preferred_model: None,
            family_guest: None,
            scope: DisclosureScope::deny_all(),
        }
    }

    /// The web app's owner, speaking in one chat thread. The session key stays
    /// the web proxy's, so per-user state is shared across threads as before.
    pub fn from_web_thread(thread_id: &str, driver_turn: bool, session_driver: bool) -> Self {
        Self {
            source: RequestSource::Web {
                thread_id: thread_id.to_string(),
                driver_turn,
                session_driver,
            },
            ..Self::from_proxy_user("web", vec!["*".into()], None)
        }
    }

    /// The web chat thread this request came from, if any.
    pub fn web_thread(&self) -> Option<&str> {
        match &self.source {
            RequestSource::Web { thread_id, .. } => Some(thread_id),
            _ => None,
        }
    }

    /// Whether this is a reply HQ started for a watched session rather than one the user typed.
    pub fn is_web_driver_turn(&self) -> bool {
        matches!(
            self.source,
            RequestSource::Web {
                driver_turn: true,
                ..
            }
        )
    }

    /// Whether this is a turn the session driver started for a watched harness session, which
    /// is held to the driver's budget and may not start or attach sessions.
    pub fn is_session_driver_turn(&self) -> bool {
        matches!(
            self.source,
            RequestSource::Web {
                session_driver: true,
                ..
            }
        )
    }

    pub fn from_proxy_user(
        name: &str,
        vault_paths: Vec<String>,
        preferred_model: Option<String>,
    ) -> Self {
        Self {
            user_id: name.to_string(),
            user_name: name.to_string(),
            source: RequestSource::ProxyApi {
                user_name: name.to_string(),
            },
            session_key: format!("hq-proxy-{name}"),
            vault_paths,
            preferred_model,
            family_guest: None,
            scope: DisclosureScope::Unrestricted,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_identity() {
        let id = RequestIdentity::local();
        assert_eq!(id.user_id, "local");
        assert_eq!(id.session_key, "hq-cli");
        assert_eq!(id.source.label(), "cli");
    }

    #[test]
    fn telegram_identity() {
        let id = RequestIdentity::from_telegram(1234567890);
        assert_eq!(id.user_id, "telegram:1234567890");
        assert_eq!(id.session_key, "hq-tg-1234567890"); // gitleaks:allow
        assert_eq!(id.source.label(), "telegram");
        assert!(id.family_guest.is_none());
    }

    #[test]
    fn discord_identity_with_family_guest() {
        let mut id = RequestIdentity::from_discord(123456789);
        assert_eq!(id.user_id, "discord:123456789");
        assert_eq!(id.session_key, "hq-dc-123456789"); // gitleaks:allow
        assert_eq!(id.source.label(), "discord");
        assert!(id.family_guest.is_none());

        id.family_guest = Some(FamilyGuestInfo {
            name: "Bob".into(),
            origin_channel_id: 123456789,
            owner_name: "Owner".into(),
            allowed_harnesses: Vec::new(),
        });
        assert_eq!(id.family_guest.as_ref().unwrap().name, "Bob");
        assert_eq!(
            id.family_guest.as_ref().unwrap().origin_channel_id,
            123456789
        );
    }
}
