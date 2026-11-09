//! The call behind an owner-approval inbox item (`sql/113_owner_approvals.sql`). Owner approval
//! writes it with the item; the notifications API reads it to quote the call verbatim.

use anyhow::Result;
use rusqlite::{OptionalExtension, params};

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

/// The tool, arguments and key of the approval behind `item_id`.
pub fn call_of(
    db: &Database,
    item_id: &str,
) -> Result<Option<(String, serde_json::Value, String)>> {
    db.with_conn(|c| {
        let row: Option<(String, String, Option<String>)> = c
            .query_row(
                "SELECT o.tool, o.args, v.dedup_key FROM owner_approvals o
                 JOIN value_items v ON v.id = o.item_id WHERE o.item_id = ?1",
                params![item_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        Ok(row.and_then(|(tool, args, key)| Some((tool, serde_json::from_str(&args).ok()?, key?))))
    })
}

/// Requests still waiting whose key starts with `prefix`, for one tool: their item ids and calls.
pub fn waiting_calls(
    db: &Database,
    prefix: &str,
    tool: &str,
) -> Result<Vec<(String, serde_json::Value)>> {
    db.with_conn(|c| {
        let mut stmt = c.prepare(
            "SELECT o.item_id, o.args FROM owner_approvals o
             JOIN value_items v ON v.id = o.item_id
             WHERE o.tool = ?1 AND substr(v.dedup_key, 1, length(?2)) = ?2
               AND v.state IN ('pending','routed','delivered')",
        )?;
        let rows = stmt.query_map(params![tool, prefix], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        let mut calls = Vec::new();
        for row in rows {
            let (id, args) = row?;
            if let Ok(args) = serde_json::from_str(&args) {
                calls.push((id, args));
            }
        }
        Ok(calls)
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
