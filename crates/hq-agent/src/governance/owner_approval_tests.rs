use super::*;
use serde_json::json;

fn approval() -> (Arc<Database>, Arc<OwnerApproval>) {
    let db = Arc::new(Database::open_memory().unwrap());
    let approval = OwnerApproval::new(db.clone(), "web:t1");
    (db, approval)
}

/// The newest request for `tool` in the test chat.
fn request_id(db: &Database, tool: &str) -> String {
    let like = format!("{KEY_PREFIX}:web:t1:{tool}:%");
    db.with_conn(|c| {
        Ok(c.query_row(
            "SELECT id FROM value_items WHERE dedup_key LIKE ?1 ORDER BY rowid DESC LIMIT 1",
            [like],
            |r| r.get::<_, String>(0),
        )?)
    })
    .unwrap()
}

#[tokio::test]
async fn a_request_keeps_its_call_without_the_runtime_keys() {
    let (db, approval) = approval();
    let args = json!({"id": "t-1", "_hq_caller_session": "s-9"});
    approval.check("task_delete", &args).await;
    let id = request_id(&db, "task_delete");
    let (tool, held): (String, String) = db
        .with_conn(|c| {
            Ok(c.query_row(
                "SELECT tool, args FROM owner_approvals WHERE item_id = ?1",
                [&id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?)
        })
        .unwrap();
    assert_eq!(tool, "task_delete");
    assert_eq!(held, r#"{"id":"t-1"}"#);
}

#[tokio::test]
async fn an_approved_call_runs_once_with_exactly_its_arguments() {
    let (db, approval) = approval();
    let args = json!({"command": "rm -rf build"});
    let first = approval.check("bash", &args).await;
    assert!(first.unwrap().contains("Waiting for the owner"));
    assert!(value_items::approve_by_id(&db, &request_id(&db, "bash")).unwrap());

    let changed = json!({"command": "rm -rf /"});
    assert!(
        approval.check("bash", &changed).await.is_some(),
        "an approval covers only the arguments it was given for"
    );
    let reordered = json!({"_hq_native_caller": "x", "command": "rm -rf build"});
    assert!(approval.check("bash", &reordered).await.is_none());
    assert!(
        approval.check("bash", &args).await.is_some(),
        "an approval is spent by the one call it lets through"
    );
}

#[tokio::test]
async fn a_declined_call_is_reported_and_not_asked_again() {
    let (db, approval) = approval();
    let args = json!({"file_path": "/tmp/x", "content": "y"});
    approval.check("write_file", &args).await;
    let id = request_id(&db, "write_file");
    assert!(value_items::dismiss_by_id(&db, &id).unwrap());
    let reply = approval.check("write_file", &args).await.unwrap();
    assert!(reply.contains("declined"), "{reply}");
    assert_eq!(request_id(&db, "write_file"), id, "no second request");
}

#[tokio::test]
async fn an_expired_request_is_asked_again() {
    let (db, approval) = approval();
    let args = json!({"command": "make"});
    approval.check("bash", &args).await;
    let id = request_id(&db, "bash");
    db.with_conn(|c| {
        c.execute(
            "UPDATE value_items SET expires_at = '2000-01-01T00:00:00Z' WHERE id = ?1",
            [&id],
        )?;
        Ok(())
    })
    .unwrap();
    approval.check("bash", &args).await;
    assert_ne!(request_id(&db, "bash"), id, "a fresh request replaces it");
}

#[tokio::test]
async fn a_chat_cannot_flood_the_inbox() {
    let (_db, approval) = approval();
    for n in 0..MAX_WAITING_APPROVALS {
        approval.check("bash", &json!({"command": n})).await;
    }
    let reply = approval
        .check("bash", &json!({"command": "one more"}))
        .await
        .unwrap();
    assert!(reply.contains("already waiting"), "{reply}");
}

#[test]
fn reads_and_notes_never_need_the_owner() {
    assert!(needs_owner("bash", "coding", &json!({})));
    assert!(needs_owner(
        "config_manage",
        "config",
        &json!({"action": "set"})
    ));
    assert!(!needs_owner(
        "config_manage",
        "config",
        &json!({"action": "get"})
    ));
    assert!(needs_owner(
        "mail_call",
        hq_tools::remote_mcp::REMOTE_MCP_CATEGORY,
        &json!({})
    ));
    assert!(!needs_owner("vault_write_note", "vault", &json!({})));
    assert!(needs_owner(
        "vault_export",
        "convert",
        &json!({"path": "a.md", "output": "/home/u/.zshrc"})
    ));
    assert!(!needs_owner(
        "vault_export",
        "convert",
        &json!({"path": "a.md"})
    ));
    assert!(needs_owner("harness_session_spawn", "harness", &json!({})));
    assert!(!needs_owner("read_file", "coding", &json!({})));
}

#[tokio::test]
async fn a_spoken_question_is_quoted_at_the_top_of_its_request() {
    let db = Arc::new(Database::open_memory().unwrap());
    let approval =
        OwnerApproval::for_turn(db.clone(), "web:t1", Some("Delete the invoice draft task."));
    approval.check("task_delete", &json!({"id": "t-9"})).await;
    let id = request_id(&db, "task_delete");
    let body: String = db
        .with_conn(|c| {
            Ok(
                c.query_row("SELECT body FROM value_items WHERE id = ?1", [&id], |r| {
                    r.get(0)
                })?,
            )
        })
        .unwrap();
    assert!(
        body.starts_with("You said: \u{201c}Delete the invoice draft task.\u{201d}"),
        "{body}"
    );
}
