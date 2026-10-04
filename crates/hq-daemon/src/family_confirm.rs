use std::path::Path;

pub use hq_tools::family_confirm::{
    PENDING_FILE, PendingFamilyAction, TTL_HOURS, add_pending, take_pending,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FamilyAction {
    Approve,
    Deny,
}

/// Parse a reply like "approve <token>", "deny <token>", or message containing the token.
/// Returns (action, token).
pub fn parse_family_command(text: &str) -> Option<(FamilyAction, String)> {
    crate::parse_token_command(
        text,
        &[
            ("approve", FamilyAction::Approve),
            ("deny", FamilyAction::Deny),
        ],
    )
    .map(|(action, token, _)| (action, token))
}

/// Approve performs the deferred remote-MCP call for real and returns
/// (reply_text, origin_channel_id) so the Discord button handler can post
/// the result into the guest's own thread, not wherever the button was
/// clicked. Deny just returns the guest-facing decline text.
pub async fn resolve_family_action(
    vault_path: &Path,
    token: &str,
    action: FamilyAction,
) -> Option<(String, u64)> {
    let pending = take_pending(vault_path, token)?;
    let origin_channel_id = pending.origin_channel_id;

    match action {
        FamilyAction::Deny => {
            let reply = format!(
                "The owner declined the request to run {}.{}.",
                pending.server_name, pending.tool
            );
            Some((reply, origin_channel_id))
        }
        FamilyAction::Approve => {
            let config = match hq_core::config::HqConfig::load() {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!(error = %e, "resolve_family_action: failed to load config");
                    return Some((
                        format!("Approved by the owner, but failed to load config: {e}"),
                        origin_channel_id,
                    ));
                }
            };
            let server = config
                .remote_mcp
                .iter()
                .find(|s| s.name == pending.server_name);
            let Some(server) = server else {
                let err_reply = format!(
                    "Approved by the owner, but remote MCP server '{}' is no longer configured.",
                    pending.server_name
                );
                return Some((err_reply, origin_channel_id));
            };

            match hq_tools::remote_mcp::call_named_server(server, &pending.tool, pending.args).await
            {
                Ok(val) => {
                    let formatted =
                        serde_json::to_string_pretty(&val).unwrap_or_else(|_| val.to_string());
                    let reply = format!(
                        "The owner approved the request to run {}.{}.\nResult:\n```json\n{}\n```",
                        pending.server_name, pending.tool, formatted
                    );
                    Some((reply, origin_channel_id))
                }
                Err(e) => {
                    let err_reply = format!(
                        "The owner approved {}.{}, but the call failed: {e}",
                        pending.server_name, pending.tool
                    );
                    Some((err_reply, origin_channel_id))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};
    use serde_json::json;
    use tempfile::tempdir;

    #[test]
    fn parse_family_command_extracts_action_and_token() {
        assert_eq!(
            parse_family_command("approve a1b2c3d4"),
            Some((FamilyAction::Approve, "a1b2c3d4".to_string()))
        );
        assert_eq!(
            parse_family_command("deny a1b2c3d4"),
            Some((FamilyAction::Deny, "a1b2c3d4".to_string()))
        );
        assert_eq!(
            parse_family_command(
                "Bob wants to call acme.test. Reply \"approve tok12345\" or \"deny tok12345\""
            ),
            Some((FamilyAction::Approve, "tok12345".to_string()))
        );
        assert_eq!(parse_family_command("no token here"), None);
    }

    #[test]
    fn pending_store_add_take_round_trip() {
        let dir = tempdir().unwrap();
        let vault = dir.path();

        let pending = PendingFamilyAction {
            token: "tok12345".into(),
            requester_name: "Bob".into(),
            origin_channel_id: 111222333,
            server_name: "acme".into(),
            tool: "run_job".into(),
            args: json!({"key": "val"}),
            summary: "Bob wants to call acme.run_job".into(),
            created_at: Utc::now(),
        };

        add_pending(vault, pending.clone()).unwrap();

        // Taking with an invalid token returns None
        assert!(take_pending(vault, "wrongtok").is_none());

        // Taking with the valid token returns the pending action
        let taken = take_pending(vault, "tok12345").unwrap();
        assert_eq!(taken.token, "tok12345");
        assert_eq!(taken.requester_name, "Bob");
        assert_eq!(taken.origin_channel_id, 111222333);

        // Taking again returns None (it was consumed)
        assert!(take_pending(vault, "tok12345").is_none());
    }

    #[test]
    fn pending_store_prunes_expired() {
        let dir = tempdir().unwrap();
        let vault = dir.path();

        let expired = PendingFamilyAction {
            token: "oldtok12".into(),
            requester_name: "Carol".into(),
            origin_channel_id: 444555,
            server_name: "acme".into(),
            tool: "query".into(),
            args: json!({}),
            summary: "Carol query".into(),
            created_at: Utc::now() - Duration::hours(25),
        };

        add_pending(vault, expired).unwrap();

        // Expired item should not be retrieved
        assert!(take_pending(vault, "oldtok12").is_none());
    }
}
