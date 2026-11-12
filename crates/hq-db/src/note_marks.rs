//! What the apps show beside a note about where it came from: who wrote it (`you`, `hq`,
//! `import`), whether it is `new` or `changed` for the owner to look at, and a one-line why.
//!
//! All of it is read from `note_provenance` and `note_seen`; nothing here writes provenance and
//! nothing reads a note's words to guess an author. A version nobody recorded, or one that was
//! edited outside HQ's tools since its record, counts as the owner's.

use anyhow::Result;
use chrono::DateTime;
use rusqlite::{Connection, OptionalExtension, params};

use crate::provenance::{ReviewState, SourceKind};

pub const WRITTEN_BY_YOU: &str = "you";
pub const WRITTEN_BY_HQ: &str = "hq";
pub const WRITTEN_BY_IMPORT: &str = "import";
pub const REVIEW_NEW: &str = "new";
pub const REVIEW_CHANGED: &str = "changed";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteMark {
    pub written_by: &'static str,
    /// `new`, `changed` or none.
    pub review: Option<&'static str>,
    /// "Saved from your chat, 10 Nov"; none when the owner wrote the note.
    pub why: Option<String>,
}

impl NoteMark {
    pub fn you() -> Self {
        Self {
            written_by: WRITTEN_BY_YOU,
            review: None,
            why: None,
        }
    }

    /// The fields to merge into a note's JSON.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "written_by": self.written_by,
            "review": self.review,
            "why": self.why,
        })
    }
}

struct Version {
    id: i64,
    kind: SourceKind,
    session: Option<String>,
    task: Option<String>,
    written_at: String,
    review: ReviewState,
}

/// The live record for exactly this version of the note. Matching by hash, not by newest id, keeps
/// a version right when the owner reverted to older text and HQ wrote the newer text again (the
/// record keeps its first id).
fn version(conn: &Connection, path: &str, hash: &str) -> Result<Option<Version>> {
    Ok(conn
        .query_row(
            "SELECT id, source_kind, session_id, task_id, written_at, review_state
               FROM note_provenance
              WHERE path = ?1 AND content_hash = ?2 AND redacted = 0 AND deleted_at IS NULL",
            params![path, hash],
            |r| {
                let kind: String = r.get(1)?;
                let review: String = r.get(5)?;
                Ok(Version {
                    id: r.get(0)?,
                    kind: SourceKind::parse(&kind).unwrap_or(SourceKind::Unknown),
                    session: r.get(2)?,
                    task: r.get(3)?,
                    written_at: r.get(4)?,
                    review: ReviewState::parse(&review).unwrap_or(ReviewState::Unreviewed),
                })
            },
        )
        .optional()?)
}

fn is_hq_kind(kind: SourceKind) -> bool {
    matches!(
        kind,
        SourceKind::Agent | SourceKind::Generated | SourceKind::System
    )
}

fn short_date(rfc3339: &str) -> String {
    DateTime::parse_from_rfc3339(rfc3339)
        .map(|d| d.format("%-d %b").to_string())
        .unwrap_or_default()
}

fn why_of(v: &Version, written_by: &str) -> String {
    let date = short_date(&v.written_at);
    let base = match (written_by, &v.task, &v.session) {
        (WRITTEN_BY_IMPORT, _, _) => "Imported",
        (_, Some(_), _) => "Saved for a task",
        (_, None, Some(_)) => "Saved from your chat",
        _ => "Saved by HQ",
    };
    if date.is_empty() {
        base.to_string()
    } else {
        format!("{base}, {date}")
    }
}

/// Whether an earlier version, skipping HQ's own, was the owner's (typed, unknown or imported).
/// Two HQ edits in a row on the owner's note therefore stay `changed`. A note with no earlier
/// record at all reads as `new`: a first record cannot say whether the note existed before it.
fn owner_wrote_before(conn: &Connection, path: &str, before_id: i64, hash: &str) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM note_provenance
              WHERE path = ?1 AND id < ?2 AND content_hash <> ?3 AND redacted = 0
                AND source_kind NOT IN ('agent','generated','system')
              LIMIT 1",
            params![path, before_id, hash],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

fn seen(conn: &Connection, path: &str, hash: &str) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM note_seen WHERE path = ?1 AND content_hash = ?2",
            params![path, hash],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// The mark for `path`. `current_hash` reads the hash of the file as it is now. A file with no
/// record for that exact version (never recorded, or edited outside HQ's tools) is the owner's.
pub fn mark(
    conn: &Connection,
    path: &str,
    current_hash: impl FnOnce() -> Option<String>,
) -> Result<NoteMark> {
    let Some(hash) = current_hash() else {
        return Ok(NoteMark::you());
    };
    let Some(v) = version(conn, path, &hash)? else {
        return Ok(NoteMark::you());
    };
    let written_by = match v.kind {
        k if is_hq_kind(k) => WRITTEN_BY_HQ,
        SourceKind::Imported => WRITTEN_BY_IMPORT,
        _ => return Ok(NoteMark::you()),
    };
    // Only the owner's Keep (`accepted`) settles a mark: agents may set the other review states.
    let review = if v.review == ReviewState::Accepted {
        None
    } else if owner_wrote_before(conn, path, v.id, &hash)? {
        // Opening a changed note does not settle the change.
        Some(REVIEW_CHANGED)
    } else if seen(conn, path, &hash)? {
        None
    } else {
        Some(REVIEW_NEW)
    };
    Ok(NoteMark {
        written_by,
        review,
        why: Some(why_of(&v, written_by)),
    })
}

/// The owner opened this version of the note. Clears `new`; a `changed` note waits for Keep or
/// Undo instead. Only notes with a record of that version get a row, one row per note.
pub fn mark_seen(conn: &Connection, path: &str, hash: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO note_seen (path, content_hash, seen_at)
         SELECT ?1, ?2, ?3
          WHERE EXISTS (SELECT 1 FROM note_provenance WHERE path = ?1 AND content_hash = ?2)
         ON CONFLICT(path) DO UPDATE SET content_hash = excluded.content_hash,
           seen_at = excluded.seen_at",
        params![path, hash, chrono::Utc::now().to_rfc3339()],
    )?;
    Ok(())
}

/// How many recently written candidates `any_new` checks.
const NEW_CANDIDATES: i64 = 200;

/// Whether any note carries a `new` or `changed` mark, for the quiet dot on Vault. It asks
/// [`mark`] about each recent candidate, so it agrees with the lists. `hash_of` reads the hash of
/// a note as it is on disk now.
pub fn any_new(conn: &Connection, hash_of: impl Fn(&str) -> Option<String>) -> Result<bool> {
    let paths: Vec<String> = {
        let mut stmt = conn.prepare(
            "SELECT path FROM note_provenance
              WHERE redacted = 0 AND deleted_at IS NULL AND review_state <> 'accepted'
                AND source_kind IN ('agent','generated','system','imported')
              GROUP BY path ORDER BY MAX(id) DESC LIMIT ?1",
        )?;
        stmt.query_map([NEW_CANDIDATES], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?
    };
    for path in paths {
        if mark(conn, &path, || hash_of(&path))?.review.is_some() {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;
    use crate::provenance::{Attribution, carry_version, move_path, record};

    fn who(kind: SourceKind, session: Option<&'static str>) -> Attribution<'static> {
        Attribution {
            actor_id: Some("someone"),
            source_kind: Some(kind),
            session_id: session,
            task_id: None,
        }
    }

    fn agent() -> Attribution<'static> {
        who(SourceKind::Agent, Some("s1"))
    }

    fn mark_of(c: &Connection, path: &str, hash: &str) -> NoteMark {
        mark(c, path, || Some(hash.to_string())).unwrap()
    }

    fn set_review(c: &Connection, id: i64, state: &str) {
        c.execute(
            "UPDATE note_provenance SET review_state = ?2 WHERE id = ?1",
            params![id, state],
        )
        .unwrap();
    }

    fn seen_rows(c: &Connection) -> i64 {
        c.query_row("SELECT COUNT(*) FROM note_seen", [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn a_note_nobody_recorded_or_the_owner_wrote_is_yours() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            assert_eq!(mark_of(c, "a.md", "h1"), NoteMark::you());
            record(c, "b.md", "h1", &who(SourceKind::Human, None))?;
            assert_eq!(mark_of(c, "b.md", "h1"), NoteMark::you());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_note_hq_wrote_is_new_with_a_why_until_the_owner_opens_it() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            record(c, "a.md", "h1", &agent())?;
            let m = mark_of(c, "a.md", "h1");
            assert_eq!(m.written_by, "hq");
            assert_eq!(m.review, Some("new"));
            assert!(m.why.unwrap().starts_with("Saved from your chat, "));
            assert!(any_new(c, |_| Some("h1".into()))?);
            mark_seen(c, "a.md", "h1")?;
            assert_eq!(mark_of(c, "a.md", "h1").review, None);
            assert!(!any_new(c, |_| Some("h1".into()))?);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn opening_clears_new_but_not_changed_and_only_keep_settles_changed() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            record(c, "a.md", "h1", &who(SourceKind::Human, None))?;
            let p = record(c, "a.md", "h2", &agent())?;
            assert_eq!(mark_of(c, "a.md", "h2").review, Some("changed"));
            mark_seen(c, "a.md", "h2")?;
            assert_eq!(mark_of(c, "a.md", "h2").review, Some("changed"));
            set_review(c, p.id, "accepted");
            assert_eq!(mark_of(c, "a.md", "h2").review, None);
            // Undo: the owner writes again, so the current version is theirs.
            record(c, "a.md", "h3", &who(SourceKind::Human, None))?;
            assert_eq!(mark_of(c, "a.md", "h3"), NoteMark::you());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn another_helpers_review_does_not_settle_a_mark() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            record(c, "n.md", "n1", &agent())?;
            let first = record(c, "n.md", "n1", &agent())?;
            record(c, "c.md", "c1", &who(SourceKind::Human, None))?;
            let edit = record(c, "c.md", "c2", &agent())?;
            for state in ["reviewed", "disputed", "superseded"] {
                set_review(c, first.id, state);
                set_review(c, edit.id, state);
                assert_eq!(mark_of(c, "n.md", "n1").review, Some("new"), "{state}");
                assert_eq!(mark_of(c, "c.md", "c2").review, Some("changed"), "{state}");
            }
            set_review(c, edit.id, "accepted");
            assert_eq!(mark_of(c, "c.md", "c2").review, None);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn several_hq_edits_in_a_row_on_the_owners_note_stay_changed() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            record(c, "a.md", "h1", &who(SourceKind::Human, None))?;
            record(c, "a.md", "h2", &agent())?;
            record(c, "a.md", "h3", &agent())?;
            assert_eq!(mark_of(c, "a.md", "h3").review, Some("changed"));
            // A link-copy that only adds an id carries the record to a new hash.
            carry_version(c, "a.md", "h3", "h4")?;
            assert_eq!(mark_of(c, "a.md", "h4").review, Some("changed"));
            // An import before HQ's edit is not HQ's either.
            record(c, "i.md", "i1", &who(SourceKind::Imported, None))?;
            record(c, "i.md", "i2", &agent())?;
            assert_eq!(mark_of(c, "i.md", "i2").review, Some("changed"));
            // HQ's own earlier version is not the owner's text: two HQ writes stay new.
            record(c, "n.md", "n1", &agent())?;
            record(c, "n.md", "n2", &agent())?;
            assert_eq!(mark_of(c, "n.md", "n2").review, Some("new"));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn opening_follows_a_link_copy_and_a_rename_and_keeps_one_row_per_note() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            record(c, "a.md", "h1", &agent())?;
            mark_seen(c, "a.md", "h1")?;
            carry_version(c, "a.md", "h1", "h2")?;
            assert_eq!(mark_of(c, "a.md", "h2").review, None, "link copy");
            move_path(c, "a.md", "b.md")?;
            assert_eq!(mark_of(c, "b.md", "h2").review, None, "rename");
            assert_eq!(seen_rows(c), 1);
            // A new HQ version is new again; opening it replaces the row.
            record(c, "b.md", "h3", &agent())?;
            assert_eq!(mark_of(c, "b.md", "h3").review, Some("new"));
            mark_seen(c, "b.md", "h3")?;
            assert_eq!(seen_rows(c), 1);
            // No record of that version, no row.
            mark_seen(c, "plain.md", "x")?;
            assert_eq!(seen_rows(c), 1);
            // Purging the deleted note's records prunes its row.
            crate::provenance::mark_deleted(c, "b.md")?;
            c.execute(
                "UPDATE note_provenance SET deleted_at = '2000-01-01T00:00:00+00:00'",
                [],
            )?;
            crate::provenance::purge_deleted(c, 1)?;
            assert_eq!(seen_rows(c), 0);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_note_edited_outside_hq_is_yours_everywhere_and_raises_no_dot() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            record(c, "a.md", "h1", &agent())?;
            assert!(any_new(c, |_| Some("h1".into()))?);
            // The file on disk changed after HQ's write.
            assert_eq!(mark_of(c, "a.md", "edited"), NoteMark::you());
            assert!(!any_new(c, |_| Some("edited".into()))?);
            assert!(!any_new(c, |_| None)?);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_version_hq_wrote_again_after_a_revert_is_still_hqs() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            record(c, "a.md", "h2", &agent())?;
            record(c, "a.md", "h1", &who(SourceKind::Human, None))?;
            // HQ writes h2 again: the old record keeps its place, with a lower id than h1's.
            record(c, "a.md", "h2", &agent())?;
            assert_eq!(mark_of(c, "a.md", "h2").written_by, "hq");
            assert_eq!(mark_of(c, "a.md", "h1"), NoteMark::you());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn an_import_and_an_edit_outside_hq_are_told_apart() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            record(c, "a.md", "h1", &who(SourceKind::Imported, None))?;
            let m = mark_of(c, "a.md", "h1");
            assert_eq!(m.written_by, "import");
            assert!(m.why.unwrap().starts_with("Imported, "));
            assert_eq!(mark_of(c, "a.md", "other"), NoteMark::you());
            Ok(())
        })
        .unwrap();
    }
}
