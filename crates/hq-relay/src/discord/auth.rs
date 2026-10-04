//! Who may talk to the Discord bot: the allowlist, or the single paired owner.

use hq_core::pairing::{self, PairPlatform};

/// Whether `user_id` is in a non-empty allowlist.
fn is_discord_user_allowed(user_id: u64, allowed: &[u64]) -> bool {
    allowed.contains(&user_id)
}

/// Owner recorded by a successful `!pair` (`hq pair --platform discord`).
/// Only consulted when `discord_allowed_user_ids` is empty. There is no trust
/// on first use: with neither an allowlist nor a paired owner, nobody is
/// authorized.
pub(super) fn discord_auth_owner_path(vault_path: &std::path::Path) -> std::path::PathBuf {
    vault_path.join("_system").join(".discord-auth-user")
}

/// Whether a Discord user (chat message author or button-click actor) may
/// act on this bot. A non-empty `allowed` list decides alone; otherwise only
/// the owner recorded by pairing is authorized.
///
/// Used for chat and for approval buttons and slash commands alike, since a
/// click is a fresh interaction at least as privileged as a message.
pub(super) async fn is_message_author_allowed(
    vault_path: &std::path::Path,
    user_id: u64,
    allowed: &[u64],
) -> bool {
    if !allowed.is_empty() {
        return is_discord_user_allowed(user_id, allowed);
    }
    match tokio::fs::read_to_string(discord_auth_owner_path(vault_path)).await {
        Ok(existing) => existing.trim().parse::<u64>().is_ok_and(|owner| owner == user_id),
        Err(_) => false,
    }
}

/// Redeems `!pair <code>` sent in a DM when no owner exists yet. True when this
/// message paired the sender, so the caller should consume it. Guild channels
/// are refused so a code is never typed where others can read it.
pub(super) fn try_pair_dm(
    vault_path: &std::path::Path,
    user_id: u64,
    is_dm: bool,
    is_human: bool,
    allowed: &[u64],
    content: &str,
    now: i64,
) -> bool {
    // Cheap in-memory checks first: the filesystem is only touched for an
    // actual `!pair` DM, never for ordinary messages.
    if !is_dm || !is_human || !allowed.is_empty() {
        return false;
    }
    let Some(code) = pairing::parse_pair_command(content) else {
        return false;
    };
    let owner_path = discord_auth_owner_path(vault_path);
    if owner_path.exists() {
        return false;
    }
    if pairing::redeem_pairing_code(vault_path, PairPlatform::Discord, code, now).is_err() {
        tracing::warn!(user_id, "discord: pairing attempt refused");
        return false;
    }
    let written = std::fs::create_dir_all(owner_path.parent().unwrap_or(vault_path))
        .and_then(|()| std::fs::write(&owner_path, user_id.to_string()));
    if let Err(e) = written {
        tracing::error!(error = %e, "discord: pairing code accepted but owner file could not be written");
        return false;
    }
    tracing::info!(user_id, "discord: pairing code accepted, recorded owner");
    true
}

/// Pure authorization decision for an incoming message, factored out of
/// `EventHandler::message` (FR-006a) so the combination of mention/DM/bot/
/// allowlist checks is exercisable without a live Discord gateway connection
/// or a constructible `serenity::Message`/`Context` — before this, only the
/// allowlist predicate (`is_message_author_allowed`) in isolation was
/// tested, never this combined decision.
pub(super) async fn authorize_incoming(
    vault_path: &std::path::Path,
    author_id: u64,
    is_real_bot: bool,
    is_mention: bool,
    is_dm: bool,
    allowed: &[u64],
) -> bool {
    // Other bots only get a response when they explicitly @mention this
    // bot, which keeps bot-to-bot channels loop-free. Already mention-gated,
    // so no further authorization check.
    if is_real_bot {
        return is_mention;
    }
    if !(is_mention || is_dm) {
        return false;
    }
    // Human senders — and webhook-posted messages, which are
    // impersonation-shaped rather than trusted-bot-shaped — must be
    // authorized (explicit allowlist, or the trust-on-first-use owner when
    // unset).
    is_message_author_allowed(vault_path, author_id, allowed).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn an_empty_allowlist_with_no_paired_owner_authorizes_nobody() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!is_message_author_allowed(dir.path(), 12345, &[]).await);
        assert!(!discord_auth_owner_path(dir.path()).exists());
    }

    #[test]
    fn a_non_empty_allowlist_only_allows_its_members() {
        let allowed = vec![111, 222];
        assert!(is_discord_user_allowed(111, &allowed));
        assert!(!is_discord_user_allowed(333, &allowed));
    }

    // FR-006a: nothing previously exercised the *combination* of
    // mention/DM/bot/allowlist checks together — only the allowlist
    // predicate in isolation. These drive `authorize_incoming` directly with
    // synthetic ids, which doesn't need a live Discord gateway connection or
    // a constructible `serenity::Message`/`Context`.
    #[tokio::test]
    async fn unauthorized_dm_sender_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            !authorize_incoming(dir.path(), 999, false, false, true, &[111]).await,
            "a DM from a non-allowlisted user must be rejected"
        );
    }

    #[tokio::test]
    async fn unauthorized_non_mention_guild_message_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            !authorize_incoming(dir.path(), 999, false, false, false, &[111]).await,
            "a guild message that neither mentions the bot nor is a DM must be rejected"
        );
    }

    #[tokio::test]
    async fn allowlisted_sender_is_authorized_via_mention_or_dm() {
        let dir = tempfile::tempdir().unwrap();
        assert!(authorize_incoming(dir.path(), 111, false, true, false, &[111]).await);
        assert!(authorize_incoming(dir.path(), 111, false, false, true, &[111]).await);
    }

    #[tokio::test]
    async fn real_bot_account_is_authorized_on_mention_regardless_of_allowlist() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            authorize_incoming(dir.path(), 999, true, true, false, &[111]).await,
            "an @-mentioning real bot account is exempt from the allowlist"
        );
    }

    #[tokio::test]
    async fn real_bot_account_without_a_mention_is_not_authorized() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!authorize_incoming(dir.path(), 999, true, false, true, &[111]).await);
    }

    #[tokio::test]
    async fn explicit_allowlist_wins_over_the_owner_file() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            is_message_author_allowed(dir.path(), 111, &[111, 222]).await,
            "explicit member must be allowed"
        );
        assert!(
            !is_message_author_allowed(dir.path(), 333, &[111, 222]).await,
            "non-member must be rejected even though the list is non-empty"
        );
    }

    fn code_for(dir: &std::path::Path) -> String {
        pairing::create_pairing_code(dir, PairPlatform::Discord, 0).unwrap()
    }

    #[tokio::test]
    async fn correct_dm_code_makes_exactly_one_owner() {
        let dir = tempfile::tempdir().unwrap();
        let cmd = format!("!pair {}", code_for(dir.path()));
        assert!(try_pair_dm(dir.path(), 42, true, true, &[], &cmd, 1));
        assert!(is_message_author_allowed(dir.path(), 42, &[]).await);
        assert!(!is_message_author_allowed(dir.path(), 99, &[]).await);
        assert!(!try_pair_dm(dir.path(), 99, true, true, &[], &cmd, 2), "code is single use");
        assert_eq!(
            std::fs::read_to_string(discord_auth_owner_path(dir.path())).unwrap().trim(),
            "42"
        );
    }

    #[tokio::test]
    async fn wrong_expired_guild_and_bot_pairing_attempts_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let code = code_for(dir.path());
        assert!(!try_pair_dm(dir.path(), 1, true, true, &[], "!pair WRONG-WRONG", 1));
        assert!(!try_pair_dm(dir.path(), 1, false, true, &[], &format!("!pair {code}"), 1), "guild");
        assert!(!try_pair_dm(dir.path(), 1, true, false, &[], &format!("!pair {code}"), 1), "bot");
        let late = pairing::PAIRING_TTL_SECS + 1;
        assert!(!try_pair_dm(dir.path(), 1, true, true, &[], &format!("!pair {code}"), late));
        assert!(!is_message_author_allowed(dir.path(), 1, &[]).await);
    }

    #[tokio::test]
    async fn pairing_is_ignored_when_an_allowlist_is_configured() {
        let dir = tempfile::tempdir().unwrap();
        let cmd = format!("!pair {}", code_for(dir.path()));
        assert!(!try_pair_dm(dir.path(), 5, true, true, &[111], &cmd, 1));
        assert!(!discord_auth_owner_path(dir.path()).exists());
    }

    #[tokio::test]
    async fn an_existing_owner_file_keeps_working_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("_system")).unwrap();
        std::fs::write(discord_auth_owner_path(dir.path()), "77").unwrap();
        assert!(is_message_author_allowed(dir.path(), 77, &[]).await);
        assert!(!is_message_author_allowed(dir.path(), 78, &[]).await);
    }
}
