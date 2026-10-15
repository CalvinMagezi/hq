//! Database operations for skill invocation telemetry.
//!
//! Each time a skill is loaded (via `load_skill` tool or hint-based auto-load),
//! an invocation row is recorded. Outcome scores are backfilled later when
//! signal is available (a task being accepted or reopened, a post-session review, how the
//! session ended).
//!
//! A row holds ids, a hash and values from small fixed lists. It never holds a prompt, a
//! document or any free text: the evidence the flywheel reads must not be a copy of what people
//! asked for.

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

/// A coarse category for the work a skill was loaded for. Never free text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    Code,
    Research,
    Writing,
    Review,
    Planning,
    Operations,
    General,
}

impl Purpose {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Code => "code",
            Self::Research => "research",
            Self::Writing => "writing",
            Self::Review => "review",
            Self::Planning => "planning",
            Self::Operations => "operations",
            Self::General => "general",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "code" => Self::Code,
            "research" => Self::Research,
            "writing" => Self::Writing,
            "review" => Self::Review,
            "planning" => Self::Planning,
            "operations" => Self::Operations,
            "general" => Self::General,
            _ => return None,
        })
    }

    /// The category a task's tags point at: the first tag that names one, else `General`. Only the
    /// tags are read, never the task's title or text.
    pub fn from_tags(tags: &[String]) -> Self {
        tags.iter()
            .find_map(|tag| match tag.trim().to_ascii_lowercase().as_str() {
                "code" | "dev" | "bug" | "bugfix" | "feature" | "refactor" => Some(Self::Code),
                "research" | "analysis" | "investigation" => Some(Self::Research),
                "docs" | "documentation" | "writing" | "content" => Some(Self::Writing),
                "review" | "qa" | "audit" => Some(Self::Review),
                "planning" | "roadmap" | "strategy" | "scoping" => Some(Self::Planning),
                "ops" | "infra" | "deploy" | "operations" | "devops" | "maintenance" => {
                    Some(Self::Operations)
                }
                _ => None,
            })
            .unwrap_or(Self::General)
    }
}

/// Whether a skill was only offered, was loaded into the session, or what the post-session review
/// made of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Used {
    Suggested,
    Loaded,
    Helped,
    Hurt,
    Unused,
}

impl Used {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Suggested => "suggested",
            Self::Loaded => "loaded",
            Self::Helped => "helped",
            Self::Hurt => "hurt",
            Self::Unused => "unused",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "suggested" => Self::Suggested,
            "loaded" => Self::Loaded,
            "helped" => Self::Helped,
            "hurt" => Self::Hurt,
            "unused" => Self::Unused,
            _ => return None,
        })
    }
}

/// What an observable acceptance signal said about the work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutcomeStatus {
    Accepted,
    Reworked,
    Rejected,
    Unknown,
}

impl OutcomeStatus {
    /// The score an outcome counts as in `mean_outcome`. `Unknown` says nothing, so it has none.
    pub fn score(self) -> Option<f64> {
        match self {
            Self::Accepted => Some(1.0),
            Self::Reworked => Some(0.5),
            Self::Rejected => Some(0.0),
            Self::Unknown => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Reworked => "reworked",
            Self::Rejected => "rejected",
            Self::Unknown => "unknown",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "accepted" => Self::Accepted,
            "reworked" => Self::Reworked,
            "rejected" => Self::Rejected,
            "unknown" => Self::Unknown,
            _ => return None,
        })
    }
}

/// What is known about an invocation beyond which skill ran in which session. Every field is
/// optional; `None` is stored as NULL and means "not recorded", never a guess.
#[derive(Debug, Clone, Copy, Default)]
pub struct Context<'a> {
    /// SHA-256 (hex) of the skill's SKILL.md at load, the digest `skills/_audit.jsonl` keeps.
    pub skill_hash: Option<&'a str>,
    /// The HQ task the session was working on.
    pub task_id: Option<&'a str>,
    pub purpose: Option<Purpose>,
    pub used: Option<Used>,
}

/// Log a new skill invocation. Returns the generated id. A load is recorded as `loaded`.
pub fn log_invocation(
    conn: &Connection,
    skill_name: &str,
    session_id: &str,
    trigger: InvocationTrigger,
) -> Result<String> {
    log_invocation_with(
        conn,
        skill_name,
        session_id,
        trigger,
        &Context {
            used: Some(Used::Loaded),
            ..Default::default()
        },
    )
}

/// [`log_invocation`] with the skill's version, the task and the purpose when they are known.
pub fn log_invocation_with(
    conn: &Connection,
    skill_name: &str,
    session_id: &str,
    trigger: InvocationTrigger,
    ctx: &Context<'_>,
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
        "INSERT INTO skill_invocations
            (id, skill_name, session_id, trigger, skill_hash, task_id, purpose, used)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            id,
            skill_name,
            session_id,
            trigger.as_str(),
            ctx.skill_hash,
            ctx.task_id,
            ctx.purpose.map(Purpose::as_str),
            ctx.used.map(Used::as_str),
        ],
    )?;
    Ok(id)
}

/// Backfill outcome score on all invocations for a given session.
/// Used when a downstream signal (the session ending, an acceptance) becomes available after
/// the skill was loaded.
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
///
/// Includes the invocations whose raw rows have aged out into the monthly roll-ups.
pub fn count_invocations(conn: &Connection, skill_name: &str) -> Result<u32> {
    let count: u32 = conn.query_row(
        "SELECT (SELECT COUNT(*) FROM skill_invocations WHERE skill_name = ?1)
              + COALESCE((SELECT SUM(invocations) FROM skill_invocation_rollups
                           WHERE skill_name = ?1), 0)",
        params![skill_name],
        |row| row.get(0),
    )?;
    Ok(count)
}

/// Aggregate outcome score (mean of non-null scores) for a given skill.
///
/// Includes the scores of invocations whose raw rows have aged out into the monthly roll-ups.
pub fn mean_outcome(conn: &Connection, skill_name: &str) -> Result<Option<f64>> {
    let score: Option<f64> = conn.query_row(
        "SELECT (COALESCE((SELECT SUM(outcome_score) FROM skill_invocations
                            WHERE skill_name = ?1), 0)
               + COALESCE((SELECT SUM(score_sum) FROM skill_invocation_rollups
                            WHERE skill_name = ?1), 0))
              / NULLIF((SELECT COUNT(outcome_score) FROM skill_invocations
                         WHERE skill_name = ?1)
                     + COALESCE((SELECT SUM(scored) FROM skill_invocation_rollups
                                  WHERE skill_name = ?1), 0), 0)",
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
        Ok(RecentSkill {
            skill_name: row.get(0)?,
            trigger: row.get(1)?,
        })
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
    // An outcome someone observed (a task accepted or reopened, a run review, a correction) is
    // stronger evidence than an opinion formed from the transcript, and is left as it is.
    let updated = conn.execute(
        "UPDATE skill_invocations
            SET outcome_score = ?1, outcome_source = ?2, updated_at = datetime('now')
          WHERE id = (SELECT id FROM skill_invocations
                       WHERE skill_name = ?3 AND loaded_at >= datetime('now', ?4)
                       ORDER BY loaded_at DESC, rowid DESC LIMIT 1)
            AND COALESCE(outcome_source, '') NOT IN ('task_acceptance', 'run_review', 'note_review')",
        params![
            outcome_score,
            outcome_source,
            skill_name,
            format!("-{hours} hours")
        ],
    )?;
    Ok(updated)
}

/// Record the post-session review's verdict on the newest invocation of `skill_name` within the
/// last `hours`. A verdict that carries a score (`helped`, `hurt`) overrides the score like
/// [`set_latest_outcome`]; `unused` says the skill was loaded and not applied, and leaves the score
/// alone.
pub fn set_latest_verdict(
    conn: &Connection,
    skill_name: &str,
    hours: u32,
    used: Used,
    outcome: Option<(f64, &str)>,
) -> Result<usize> {
    let (score, source) = match outcome {
        Some((score, source)) => (Some(score), Some(source)),
        None => (None, None),
    };
    // `used` is always recorded; the score and its source change only when the verdict carries
    // one and the existing outcome is not an observed one.
    let updated = conn.execute(
        "UPDATE skill_invocations
            SET used = ?1,
                outcome_score = CASE
                    WHEN COALESCE(outcome_source, '') IN ('task_acceptance', 'run_review', 'note_review')
                    THEN outcome_score ELSE COALESCE(?2, outcome_score) END,
                outcome_source = CASE
                    WHEN COALESCE(outcome_source, '') IN ('task_acceptance', 'run_review', 'note_review')
                    THEN outcome_source ELSE COALESCE(?3, outcome_source) END,
                updated_at = datetime('now')
          WHERE id = (SELECT id FROM skill_invocations
                       WHERE skill_name = ?4 AND loaded_at >= datetime('now', ?5)
                       ORDER BY loaded_at DESC, rowid DESC LIMIT 1)",
        params![
            used.as_str(),
            score,
            source,
            skill_name,
            format!("-{hours} hours")
        ],
    )?;
    Ok(updated)
}

/// How long a raw invocation row is kept, in days. Past this it is rolled up into counts.
pub const RETENTION_DAYS: u32 = 90;

/// What a retention pass did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Retained {
    /// Raw rows folded into the monthly roll-ups and removed.
    pub rolled_up: usize,
}

/// Fold raw invocation rows older than `days` into `skill_invocation_rollups` (counts and a score
/// sum per skill, version and month, no ids) and delete them. The skills and the audit trail are
/// not touched. Safe to run again: it only ever moves rows that are still there.
pub fn apply_retention(conn: &Connection, days: u32) -> Result<Retained> {
    let cutoff = format!("-{days} days");
    conn.execute_batch("SAVEPOINT skill_retention")?;
    let result = (|| -> Result<usize> {
        conn.execute(
            "INSERT INTO skill_invocation_rollups
               (skill_name, skill_hash, month, invocations, scored, score_sum,
                accepted, reworked, rejected, helped, hurt, unused)
             SELECT skill_name, COALESCE(skill_hash, ''), strftime('%Y-%m', loaded_at),
                    COUNT(*), COUNT(outcome_score), COALESCE(SUM(outcome_score), 0),
                    COALESCE(SUM(outcome_status = 'accepted'), 0),
                    COALESCE(SUM(outcome_status = 'reworked'), 0),
                    COALESCE(SUM(outcome_status = 'rejected'), 0),
                    COALESCE(SUM(used = 'helped'), 0),
                    COALESCE(SUM(used = 'hurt'), 0),
                    COALESCE(SUM(used = 'unused'), 0)
               FROM skill_invocations
              WHERE loaded_at < datetime('now', ?1)
              GROUP BY skill_name, COALESCE(skill_hash, ''), strftime('%Y-%m', loaded_at)
             ON CONFLICT (skill_name, skill_hash, month) DO UPDATE SET
                invocations = invocations + excluded.invocations,
                scored = scored + excluded.scored,
                score_sum = score_sum + excluded.score_sum,
                accepted = accepted + excluded.accepted,
                reworked = reworked + excluded.reworked,
                rejected = rejected + excluded.rejected,
                helped = helped + excluded.helped,
                hurt = hurt + excluded.hurt,
                unused = unused + excluded.unused",
            params![cutoff],
        )?;
        Ok(conn.execute(
            "DELETE FROM skill_invocations WHERE loaded_at < datetime('now', ?1)",
            params![cutoff],
        )?)
    })();
    match result {
        Ok(rolled_up) => {
            conn.execute_batch("RELEASE skill_retention")?;
            Ok(Retained { rolled_up })
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK TO skill_retention; RELEASE skill_retention");
            Err(e)
        }
    }
}

/// What to remove when the owner asks for telemetry to be deleted.
#[derive(Debug, Clone, Copy)]
pub enum Purge<'a> {
    /// One invocation, by its id.
    Record(&'a str),
    /// Invocations loaded in a date range, optionally of one skill. `from` is inclusive and
    /// `before` exclusive, both `YYYY-MM-DD` (UTC). At least one of the three must be given:
    /// there is no way to say "everything" by leaving the filters out.
    Range {
        skill: Option<&'a str>,
        from: Option<&'a str>,
        before: Option<&'a str>,
    },
}

/// What a purge removed.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Purged {
    /// Raw invocation rows.
    pub records: usize,
    /// Monthly roll-ups that lie entirely inside the range. They hold counts only, but a month
    /// the owner asked to be gone is gone.
    pub rollups: usize,
}

fn date(label: &str, text: &str) -> Result<String> {
    let parsed = chrono::NaiveDate::parse_from_str(text.trim(), "%Y-%m-%d")
        .map_err(|_| anyhow::anyhow!("{label} must be a date like 2026-01-31, got '{text}'"))?;
    Ok(parsed.format("%Y-%m-%d").to_string())
}

/// Delete telemetry on request. Only invocation rows and roll-ups go: the skills, their
/// versions and the audit trail are not touched.
pub fn purge(conn: &Connection, what: Purge<'_>) -> Result<Purged> {
    match what {
        Purge::Record(id) => {
            let records =
                conn.execute("DELETE FROM skill_invocations WHERE id = ?1", params![id])?;
            Ok(Purged {
                records,
                rollups: 0,
            })
        }
        Purge::Range {
            skill,
            from,
            before,
        } => {
            if skill.is_none() && from.is_none() && before.is_none() {
                anyhow::bail!(
                    "give a skill, a start date or an end date: a purge is never unbounded"
                );
            }
            let from = from.map(|d| date("--from", d)).transpose()?;
            let before = before.map(|d| date("--before", d)).transpose()?;
            if let (Some(f), Some(b)) = (&from, &before)
                && f >= b
            {
                anyhow::bail!("the start date must be before the end date");
            }
            // Dates compare as text against `loaded_at` ("YYYY-MM-DD HH:MM:SS").
            let records = conn.execute(
                "DELETE FROM skill_invocations
                  WHERE (?1 IS NULL OR skill_name = ?1)
                    AND (?2 IS NULL OR loaded_at >= ?2)
                    AND (?3 IS NULL OR loaded_at < ?3)",
                params![skill, from, before],
            )?;
            // A roll-up month goes only when all of it is inside the range.
            let rollups = conn.execute(
                "DELETE FROM skill_invocation_rollups
                  WHERE (?1 IS NULL OR skill_name = ?1)
                    AND (?2 IS NULL OR date(month || '-01') >= ?2)
                    AND (?3 IS NULL OR date(month || '-01', '+1 month') <= ?3)",
                params![skill, from, before],
            )?;
            Ok(Purged { records, rollups })
        }
    }
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

            let n = update_outcome_by_session(conn, "sess-1", 0.85, "task_accepted")?;
            assert_eq!(n, 1);

            let mean = mean_outcome(conn, "skill-a")?.unwrap();
            assert!((mean - 0.85).abs() < 1e-6);

            let count = count_invocations(conn, "skill-a")?;
            assert_eq!(count, 1);

            let recent = recent_invocations(conn, "skill-a", 5)?;
            assert_eq!(recent.len(), 1);
            assert_eq!(recent[0].outcome_source.as_deref(), Some("task_accepted"));

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

    #[test]
    fn a_load_records_the_version_task_and_purpose_it_was_given() -> Result<()> {
        let db = Database::open_memory()?;
        db.with_conn(|conn| {
            let id = log_invocation_with(
                conn,
                "sheets",
                "sess-1",
                InvocationTrigger::AutoLoad,
                &Context {
                    skill_hash: Some("ab12"),
                    task_id: Some("tk-1"),
                    purpose: Some(Purpose::Code),
                    used: Some(Used::Loaded),
                },
            )?;
            let row: (String, String, String, String) = conn.query_row(
                "SELECT skill_hash, task_id, purpose, used FROM skill_invocations WHERE id = ?1",
                [&id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )?;
            assert_eq!(
                row,
                ("ab12".into(), "tk-1".into(), "code".into(), "loaded".into())
            );
            Ok(())
        })
    }

    #[test]
    fn the_plain_log_call_records_a_load_and_nothing_it_does_not_know() -> Result<()> {
        let db = Database::open_memory()?;
        db.with_conn(|conn| {
            let id = log_invocation(conn, "sheets", "sess-1", InvocationTrigger::LoadSkill)?;
            let row: (
                Option<String>,
                Option<String>,
                Option<String>,
                Option<String>,
            ) = conn.query_row(
                "SELECT skill_hash, task_id, purpose, used FROM skill_invocations WHERE id = ?1",
                [&id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )?;
            assert_eq!(row, (None, None, None, Some("loaded".into())));
            Ok(())
        })
    }

    #[test]
    fn the_table_has_no_column_that_could_hold_a_prompt_or_a_document() -> Result<()> {
        let db = Database::open_memory()?;
        let columns: Vec<String> = db.with_conn(|conn| {
            let mut stmt =
                conn.prepare("SELECT name FROM pragma_table_info('skill_invocations')")?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        })?;
        // Every column is an id, a hash, a timestamp, a score, or a value from a fixed list.
        let expected = [
            "id",
            "skill_name",
            "session_id",
            "trigger",
            "loaded_at",
            "outcome_score",
            "outcome_source",
            "updated_at",
            "skill_hash",
            "task_id",
            "purpose",
            "used",
            "outcome_status",
            "outcome_ref",
        ];
        let mut got = columns.clone();
        got.sort();
        let mut want: Vec<String> = expected.iter().map(|s| s.to_string()).collect();
        want.sort();
        assert_eq!(
            got, want,
            "a new column needs a decision about what it may hold"
        );
        Ok(())
    }

    #[test]
    fn a_value_outside_the_fixed_lists_is_refused_by_the_database() -> Result<()> {
        let db = Database::open_memory()?;
        db.with_conn(|conn| {
            log_invocation(conn, "sheets", "sess-1", InvocationTrigger::LoadSkill)?;
            for (column, value) in [
                ("purpose", "please summarise my private contract"),
                ("used", "maybe"),
                ("outcome_status", "pretty good"),
            ] {
                let result = conn.execute(
                    &format!("UPDATE skill_invocations SET {column} = ?1"),
                    [value],
                );
                assert!(result.is_err(), "{column} accepted {value:?}");
            }
            Ok(())
        })
    }

    #[test]
    fn purpose_comes_from_the_tags_and_never_from_the_text_of_a_task() {
        let tags = |t: &[&str]| t.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            Purpose::from_tags(&tags(&["agent-hq", "Bugfix"])),
            Purpose::Code
        );
        assert_eq!(Purpose::from_tags(&tags(&["research"])), Purpose::Research);
        assert_eq!(Purpose::from_tags(&tags(&["docs"])), Purpose::Writing);
        assert_eq!(Purpose::from_tags(&tags(&["qa"])), Purpose::Review);
        assert_eq!(Purpose::from_tags(&tags(&["roadmap"])), Purpose::Planning);
        assert_eq!(Purpose::from_tags(&tags(&["deploy"])), Purpose::Operations);
        assert_eq!(
            Purpose::from_tags(&tags(&["something-else"])),
            Purpose::General
        );
        assert_eq!(Purpose::from_tags(&[]), Purpose::General);
        for p in [
            Purpose::Code,
            Purpose::Research,
            Purpose::Writing,
            Purpose::Review,
            Purpose::Planning,
            Purpose::Operations,
            Purpose::General,
        ] {
            assert_eq!(Purpose::parse(p.as_str()), Some(p));
        }
    }

    #[test]
    fn a_verdict_sets_used_and_only_a_scored_one_moves_the_score() -> Result<()> {
        let db = Database::open_memory()?;
        db.with_conn(|conn| {
            log_invocation(conn, "sheets", "sess-1", InvocationTrigger::LoadSkill)?;
            update_outcome_by_session(conn, "sess-1", 1.0, "session_result")?;

            // Not applied: recorded, score untouched.
            assert_eq!(
                set_latest_verdict(conn, "sheets", 6, Used::Unused, None)?,
                1
            );
            let (used, score, source): (String, f64, String) = conn.query_row(
                "SELECT used, outcome_score, outcome_source FROM skill_invocations",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?;
            assert_eq!(
                (used.as_str(), score, source.as_str()),
                ("unused", 1.0, "session_result")
            );

            // Hurt: recorded and the score is overridden.
            set_latest_verdict(conn, "sheets", 6, Used::Hurt, Some((0.0, "review")))?;
            let (used, score, source): (String, f64, String) = conn.query_row(
                "SELECT used, outcome_score, outcome_source FROM skill_invocations",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?;
            assert_eq!(
                (used.as_str(), score, source.as_str()),
                ("hurt", 0.0, "review")
            );
            Ok(())
        })
    }

    /// A load `days_ago` days back, with an outcome score and status.
    fn aged(
        conn: &Connection,
        skill: &str,
        days_ago: i64,
        score: Option<f64>,
        status: Option<&str>,
    ) -> Result<String> {
        let id = log_invocation_with(
            conn,
            skill,
            "sess",
            InvocationTrigger::LoadSkill,
            &Context {
                skill_hash: Some("v1"),
                used: Some(Used::Loaded),
                ..Default::default()
            },
        )?;
        conn.execute(
            "UPDATE skill_invocations
                SET loaded_at = datetime('now', ?2), outcome_score = ?3, outcome_status = ?4
              WHERE id = ?1",
            params![id, format!("-{days_ago} days"), score, status],
        )?;
        Ok(id)
    }

    fn raw_count(conn: &Connection) -> i64 {
        conn.query_row("SELECT COUNT(*) FROM skill_invocations", [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn old_rows_become_counts_and_the_aggregates_do_not_move() -> Result<()> {
        let db = Database::open_memory()?;
        db.with_conn(|conn| {
            aged(conn, "sheets", 200, Some(1.0), Some("accepted"))?;
            aged(conn, "sheets", 190, Some(0.0), Some("rejected"))?;
            aged(conn, "sheets", 100, None, None)?;
            aged(conn, "sheets", 5, Some(0.5), Some("reworked"))?;
            aged(conn, "docs", 3, Some(1.0), Some("accepted"))?;
            let count_before = count_invocations(conn, "sheets")?;
            let mean_before = mean_outcome(conn, "sheets")?;

            let kept = apply_retention(conn, RETENTION_DAYS)?;

            assert_eq!(kept, Retained { rolled_up: 3 });
            assert_eq!(raw_count(conn), 2, "the recent rows stay");
            assert_eq!(count_invocations(conn, "sheets")?, count_before);
            assert_eq!(count_invocations(conn, "sheets")?, 4);
            let mean_after = mean_outcome(conn, "sheets")?.unwrap();
            assert!((mean_after - mean_before.unwrap()).abs() < 1e-9);
            assert!((mean_after - 0.5).abs() < 1e-9, "(1 + 0 + 0.5) / 3");

            // The rolled-up counts by month.
            let (invocations, scored, accepted, rejected): (i64, i64, i64, i64) = conn.query_row(
                "SELECT SUM(invocations), SUM(scored), SUM(accepted), SUM(rejected)
                   FROM skill_invocation_rollups WHERE skill_name = 'sheets'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )?;
            assert_eq!((invocations, scored, accepted, rejected), (3, 2, 1, 1));
            Ok(())
        })
    }

    #[test]
    fn a_second_pass_does_nothing_and_a_later_one_adds_to_the_same_month() -> Result<()> {
        let db = Database::open_memory()?;
        db.with_conn(|conn| {
            aged(conn, "sheets", 200, Some(1.0), Some("accepted"))?;
            assert_eq!(apply_retention(conn, 90)?.rolled_up, 1);
            assert_eq!(apply_retention(conn, 90)?.rolled_up, 0);

            // Another row from the same month ages out later.
            let id = aged(conn, "sheets", 200, Some(0.0), Some("rejected"))?;
            conn.execute(
                "UPDATE skill_invocations SET loaded_at = (SELECT MIN(loaded_at) FROM skill_invocations)
                  WHERE id = ?1",
                [&id],
            )?;
            assert_eq!(apply_retention(conn, 90)?.rolled_up, 1);
            let months: i64 =
                conn.query_row("SELECT COUNT(*) FROM skill_invocation_rollups", [], |r| r.get(0))?;
            assert_eq!(months, 1, "same skill, version and month is one row");
            assert_eq!(count_invocations(conn, "sheets")?, 2);
            Ok(())
        })
    }

    #[test]
    fn a_rollup_holds_nothing_that_identifies_a_use() -> Result<()> {
        let db = Database::open_memory()?;
        let mut columns: Vec<String> = db.with_conn(|conn| {
            let mut stmt =
                conn.prepare("SELECT name FROM pragma_table_info('skill_invocation_rollups')")?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        })?;
        columns.sort();
        let mut expected: Vec<String> = [
            "skill_name",
            "skill_hash",
            "month",
            "invocations",
            "scored",
            "score_sum",
            "accepted",
            "reworked",
            "rejected",
            "helped",
            "hurt",
            "unused",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        expected.sort();
        assert_eq!(
            columns, expected,
            "no id, session, task or ref survives a roll-up"
        );
        Ok(())
    }

    #[test]
    fn a_purge_by_id_removes_that_one_record() -> Result<()> {
        let db = Database::open_memory()?;
        db.with_conn(|conn| {
            let keep = aged(conn, "sheets", 1, Some(1.0), None)?;
            let drop = aged(conn, "sheets", 1, Some(0.0), None)?;
            let gone = purge(conn, Purge::Record(&drop))?;
            assert_eq!(
                gone,
                Purged {
                    records: 1,
                    rollups: 0
                }
            );
            assert_eq!(raw_count(conn), 1);
            let left: String =
                conn.query_row("SELECT id FROM skill_invocations", [], |r| r.get(0))?;
            assert_eq!(left, keep);
            assert_eq!(purge(conn, Purge::Record("inv_nope"))?.records, 0);
            Ok(())
        })
    }

    #[test]
    fn a_purge_by_range_is_bounded_by_skill_and_dates() -> Result<()> {
        let db = Database::open_memory()?;
        db.with_conn(|conn| {
            aged(conn, "sheets", 40, None, None)?;
            aged(conn, "sheets", 20, None, None)?;
            aged(conn, "docs", 20, None, None)?;
            let day = |n: i64| -> String {
                conn.query_row("SELECT date('now', ?1)", [format!("-{n} days")], |r| {
                    r.get(0)
                })
                .unwrap()
            };

            // Before 30 days ago: only the oldest sheets row.
            let gone = purge(
                conn,
                Purge::Range {
                    skill: None,
                    from: None,
                    before: Some(&day(30)),
                },
            )?;
            assert_eq!(gone.records, 1);

            // One skill, any date.
            let gone = purge(
                conn,
                Purge::Range {
                    skill: Some("docs"),
                    from: None,
                    before: None,
                },
            )?;
            assert_eq!(gone.records, 1);
            assert_eq!(
                raw_count(conn),
                1,
                "the sheets row from 20 days ago is left"
            );

            // A window that holds nothing.
            let gone = purge(
                conn,
                Purge::Range {
                    skill: Some("sheets"),
                    from: Some(&day(10)),
                    before: Some(&day(5)),
                },
            )?;
            assert_eq!(gone.records, 0);
            Ok(())
        })
    }

    #[test]
    fn a_purge_takes_the_rollup_months_that_lie_inside_the_range() -> Result<()> {
        let db = Database::open_memory()?;
        db.with_conn(|conn| {
            for (month, skill) in [("2025-01", "sheets"), ("2025-02", "sheets"), ("2025-02", "docs")] {
                conn.execute(
                    "INSERT INTO skill_invocation_rollups (skill_name, month, invocations) VALUES (?1, ?2, 3)",
                    params![skill, month],
                )?;
            }
            // Covers all of January and half of February for sheets.
            let gone = purge(
                conn,
                Purge::Range { skill: Some("sheets"), from: Some("2025-01-01"), before: Some("2025-02-15") },
            )?;
            assert_eq!(gone.rollups, 1, "only the month entirely inside goes");
            let left: i64 =
                conn.query_row("SELECT COUNT(*) FROM skill_invocation_rollups", [], |r| r.get(0))?;
            assert_eq!(left, 2);
            Ok(())
        })
    }

    #[test]
    fn a_purge_that_names_nothing_or_names_it_badly_is_refused() -> Result<()> {
        let db = Database::open_memory()?;
        db.with_conn(|conn| {
            aged(conn, "sheets", 1, None, None)?;
            let none = Purge::Range {
                skill: None,
                from: None,
                before: None,
            };
            assert!(purge(conn, none).is_err(), "never unbounded");
            for (from, before) in [
                (Some("yesterday"), None),
                (None, Some("2026-13-40")),
                (Some("2026-02-01"), Some("2026-01-01")),
            ] {
                assert!(
                    purge(
                        conn,
                        Purge::Range {
                            skill: None,
                            from,
                            before
                        }
                    )
                    .is_err(),
                    "{from:?} {before:?}"
                );
            }
            assert_eq!(raw_count(conn), 1);
            Ok(())
        })
    }
}
