//! Keyword search over what HQ remembers (the `memories` table), by the words of a message.
//!
//! The index is `memories_fts` (migration 154), kept in step with `memories` by triggers. A hit is
//! only an id and a score: the caller reads the row, so this stays free of the memory crate's types.
//! Same shape as `chat_search`: every term must match, and when none of the rows has them all, any
//! term may.

use anyhow::Result;
use rusqlite::{Connection, params};

use crate::search::{sanitize_fts_query, sanitize_fts_query_any};

/// Weights for `bm25`, in column order: summary, entities, topics. A name or place shared with the
/// question is the strongest sign a memory is about the same thing.
const WEIGHT_SUMMARY: f64 = 1.0;
const WEIGHT_ENTITIES: f64 = 3.0;
const WEIGHT_TOPICS: f64 = 0.5;

/// Most hits one call may return.
pub const MAX_LIMIT: i64 = 100;

/// Whose view of the memories a search is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    /// What HQ may bring up on its own in a chat: active rows from the owner, and nothing held
    /// back for review, marked out of date, or from a shared channel it cannot tie to the owner.
    Recall,
    /// What the owner may look through and correct: every row.
    Owner,
}

/// A memory that matched.
#[derive(Debug, Clone, PartialEq)]
pub struct MemoryHit {
    pub id: i64,
    /// Higher is a better match. Only comparable within one search.
    pub score: f64,
}

/// Memories matching `query`, best first. Returns nothing for a query with no searchable words.
pub fn search_keyword(
    conn: &Connection,
    query: &str,
    view: View,
    limit: i64,
) -> Result<Vec<MemoryHit>> {
    let mut hits = run(conn, &sanitize_fts_query(query), view, limit)?;
    if hits.is_empty() {
        hits = run(conn, &sanitize_fts_query_any(query), view, limit)?;
    }
    Ok(hits)
}

fn run(conn: &Connection, fts: &str, view: View, limit: i64) -> Result<Vec<MemoryHit>> {
    if fts.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare(
        "SELECT m.id, bm25(memories_fts, ?2, ?3, ?4)
           FROM memories_fts
           JOIN memories m ON m.id = memories_fts.rowid
          WHERE memories_fts MATCH ?1
            AND (?5 OR (m.status = 'active' AND m.origin = 'owner'))
          ORDER BY bm25(memories_fts, ?2, ?3, ?4)
          LIMIT ?6",
    )?;
    let rows = stmt.query_map(
        params![
            fts,
            WEIGHT_SUMMARY,
            WEIGHT_ENTITIES,
            WEIGHT_TOPICS,
            view == View::Owner,
            limit.clamp(1, MAX_LIMIT)
        ],
        |row| {
            // bm25 is smaller for a better match, and negative: flip it so bigger is better.
            let rank: f64 = row.get(1)?;
            Ok(MemoryHit {
                id: row.get(0)?,
                score: -rank,
            })
        },
    )?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::migrations::run(&conn).unwrap();
        conn
    }

    fn add(conn: &Connection, summary: &str, entities: &str, topics: &str) -> i64 {
        conn.execute(
            "INSERT INTO memories (source, harness, summary, content, entities, topics, importance, created_at)
             VALUES ('chat-turn', 'web', ?1, ?1, ?2, ?3, 0.5, '2026-01-01T00:00:00+00:00')",
            params![summary, entities, topics],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    fn ids(hits: &[MemoryHit]) -> Vec<i64> {
        hits.iter().map(|h| h.id).collect()
    }

    fn found(conn: &Connection, q: &str, view: View) -> Vec<i64> {
        ids(&search_keyword(conn, q, view, 10).unwrap())
    }

    #[test]
    fn a_memory_is_found_by_the_words_of_its_summary() {
        let conn = db();
        let feeds = add(&conn, "Acme Feeds delivers on Thursdays", "[]", "[]");
        add(&conn, "The loan repayment is due on the 15th", "[]", "[]");

        // The meaningful words find exactly the one memory.
        assert_eq!(found(&conn, "feeds deliver", View::Recall), vec![feeds]);
    }

    /// A question with filler words has no memory holding all of them, so the any-word fallback
    /// runs and a filler word ("the") matches an unrelated row. Ranking still puts the memory about
    /// the question first, but removing filler words before searching is the caller's job: this
    /// layer searches the words it is given.
    #[test]
    fn filler_words_in_a_question_pull_in_unrelated_rows_but_do_not_outrank_the_right_one() {
        let conn = db();
        let feeds = add(&conn, "Acme Feeds delivers on Thursdays", "[]", "[]");
        let loan = add(&conn, "The loan repayment is due on the 15th", "[]", "[]");

        let hits = found(&conn, "when do the feeds deliver", View::Recall);

        assert_eq!(hits.first(), Some(&feeds));
        assert!(hits.contains(&loan), "the filler word matched it: {hits:?}");
    }

    #[test]
    fn a_word_in_a_different_form_still_matches() {
        let conn = db();
        let id = add(&conn, "Deliveries arrive on Thursday", "[]", "[]");
        // The index stems words, so a plural and a verb form meet the same root.
        assert_eq!(found(&conn, "delivery", View::Recall), vec![id]);
    }

    #[test]
    fn every_word_must_match_until_nothing_does_and_then_any_word_may() {
        let conn = db();
        let both = add(&conn, "Mira works Monday at the counter", "[]", "[]");
        let one = add(
            &conn,
            "Monday is the day for the school canteen account",
            "[]",
            "[]",
        );

        // Two words in one memory: only that memory.
        assert_eq!(found(&conn, "Mira Monday", View::Recall), vec![both]);
        // No memory holds both words, so a memory with either is returned.
        let any = found(&conn, "Monday tobias", View::Recall);
        assert!(any.contains(&both) && any.contains(&one), "{any:?}");
    }

    #[test]
    fn a_name_in_the_entities_outranks_a_passing_mention() {
        let conn = db();
        let mention = add(
            &conn,
            "Remember to ask whether Northwind could match the price",
            "[]",
            "[]",
        );
        let about = add(
            &conn,
            "Gives a discount on large orders",
            "[\"Northwind\"]",
            "[]",
        );

        let hits = search_keyword(&conn, "Northwind", View::Recall, 10).unwrap();
        assert_eq!(ids(&hits), vec![about, mention]);
        assert!(hits[0].score > hits[1].score);
    }

    #[test]
    fn topics_are_searchable_too() {
        let conn = db();
        let id = add(
            &conn,
            "Decided to switch suppliers",
            "[]",
            "[\"packaging\",\"decision\"]",
        );
        assert_eq!(found(&conn, "packaging", View::Recall), vec![id]);
    }

    #[test]
    fn what_may_surface_on_its_own_is_narrower_than_what_the_owner_may_see() {
        let conn = db();
        let active = add(&conn, "hours are 7:30 to 5:30", "[]", "[]");
        let held = add(&conn, "hours are posted on the door", "[]", "[]");
        let outdated = add(&conn, "hours were 8 to 6", "[]", "[]");
        let shared = add(&conn, "hours in the group chat", "[]", "[]");
        conn.execute(
            "UPDATE memories SET status = 'review' WHERE id = ?1",
            [held],
        )
        .unwrap();
        conn.execute(
            "UPDATE memories SET status = 'superseded' WHERE id = ?1",
            [outdated],
        )
        .unwrap();
        conn.execute(
            "UPDATE memories SET origin = 'unknown_channel' WHERE id = ?1",
            [shared],
        )
        .unwrap();

        assert_eq!(found(&conn, "hours", View::Recall), vec![active]);
        let mut all = found(&conn, "hours", View::Owner);
        all.sort_unstable();
        assert_eq!(all, vec![active, held, outdated, shared]);
    }

    #[test]
    fn an_edit_is_reindexed_and_a_delete_removes_the_row() {
        let conn = db();
        let id = add(&conn, "Pays staff on Friday", "[]", "[]");
        assert_eq!(found(&conn, "friday", View::Recall), vec![id]);

        conn.execute(
            "UPDATE memories SET summary = 'Pays staff on Saturday' WHERE id = ?1",
            [id],
        )
        .unwrap();
        assert!(found(&conn, "friday", View::Recall).is_empty());
        assert_eq!(found(&conn, "saturday", View::Recall), vec![id]);

        conn.execute("DELETE FROM memories WHERE id = ?1", [id])
            .unwrap();
        assert!(found(&conn, "saturday", View::Owner).is_empty());
    }

    #[test]
    fn a_change_that_is_not_to_the_text_leaves_the_index_alone() {
        let conn = db();
        let id = add(&conn, "Pays staff on Friday", "[]", "[]");
        conn.execute(
            "UPDATE memories SET importance = 0.1, access_count = 5 WHERE id = ?1",
            [id],
        )
        .unwrap();
        assert_eq!(found(&conn, "friday", View::Recall), vec![id]);
    }

    #[test]
    fn a_query_made_of_punctuation_or_search_operators_is_safe_and_finds_nothing() {
        let conn = db();
        add(&conn, "AND OR NOT are only words here", "[]", "[]");
        for q in [
            "",
            "   ",
            "((",
            "\"unterminated",
            "AND OR (",
            "*",
            "a:b NEAR(c)",
        ] {
            let r = search_keyword(&conn, q, View::Recall, 10);
            assert!(r.is_ok(), "{q:?} should not be an error: {:?}", r.err());
        }
        assert!(found(&conn, "", View::Recall).is_empty());
        assert!(found(&conn, "((", View::Recall).is_empty());
    }

    #[test]
    fn the_limit_is_kept_within_bounds() {
        let conn = db();
        for i in 0..5 {
            add(&conn, &format!("shared word number {i}"), "[]", "[]");
        }
        assert_eq!(
            search_keyword(&conn, "shared", View::Recall, 2)
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            search_keyword(&conn, "shared", View::Recall, 0)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            search_keyword(&conn, "shared", View::Recall, -5)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            search_keyword(&conn, "shared", View::Recall, 10_000)
                .unwrap()
                .len(),
            5
        );
    }
}
