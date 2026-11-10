//! A restricted audience (a family member in Discord, an unresolved person) must not read a note
//! that was not labelled for them through any tool, not only through the ones the vault scope
//! wrapper wraps by name. `scope_vault_tools` wraps tools called `vault_*` and `memory_*`; the
//! tools below come from the same factory or from the shortcut and export factories and do not
//! match, so each is tried here against a private note.

use super::*;
use crate::session::SessionConfig;
use hq_core::config::HqConfig;
use hq_core::identity::RequestIdentity;
use hq_core::privacy::DisclosureScope;

const CLOUD_MODEL: &str = "claude-sonnet-5-5";
const SECRET: &str = "ZEBRA-MARMALADE-7731";
const NOTE: &str = "Notebooks/Private/plan.md";

async fn session_for(
    vault: &std::path::Path,
    scope: DisclosureScope,
) -> crate::session::AgentSession {
    std::fs::create_dir_all(vault.join("Notebooks/Private")).unwrap();
    std::fs::write(
        vault.join(NOTE),
        format!("---\ntitle: Private plan\n---\nThe code word is {SECRET}.\n"),
    )
    .unwrap();
    let config = HqConfig {
        openrouter_api_key: Some("test-key-no-network".to_string()),
        vault_path: vault.to_path_buf(),
        ..HqConfig::default()
    };
    // Index the note the way the vault watcher would, so search-backed tools have it to find.
    let db = hq_db::Database::open(&config.db_path()).unwrap();
    db.with_conn(|c| hq_db::search::reconcile_path(c, vault, NOTE).map(|_| ()))
        .unwrap();
    drop(db);

    let mut identity = RequestIdentity::from_discord(1);
    identity.scope = scope;
    SessionBuilder::from_config(&config)
        .working_dir(vault.to_path_buf())
        .with_identity(identity)
        .session_config(SessionConfig {
            model: CLOUD_MODEL.to_string(),
            ..SessionConfig::default()
        })
        .build()
        .await
        .unwrap()
}

fn text(result: &hq_core::types::ToolResult) -> String {
    result.content.iter().map(|c| c.text.as_str()).collect()
}

/// Every tool that can return note text, with arguments that aim it at the private note.
fn attempts() -> Vec<(&'static str, serde_json::Value)> {
    use serde_json::json;
    vec![
        ("vault_read", json!({"path": NOTE})),
        ("vault_search", json!({"query": SECRET})),
        (
            "context_packet",
            json!({"task": "the code word", "refs": [NOTE], "queries": [SECRET]}),
        ),
        ("note_provenance", json!({"path": NOTE})),
        ("note_review", json!({"path": NOTE})),
        ("vault_find", json!({"topic": SECRET})),
        ("vault_note", json!({"topic": SECRET, "path": NOTE})),
        ("vault_log", json!({"topic": SECRET})),
        ("hq", json!({"request": format!("find {SECRET}")})),
        ("vault_export", json!({"path": NOTE, "format": "md"})),
        ("vault_export_pdf", json!({"path": NOTE})),
        (
            "convert_from_markdown",
            json!({"path": NOTE, "format": "docx"}),
        ),
    ]
}

#[tokio::test]
async fn a_restricted_audience_cannot_read_an_unlabelled_note_through_any_tool() {
    let vault = tempfile::TempDir::new().unwrap();
    let session = session_for(vault.path(), DisclosureScope::deny_all()).await;
    let names = session.tool_names().await;
    let mut leaks = Vec::new();
    let mut present = Vec::new();
    for (tool, args) in attempts() {
        if !names.iter().any(|n| n == tool) {
            continue;
        }
        present.push(tool);
        // An error is a refusal; only a result that carries the text is a leak.
        if let Ok(reply) = session.call_tool_for_test(tool, args).await
            && text(&reply).contains(SECRET)
        {
            leaks.push(tool);
        }
    }
    assert!(
        present.contains(&"vault_read"),
        "the control tool is missing: {names:?}"
    );
    assert!(
        leaks.is_empty(),
        "returned a private note: {leaks:?} (tools present: {present:?})"
    );
}

/// The scope must narrow a restricted audience and nobody else: the owner's own session reads the
/// same note through the same tools.
#[tokio::test]
async fn the_owner_still_reads_the_note_through_the_same_tools() {
    let vault = tempfile::TempDir::new().unwrap();
    let session = session_for(vault.path(), DisclosureScope::Unrestricted).await;
    for (tool, args) in attempts() {
        if matches!(tool, "context_packet" | "vault_find" | "vault_read") {
            let reply = session.call_tool_for_test(tool, args).await.unwrap();
            assert!(text(&reply).contains(SECRET), "{tool}: {}", text(&reply));
        }
    }
}
