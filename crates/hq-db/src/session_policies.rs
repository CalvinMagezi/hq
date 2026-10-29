//! The command policy each session started with (`sql/105_session_policies.sql`).

use anyhow::Result;
use hq_core::command_policy::Policy;
use rusqlite::{Connection, OptionalExtension, params};

/// Stores the policy `session_id` started with. A resume stores it again.
pub fn record(c: &Connection, session_id: &str, policy: &Policy) -> Result<()> {
    c.execute(
        "INSERT INTO session_policies (session_id, name, rules) VALUES (?1, ?2, ?3) \
         ON CONFLICT(session_id) DO UPDATE SET name = excluded.name, rules = excluded.rules",
        params![session_id, policy.name, serde_json::to_string(policy)?],
    )?;
    Ok(())
}

/// The policy `session_id` started with; `None` when it has none (or its rules no longer
/// read, which leaves every dialog to the owner).
pub fn get(c: &Connection, session_id: &str) -> Result<Option<Policy>> {
    let rules: Option<String> = c
        .query_row(
            "SELECT rules FROM session_policies WHERE session_id = ?1",
            params![session_id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(rules.and_then(|r| serde_json::from_str(&r).ok()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;

    #[test]
    fn a_policy_is_stored_whole_and_read_back() {
        let db = Database::open_memory().unwrap();
        let builder = hq_core::command_policy::resolve("builder", &Default::default())
            .unwrap()
            .unwrap();
        db.with_conn(|c| {
            assert_eq!(get(c, "hs-1")?, None);
            record(c, "hs-1", &builder)?;
            assert_eq!(get(c, "hs-1")?.as_ref(), Some(&builder));
            Ok(())
        })
        .unwrap();
    }
}
