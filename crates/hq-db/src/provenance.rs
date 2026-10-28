//! Who produced which version of a note, and whether anyone has reviewed it.
//!
//! A record is made by a write path that knows who is writing (a tool call from an attested
//! session, the web API, a command run by the owner). Nothing here works out an author: a
//! version no write path recorded has no record, and a record whose writer was not known says
//! `unknown`. Rows are keyed by the note's path and the hash of its raw file, so a later edit
//! never inherits an earlier author.
//!
//! Provenance says where a contribution came from. It is not trust: an identified agent's note is
//! not verified, and review state changes only through an explicit action, never as a side effect
//! of a write or a read.

use anyhow::{Result, bail};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, Row, params};

/// What kind of writer produced a version. A classification, not a trust rating.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    Human,
    Agent,
    Imported,
    Generated,
    System,
    Unknown,
}

impl SourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Human => "human",
            Self::Agent => "agent",
            Self::Imported => "imported",
            Self::Generated => "generated",
            Self::System => "system",
            Self::Unknown => "unknown",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "human" => Self::Human,
            "agent" => Self::Agent,
            "imported" => Self::Imported,
            "generated" => Self::Generated,
            "system" => Self::System,
            "unknown" => Self::Unknown,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewState {
    Unreviewed,
    Reviewed,
    Accepted,
    Superseded,
    Disputed,
}

impl ReviewState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unreviewed => "unreviewed",
            Self::Reviewed => "reviewed",
            Self::Accepted => "accepted",
            Self::Superseded => "superseded",
            Self::Disputed => "disputed",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "unreviewed" => Self::Unreviewed,
            "reviewed" => Self::Reviewed,
            "accepted" => Self::Accepted,
            "superseded" => Self::Superseded,
            "disputed" => Self::Disputed,
            _ => return None,
        })
    }
}

/// What a write path can say about itself. Every field is optional because most paths know only
/// some of it; a field left `None` is stored as unknown, not looked up or guessed.
#[derive(Debug, Clone, Default)]
pub struct Attribution<'a> {
    pub actor_id: Option<&'a str>,
    pub source_kind: Option<SourceKind>,
    pub session_id: Option<&'a str>,
    pub task_id: Option<&'a str>,
}

impl Attribution<'_> {
    /// Whether this says anything at all about who wrote the version.
    fn is_known(&self) -> bool {
        self.actor_id.is_some_and(|a| a != UNKNOWN)
            || self.source_kind.is_some_and(|k| k != SourceKind::Unknown)
            || self.session_id.is_some()
            || self.task_id.is_some()
    }
}

pub const UNKNOWN: &str = "unknown";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provenance {
    pub id: i64,
    pub path: String,
    pub content_hash: String,
    pub actor_id: String,
    pub source_kind: SourceKind,
    pub session_id: Option<String>,
    pub task_id: Option<String>,
    pub written_at: String,
    pub review_state: ReviewState,
    pub reviewed_by: Option<String>,
    pub reviewed_at: Option<String>,
    pub review_reason: Option<String>,
    pub recorded_late: bool,
    pub redacted: bool,
    pub deleted_at: Option<String>,
}

impl Provenance {
    /// Whether the writer of this version is not known.
    pub fn is_unattributed(&self) -> bool {
        self.actor_id == UNKNOWN
            && self.source_kind == SourceKind::Unknown
            && self.session_id.is_none()
            && self.task_id.is_none()
    }
}

const COLS: &str = "id, path, content_hash, actor_id, source_kind, session_id, task_id, written_at, \
    review_state, reviewed_by, reviewed_at, review_reason, recorded_late, redacted, deleted_at";

fn from_row(row: &Row<'_>) -> rusqlite::Result<Provenance> {
    let kind: String = row.get(4)?;
    let state: String = row.get(8)?;
    Ok(Provenance {
        id: row.get(0)?,
        path: row.get(1)?,
        content_hash: row.get(2)?,
        actor_id: row.get(3)?,
        source_kind: SourceKind::parse(&kind).unwrap_or(SourceKind::Unknown),
        session_id: row.get(5)?,
        task_id: row.get(6)?,
        written_at: row.get(7)?,
        review_state: ReviewState::parse(&state).unwrap_or(ReviewState::Unreviewed),
        reviewed_by: row.get(9)?,
        reviewed_at: row.get(10)?,
        review_reason: row.get(11)?,
        recorded_late: row.get::<_, i64>(12)? != 0,
        redacted: row.get::<_, i64>(13)? != 0,
        deleted_at: row.get(14)?,
    })
}

fn now() -> String {
    Utc::now().to_rfc3339()
}

fn clean(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|v| !v.is_empty())
}

fn by_version(conn: &Connection, path: &str, content_hash: &str) -> Result<Option<Provenance>> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLS} FROM note_provenance WHERE path = ?1 AND content_hash = ?2"),
            params![path, content_hash],
            from_row,
        )
        .optional()?)
}

/// One record by id. Redacted records are not returned.
pub fn get(conn: &Connection, id: i64) -> Result<Option<Provenance>> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLS} FROM note_provenance WHERE id = ?1 AND redacted = 0"),
            params![id],
            from_row,
        )
        .optional()?)
}

/// Record who produced the version of `path` whose raw file hashes to `content_hash`.
///
/// If that version is already recorded the first attribution stands: a later write of the same
/// bytes does not take the credit. The one exception is a version recorded as unknown, which a
/// later record that does know fills in, flagged as recorded late. Returns the row as stored.
pub fn record(
    conn: &Connection,
    path: &str,
    content_hash: &str,
    who: &Attribution<'_>,
) -> Result<Provenance> {
    if path.trim().is_empty() || content_hash.trim().is_empty() {
        bail!("a provenance record needs a path and the hash of the version it describes");
    }
    let actor = clean(who.actor_id).unwrap_or(UNKNOWN);
    let kind = who.source_kind.unwrap_or(SourceKind::Unknown);
    let (session, task) = (clean(who.session_id), clean(who.task_id));

    conn.execute_batch("SAVEPOINT note_provenance_record")?;
    let result = (|| -> Result<Provenance> {
        conn.execute(
            "INSERT OR IGNORE INTO note_provenance
               (path, content_hash, actor_id, source_kind, session_id, task_id, written_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                path,
                content_hash,
                actor,
                kind.as_str(),
                session,
                task,
                now()
            ],
        )?;
        let stored = by_version(conn, path, content_hash)?
            .ok_or_else(|| anyhow::anyhow!("provenance row vanished after insert"))?;
        if stored.is_unattributed() && who.is_known() {
            conn.execute(
                "UPDATE note_provenance
                    SET actor_id = ?2, source_kind = ?3, session_id = ?4, task_id = ?5, recorded_late = 1
                  WHERE id = ?1",
                params![stored.id, actor, kind.as_str(), session, task],
            )?;
            return by_version(conn, path, content_hash)?
                .ok_or_else(|| anyhow::anyhow!("provenance row vanished after update"));
        }
        Ok(stored)
    })();
    match result {
        Ok(row) => {
            conn.execute_batch("RELEASE note_provenance_record")?;
            // A task's activity view lists these, so whoever watches tasks hears of it.
            crate::tasks::changed_outside_tx(conn);
            Ok(row)
        }
        Err(e) => {
            let _ = conn.execute_batch(
                "ROLLBACK TO note_provenance_record; RELEASE note_provenance_record",
            );
            Err(e)
        }
    }
}

/// Add attribution to a version that was recorded without it (or not recorded at all), once more
/// is known. Flagged as late. Never replaces attribution that is already there. Returns whether
/// anything was added.
pub fn link_late(
    conn: &Connection,
    path: &str,
    content_hash: &str,
    who: &Attribution<'_>,
) -> Result<bool> {
    if !who.is_known() {
        return Ok(false);
    }
    let before = by_version(conn, path, content_hash)?;
    let after = record(conn, path, content_hash, who)?;
    Ok(match before {
        None => {
            // Not recorded before: it is new, but only now attributed.
            conn.execute(
                "UPDATE note_provenance SET recorded_late = 1 WHERE id = ?1",
                [after.id],
            )?;
            true
        }
        Some(before) => before.is_unattributed() && !after.is_unattributed(),
    })
}

/// The record for exactly this version of the note, if one exists. `None` means nobody recorded
/// who produced it: it may have been edited outside HQ's tools since the last record.
pub fn current(conn: &Connection, path: &str, content_hash: &str) -> Result<Option<Provenance>> {
    Ok(by_version(conn, path, content_hash)?.filter(|p| !p.redacted))
}

/// Who suggested a pin on this version of a note, when the version came from an
/// agent and the owner has not accepted it: such a pin is shown to the owner but not
/// trusted. `Reviewed` does not count, since agents may set it. `None` for the
/// owner's own writes, accepted versions, and versions nobody recorded (edited
/// outside HQ's tools).
pub fn pin_suggested_by(conn: &Connection, path: &str, raw: &str) -> Result<Option<String>> {
    let hash = crate::search::content_hash(raw);
    let Some(p) = current(conn, path, &hash)? else {
        return Ok(None);
    };
    let from_agent = matches!(p.source_kind, SourceKind::Agent | SourceKind::Generated);
    let accepted = p.review_state == ReviewState::Accepted;
    Ok((from_agent && !accepted).then_some(p.actor_id))
}

/// Records for a note, newest first. Redacted records are left out.
pub fn history(conn: &Connection, path: &str, limit: usize) -> Result<Vec<Provenance>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLS} FROM note_provenance
          WHERE path = ?1 AND redacted = 0
          ORDER BY id DESC LIMIT ?2"
    ))?;
    let rows = stmt.query_map(params![path, limit as i64], from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// The latest record for each note a task produced, newest first.
pub fn for_task(conn: &Connection, task_id: &str, limit: usize) -> Result<Vec<Provenance>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLS} FROM note_provenance p
          WHERE task_id = ?1 AND redacted = 0 AND deleted_at IS NULL
            AND id = (SELECT MAX(id) FROM note_provenance q
                       WHERE q.path = p.path AND q.task_id = ?1 AND q.redacted = 0)
          ORDER BY id DESC LIMIT ?2"
    ))?;
    let rows = stmt.query_map(params![task_id, limit as i64], from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// The latest record for each note a session wrote, newest first.
pub fn for_session(conn: &Connection, session_id: &str, limit: usize) -> Result<Vec<Provenance>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLS} FROM note_provenance p
          WHERE session_id = ?1 AND redacted = 0 AND deleted_at IS NULL
            AND id = (SELECT MAX(id) FROM note_provenance q
                       WHERE q.path = p.path AND q.session_id = ?1 AND q.redacted = 0)
          ORDER BY id DESC LIMIT ?2"
    ))?;
    let rows = stmt.query_map(params![session_id, limit as i64], from_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// A note moved: its records follow it. A version already recorded at the destination keeps that
/// record, and the duplicate from the old path is dropped.
pub fn move_path(conn: &Connection, from: &str, to: &str) -> Result<usize> {
    if from == to {
        return Ok(0);
    }
    let moved = conn.execute(
        "UPDATE OR IGNORE note_provenance SET path = ?2, deleted_at = NULL WHERE path = ?1",
        [from, to],
    )?;
    conn.execute("DELETE FROM note_provenance WHERE path = ?1", [from])?;
    Ok(moved)
}

/// A note was deleted: keep its records, marked, until `purge_deleted` removes them. Returns how
/// many records were marked.
pub fn mark_deleted(conn: &Connection, path: &str) -> Result<usize> {
    Ok(conn.execute(
        "UPDATE note_provenance SET deleted_at = ?2 WHERE path = ?1 AND deleted_at IS NULL",
        params![path, now()],
    )?)
}

/// A note came back (restored from the trash, recreated): its old records are live again.
pub fn clear_deleted(conn: &Connection, path: &str) -> Result<usize> {
    Ok(conn.execute(
        "UPDATE note_provenance SET deleted_at = NULL WHERE path = ?1 AND deleted_at IS NOT NULL",
        [path],
    )?)
}

/// Remove records of notes deleted more than `days` ago. Returns how many.
pub fn purge_deleted(conn: &Connection, days: i64) -> Result<usize> {
    let cutoff = (Utc::now() - chrono::Duration::days(days)).to_rfc3339();
    Ok(conn.execute(
        "DELETE FROM note_provenance WHERE deleted_at IS NOT NULL AND deleted_at < ?1",
        [cutoff],
    )?)
}

/// Remove who wrote a version from the record, on request (a privacy or retention rule), without
/// touching the note. The row stays so history and counts still add up; its actor, session and
/// task are wiped and it no longer appears in lookups.
pub fn redact(conn: &Connection, id: i64) -> Result<bool> {
    let changed = conn.execute(
        "UPDATE note_provenance
            SET actor_id = ?2, source_kind = 'unknown', session_id = NULL, task_id = NULL,
                reviewed_by = NULL, review_reason = NULL, redacted = 1
          WHERE id = ?1 AND redacted = 0",
        params![id, "redacted"],
    )?;
    if changed > 0 {
        conn.execute(
            "DELETE FROM note_review_events WHERE provenance_id = ?1",
            [id],
        )?;
    }
    Ok(changed > 0)
}

/// Set a record's review state, with who decided and why, and keep the history. This is the only
/// way review state changes.
pub fn set_review_state(
    conn: &Connection,
    id: i64,
    state: ReviewState,
    reviewer: &str,
    reason: Option<&str>,
) -> Result<()> {
    let reviewer = reviewer.trim();
    if reviewer.is_empty() {
        bail!("a review needs the name of who made it");
    }
    let at = now();
    let reason = clean(reason);
    let changed = conn.execute(
        "UPDATE note_provenance
            SET review_state = ?2, reviewed_by = ?3, reviewed_at = ?4, review_reason = ?5
          WHERE id = ?1 AND redacted = 0",
        params![id, state.as_str(), reviewer, at, reason],
    )?;
    if changed == 0 {
        bail!("no such provenance record: {id}");
    }
    conn.execute(
        "INSERT INTO note_review_events (provenance_id, state, reviewer, reason, at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![id, state.as_str(), reviewer, reason, at],
    )?;
    crate::tasks::changed_outside_tx(conn);
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewEvent {
    pub state: ReviewState,
    pub reviewer: String,
    pub reason: Option<String>,
    pub at: String,
}

/// Every review decision on a record, oldest first.
pub fn review_history(conn: &Connection, id: i64) -> Result<Vec<ReviewEvent>> {
    let mut stmt = conn.prepare(
        "SELECT state, reviewer, reason, at FROM note_review_events
          WHERE provenance_id = ?1 ORDER BY id",
    )?;
    let rows = stmt.query_map([id], |row| {
        let state: String = row.get(0)?;
        Ok(ReviewEvent {
            state: ReviewState::parse(&state).unwrap_or(ReviewState::Unreviewed),
            reviewer: row.get(1)?,
            reason: row.get(2)?,
            at: row.get(3)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// The time a record was written, parsed.
pub fn written_at(p: &Provenance) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(&p.written_at)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::migrations::run(&conn).unwrap();
        conn
    }

    #[test]
    fn a_pin_is_a_suggestion_only_on_an_unreviewed_agent_version() {
        let conn = db();
        let raw = "---\npinned: true\n---\nx";
        let hash = crate::search::content_hash(raw);
        assert_eq!(
            pin_suggested_by(&conn, "n.md", raw).unwrap(),
            None,
            "nobody recorded it"
        );
        let rec = record(&conn, "n.md", &hash, &agent("hs-1", "hs-1", None)).unwrap();
        assert_eq!(
            pin_suggested_by(&conn, "n.md", raw).unwrap().as_deref(),
            Some("hs-1")
        );
        set_review_state(&conn, rec.id, ReviewState::Accepted, "owner", None).unwrap();
        assert_eq!(
            pin_suggested_by(&conn, "n.md", raw).unwrap(),
            None,
            "the owner accepted it"
        );

        let edited = "---\npinned: true\n---\ny";
        record(
            &conn,
            "n.md",
            &crate::search::content_hash(edited),
            &agent("hs-2", "hs-2", None),
        )
        .unwrap();
        assert_eq!(
            pin_suggested_by(&conn, "n.md", edited).unwrap().as_deref(),
            Some("hs-2"),
            "an agent's later edit is unreviewed again"
        );
        let later = current(&conn, "n.md", &crate::search::content_hash(edited))
            .unwrap()
            .unwrap();
        set_review_state(&conn, later.id, ReviewState::Reviewed, "hs-3", None).unwrap();
        assert_eq!(
            pin_suggested_by(&conn, "n.md", edited).unwrap().as_deref(),
            Some("hs-2"),
            "another agent's review does not vouch for a pin"
        );
    }

    fn agent<'a>(actor: &'a str, session: &'a str, task: Option<&'a str>) -> Attribution<'a> {
        Attribution {
            actor_id: Some(actor),
            source_kind: Some(SourceKind::Agent),
            session_id: Some(session),
            task_id: task,
        }
    }

    #[test]
    fn a_version_is_recorded_with_who_wrote_it_and_starts_unreviewed() {
        let conn = db();
        let p = record(
            &conn,
            "Notebooks/a.md",
            "h1",
            &agent("hq", "s-1", Some("T-1")),
        )
        .unwrap();

        assert_eq!(
            (
                p.actor_id.as_str(),
                p.source_kind,
                p.session_id.as_deref(),
                p.task_id.as_deref()
            ),
            ("hq", SourceKind::Agent, Some("s-1"), Some("T-1"))
        );
        assert_eq!(p.review_state, ReviewState::Unreviewed);
        assert!(!p.recorded_late && !p.redacted && p.deleted_at.is_none());
        assert_eq!(current(&conn, "Notebooks/a.md", "h1").unwrap(), Some(p));
    }

    #[test]
    fn a_later_edit_does_not_inherit_an_earlier_author() {
        let conn = db();
        record(&conn, "Notebooks/a.md", "h1", &agent("hq", "s-1", None)).unwrap();

        // The file now hashes differently and nobody recorded that edit.
        assert_eq!(current(&conn, "Notebooks/a.md", "h2").unwrap(), None);
        let history = history(&conn, "Notebooks/a.md", 10).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].content_hash, "h1");
    }

    #[test]
    fn what_a_write_path_does_not_know_is_stored_as_unknown_and_never_derived() {
        let conn = db();
        // A session id with no actor: the actor is not worked out from the session.
        let who = Attribution {
            session_id: Some("s-9"),
            ..Attribution::default()
        };
        let p = record(&conn, "Notebooks/a.md", "h1", &who).unwrap();
        assert_eq!(p.actor_id, UNKNOWN);
        assert_eq!(p.source_kind, SourceKind::Unknown);
        assert_eq!(p.session_id.as_deref(), Some("s-9"));

        let nothing = record(&conn, "Notebooks/b.md", "h1", &Attribution::default()).unwrap();
        assert!(nothing.is_unattributed());
        // Blank strings count as not known.
        let blank = Attribution {
            actor_id: Some("  "),
            session_id: Some(""),
            ..Attribution::default()
        };
        assert!(
            record(&conn, "Notebooks/c.md", "h1", &blank)
                .unwrap()
                .is_unattributed()
        );
    }

    #[test]
    fn attribution_added_later_is_flagged_and_never_overwrites_what_was_known() {
        let conn = db();
        record(&conn, "Notebooks/a.md", "h1", &Attribution::default()).unwrap();

        assert!(
            link_late(
                &conn,
                "Notebooks/a.md",
                "h1",
                &agent("hq", "s-1", Some("T-1"))
            )
            .unwrap()
        );
        let p = current(&conn, "Notebooks/a.md", "h1").unwrap().unwrap();
        assert_eq!((p.actor_id.as_str(), p.recorded_late), ("hq", true));

        // A second claim for the same bytes does not take the credit.
        assert!(!link_late(&conn, "Notebooks/a.md", "h1", &agent("other", "s-2", None)).unwrap());
        let again = record(&conn, "Notebooks/a.md", "h1", &agent("other", "s-2", None)).unwrap();
        assert_eq!(again.actor_id, "hq");
        assert_eq!(again.session_id.as_deref(), Some("s-1"));

        // Nothing known, nothing added.
        assert!(!link_late(&conn, "Notebooks/b.md", "h1", &Attribution::default()).unwrap());
        assert_eq!(current(&conn, "Notebooks/b.md", "h1").unwrap(), None);
    }

    #[test]
    fn a_version_first_seen_with_late_attribution_is_flagged_late() {
        let conn = db();
        assert!(link_late(&conn, "Notebooks/a.md", "h1", &agent("hq", "s-1", None)).unwrap());
        assert!(
            current(&conn, "Notebooks/a.md", "h1")
                .unwrap()
                .unwrap()
                .recorded_late
        );
    }

    #[test]
    fn a_move_carries_the_records_and_keeps_the_destinations_own() {
        let conn = db();
        record(&conn, "Notebooks/old.md", "h1", &agent("hq", "s-1", None)).unwrap();
        record(&conn, "Notebooks/old.md", "h2", &agent("hq", "s-2", None)).unwrap();
        // The destination already has a record for h2, from someone else.
        record(
            &conn,
            "Notebooks/new.md",
            "h2",
            &agent("claude", "s-3", None),
        )
        .unwrap();

        move_path(&conn, "Notebooks/old.md", "Notebooks/new.md").unwrap();

        assert!(history(&conn, "Notebooks/old.md", 10).unwrap().is_empty());
        let moved = history(&conn, "Notebooks/new.md", 10).unwrap();
        assert_eq!(moved.len(), 2);
        let h1 = current(&conn, "Notebooks/new.md", "h1").unwrap().unwrap();
        assert_eq!(h1.session_id.as_deref(), Some("s-1"));
        let h2 = current(&conn, "Notebooks/new.md", "h2").unwrap().unwrap();
        assert_eq!(h2.actor_id, "claude", "the destination's own record stays");
    }

    #[test]
    fn a_deleted_note_keeps_its_records_for_a_while_then_they_are_purged() {
        let conn = db();
        record(
            &conn,
            "Notebooks/a.md",
            "h1",
            &agent("hq", "s-1", Some("T-1")),
        )
        .unwrap();
        record(
            &conn,
            "Notebooks/b.md",
            "h1",
            &agent("hq", "s-1", Some("T-1")),
        )
        .unwrap();

        assert_eq!(mark_deleted(&conn, "Notebooks/a.md").unwrap(), 1);
        let kept = history(&conn, "Notebooks/a.md", 10).unwrap();
        assert!(kept[0].deleted_at.is_some(), "the tombstone is visible");
        // A deleted note no longer counts as something the task produced.
        let produced: Vec<_> = for_task(&conn, "T-1", 10)
            .unwrap()
            .into_iter()
            .map(|p| p.path)
            .collect();
        assert_eq!(produced, vec!["Notebooks/b.md"]);

        assert_eq!(purge_deleted(&conn, 90).unwrap(), 0, "too recent to purge");
        conn.execute(
            "UPDATE note_provenance SET deleted_at = ?1 WHERE path = 'Notebooks/a.md'",
            [(Utc::now() - chrono::Duration::days(91)).to_rfc3339()],
        )
        .unwrap();
        assert_eq!(purge_deleted(&conn, 90).unwrap(), 1);
        assert!(history(&conn, "Notebooks/a.md", 10).unwrap().is_empty());
        assert_eq!(history(&conn, "Notebooks/b.md", 10).unwrap().len(), 1);

        // A restored note is live again.
        mark_deleted(&conn, "Notebooks/b.md").unwrap();
        assert_eq!(clear_deleted(&conn, "Notebooks/b.md").unwrap(), 1);
        assert!(
            history(&conn, "Notebooks/b.md", 10).unwrap()[0]
                .deleted_at
                .is_none()
        );
    }

    #[test]
    fn redacting_a_record_wipes_who_wrote_it_but_leaves_the_note_and_the_row() {
        let conn = db();
        let p = record(
            &conn,
            "Notebooks/a.md",
            "h1",
            &agent("hq", "s-1", Some("T-1")),
        )
        .unwrap();
        set_review_state(
            &conn,
            p.id,
            ReviewState::Accepted,
            "owner",
            Some("looks right"),
        )
        .unwrap();

        assert!(redact(&conn, p.id).unwrap());
        assert!(!redact(&conn, p.id).unwrap(), "already redacted");

        assert_eq!(current(&conn, "Notebooks/a.md", "h1").unwrap(), None);
        assert!(history(&conn, "Notebooks/a.md", 10).unwrap().is_empty());
        assert!(for_task(&conn, "T-1", 10).unwrap().is_empty());
        assert!(for_session(&conn, "s-1", 10).unwrap().is_empty());
        let (actor, session, task, count): (String, Option<String>, Option<String>, i64) = conn
            .query_row(
                "SELECT actor_id, session_id, task_id,
                        (SELECT COUNT(*) FROM note_review_events WHERE provenance_id = ?1)
                   FROM note_provenance WHERE id = ?1",
                [p.id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(
            (actor.as_str(), session, task, count),
            ("redacted", None, None, 0)
        );
    }

    #[test]
    fn review_state_changes_only_through_the_action_and_keeps_its_history() {
        let conn = db();
        let p = record(&conn, "Notebooks/a.md", "h1", &agent("hq", "s-1", None)).unwrap();
        // Recording again, or reading, never changes it.
        record(&conn, "Notebooks/a.md", "h1", &agent("hq", "s-1", None)).unwrap();
        assert_eq!(
            current(&conn, "Notebooks/a.md", "h1")
                .unwrap()
                .unwrap()
                .review_state,
            ReviewState::Unreviewed
        );

        set_review_state(
            &conn,
            p.id,
            ReviewState::Disputed,
            "owner",
            Some("contradicts the PRD"),
        )
        .unwrap();
        set_review_state(
            &conn,
            p.id,
            ReviewState::Accepted,
            "owner",
            Some("PRD was out of date"),
        )
        .unwrap();

        let now = current(&conn, "Notebooks/a.md", "h1").unwrap().unwrap();
        assert_eq!(now.review_state, ReviewState::Accepted);
        assert_eq!(now.reviewed_by.as_deref(), Some("owner"));
        assert_eq!(now.review_reason.as_deref(), Some("PRD was out of date"));
        let trail = review_history(&conn, p.id).unwrap();
        assert_eq!(
            trail.iter().map(|e| e.state).collect::<Vec<_>>(),
            vec![ReviewState::Disputed, ReviewState::Accepted]
        );
        assert_eq!(trail[0].reason.as_deref(), Some("contradicts the PRD"));

        assert!(set_review_state(&conn, p.id, ReviewState::Reviewed, "  ", None).is_err());
        assert!(set_review_state(&conn, 9_999, ReviewState::Reviewed, "owner", None).is_err());
    }

    #[test]
    fn what_a_task_or_session_produced_is_the_latest_version_of_each_note() {
        let conn = db();
        record(
            &conn,
            "Notebooks/a.md",
            "h1",
            &agent("hq", "s-1", Some("T-1")),
        )
        .unwrap();
        record(
            &conn,
            "Notebooks/a.md",
            "h2",
            &agent("hq", "s-1", Some("T-1")),
        )
        .unwrap();
        record(
            &conn,
            "Notebooks/b.md",
            "h1",
            &agent("claude", "s-2", Some("T-2")),
        )
        .unwrap();

        let task = for_task(&conn, "T-1", 10).unwrap();
        assert_eq!(task.len(), 1);
        assert_eq!(
            (task[0].path.as_str(), task[0].content_hash.as_str()),
            ("Notebooks/a.md", "h2")
        );
        assert_eq!(for_session(&conn, "s-2", 10).unwrap().len(), 1);
        // Two writers on different notes are never confused for one another.
        assert!(
            for_task(&conn, "T-1", 10)
                .unwrap()
                .iter()
                .all(|p| p.actor_id == "hq")
        );
        assert!(for_task(&conn, "T-9", 10).unwrap().is_empty());
    }

    #[test]
    fn a_record_needs_a_path_and_a_version() {
        let conn = db();
        assert!(record(&conn, "", "h1", &Attribution::default()).is_err());
        assert!(record(&conn, "Notebooks/a.md", " ", &Attribution::default()).is_err());
    }
}
