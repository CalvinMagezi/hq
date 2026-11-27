//! Bringing up what HQ remembers for each message (`daemon.memory_recall`).
//!
//! In `shadow` mode the work is done and written down, and nothing reaches the model. In `query`
//! mode the memories that fit the message are added to the same per-turn note as the tool hint, so
//! the system prompt and the history before the note stay byte-identical and stay cached.
//!
//! It must never slow or fail a chat: the work runs off the async workers under a time limit,
//! and any failure or timeout is simply silence.

use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hq_core::config::MemoryRecall;
use hq_db::memory_recall_log::{self, Entry};
use hq_memory::recall::{Audience, RecallQuery, Recalled};

use crate::session::{AgentSession, TurnRecall};

/// How long recall may take before the message is sent without it.
const BUDGET: Duration = Duration::from_millis(150);
/// How long embedding the question may take. Past this the question goes by its words alone.
const EMBED_BUDGET: Duration = Duration::from_millis(60);

/// Look up memories for every prompt of `session`, in the way `mode` says. Only the owner's own
/// sessions are given one (the caller checks), and `Recent` and `Off` need none.
pub fn wire_turn_recall(session: &mut AgentSession, db_path: &Path, mode: MemoryRecall) {
    if !matches!(mode, MemoryRecall::Shadow | MemoryRecall::Query) {
        return;
    }
    let Ok(db) = hq_db::Database::open(db_path) else {
        return;
    };
    // Finding a memory by meaning needs the on-device model, and only if it is already on the
    // machine: nothing is fetched here.
    let meaning = hq_core::config::HqConfig::load().is_ok_and(|c| c.embeds_locally())
        && hq_memory::local_embed::is_ready();
    if meaning && let Ok(runtime) = tokio::runtime::Handle::try_current() {
        // Load the model in the background so a question never waits for it.
        runtime.spawn(async {
            let _ = hq_memory::generate_embedding_local("warm").await;
        });
    }
    let recall = MemoryTurnRecall::new(db, mode, session.session_id.clone()).with_meaning(meaning);
    session.set_turn_recall(Arc::new(recall));
}

/// The memory lookup for one chat session.
pub struct MemoryTurnRecall {
    db: hq_db::Database,
    mode: MemoryRecall,
    session_id: String,
    /// Memories already shown in this session, so a long chat does not repeat itself. A web or
    /// relay chat is rebuilt for every message and starts empty each time.
    shown: Mutex<HashSet<i64>>,
    /// Whether to also look by meaning, with the on-device model.
    meaning: bool,
    /// The memories shown by the last call, for the "remembered N notes" chip.
    last: Mutex<Vec<i64>>,
}

impl MemoryTurnRecall {
    pub fn new(db: hq_db::Database, mode: MemoryRecall, session_id: String) -> Self {
        Self {
            db,
            mode,
            session_id,
            shown: Mutex::new(HashSet::new()),
            meaning: false,
            last: Mutex::new(Vec::new()),
        }
    }

    /// Also look by meaning. A question that cannot be embedded in time goes by its words alone.
    pub fn with_meaning(mut self, on: bool) -> Self {
        self.meaning = on;
        self
    }

    fn mode_name(&self) -> &'static str {
        match self.mode {
            MemoryRecall::Query => "query",
            _ => "shadow",
        }
    }
}

#[async_trait::async_trait]
impl TurnRecall for MemoryTurnRecall {
    async fn recall(&self, message: &str, prior_message: Option<&str>) -> Option<String> {
        let started = Instant::now();
        let exclude: Vec<i64> = self
            .shown
            .lock()
            .map(|s| s.iter().copied().collect())
            .unwrap_or_default();
        let vector = if self.meaning {
            tokio::time::timeout(EMBED_BUDGET, hq_memory::generate_embedding_local(message))
                .await
                .ok()
                .and_then(Result::ok)
        } else {
            None
        };
        let (db, message, prior) = (
            self.db.clone(),
            message.to_string(),
            prior_message.map(str::to_string),
        );
        let work = tokio::task::spawn_blocking(move || {
            let meaning = vector.as_deref().map(|vector| hq_memory::recall::Meaning {
                vector,
                model: hq_memory::LOCAL_EMBEDDING_MODEL,
            });
            hq_memory::recall::recall_with_meaning(
                &db,
                &RecallQuery {
                    message: &message,
                    prior_message: prior.as_deref(),
                    audience: Audience::Owner,
                    exclude: &exclude,
                },
                meaning,
            )
        });

        let (recalled, failure) = match tokio::time::timeout(BUDGET, work).await {
            Ok(Ok(Ok(recalled))) => (recalled, None),
            Ok(Ok(Err(e))) => {
                tracing::debug!(error = %e, "memory recall failed");
                (Recalled::default(), Some("error"))
            }
            Ok(Err(e)) => {
                tracing::debug!(error = %e, "memory recall task failed");
                (Recalled::default(), Some("error"))
            }
            Err(_) => (Recalled::default(), Some("timed out")),
        };

        let ids: Vec<i64> = recalled.items.iter().map(|i| i.id).collect();
        let shown_now = self.mode == MemoryRecall::Query && !ids.is_empty();
        self.write_down(
            &ids,
            recalled.tokens(),
            failure.or(recalled.silent_because),
            started.elapsed(),
            shown_now,
        );

        if !shown_now {
            return None;
        }
        if let Ok(mut shown) = self.shown.lock() {
            shown.extend(ids.iter().copied());
        }
        if let Ok(mut last) = self.last.lock() {
            *last = ids;
        }
        Some(recalled.block)
    }

    fn last_shown(&self) -> Vec<i64> {
        self.last.lock().map(|l| l.clone()).unwrap_or_default()
    }
}

impl MemoryTurnRecall {
    /// Record what happened, and count the memories shown as used. Off the async workers, and
    /// never waited for: a slow disk must not slow a reply.
    fn write_down(
        &self,
        ids: &[i64],
        tokens: usize,
        silent_because: Option<&'static str>,
        took: Duration,
        shown: bool,
    ) {
        let (db, ids) = (self.db.clone(), ids.to_vec());
        let (session_id, mode) = (self.session_id.clone(), self.mode_name());
        let latency_ms = took.as_millis() as u64;
        tokio::task::spawn_blocking(move || {
            let logged = db.with_conn(|conn| {
                memory_recall_log::record(
                    conn,
                    &Entry {
                        session_id: &session_id,
                        mode,
                        memory_ids: &ids,
                        tokens,
                        silent_because,
                        latency_ms,
                    },
                )
            });
            if let Err(e) = logged {
                tracing::debug!(error = %e, "memory recall log not written");
            }
            // A memory counts as used only when the model was actually shown it.
            if shown {
                for id in &ids {
                    let _ = hq_memory::touch_memory(&db, *id);
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_memory::{StoreMemoryParams, store_memory};

    fn db_with_feeds() -> (hq_db::Database, i64) {
        let db = hq_db::Database::open_memory().unwrap();
        let id = store_memory(
            &db,
            &StoreMemoryParams {
                source: "chat-turn",
                harness: "web",
                raw_text: "",
                summary: "Acme Feeds delivers on Thursdays",
                entities: &["Acme Feeds".to_string()],
                topics: &[],
                importance: 0.5,
                user_id: None,
            },
        )
        .unwrap();
        (db, id)
    }

    /// The log and the use count are written in the background, so wait for them.
    async fn settle() {
        for _ in 0..50 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    fn logged(db: &hq_db::Database) -> (i64, i64) {
        db.with_conn(|c| {
            Ok(c.query_row(
                "SELECT COUNT(*), COALESCE(SUM(items), 0) FROM memory_recall_log",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?)
        })
        .unwrap()
    }

    fn uses(db: &hq_db::Database, id: i64) -> i64 {
        db.with_conn(|c| {
            Ok(c.query_row(
                "SELECT access_count FROM memories WHERE id = ?1",
                [id],
                |r| r.get(0),
            )?)
        })
        .unwrap()
    }

    #[tokio::test]
    async fn query_mode_hands_back_the_block_once_and_counts_the_memory_as_used() {
        let (db, id) = db_with_feeds();
        let recall = MemoryTurnRecall::new(db.clone(), MemoryRecall::Query, "s".into());

        let note = recall.recall("When does Acme Feeds deliver?", None).await;
        let block = note.expect("a block");
        assert!(
            block.contains("Acme Feeds delivers on Thursdays"),
            "{block}"
        );
        assert_eq!(recall.last_shown(), vec![id], "says which note was shown");

        // The same memory is not shown a second time in this chat.
        assert!(
            recall
                .recall("When does Acme Feeds deliver?", None)
                .await
                .is_none()
        );

        settle().await;
        assert_eq!(uses(&db, id), 1, "used once: it was shown once");
        assert_eq!(
            logged(&db),
            (2, 1),
            "two turns written down, one with a memory"
        );
    }

    #[tokio::test]
    async fn shadow_mode_shows_no_note_so_there_is_nothing_to_say_was_remembered() {
        let (db, _) = db_with_feeds();
        let recall = MemoryTurnRecall::new(db, MemoryRecall::Shadow, "s".into());
        assert!(
            recall
                .recall("When does Acme Feeds deliver?", None)
                .await
                .is_none()
        );
        assert!(recall.last_shown().is_empty());
    }

    #[tokio::test]
    async fn shadow_mode_writes_down_what_it_would_have_said_and_shows_nothing() {
        let (db, id) = db_with_feeds();
        let recall = MemoryTurnRecall::new(db.clone(), MemoryRecall::Shadow, "s".into());

        let note = recall.recall("When does Acme Feeds deliver?", None).await;

        assert!(note.is_none(), "the model is shown nothing in shadow mode");
        settle().await;
        assert_eq!(logged(&db), (1, 1), "but it is written down");
        assert_eq!(
            uses(&db, id),
            0,
            "a memory nobody was shown is not counted as used"
        );
        let mode: String = db
            .with_conn(|c| Ok(c.query_row("SELECT mode FROM memory_recall_log", [], |r| r.get(0))?))
            .unwrap();
        assert_eq!(mode, "shadow");
    }

    #[tokio::test]
    async fn a_greeting_is_written_down_as_silence_and_shows_nothing() {
        let (db, _) = db_with_feeds();
        let recall = MemoryTurnRecall::new(db.clone(), MemoryRecall::Query, "s".into());

        assert!(recall.recall("Hi", None).await.is_none());

        settle().await;
        let reason: Option<String> = db
            .with_conn(|c| {
                Ok(
                    c.query_row("SELECT silent_because FROM memory_recall_log", [], |r| {
                        r.get(0)
                    })?,
                )
            })
            .unwrap();
        assert_eq!(reason.as_deref(), Some("asks about nothing"));
    }

    #[tokio::test]
    async fn a_broken_index_is_silence_and_not_a_failed_reply() {
        let (db, _) = db_with_feeds();
        db.with_conn(|c| {
            c.execute_batch("DROP TABLE memories_fts")?;
            Ok(())
        })
        .unwrap();
        let recall = MemoryTurnRecall::new(db.clone(), MemoryRecall::Query, "s".into());

        let note = recall.recall("When does Acme Feeds deliver?", None).await;

        assert!(note.is_none());
        settle().await;
        let reason: Option<String> = db
            .with_conn(|c| {
                Ok(
                    c.query_row("SELECT silent_because FROM memory_recall_log", [], |r| {
                        r.get(0)
                    })?,
                )
            })
            .unwrap();
        assert_eq!(reason.as_deref(), Some("error"));
    }

    #[test]
    fn only_shadow_and_query_look_anything_up() {
        // `Recent` and `Off` never get a lookup, so there is nothing to slow a reply down.
        for mode in [MemoryRecall::Recent, MemoryRecall::Off] {
            assert!(!matches!(mode, MemoryRecall::Shadow | MemoryRecall::Query));
        }
    }
}
