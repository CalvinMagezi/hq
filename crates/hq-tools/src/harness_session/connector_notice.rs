//! The owner hears about every coding session a claude.ai connector starts: those sessions are
//! not gated by an approval (`docs/plans/claude-connector.md`), so this notice is the control.

use hq_core::types::{ValueItem, ValueKind};
use hq_db::Database;
use serde_json::Value;
use std::path::Path;

/// Where the notice's prompt is cut. Telegram caps a message near 4 KB, and the cut says so.
const MAX_PROMPT_CHARS: usize = 2500;
const NOTICE_SOURCE: &str = "mcp-connector";

/// Every field of the call that reaches the agent as instructions, labelled, so nothing the
/// model wrote into a side field is missing from what the owner reads.
pub fn connector_brief(args: &Value, fields: &[&str]) -> String {
    fields
        .iter()
        .filter_map(|f| {
            let v = args.get(*f)?.as_str()?.trim();
            (!v.is_empty()).then(|| format!("{f}: {v}"))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The notice for a session started on a connector grant, or `None` for any other caller and
/// for a handoff that found a session already working (nothing started).
pub fn connector_start_notice(
    args: &Value,
    report: &Value,
    brief: &str,
    cwd: &str,
) -> Option<ValueItem> {
    let label = super::connector_label(args)?;
    if report["handoff"] == "existing_session" {
        return None;
    }
    let field = |k: &str| report[k].as_str().unwrap_or("unknown").to_string();
    let shown: String = brief.chars().take(MAX_PROMPT_CHARS).collect();
    let cut = if brief.chars().count() > MAX_PROMPT_CHARS {
        "\n\n[Cut here. The full text is in the session's task thread and on the Sessions page.]"
    } else {
        ""
    };
    let body = format!(
        "Started by the claude.ai connection \"{label}\".\nSession: {}\nHost: {}\nFolder: {cwd}\n\nWhat it was told:\n{}{cut}\n\nIf you did not ask Claude for this, stop the session and disconnect it in Settings.",
        field("session_id"),
        field("host"),
        if shown.is_empty() {
            "(nothing)"
        } else {
            &shown
        },
    );
    Some(ValueItem::new(
        NOTICE_SOURCE,
        ValueKind::Fyi,
        "Claude started a coding session",
        body,
    ))
}

/// Sends the notice straight to the owner's chat, past the relay's batching. Best effort: the
/// session is already running, and a failure is logged.
pub fn notify_connector_start(
    vault_path: &Path,
    db: &Database,
    args: &Value,
    report: &Value,
    brief: &str,
    cwd: &str,
) {
    let Some(item) = connector_start_notice(args, report, brief, cwd) else {
        return;
    };
    let sent = hq_db::value_items::insert(db, &item)
        .and_then(|()| hq_db::value_items::deliver(vault_path, db, &item, true));
    if !matches!(sent, Ok(true)) {
        tracing::warn!("connector: the session start notice did not reach the relay mailbox");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn report() -> Value {
        json!({"session_id": "hs-claude-code-1", "host": "local", "cwd": "/srv/work/app"})
    }

    #[test]
    fn only_a_connector_start_gets_a_notice() {
        assert!(connector_start_notice(&json!({}), &report(), "fix it", "/srv/work/app").is_none());
        let args = json!({ super::super::CONNECTOR_ARG: "Claude on the web" });
        let item =
            connector_start_notice(&args, &report(), "fix the login bug", "/srv/work/app").unwrap();
        for part in [
            "Claude on the web",
            "hs-claude-code-1",
            "/srv/work/app",
            "fix the login bug",
        ] {
            assert!(item.body.contains(part), "{part}: {}", item.body);
        }
    }

    #[test]
    fn every_instruction_field_is_shown_and_an_existing_session_sends_nothing() {
        let args = json!({
            super::super::CONNECTOR_ARG: "Claude",
            "prompt": "fix it", "goal": "ship", "done_criteria": "tests pass", "title": ""
        });
        let brief = connector_brief(&args, &["prompt", "goal", "done_criteria", "title"]);
        assert_eq!(
            brief,
            "prompt: fix it\ngoal: ship\ndone_criteria: tests pass"
        );
        let existing = json!({"handoff": "existing_session", "session_id": "hs-1"});
        assert!(connector_start_notice(&args, &existing, &brief, "/srv").is_none());
    }

    #[test]
    fn a_long_prompt_is_cut_visibly() {
        let args = json!({ super::super::CONNECTOR_ARG: "Claude" });
        let long = "x".repeat(MAX_PROMPT_CHARS + 50);
        let item = connector_start_notice(&args, &report(), &long, "/srv").unwrap();
        assert!(item.body.contains("[Cut here."));
        let short = connector_start_notice(&args, &report(), "short", "/srv").unwrap();
        assert!(!short.body.contains("[Cut here."));
    }

    #[test]
    fn the_notice_reaches_the_relay_mailbox_as_an_interrupt() {
        let vault = tempfile::TempDir::new().unwrap();
        let db = Database::open_memory().unwrap();
        let args = json!({ super::super::CONNECTOR_ARG: "Claude" });
        notify_connector_start(vault.path(), &db, &args, &report(), "go", "/srv");
        let inbox = hq_core::mailbox::peek_messages(vault.path(), "relay").unwrap();
        assert_eq!(inbox.len(), 1);
        assert_eq!(
            inbox[0]
                .meta
                .get(hq_core::mailbox::META_INTERRUPT)
                .map(String::as_str),
            Some("true")
        );
    }
}
