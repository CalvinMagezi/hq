//! The owner's own view of what HQ remembers: list it, search it, correct it, pin it, mark it out
//! of date, and keep what was held back. Only the owner's own routes and commands call this: it
//! sees every row, including ones recall would never bring up on its own.
//!
//! Removing a memory is not here. Forgetting has to reach the notes and files made from it, so it
//! lives in `hq_memory::forget`; this file only holds the part that is a single database row.

use anyhow::{Result, bail};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

use crate::memories_search::{View, search_keyword};

/// The longest a corrected memory may be, in characters.
pub const MAX_SUMMARY_CHARS: usize = 1000;
/// The most rows a list or a search returns at once.
pub const MAX_PAGE: i64 = 200;

/// What the owner sees of one memory.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OwnerRow {
    pub id: i64,
    pub summary: String,
    pub entities: Vec<String>,
    pub topics: Vec<String>,
    pub importance: f64,
    pub created_at: String,
    /// `active`, `review` or `superseded`.
    pub status: String,
    /// `owner`, or `unknown_channel` for a memory from a shared channel.
    pub origin: String,
    /// `owner_said`, `model_inferred` or `owner_edited`.
    pub provenance: String,
    pub pinned: bool,
    /// How many times it has been brought up in a chat.
    pub used: i64,
    pub thread_id: Option<String>,
    /// The interface it was learned in: web, telegram, discord, cli.
    pub harness: String,
}

/// A memory's standing, as the owner can set it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standing {
    /// Brought up when it fits.
    Active,
    /// Held back until the owner has looked at it.
    Review,
    /// No longer true. Kept, so the owner can see it, but never brought up.
    OutOfDate,
}

impl Standing {
    fn column(self) -> &'static str {
        match self {
            Standing::Active => "active",
            Standing::Review => "review",
            Standing::OutOfDate => "superseded",
        }
    }
}

/// Which memories a list shows.
#[derive(Debug, Clone, Default)]
pub struct ListFilter {
    /// Only this status (`active`, `review`, `superseded`).
    pub status: Option<String>,
    /// Only memories from a shared channel that HQ cannot tie to the owner.
    pub unattributed_only: bool,
    pub pinned_only: bool,
    pub limit: i64,
    pub offset: i64,
}

const SELECT: &str = "SELECT id, COALESCE(NULLIF(summary, ''), content, ''), entities, topics,
        importance, created_at, status, origin, provenance, pinned, access_count, thread_id, harness
   FROM memories";

fn row_of(r: &rusqlite::Row<'_>) -> rusqlite::Result<OwnerRow> {
    let list = |idx: usize| -> rusqlite::Result<Vec<String>> {
        let raw: Option<String> = r.get(idx)?;
        Ok(raw
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default())
    };
    // A legacy numeric timestamp must not make the whole list fail.
    let created_at = match r.get::<_, rusqlite::types::Value>(5)? {
        rusqlite::types::Value::Text(t) => t,
        rusqlite::types::Value::Integer(n) => n.to_string(),
        _ => String::new(),
    };
    Ok(OwnerRow {
        id: r.get(0)?,
        summary: r.get(1)?,
        entities: list(2)?,
        topics: list(3)?,
        importance: r.get(4)?,
        created_at,
        status: r.get(6)?,
        origin: r.get(7)?,
        provenance: r.get(8)?,
        pinned: r.get::<_, i64>(9)? != 0,
        used: r.get(10)?,
        thread_id: r.get(11)?,
        harness: r.get(12)?,
    })
}

/// Every memory, newest first, narrowed by `filter`.
pub fn list(conn: &Connection, filter: &ListFilter) -> Result<Vec<OwnerRow>> {
    let sql = format!(
        "{SELECT}
          WHERE (?1 IS NULL OR status = ?1)
            AND (?2 = 0 OR origin <> 'owner')
            AND (?3 = 0 OR pinned = 1)
          ORDER BY created_at DESC, id DESC
          LIMIT ?4 OFFSET ?5"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(
        params![
            filter.status,
            filter.unattributed_only,
            filter.pinned_only,
            filter.limit.clamp(1, MAX_PAGE),
            filter.offset.max(0)
        ],
        row_of,
    )?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// The memories matching `query`, best first, whatever their standing.
pub fn search(conn: &Connection, query: &str, limit: i64) -> Result<Vec<OwnerRow>> {
    let mut out = Vec::new();
    for hit in search_keyword(conn, query, View::Owner, limit.clamp(1, MAX_PAGE))? {
        if let Some(row) = get(conn, hit.id)? {
            out.push(row);
        }
    }
    Ok(out)
}

/// One memory, if it exists.
pub fn get(conn: &Connection, id: i64) -> Result<Option<OwnerRow>> {
    let sql = format!("{SELECT} WHERE id = ?1");
    Ok(conn.query_row(&sql, [id], row_of).optional()?)
}

/// How many memories there are of each kind.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Counts {
    pub total: i64,
    pub active: i64,
    pub review: i64,
    pub out_of_date: i64,
    /// From a shared channel, held back until the owner keeps them.
    pub unattributed: i64,
    pub pinned: i64,
}

pub fn counts(conn: &Connection) -> Result<Counts> {
    Ok(conn.query_row(
        "SELECT COUNT(*),
                COALESCE(SUM(status = 'active'), 0),
                COALESCE(SUM(status = 'review'), 0),
                COALESCE(SUM(status = 'superseded'), 0),
                COALESCE(SUM(origin <> 'owner'), 0),
                COALESCE(SUM(pinned = 1), 0)
           FROM memories",
        [],
        |r| {
            Ok(Counts {
                total: r.get(0)?,
                active: r.get(1)?,
                review: r.get(2)?,
                out_of_date: r.get(3)?,
                unattributed: r.get(4)?,
                pinned: r.get(5)?,
            })
        },
    )?)
}

/// Replaces a memory's text with the owner's own. It is now theirs: marked `owner_edited`, made
/// active, and brought back to the owner's side if it came from a shared channel. Secrets pasted
/// into it are removed. Returns false when there is no such memory.
pub fn edit(conn: &Connection, id: i64, summary: &str) -> Result<bool> {
    let text = hq_core::redact::redact_secrets(summary.trim());
    if text.is_empty() {
        bail!("a memory cannot be empty; forget it instead");
    }
    if text.chars().count() > MAX_SUMMARY_CHARS {
        bail!("a memory can be at most {MAX_SUMMARY_CHARS} characters");
    }
    let changed = conn.execute(
        "UPDATE memories
            SET summary = ?2, content = ?2, provenance = 'owner_edited', status = 'active',
                origin = 'owner', content_hash = NULL
          WHERE id = ?1",
        params![id, text],
    )?;
    Ok(changed > 0)
}

/// Pins a memory so it is never aged out, or lets it age again.
pub fn set_pinned(conn: &Connection, id: i64, pinned: bool) -> Result<bool> {
    Ok(conn.execute(
        "UPDATE memories SET pinned = ?2 WHERE id = ?1",
        params![id, pinned],
    )? > 0)
}

/// Sets how a memory is treated. Making one active also vouches for it: a memory from a shared
/// channel the owner keeps becomes theirs.
pub fn set_standing(conn: &Connection, id: i64, standing: Standing) -> Result<bool> {
    let changed = match standing {
        Standing::Active => conn.execute(
            "UPDATE memories SET status = 'active', origin = 'owner' WHERE id = ?1",
            [id],
        )?,
        other => conn.execute(
            "UPDATE memories SET status = ?2 WHERE id = ?1",
            params![id, other.column()],
        )?,
    };
    Ok(changed > 0)
}

/// Remembers that the owner had this text forgotten, so it is not learned again.
pub fn add_tombstone(conn: &Connection, content_hash: &str) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO memory_tombstones (content_hash, forgotten_at) VALUES (?1, ?2)",
        params![content_hash, chrono::Utc::now().to_rfc3339()],
    )?;
    Ok(())
}

/// Whether the owner had exactly this text forgotten.
pub fn is_tombstoned(conn: &Connection, content_hash: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM memory_tombstones WHERE content_hash = ?1)",
        [content_hash],
        |r| r.get(0),
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::migrations::run(&conn).unwrap();
        conn
    }

    fn add(conn: &Connection, summary: &str, created: &str) -> i64 {
        conn.execute(
            "INSERT INTO memories (source, harness, summary, content, entities, topics, importance, created_at)
             VALUES ('chat-turn', 'web', ?1, ?1, '[\"Acme\"]', '[\"suppliers\"]', 0.5, ?2)",
            params![summary, created],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn all() -> ListFilter {
        ListFilter {
            limit: 100,
            ..ListFilter::default()
        }
    }

    #[test]
    fn the_list_shows_everything_newest_first_with_what_the_owner_needs_to_judge_it() {
        let conn = db();
        let old = add(
            &conn,
            "Acme delivers on Tuesdays",
            "2026-01-01T00:00:00+00:00",
        );
        let new = add(
            &conn,
            "Acme delivers on Thursdays",
            "2026-06-01T00:00:00+00:00",
        );
        conn.execute(
            "UPDATE memories SET status = 'superseded' WHERE id = ?1",
            [old],
        )
        .unwrap();

        let rows = list(&conn, &all()).unwrap();

        assert_eq!(
            rows.iter().map(|r| r.id).collect::<Vec<_>>(),
            vec![new, old]
        );
        assert_eq!(
            rows[1].status, "superseded",
            "an out-of-date memory is still listed"
        );
        assert_eq!(rows[0].entities, vec!["Acme"]);
        assert_eq!(rows[0].topics, vec!["suppliers"]);
        assert_eq!(
            (rows[0].origin.as_str(), rows[0].provenance.as_str()),
            ("owner", "model_inferred")
        );
        assert_eq!(rows[0].harness, "web");
    }

    #[test]
    fn a_list_can_be_narrowed_and_paged() {
        let conn = db();
        let a = add(&conn, "one", "2026-01-01T00:00:00+00:00");
        let b = add(&conn, "two", "2026-01-02T00:00:00+00:00");
        let c = add(&conn, "three", "2026-01-03T00:00:00+00:00");
        conn.execute(
            "UPDATE memories SET origin = 'unknown_channel' WHERE id = ?1",
            [b],
        )
        .unwrap();
        conn.execute("UPDATE memories SET pinned = 1 WHERE id = ?1", [a])
            .unwrap();
        conn.execute("UPDATE memories SET status = 'review' WHERE id = ?1", [c])
            .unwrap();

        let ids = |f: ListFilter| {
            list(&conn, &f)
                .unwrap()
                .iter()
                .map(|r| r.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            ids(ListFilter {
                unattributed_only: true,
                ..all()
            }),
            vec![b]
        );
        assert_eq!(
            ids(ListFilter {
                pinned_only: true,
                ..all()
            }),
            vec![a]
        );
        assert_eq!(
            ids(ListFilter {
                status: Some("review".into()),
                ..all()
            }),
            vec![c]
        );
        assert_eq!(
            ids(ListFilter {
                limit: 1,
                offset: 1,
                ..all()
            }),
            vec![b]
        );
        assert_eq!(
            ids(ListFilter { limit: 0, ..all() }).len(),
            1,
            "a limit under one is one"
        );
    }

    #[test]
    fn a_search_finds_a_memory_whatever_its_standing() {
        let conn = db();
        let held = add(
            &conn,
            "Group chat moves to Sundays",
            "2026-01-01T00:00:00+00:00",
        );
        conn.execute(
            "UPDATE memories SET origin = 'unknown_channel', status = 'review' WHERE id = ?1",
            [held],
        )
        .unwrap();

        let found = search(&conn, "sundays", 10).unwrap();

        assert_eq!(found.iter().map(|r| r.id).collect::<Vec<_>>(), vec![held]);
        assert_eq!(found[0].status, "review");
    }

    #[test]
    fn counts_say_how_many_of_each_kind_there_are() {
        let conn = db();
        let a = add(&conn, "one", "2026-01-01T00:00:00+00:00");
        let b = add(&conn, "two", "2026-01-02T00:00:00+00:00");
        add(&conn, "three", "2026-01-03T00:00:00+00:00");
        conn.execute(
            "UPDATE memories SET status = 'review', origin = 'unknown_channel' WHERE id = ?1",
            [a],
        )
        .unwrap();
        conn.execute(
            "UPDATE memories SET status = 'superseded', pinned = 1 WHERE id = ?1",
            [b],
        )
        .unwrap();

        assert_eq!(
            counts(&conn).unwrap(),
            Counts {
                total: 3,
                active: 1,
                review: 1,
                out_of_date: 1,
                unattributed: 1,
                pinned: 1
            }
        );
        assert_eq!(counts(&db()).unwrap(), Counts::default());
    }

    #[test]
    fn editing_makes_a_memory_the_owners_own_and_brings_it_back_into_play() {
        let conn = db();
        let id = add(
            &conn,
            "Acme delivers on Tuesdays",
            "2026-01-01T00:00:00+00:00",
        );
        conn.execute(
            "UPDATE memories SET status = 'review', origin = 'unknown_channel' WHERE id = ?1",
            [id],
        )
        .unwrap();

        assert!(edit(&conn, id, "  Acme delivers on Thursdays  ").unwrap());

        let row = get(&conn, id).unwrap().unwrap();
        assert_eq!(row.summary, "Acme delivers on Thursdays");
        assert_eq!(
            (
                row.status.as_str(),
                row.origin.as_str(),
                row.provenance.as_str()
            ),
            ("active", "owner", "owner_edited")
        );
        // The search index follows the edit.
        assert!(
            search_keyword(&conn, "tuesdays", View::Recall, 5)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            search_keyword(&conn, "thursdays", View::Recall, 5)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn an_edit_refuses_an_empty_or_huge_text_and_hides_a_pasted_secret() {
        let conn = db();
        let id = add(&conn, "a fact", "2026-01-01T00:00:00+00:00");
        assert!(edit(&conn, id, "   ").is_err());
        assert!(edit(&conn, id, &"x".repeat(MAX_SUMMARY_CHARS + 1)).is_err());
        assert!(!edit(&conn, 9999, "no such memory").unwrap());

        let key = ["sk-", "abcdefghijklmnopqrstuvwx1234"].concat();
        edit(&conn, id, &format!("the key is {key}")).unwrap();
        let stored = get(&conn, id).unwrap().unwrap().summary;
        assert!(!stored.contains(&key), "{stored}");
    }

    #[test]
    fn keeping_a_held_back_memory_makes_it_the_owners_and_out_of_date_keeps_it_out_of_play() {
        let conn = db();
        let id = add(
            &conn,
            "Group chat moves to Sundays",
            "2026-01-01T00:00:00+00:00",
        );
        conn.execute(
            "UPDATE memories SET status = 'review', origin = 'unknown_channel' WHERE id = ?1",
            [id],
        )
        .unwrap();
        assert!(
            search_keyword(&conn, "sundays", View::Recall, 5)
                .unwrap()
                .is_empty()
        );

        assert!(set_standing(&conn, id, Standing::Active).unwrap());
        assert_eq!(
            search_keyword(&conn, "sundays", View::Recall, 5)
                .unwrap()
                .len(),
            1
        );

        assert!(set_standing(&conn, id, Standing::OutOfDate).unwrap());
        assert!(
            search_keyword(&conn, "sundays", View::Recall, 5)
                .unwrap()
                .is_empty()
        );
        assert_eq!(get(&conn, id).unwrap().unwrap().status, "superseded");
        assert!(!set_standing(&conn, 9999, Standing::Active).unwrap());
    }

    #[test]
    fn pinning_can_be_set_and_cleared() {
        let conn = db();
        let id = add(&conn, "a fact", "2026-01-01T00:00:00+00:00");
        assert!(set_pinned(&conn, id, true).unwrap());
        assert!(get(&conn, id).unwrap().unwrap().pinned);
        assert!(set_pinned(&conn, id, false).unwrap());
        assert!(!get(&conn, id).unwrap().unwrap().pinned);
    }

    #[test]
    fn a_forgotten_text_is_remembered_only_as_a_hash() {
        let conn = db();
        assert!(!is_tombstoned(&conn, "abc").unwrap());
        add_tombstone(&conn, "abc").unwrap();
        add_tombstone(&conn, "abc").unwrap();
        assert!(is_tombstoned(&conn, "abc").unwrap());
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM memory_tombstones", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1);
    }
}
