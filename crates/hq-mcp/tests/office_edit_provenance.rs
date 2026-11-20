//! An agent's `office_edit` goes through the runtime's write seam: the text it replaces is kept
//! for the owner's Undo and the write is recorded as the agent's. Needs the `suite` feature:
//! `cargo test -p hq-mcp --features suite --test office_edit_provenance`.
#![cfg(feature = "suite")]

use std::sync::Arc;

use hq_mcp::registry::create_default_registry;
use serde_json::json;

#[tokio::test]
async fn an_agent_edit_keeps_the_earlier_text_and_records_the_writer() {
    let dir = tempfile::tempdir().unwrap();
    let vault = dir.path();
    std::fs::create_dir_all(vault.join("Notebooks")).unwrap();
    let before = "---\nsuite_kind: doc\n---\n# Plan\n\nFirst.\n";
    std::fs::write(vault.join("Notebooks/plan.md"), before).unwrap();
    let db = Arc::new(hq_db::Database::open_memory().unwrap());
    let client = Arc::new(hq_vault::VaultClient::new(vault.to_path_buf()).unwrap());
    let registry = create_default_registry(
        client,
        db.clone(),
        vault.join("skills"),
        vault.join("Agents"),
        None,
    );
    let tool = registry
        .get("office_edit")
        .expect("office_edit is registered");
    let caller = json!({"_hq_native_session": "s1", "_hq_native_agent": "hq"});
    let mut read = json!({"action": "blocks", "note": "Notebooks/plan.md"});
    read.as_object_mut()
        .unwrap()
        .extend(caller.as_object().unwrap().clone());
    let listed = tool.execute(read).await.unwrap();
    let first = listed["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["src"].as_str().unwrap().contains("First"))
        .unwrap();
    let mut edit = json!({"action": "replace_block", "note": "Notebooks/plan.md",
        "index": first["index"], "hash": first["hash"], "markdown": "Changed."});
    edit.as_object_mut()
        .unwrap()
        .extend(caller.as_object().unwrap().clone());
    tool.execute(edit).await.unwrap();

    let after = std::fs::read_to_string(vault.join("Notebooks/plan.md")).unwrap();
    assert!(after.contains("Changed."));
    let prev = db
        .with_conn(|c| hq_db::note_keep::get(c, "Notebooks/plan.md"))
        .unwrap()
        .expect("the earlier text is kept for Undo");
    assert_eq!(prev.text, before);
    let hash = hq_db::search::content_hash(&after);
    let who = db
        .with_conn(|c| hq_db::provenance::current(c, "Notebooks/plan.md", &hash))
        .unwrap()
        .expect("the write is recorded");
    assert_eq!(who.source_kind, hq_db::provenance::SourceKind::Agent);
}
