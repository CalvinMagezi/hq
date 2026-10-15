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
}
