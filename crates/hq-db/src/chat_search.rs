//! Search over chat history: messages by keyword, fold summaries by keyword and by meaning
//! (`docs/plans/context-fold.md`). Backs `session_search` and `history_restore`.

use anyhow::Result;
use rusqlite::{Connection, params};
use serde::Serialize;

use crate::chat::ChatMessage;
use crate::search::{
    bytes_to_embedding, cosine_similarity, embedding_to_bytes, sanitize_fts_query,
    sanitize_fts_query_any,
};
use crate::thread_folds::{self, Fold};

/// Filters shared by both kinds of hit.
#[derive(Debug, Clone, Default)]
pub struct Filters {
    pub thread_id: Option<String>,
    pub role: Option<String>,
    pub after: Option<String>,
    pub before: Option<String>,
    pub limit: i64,
}

/// A message that matched, with the block that now stands for it in the model's view, if any.
#[derive(Debug, Clone, Serialize)]
pub struct MessageHit {
    #[serde(flatten)]
    pub message: ChatMessage,
    pub message_ref: String,
    /// The active block whose range holds this message (`f:...`), when it has been folded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folded_into: Option<String>,
}

/// A fold summary that matched.
#[derive(Debug, Clone, Serialize)]
pub struct FoldHit {
    pub fold_ref: String,
    pub thread_id: String,
    pub tier: i64,
    pub message_count: i64,
    pub summary: String,
    /// `keyword` or `meaning`.
    pub matched_by: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub similarity: Option<f32>,
}

/// Messages matching `query`, best first. All terms must match; when none do, any term may.
pub fn search_messages(conn: &Connection, query: &str, f: &Filters) -> Result<Vec<MessageHit>> {
    let mut hits = run_message_query(conn, &sanitize_fts_query(query), f)?;
    if hits.is_empty() {
        hits = run_message_query(conn, &sanitize_fts_query_any(query), f)?;
    }
    for hit in &mut hits {
        hit.folded_into = covering_fold(conn, &hit.message)?.map(|b| b.short_ref());
    }
    Ok(hits)
}

fn run_message_query(conn: &Connection, fts: &str, f: &Filters) -> Result<Vec<MessageHit>> {
    if fts.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare(
        "SELECT m.message_id, m.thread_id, m.role, m.content, m.created_at
           FROM chat_fts
           JOIN chat_messages m ON m.message_id = chat_fts.message_id
          WHERE chat_fts MATCH ?1
            AND (?2 IS NULL OR m.thread_id = ?2)
            AND (?3 IS NULL OR m.role = ?3)
            AND (?4 IS NULL OR m.created_at > ?4)
            AND (?5 IS NULL OR m.created_at < ?5)
          ORDER BY bm25(chat_fts)
          LIMIT ?6",
    )?;
    let rows = stmt.query_map(
        params![
            fts,
            f.thread_id,
            f.role,
            f.after,
            f.before,
            f.limit.clamp(1, 100)
        ],
        |row| {
            let message = ChatMessage {
                message_id: row.get(0)?,
                thread_id: row.get(1)?,
                role: row.get(2)?,
                content: row.get(3)?,
                created_at: row.get(4)?,
            };
            Ok(MessageHit {
                message_ref: thread_folds::message_ref(&message.message_id),
                message,
                folded_into: None,
            })
        },
    )?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

/// The active block whose range holds `message`, if it has been folded.
pub fn covering_fold(conn: &Connection, message: &ChatMessage) -> Result<Option<Fold>> {
    let blocks = thread_folds::active(conn, &message.thread_id)?;
    for block in blocks {
        let covered: bool = conn.query_row(
            "SELECT EXISTS (
                SELECT 1 FROM chat_messages a, chat_messages b, chat_messages m
                 WHERE a.message_id = ?1 AND b.message_id = ?2 AND m.message_id = ?3
                   AND (m.created_at, m.rowid) >= (a.created_at, a.rowid)
                   AND (m.created_at, m.rowid) <= (b.created_at, b.rowid))",
            params![block.first_msg_id, block.last_msg_id, message.message_id],
            |row| row.get(0),
        )?;
        if covered {
            return Ok(Some(block));
        }
    }
    Ok(None)
}

/// Fold summaries matching `query` by keyword, best first. Superseded blocks are included: their
/// refs still restore, and an older, more detailed summary can hold the exact value.
pub fn search_folds(conn: &Connection, query: &str, f: &Filters) -> Result<Vec<FoldHit>> {
    let mut hits = run_fold_query(conn, &sanitize_fts_query(query), f)?;
    if hits.is_empty() {
        hits = run_fold_query(conn, &sanitize_fts_query_any(query), f)?;
    }
    Ok(hits)
}

fn run_fold_query(conn: &Connection, fts: &str, f: &Filters) -> Result<Vec<FoldHit>> {
    if fts.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare(
        "SELECT t.id, t.thread_id, t.tier, t.message_count, t.summary
           FROM fold_fts
           JOIN thread_folds t ON t.id = fold_fts.fold_id
          WHERE fold_fts MATCH ?1 AND (?2 IS NULL OR t.thread_id = ?2)
          ORDER BY bm25(fold_fts)
          LIMIT ?3",
    )?;
    let rows = stmt.query_map(params![fts, f.thread_id, f.limit.clamp(1, 20)], |row| {
        let id: String = row.get(0)?;
        Ok(FoldHit {
            fold_ref: format!("f:{}", thread_folds::short(&id)),
            thread_id: row.get(1)?,
            tier: row.get(2)?,
            message_count: row.get(3)?,
            summary: row.get(4)?,
            matched_by: "keyword",
            similarity: None,
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

/// Stores a fold summary's embedding. Replaces an earlier one for the same block.
pub fn store_fold_embedding(
    conn: &Connection,
    fold: &Fold,
    model: &str,
    vector: &[f32],
) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO thread_fold_embeddings (fold_id, thread_id, model, vector)
         VALUES (?1, ?2, ?3, ?4)",
        params![fold.id, fold.thread_id, model, embedding_to_bytes(vector)],
    )?;
    Ok(())
}

/// Fold summaries closest in meaning to `query_vector`, from vectors made by `model` only.
/// Hits under `min_similarity` are left out.
pub fn search_folds_by_meaning(
    conn: &Connection,
    query_vector: &[f32],
    model: &str,
    f: &Filters,
    min_similarity: f32,
) -> Result<Vec<FoldHit>> {
    let mut stmt = conn.prepare(
        "SELECT t.id, t.thread_id, t.tier, t.message_count, t.summary, e.vector
           FROM thread_fold_embeddings e
           JOIN thread_folds t ON t.id = e.fold_id
          WHERE e.model = ?1 AND (?2 IS NULL OR e.thread_id = ?2)
          LIMIT 5000",
    )?;
    let mut hits: Vec<FoldHit> = stmt
        .query_map(params![model, f.thread_id], |row| {
            let id: String = row.get(0)?;
            let bytes: Vec<u8> = row.get(5)?;
            let similarity = cosine_similarity(query_vector, &bytes_to_embedding(&bytes));
            Ok(FoldHit {
                fold_ref: format!("f:{}", thread_folds::short(&id)),
                thread_id: row.get(1)?,
                tier: row.get(2)?,
                message_count: row.get(3)?,
                summary: row.get(4)?,
                matched_by: "meaning",
                similarity: Some(similarity),
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    hits.retain(|h| h.similarity.unwrap_or(0.0) >= min_similarity);
    hits.sort_by(|a, b| {
        b.similarity
            .partial_cmp(&a.similarity)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    hits.truncate(f.limit.clamp(1, 20) as usize);
    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::{add_message, create_thread, delete_messages_from};
    use crate::thread_folds::{NewFold, insert};

    fn setup() -> (Connection, String, Vec<String>) {
        let conn = Connection::open_in_memory().unwrap();
        crate::migrations::run(&conn).unwrap();
        let t = create_thread(&conn, "t", "user", "user").unwrap().thread_id;
        let texts = [
            ("user", "The school fees are UGX 1,350,000 this term"),
            ("assistant", "Noted, the fees are due on the 20th"),
            ("user", "Book the bus to Mbarara for Friday"),
            ("assistant", "Booked the morning bus"),
        ];
        let ids = texts
            .iter()
            .map(|(r, c)| add_message(&conn, &t, r, c).unwrap().message_id)
            .collect();
        (conn, t, ids)
    }

    fn filters() -> Filters {
        Filters {
            limit: 10,
            ..Default::default()
        }
    }

    #[test]
    fn messages_are_found_by_word_and_by_stem() {
        let (conn, _, ids) = setup();
        let hits = search_messages(&conn, "fee", &filters()).unwrap();
        assert_eq!(hits.len(), 2, "porter stems fees to fee");
        let hits = search_messages(&conn, "bus Mbarara", &filters()).unwrap();
        assert_eq!(hits[0].message.message_id, ids[2]);
        assert!(
            search_messages(&conn, "AND OR (", &filters())
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn no_all_terms_match_falls_back_to_any() {
        let (conn, _, _) = setup();
        let hits = search_messages(&conn, "bus helicopter", &filters()).unwrap();
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn deleted_messages_leave_the_index() {
        let (conn, t, ids) = setup();
        delete_messages_from(&conn, &t, &ids[2]).unwrap();
        assert!(
            search_messages(&conn, "bus", &filters())
                .unwrap()
                .is_empty()
        );
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM chat_fts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 2);
    }

    #[test]
    fn a_folded_message_names_its_block_and_folds_are_searchable() {
        let (conn, t, ids) = setup();
        let fold = insert(
            &conn,
            &NewFold {
                thread_id: &t,
                tier: 1,
                first_msg_id: &ids[0],
                last_msg_id: &ids[1],
                message_count: 2,
                summary: "Goal as of this block: term two school fees, UGX 1,350,000",
                source_tokens: 40,
                summary_tokens: 12,
                tainted: false,
                model: "test",
            },
        )
        .unwrap();
        let hits = search_messages(&conn, "fees", &filters()).unwrap();
        assert!(hits.iter().all(|h| h.folded_into == Some(fold.short_ref())));
        let bus = search_messages(&conn, "bus", &filters()).unwrap();
        assert!(bus.iter().all(|h| h.folded_into.is_none()));
        let folds = search_folds(&conn, "school", &filters()).unwrap();
        assert_eq!(folds[0].fold_ref, fold.short_ref());

        store_fold_embedding(&conn, &fold, "m", &[1.0, 0.0]).unwrap();
        let near = search_folds_by_meaning(&conn, &[0.9, 0.1], "m", &filters(), 0.5).unwrap();
        assert_eq!(near.len(), 1);
        assert!(
            search_folds_by_meaning(&conn, &[0.0, 1.0], "m", &filters(), 0.5)
                .unwrap()
                .is_empty()
        );
        assert!(
            search_folds_by_meaning(&conn, &[1.0, 0.0], "other", &filters(), 0.5)
                .unwrap()
                .is_empty()
        );

        // Deleting the folded messages takes the block, its index row and its vector with it.
        delete_messages_from(&conn, &t, &ids[0]).unwrap();
        assert!(
            search_folds(&conn, "school", &filters())
                .unwrap()
                .is_empty()
        );
        let vectors: i64 = conn
            .query_row("SELECT COUNT(*) FROM thread_fold_embeddings", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(vectors, 0);
    }
}
