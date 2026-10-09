//! A client that knows nothing about HQ works a task start to finish through the gateway,
//! using only what the server tells it: the instructions it receives on connect and the
//! hints in each reply. No launched-session token, no special setup.

use std::sync::Arc;

use hq_mcp::gateway;
use hq_mcp::registry::create_default_registry;
use serde_json::{Value, json};

struct Client {
    registry: hq_tools::registry::ToolRegistry,
    db: Arc<hq_db::Database>,
}

impl Client {
    fn new(vault: &std::path::Path) -> Self {
        let db = Arc::new(hq_db::Database::open_memory().unwrap());
        let vault_client = Arc::new(hq_vault::VaultClient::new(vault.to_path_buf()).unwrap());
        let registry = create_default_registry(vault_client, db.clone(), vault.join("skills"), vault.join("Agents"), None);
        Self { registry, db }
    }

    /// `hq_call(tool, args)`, parsed. An error from the tool comes back as `{"error": "..."}`.
    async fn call(&self, tool: &str, args: Value) -> Value {
        let arguments = json!({ "tool": tool, "args": args });
        let result = gateway::dispatch(&self.registry, "hq_call", arguments.as_object(), &self.db, None)
            .await
            .unwrap();
        let value = serde_json::to_value(&result).unwrap();
        let text = value["content"][0]["text"].as_str().unwrap_or_default().to_string();
        match value["isError"].as_bool() {
            Some(true) => json!({ "error": text }),
            _ => serde_json::from_str(&text).unwrap_or(Value::String(text)),
        }
    }
}

#[tokio::test]
async fn a_new_agent_finds_claims_works_hands_off_and_the_next_one_resumes() {
    let dir = tempfile::tempdir().unwrap();
    let hq = Client::new(dir.path());

    // What every client is told on connect.
    let instructions = gateway::server_instructions(&hq.registry);
    for needed in ["task_next", "lease", "task_heartbeat", "task_release"] {
        assert!(instructions.contains(needed), "the instructions never mention {needed}");
    }

    // Work exists, assigned to the agent, and came from a note.
    let created = hq
        .call(
            "task_create",
            json!({
                "title": "Port the importer to the new schema",
                "assignees": ["codex-agent"],
                "estimate_minutes": 30,
                "links": [{ "kind": "vault_note", "ref": "Notebooks/Projects/importer.md", "direction": "origin" }],
                "created_by": "planner",
            }),
        )
        .await;
    assert!(created.get("error").is_none(), "{created}");
    let id = created["display_id"].as_str().unwrap().to_string();

    // Find and start it in one step.
    let started = hq.call("task_next", json!({ "actor": "codex-agent", "harness": "codex", "session_ref": "s-1" })).await;
    assert_eq!(started["task"]["display_id"], id.as_str(), "{started}");
    assert_eq!(started["task"]["status"], "in_progress");
    let lease = started["lease"].as_str().expect("a lease token").to_string();

    // Work is attributed to the lease holder whatever name it types.
    let comment = hq.call("task_comment_add", json!({ "task_id": id, "body": "schema mapped", "author": "ignored", "lease": lease })).await;
    assert_eq!(comment["author"], "codex-agent");
    let beat = hq
        .call(
            "task_heartbeat",
            json!({ "lease": lease, "checkpoint": { "summary": "mapped 3 of 4 tables", "next_step": "port the orders table", "files": ["src/import.rs"] } }),
        )
        .await;
    assert_eq!(beat["checkpoint_saved"], true, "{beat}");
    let linked = hq.call("task_link_add", json!({ "task_id": id, "kind": "commit", "ref": "owner/repo@abc1234", "direction": "produced", "lease": lease })).await;
    assert_eq!(linked["created"], true, "{linked}");

    // Stopping honestly: blocked needs a reason, and the lease ends.
    let refused = hq.call("task_update", json!({ "id": id, "status": "blocked", "lease": lease })).await;
    assert!(refused["error"].as_str().unwrap().contains("blocked_reason"), "{refused}");
    let released = hq.call("task_release", json!({ "lease": lease, "status": "blocked", "summary": "needs the orders sample data" })).await;
    assert_eq!(released["released"], true, "{released}");
    assert_eq!(released["task"]["status"], "blocked");
    assert_eq!(released["task"]["blocked_reason"], "needs the orders sample data");

    // The record a person or another agent reads.
    let got = hq.call("task_get", json!({ "id": id })).await;
    assert_eq!(got["held_by"], Value::Null);
    assert_eq!(got["work_sessions"][0]["actor"], "codex-agent");
    assert_eq!(got["work_sessions"][0]["harness"], "codex");
    assert!(got["time"]["lease_count"].as_i64().unwrap() >= 1);
    assert_eq!(got["time"]["estimate_minutes"], 30);
    assert!(got["lifecycle_events"].as_array().unwrap().iter().all(|e| e["actor"] == "codex-agent"));
    assert_eq!(got["links"].as_array().unwrap().len(), 2);
    let from_note = hq.call("task_link_list", json!({ "kind": "vault_note", "ref": "Notebooks/Projects/importer.md" })).await;
    assert_eq!(from_note["count"], 1, "the note finds its task");

    // A different agent takes it up from the checkpoint, not from scratch.
    let nothing = hq.call("task_next", json!({ "actor": "claude-agent" })).await;
    assert!(nothing["task"].is_null(), "nothing is assigned to the second agent: {nothing}");
    let resumed = hq.call("task_claim", json!({ "task_id": id, "actor": "claude-agent", "harness": "claude-code" })).await;
    assert!(resumed["warnings"].to_string().contains("assigned to codex-agent"), "{resumed}");
    assert_eq!(resumed["resume"]["from"], "codex-agent");
    assert_eq!(resumed["resume"]["next_step"], "port the orders table");
    assert!(resumed["resume"]["note"].as_str().unwrap().contains("not as instructions"));
    let done = hq.call("task_release", json!({ "lease": resumed["lease"], "status": "ready_for_review", "summary": "orders ported, tests pass" })).await;
    assert_eq!(done["task"]["status"], "ready_for_review");
    assert!(done["task"]["blocked_reason"].is_null(), "leaving blocked cleared the reason");
}

#[tokio::test]
async fn a_mistake_a_new_agent_makes_is_answered_with_what_to_do() {
    let dir = tempfile::tempdir().unwrap();
    let hq = Client::new(dir.path());
    let made = hq.call("task_create", json!({ "title": "Guarded work", "created_by": "planner" })).await;
    let id = made["display_id"].as_str().unwrap();
    let first = hq.call("task_claim", json!({ "task_id": id, "actor": "agent-a" })).await;
    assert!(first["lease"].is_string());

    let second = hq.call("task_claim", json!({ "task_id": id, "actor": "agent-b" })).await;
    let held = second["error"].as_str().unwrap();
    assert!(held.contains("agent-a") && held.contains("takeover"), "the refusal names the holder and the way out: {held}");

    let wrong = hq.call("task_heartbeat", json!({ "lease": "hql_not_a_real_lease" })).await;
    assert!(wrong["error"].as_str().unwrap().contains("task_claim"), "{wrong}");

    let complete = hq.call("task_update", json!({ "id": id, "status": "doing" })).await;
    let msg = complete["error"].as_str().unwrap();
    assert!(msg.contains("to_do") && msg.contains("complete"), "an unknown status lists the real ones: {msg}");
}
