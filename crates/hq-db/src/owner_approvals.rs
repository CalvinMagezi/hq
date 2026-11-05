//! The call behind an owner-approval inbox item (`sql/113_owner_approvals.sql`). Owner approval
//! writes it with the item; the notifications API reads it to quote the call verbatim.

use anyhow::Result;
use rusqlite::params;

use crate::Database;

/// Rows older than this are dropped when a new one is written; their items expired long ago.
const KEEP_DAYS: i64 = 30;

/// Records the call `item_id` asks about. `args` is the JSON the runtime holds.
pub fn record(db: &Database, item_id: &str, tool: &str, args: &serde_json::Value) -> Result<()> {
    let args = serde_json::to_string(args)?;
    db.with_conn(|c| {
        c.execute(
            "DELETE FROM owner_approvals WHERE created_at < datetime('now', ?1)",
            params![format!("-{KEEP_DAYS} days")],
        )?;
        c.execute(
            "INSERT OR IGNORE INTO owner_approvals (item_id, tool, args) VALUES (?1, ?2, ?3)",
            params![item_id, tool, args],
        )?;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_record_for_the_same_item_keeps_the_first() {
        let db = Database::open_memory().unwrap();
        record(&db, "i1", "task_delete", &serde_json::json!({"id": "a"})).unwrap();
        record(&db, "i1", "bash", &serde_json::json!({"command": "rm x"})).unwrap();
        let (tool, args): (String, String) = db
            .with_conn(|c| {
                Ok(c.query_row(
                    "SELECT tool, args FROM owner_approvals WHERE item_id = 'i1'",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?)
            })
            .unwrap();
        assert_eq!(tool, "task_delete");
        assert_eq!(args, r#"{"id":"a"}"#);
    }
}
