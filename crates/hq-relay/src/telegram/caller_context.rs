//! Per-turn caller awareness block for Telegram relay dispatch.

use super::access::TelegramRole;
use hq_core::identity::RequestIdentity;

/// Runtime facts about this HQ instance.
#[derive(Debug, Clone)]
pub struct InstanceMeta {
    pub hostname: String,
    pub vault_path: String,
    pub relay_model: String,
}

/// Who is speaking on Telegram this turn.
#[derive(Debug, Clone)]
pub struct CallerContext {
    pub identity: RequestIdentity,
    pub role: TelegramRole,
    pub display_name: String,
    pub owner_name: String,
    pub owner_chat_id: i64,
    pub caller_chat_id: i64,
    pub instance: InstanceMeta,
}

/// Markdown block prepended to the system prompt.
pub fn build_caller_block(ctx: &CallerContext) -> String {
    let role_label = match ctx.role {
        TelegramRole::Owner => "owner",
        TelegramRole::Guest => "guest",
    };

    let guest_policy = if ctx.role == TelegramRole::Guest {
        format!(
            r#"

### Guest policy
You are {owner}'s personal assistant. {guest} is a trusted guest. Answer helpfully using the same knowledge and voice as when speaking to {owner}. Do not expose {owner}'s private notes or tags unless directly relevant and safe. Refuse destructive actions (deploy, shell, file writes, computer control, sub-agents). Offer to summarize the request for {owner} when escalation is needed. Never send proactive notifications to this chat."#,
            owner = ctx.owner_name,
            guest = ctx.display_name,
        )
    } else {
        String::new()
    };

    format!(
        r#"## HQ Instance (read first)

- **Instance**: Agent-HQ relay on {hostname}
- **Vault**: `{vault_path}`
- **Owner**: {owner_name} (telegram:{owner_chat_id}) — this is whose personal assistant you are
- **Caller**: {display_name} — role: {role_label}, session: {session_key}
- **Channel**: Telegram DM, chat_id {caller_chat_id}
- **Relay model**: `{relay_model}`{guest_policy}
"#,
        hostname = ctx.instance.hostname,
        vault_path = ctx.instance.vault_path,
        owner_name = ctx.owner_name,
        owner_chat_id = ctx.owner_chat_id,
        display_name = ctx.display_name,
        role_label = role_label,
        session_key = ctx.identity.session_key,
        caller_chat_id = ctx.caller_chat_id,
        relay_model = ctx.instance.relay_model,
        guest_policy = guest_policy,
    )
}

/// One-line intro for a guest's first message in a thread.
pub fn guest_intro_line(ctx: &CallerContext) -> String {
    format!(
        "Hi {} — you're on {}'s HQ assistant. Ask me anything I can help with; I'll loop {} in for approvals or sensitive changes.",
        ctx.display_name, ctx.owner_name, ctx.owner_name
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::telegram::access::TelegramRole;
    use hq_core::identity::RequestIdentity;

    fn sample_ctx(role: TelegramRole) -> CallerContext {
        let mut identity = RequestIdentity::from_telegram(456);
        identity.user_name = "Bob".into();
        CallerContext {
            identity,
            role,
            display_name: "Bob".into(),
            owner_name: "Alex".into(),
            owner_chat_id: 123,
            caller_chat_id: 456,
            instance: InstanceMeta {
                hostname: "testhost".into(),
                vault_path: "/vault".into(),
                relay_model: "qwen".into(),
            },
        }
    }

    #[test]
    fn guest_block_contains_owner_and_guest_policy() {
        let block = build_caller_block(&sample_ctx(TelegramRole::Guest));
        assert!(block.contains("Alex"));
        assert!(block.contains("role: guest"));
        assert!(block.contains("Bob"));
        assert!(block.contains("Guest policy"));
    }

    #[test]
    fn owner_block_has_no_guest_policy() {
        let block = build_caller_block(&sample_ctx(TelegramRole::Owner));
        assert!(block.contains("role: owner"));
        assert!(!block.contains("Guest policy"));
    }
}
