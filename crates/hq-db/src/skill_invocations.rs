//! Database operations for skill invocation telemetry.
//!
//! Each time a skill is loaded (via `load_skill` tool or hint-based auto-load),
//! an invocation row is recorded. Outcome scores are backfilled later when
//! signal is available (AIDC acceptance, coordinator success, etc.).

use anyhow::Result;
use rusqlite::{Connection, params};

#[derive(Debug, Clone, PartialEq)]
pub enum InvocationTrigger {
    LoadSkill,
    AutoLoad,
    Other(String),
}

impl InvocationTrigger {
    pub fn as_str(&self) -> &str {
        match self {
            InvocationTrigger::LoadSkill => "load_skill",
            InvocationTrigger::AutoLoad => "auto_load",
            InvocationTrigger::Other(s) => s.as_str(),
        }
    }
}

/// Log a new skill invocation. Returns the generated id.
pub fn log_invocation(
    conn: &Connection,
    skill_name: &str,
    session_id: &str,
    trigger: InvocationTrigger,
) -> Result<String> {
    let id = format!(
        "inv_{}",
        uuid::Uuid::new_v4()
            .to_string()
            .split('-')
            .next()
            .unwrap_or("0")
    );
    conn.execute(
        "INSERT INTO skill_invocations (id, skill_name, session_id, trigger)
         VALUES (?1, ?2, ?3, ?4)",
        params![id, skill_name, session_id, trigger.as_str()],
    )?;
    Ok(id)
}

/// Backfill outcome score on all invocations for a given session.
/// Used when a downstream signal (AIDC accepted, coordinator success) becomes
/// available after the skill was loaded.
pub fn update_outcome_by_session(
    conn: &Connection,
    session_id: &str,
    outcome_score: f64,
    outcome_source: &str,
) -> Result<usize> {
    let updated = conn.execute(
        "UPDATE skill_invocations
            SET outcome_score = ?1,
                outcome_source = ?2,
                updated_at = datetime('now')
          WHERE session_id = ?3
            AND outcome_score IS NULL",
        params![outcome_score, outcome_source, session_id],
    )?;
    Ok(updated)
}

/// Count invocations for a given skill.
pub fn count_invocations(conn: &Connection, skill_name: &str) -> Result<u32> {
    let count: u32 = conn.query_row(
        "SELECT COUNT(*) FROM skill_invocations WHERE skill_name = ?1",
        params![skill_name],
        |row| row.get(0),
    )?;
    Ok(count)
}

/// Aggregate outcome score (mean of non-null scores) for a given skill.
pub fn mean_outcome(conn: &Connection, skill_name: &str) -> Result<Option<f64>> {
    let score: Option<f64> = conn.query_row(
        "SELECT AVG(outcome_score) FROM skill_invocations
          WHERE skill_name = ?1 AND outcome_score IS NOT NULL",
        params![skill_name],
        |row| row.get(0),
    )?;
    Ok(score)
}

/// A skill loaded recently, with how its latest load happened.
#[derive(Debug, Clone, PartialEq)]
pub struct RecentSkill {
    pub skill_name: String,
    pub trigger: String,
}

/// Distinct skills loaded in the last `hours`, newest first, across sessions.
/// The relay builds a fresh session per message, so a correction usually
/// arrives in a later session than the one that loaded the skill.
pub fn recent_skills(conn: &Connection, hours: u32, limit: u32) -> Result<Vec<RecentSkill>> {
    // SQLite fills bare columns from the row that holds the MAX, so `trigger` is the latest load's.
    let mut stmt = conn.prepare(
        "SELECT skill_name, trigger, MAX(loaded_at) AS last
           FROM skill_invocations
          WHERE loaded_at >= datetime('now', ?1)
          GROUP BY skill_name
          ORDER BY last DESC
          LIMIT ?2",
    )?;
    let rows = stmt.query_map(params![format!("-{hours} hours"), limit], |row| {
        Ok(RecentSkill { skill_name: row.get(0)?, trigger: row.get(1)? })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Set the outcome on the newest invocation of `skill_name` within the last
/// `hours`, overriding any earlier score: a later judgement is stronger evidence.
pub fn set_latest_outcome(
    conn: &Connection,
    skill_name: &str,
    hours: u32,
    outcome_score: f64,
    outcome_source: &str,
) -> Result<usize> {
    let updated = conn.execute(
        "UPDATE skill_invocations
            SET outcome_score = ?1, outcome_source = ?2, updated_at = datetime('now')
          WHERE id = (SELECT id FROM skill_invocations
                       WHERE skill_name = ?3 AND loaded_at >= datetime('now', ?4)
                       ORDER BY loaded_at DESC, rowid DESC LIMIT 1)",
        params![outcome_score, outcome_source, skill_name, format!("-{hours} hours")],
    )?;
    Ok(updated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;

    struct SkillInvocationRow {
        session_id: String,
        outcome_score: Option<f64>,
        outcome_source: Option<String>,
    }

    fn recent_invocations(
        conn: &Connection,
        skill_name: &str,
        limit: u32,
    ) -> Result<Vec<SkillInvocationRow>> {
        let mut stmt = conn.prepare(
            "SELECT session_id, outcome_score, outcome_source
               FROM skill_invocations
              WHERE skill_name = ?1
              ORDER BY loaded_at DESC
              LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![skill_name, limit], |row| {
            Ok(SkillInvocationRow {
                session_id: row.get(0)?,
                outcome_score: row.get(1)?,
                outcome_source: row.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    #[test]
    fn log_and_update_outcome_roundtrip() -> Result<()> {
        let db = Database::open_memory()?;
        db.with_conn(|conn| {
            let id = log_invocation(conn, "skill-a", "sess-1", InvocationTrigger::LoadSkill)?;
            assert!(id.starts_with("inv_"));

            let n = update_outcome_by_session(conn, "sess-1", 0.85, "aidc_accepted")?;
            assert_eq!(n, 1);

            let mean = mean_outcome(conn, "skill-a")?.unwrap();
            assert!((mean - 0.85).abs() < 1e-6);

            let count = count_invocations(conn, "skill-a")?;
            assert_eq!(count, 1);

            let recent = recent_invocations(conn, "skill-a", 5)?;
            assert_eq!(recent.len(), 1);
            assert_eq!(recent[0].outcome_source.as_deref(), Some("aidc_accepted"));

            Ok(())
        })
    }

    #[test]
    fn update_does_not_overwrite_existing_outcome() -> Result<()> {
        let db = Database::open_memory()?;
        db.with_conn(|conn| {
            log_invocation(conn, "skill-a", "sess-1", InvocationTrigger::LoadSkill)?;
            update_outcome_by_session(conn, "sess-1", 0.5, "first")?;
            let n = update_outcome_by_session(conn, "sess-1", 0.9, "second")?;
            assert_eq!(n, 0, "second update should skip rows with existing outcome");

            let recent = recent_invocations(conn, "skill-a", 5)?;
            assert_eq!(recent[0].outcome_score, Some(0.5));
            assert_eq!(recent[0].outcome_source.as_deref(), Some("first"));
            Ok(())
        })
    }

    #[test]
    fn recent_skills_span_sessions_and_keep_the_latest_trigger() -> Result<()> {
        let db = Database::open_memory()?;
        db.with_conn(|conn| {
            log_invocation(conn, "sheets", "sess-1", InvocationTrigger::AutoLoad)?;
            log_invocation(conn, "sheets", "sess-2", InvocationTrigger::LoadSkill)?;
            log_invocation(conn, "docs", "sess-3", InvocationTrigger::AutoLoad)?;
            conn.execute(
                "UPDATE skill_invocations SET loaded_at = datetime('now', '-1 hours') WHERE session_id = 'sess-1'",
                [],
            )?;
            conn.execute(
                "INSERT INTO skill_invocations (id, skill_name, session_id, loaded_at)
                 VALUES ('old', 'stale', 's', datetime('now', '-2 days'))",
                [],
            )?;
            let recent = recent_skills(conn, 6, 5)?;
            let names: Vec<&str> = recent.iter().map(|r| r.skill_name.as_str()).collect();
            assert_eq!(names.len(), 2);
            assert!(!names.contains(&"stale"));
            let sheets = recent.iter().find(|r| r.skill_name == "sheets").unwrap();
            assert_eq!(sheets.trigger, "load_skill");
            Ok(())
        })
    }

    #[test]
    fn latest_outcome_overrides_only_the_newest_invocation() -> Result<()> {
        let db = Database::open_memory()?;
        db.with_conn(|conn| {
            log_invocation(conn, "sheets", "sess-1", InvocationTrigger::LoadSkill)?;
            log_invocation(conn, "sheets", "sess-2", InvocationTrigger::LoadSkill)?;
            update_outcome_by_session(conn, "sess-2", 1.0, "session_result")?;
            assert_eq!(set_latest_outcome(conn, "sheets", 6, 0.0, "review")?, 1);
            let rows = recent_invocations(conn, "sheets", 5)?;
            let s2 = rows.iter().find(|r| r.session_id == "sess-2").unwrap();
            assert_eq!(s2.outcome_score, Some(0.0));
            assert_eq!(s2.outcome_source.as_deref(), Some("review"));
            let s1 = rows.iter().find(|r| r.session_id == "sess-1").unwrap();
            assert_eq!(s1.outcome_score, None);
            assert_eq!(set_latest_outcome(conn, "missing", 6, 1.0, "review")?, 0);
            Ok(())
        })
    }
}
