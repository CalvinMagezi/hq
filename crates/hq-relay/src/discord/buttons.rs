//! Status embed and the email and value-bus action buttons.

use super::*;

/// Build a rich status embed for the !status and /status commands.
pub fn build_status_embed(model: &str, message_count: usize) -> CreateEmbed {
    CreateEmbed::new()
        .title("HQ Status")
        .field("Model", model, true)
        .field("Messages", message_count.to_string(), true)
        .colour(serenity::model::Colour::from_rgb(53, 181, 68))
}

const EMAIL_SEND_PREFIX: &str = "email_send_";
const EMAIL_SKIP_PREFIX: &str = "email_skip_";
const VALUE_APPROVE_PREFIX: &str = "value_approve_";
const VALUE_DISMISS_PREFIX: &str = "value_dismiss_";
const FAMILY_APPROVE_PREFIX: &str = "family_approve_";
const FAMILY_DENY_PREFIX: &str = "family_deny_";

/// Build a Discord button custom_id for an email-triage send action.
pub(crate) fn make_email_send_button_id(token: &str) -> String {
    format!("{EMAIL_SEND_PREFIX}{token}")
}

/// Build a Discord button custom_id for an email-triage skip action.
pub(crate) fn make_email_skip_button_id(token: &str) -> String {
    format!("{EMAIL_SKIP_PREFIX}{token}")
}

/// Parse an email-triage button custom_id back to (action, token).
pub(crate) fn parse_email_button_id(custom_id: &str) -> Option<(&str, &str)> {
    if let Some(id) = custom_id.strip_prefix(EMAIL_SEND_PREFIX) {
        Some(("send", id))
    } else if let Some(id) = custom_id.strip_prefix(EMAIL_SKIP_PREFIX) {
        Some(("skip", id))
    } else {
        None
    }
}

/// Build a Discord message for an email-triage reply-needed notification with
/// Send/Skip buttons.
pub(crate) fn build_email_action_message(
    text: &str,
    token: &str,
) -> serenity::builder::CreateMessage {
    use serenity::builder::{CreateActionRow, CreateButton};
    use serenity::model::application::ButtonStyle;

    let send_btn = CreateButton::new(make_email_send_button_id(token))
        .label("Send")
        .style(ButtonStyle::Success);
    let skip_btn = CreateButton::new(make_email_skip_button_id(token))
        .label("Skip")
        .style(ButtonStyle::Secondary);
    let row = CreateActionRow::Buttons(vec![send_btn, skip_btn]);

    CreateMessage::new().content(text).components(vec![row])
}

/// Build a Discord button custom_id for a value-bus approve action.
pub(crate) fn make_value_approve_button_id(token: &str) -> String {
    format!("{VALUE_APPROVE_PREFIX}{token}")
}

/// Build a Discord button custom_id for a value-bus dismiss action.
pub(crate) fn make_value_dismiss_button_id(token: &str) -> String {
    format!("{VALUE_DISMISS_PREFIX}{token}")
}

/// Parse a value-bus button custom_id back to (action, token).
pub(crate) fn parse_value_button_id(custom_id: &str) -> Option<(&str, &str)> {
    if let Some(id) = custom_id.strip_prefix(VALUE_APPROVE_PREFIX) {
        Some(("approve", id))
    } else if let Some(id) = custom_id.strip_prefix(VALUE_DISMISS_PREFIX) {
        Some(("dismiss", id))
    } else {
        None
    }
}

/// Build a Discord message for a value-bus item with Approve/Dismiss buttons,
/// with inline buttons.
pub(crate) fn build_value_action_message(
    text: &str,
    token: &str,
) -> serenity::builder::CreateMessage {
    use serenity::builder::{CreateActionRow, CreateButton};
    use serenity::model::application::ButtonStyle;

    let approve_btn = CreateButton::new(make_value_approve_button_id(token))
        .label("Approve")
        .style(ButtonStyle::Success);
    let dismiss_btn = CreateButton::new(make_value_dismiss_button_id(token))
        .label("Dismiss")
        .style(ButtonStyle::Secondary);
    let row = CreateActionRow::Buttons(vec![approve_btn, dismiss_btn]);

    CreateMessage::new().content(text).components(vec![row])
}

/// Build a Discord button custom_id for a family-action approve action.
pub(crate) fn make_family_approve_button_id(token: &str) -> String {
    format!("{FAMILY_APPROVE_PREFIX}{token}")
}

/// Build a Discord button custom_id for a family-action deny action.
pub(crate) fn make_family_deny_button_id(token: &str) -> String {
    format!("{FAMILY_DENY_PREFIX}{token}")
}

/// Parse a family-action button custom_id back to (action, token).
pub(crate) fn parse_family_button_id(custom_id: &str) -> Option<(&str, &str)> {
    if let Some(id) = custom_id.strip_prefix(FAMILY_APPROVE_PREFIX) {
        Some(("approve", id))
    } else if let Some(id) = custom_id.strip_prefix(FAMILY_DENY_PREFIX) {
        Some(("deny", id))
    } else {
        None
    }
}

/// Build a Discord message for a family-action confirmation with Approve/Deny buttons.
pub(crate) fn build_family_action_message(
    text: &str,
    token: &str,
) -> serenity::builder::CreateMessage {
    use serenity::builder::{CreateActionRow, CreateButton};
    use serenity::model::application::ButtonStyle;

    let approve_btn = CreateButton::new(make_family_approve_button_id(token))
        .label("Approve")
        .style(ButtonStyle::Success);
    let deny_btn = CreateButton::new(make_family_deny_button_id(token))
        .label("Deny")
        .style(ButtonStyle::Danger);
    let row = CreateActionRow::Buttons(vec![approve_btn, deny_btn]);

    CreateMessage::new().content(text).components(vec![row])
}

/// Bounds and character-class validation for tokens embedded in Discord button
/// custom_ids (email token, value-bus token). Shared by all
/// interaction-dispatch branches to prevent custom_id injection into downstream
/// vault/db lookups.
pub(super) fn is_valid_button_token(id: &str) -> bool {
    id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_button_id_round_trips() {
        assert_eq!(make_email_send_button_id("a1b2c3"), "email_send_a1b2c3");
        assert_eq!(make_email_skip_button_id("a1b2c3"), "email_skip_a1b2c3");
        assert_eq!(
            parse_email_button_id("email_send_a1b2c3"),
            Some(("send", "a1b2c3"))
        );
        assert_eq!(
            parse_email_button_id("email_skip_a1b2c3"),
            Some(("skip", "a1b2c3"))
        );
        assert_eq!(parse_email_button_id("value_approve_a1b2c3"), None);
        assert_eq!(parse_email_button_id("approve_prop-1"), None);
    }

    #[test]
    fn value_button_id_round_trips() {
        assert_eq!(
            make_value_approve_button_id("a1b2c3d4"),
            "value_approve_a1b2c3d4"
        );
        assert_eq!(
            make_value_dismiss_button_id("a1b2c3d4"),
            "value_dismiss_a1b2c3d4"
        );
        assert_eq!(
            parse_value_button_id("value_approve_a1b2c3d4"),
            Some(("approve", "a1b2c3d4"))
        );
        assert_eq!(
            parse_value_button_id("value_dismiss_a1b2c3d4"),
            Some(("dismiss", "a1b2c3d4"))
        );
        assert_eq!(parse_value_button_id("email_send_a1b2c3d4"), None);
        assert_eq!(parse_value_button_id("skip_prop-1"), None);
    }

    #[test]
    fn family_button_id_round_trips() {
        assert_eq!(
            make_family_approve_button_id("a1b2c3d4"),
            "family_approve_a1b2c3d4"
        );
        assert_eq!(
            make_family_deny_button_id("a1b2c3d4"),
            "family_deny_a1b2c3d4"
        );
        assert_eq!(
            parse_family_button_id("family_approve_a1b2c3d4"),
            Some(("approve", "a1b2c3d4"))
        );
        assert_eq!(
            parse_family_button_id("family_deny_a1b2c3d4"),
            Some(("deny", "a1b2c3d4"))
        );
        assert_eq!(parse_family_button_id("email_send_a1b2c3d4"), None);
        assert_eq!(parse_family_button_id("value_approve_a1b2c3d4"), None);
    }

    #[test]
    fn button_token_validation_matches_existing_proposal_id_rigor() {
        assert!(is_valid_button_token("abc-123_XYZ"));
        assert!(is_valid_button_token(&"a".repeat(64)));
        assert!(!is_valid_button_token(&"a".repeat(65)));
        assert!(!is_valid_button_token("tok;rm -rf"));
        assert!(!is_valid_button_token("tok/with/slash"));
    }
}
