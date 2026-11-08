//! `session_search` and `history_restore`: finding and bringing back earlier chat history.
//!
//! Long chats are folded (`docs/plans/context-fold.md`): older messages reach the model only as
//! summary blocks, and the originals stay in `chat_messages`. `session_search` finds messages by
//! keyword (FTS5, with the old substring scan as a fallback for partial words and ids) and fold
//! summaries by keyword and, with the on-device embedder, by meaning. `history_restore` returns
//! the original messages behind a block or around one message, so an exact amount, quote or step
//! a summary left out can be read again.

use anyhow::Result;
use async_trait::async_trait;
use hq_db::chat_search::{self, FoldHit, MessageHit};
use hq_db::{Database, chat::ChatMessage, thread_folds};
use rusqlite::params;
use serde_json::{Value, json};
use std::sync::Arc;

use crate::registry::HqTool;
use crate::vault::QueryEmbedder;

/// Filters for one `session_search` query, already normalized (limit clamped to 1..=100).
pub struct SearchFilters {
    pub query: String,
    pub thread_id: Option<String>,
    pub role: Option<String>,
    pub after: Option<String>,
    pub before: Option<String>,
    pub limit: i64,
}

/// What one search found: messages, and the fold summaries that matched.
#[derive(Default)]
pub struct SearchOutcome {
    pub messages: Vec<MessageHit>,
    pub summaries: Vec<FoldHit>,
}

/// The Provider side of `SessionSearchTool`'s Definition/Consumer split: how a search is run.
/// The tool depends only on this trait; argument parsing and the schema stay in the tool.
#[async_trait]
pub trait SessionSearchProvider: Send + Sync {
    async fn search(&self, filters: SearchFilters) -> Result<SearchOutcome>;
}

/// Similarity under which a meaning match is noise for the local Model2Vec model.
const MIN_MEANING_SIMILARITY: f32 = 0.45;

pub struct SqliteSessionSearchProvider {
    db: Arc<Database>,
    embedder: Option<(QueryEmbedder, String)>,
}

#[async_trait]
impl SessionSearchProvider for SqliteSessionSearchProvider {
    async fn search(&self, filters: SearchFilters) -> Result<SearchOutcome> {
        let f = chat_search::Filters {
            thread_id: filters.thread_id.clone(),
            role: filters.role.clone(),
            after: filters.after.clone(),
            before: filters.before.clone(),
            limit: filters.limit,
        };
        let query_vector = match &self.embedder {
            Some((embed, model)) => match embed(filters.query.clone()).await {
                Ok(v) => Some((v, model.clone())),
                Err(e) => {
                    tracing::debug!("session_search: no query embedding, keyword only: {e:#}");
                    None
                }
            },
            None => None,
        };
        self.db.with_conn(|conn| {
            let mut messages = chat_search::search_messages(conn, &filters.query, &f)?;
            if messages.is_empty() {
                messages = substring_scan(conn, &filters)?;
            }
            let mut summaries = chat_search::search_folds(conn, &filters.query, &f)?;
            if let Some((vector, model)) = &query_vector {
                for hit in chat_search::search_folds_by_meaning(
                    conn,
                    vector,
                    model,
                    &f,
                    MIN_MEANING_SIMILARITY,
                )? {
                    if !summaries.iter().any(|s| s.fold_ref == hit.fold_ref) {
                        summaries.push(hit);
                    }
                }
            }
            summaries.truncate(5);
            Ok(SearchOutcome {
                messages,
                summaries,
            })
        })
    }
}

/// The substring scan session_search always had: finds partial words, ids and code fragments
/// that the keyword index splits differently.
fn substring_scan(conn: &rusqlite::Connection, f: &SearchFilters) -> Result<Vec<MessageHit>> {
    let pattern = format!("%{}%", escape_like(&f.query));
    let mut stmt = conn.prepare(
        "SELECT message_id, thread_id, role, content, created_at
           FROM chat_messages
          WHERE content LIKE ?1 ESCAPE '\\'
            AND (?2 IS NULL OR thread_id = ?2)
            AND (?3 IS NULL OR role = ?3)
            AND (?4 IS NULL OR created_at > ?4)
            AND (?5 IS NULL OR created_at < ?5)
          ORDER BY created_at DESC
          LIMIT ?6",
    )?;
    let rows = stmt.query_map(
        params![pattern, f.thread_id, f.role, f.after, f.before, f.limit],
        |row| {
            Ok(ChatMessage {
                message_id: row.get(0)?,
                thread_id: row.get(1)?,
                role: row.get(2)?,
                content: row.get(3)?,
                created_at: row.get(4)?,
            })
        },
    )?;
    let mut out = Vec::new();
    for row in rows {
        let message = row?;
        let folded_into = chat_search::covering_fold(conn, &message)?.map(|b| b.short_ref());
        out.push(MessageHit {
            message_ref: thread_folds::message_ref(&message.message_id),
            message,
            folded_into,
        });
    }
    Ok(out)
}

pub struct SessionSearchTool {
    provider: Arc<dyn SessionSearchProvider>,
}

impl SessionSearchTool {
    /// Searches with the on-device embedder when the owner chose local embeddings; keyword only
    /// otherwise, so a search never becomes a paid network call.
    pub fn new(db: Arc<Database>) -> Self {
        Self {
            provider: Arc::new(SqliteSessionSearchProvider {
                db,
                embedder: crate::vault::configured_query_embedder(),
            }),
        }
    }

    /// Inject a non-default provider (e.g. a fake for tests, or a future alternate backend).
    #[cfg(test)]
    pub fn with_provider(provider: Arc<dyn SessionSearchProvider>) -> Self {
        Self { provider }
    }

    #[cfg(test)]
    fn with_embedder(db: Arc<Database>, embedder: (QueryEmbedder, String)) -> Self {
        Self {
            provider: Arc::new(SqliteSessionSearchProvider {
                db,
                embedder: Some(embedder),
            }),
        }
    }
}

#[async_trait]
impl HqTool for SessionSearchTool {
    fn name(&self) -> &str {
        "session_search"
    }

    fn description(&self) -> &str {
        "Search earlier chat history: messages by keywords, role, thread and date, plus the \
         summaries that older parts of long chats were folded into. A folded message says which \
         summary (f:...) stands for it; history_restore returns the originals."
    }

    fn behavioral_prompt(&self) -> Option<&str> {
        Some(
            "Older parts of a long chat reach you only as summaries marked \"Earlier in this \
             chat\". When the owner asks about something from before and the summary lacks the \
             exact amount, date, quote or step, search with session_search and read the originals \
             with history_restore before answering, instead of guessing.",
        )
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Words to search for in messages and summaries."
                },
                "thread_id": {
                    "type": "string",
                    "description": "Only this chat thread."
                },
                "role": {
                    "type": "string",
                    "description": "Only messages from this role, \"user\" or \"assistant\"."
                },
                "after": {
                    "type": "string",
                    "description": "Only messages after this ISO-8601 time."
                },
                "before": {
                    "type": "string",
                    "description": "Only messages before this ISO-8601 time."
                },
                "limit": {
                    "type": "integer",
                    "default": 20,
                    "maximum": 100,
                    "description": "Max messages."
                }
            },
            "required": ["query"]
        })
    }

    fn category(&self) -> &str {
        "memory"
    }

    fn search_hint(&self) -> Option<&str> {
        Some("search past conversation messages and folded chat summaries")
    }

    fn is_read_only(&self) -> bool {
        true
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if query.is_empty() {
            return Ok(json!({ "error": "missing required field: query" }));
        }
        let text = |k: &str| args.get(k).and_then(|v| v.as_str()).map(str::to_string);
        let limit = args
            .get("limit")
            .and_then(|v| v.as_i64())
            .unwrap_or(20)
            .clamp(1, 100);

        let outcome = self
            .provider
            .search(SearchFilters {
                query: query.to_string(),
                thread_id: text("thread_id"),
                role: text("role"),
                after: text("after"),
                before: text("before"),
                limit,
            })
            .await?;

        let count = outcome.messages.len();
        let results: Vec<Value> = outcome
            .messages
            .into_iter()
            .map(|h| {
                let mut v = json!({
                    "message_id": h.message.message_id,
                    "message_ref": h.message_ref,
                    "thread_id": h.message.thread_id,
                    "role": h.message.role,
                    "content": h.message.content,
                    "created_at": h.message.created_at,
                });
                if let Some(f) = h.folded_into {
                    v["folded_into"] = json!(f);
                }
                v
            })
            .collect();
        let mut out = json!({ "results": results, "count": count });
        if !outcome.summaries.is_empty() {
            out["summaries"] = json!(outcome.summaries);
        }
        Ok(out)
    }
}

/// Most messages one `history_restore` call returns.
const MAX_RESTORE_MESSAGES: i64 = 60;

/// Characters of message text one `history_restore` call returns at most.
const MAX_RESTORE_CHARS: usize = 40_000;

/// `history_restore`: the original messages behind a fold block, or around one message.
pub struct HistoryRestoreTool {
    db: Arc<Database>,
}

impl HistoryRestoreTool {
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    fn restore(&self, reference: &str, thread_id: Option<&str>, around: i64) -> Result<Value> {
        let reference = reference.trim();
        self.db.with_conn(|conn| {
            if let Some(fold) = thread_folds::find_by_ref(conn, thread_id, reference)? {
                let messages = thread_folds::messages_between(
                    conn,
                    &fold.thread_id,
                    &fold.first_msg_id,
                    &fold.last_msg_id,
                    MAX_RESTORE_MESSAGES,
                )?;
                return Ok(render(
                    json!({"fold_ref": fold.short_ref(), "thread_id": fold.thread_id, "summarized_messages": fold.message_count}),
                    messages.into_iter().map(|m| m.message).collect(),
                    fold.message_count,
                ));
            }
            let Some(message) = find_message(conn, thread_id, reference)? else {
                return Ok(json!({
                    "error": format!("nothing found for {reference}: pass a fold ref (f:...) or a message ref (m:...) from the chat or from session_search")
                }));
            };
            let around = around.clamp(0, 20);
            let before = context_before(conn, &message, around)?;
            let after = thread_folds::messages_after(conn, &message.thread_id, Some(&message.message_id), 10_000)?
                .into_iter()
                .take(around as usize)
                .map(|m| m.message);
            let mut all = before;
            all.push(message.clone());
            all.extend(after);
            let n = all.len() as i64;
            Ok(render(
                json!({"message_ref": thread_folds::message_ref(&message.message_id), "thread_id": message.thread_id}),
                all,
                n,
            ))
        })
    }
}

fn find_message(
    conn: &rusqlite::Connection,
    thread_id: Option<&str>,
    reference: &str,
) -> Result<Option<ChatMessage>> {
    let prefix = reference.trim_start_matches("m:");
    if prefix.len() < 4 || !prefix.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
        return Ok(None);
    }
    let row = conn.query_row(
        "SELECT message_id, thread_id, role, content, created_at FROM chat_messages
          WHERE substr(message_id, 1, length(?1)) = ?1 AND (?2 IS NULL OR thread_id = ?2)
          ORDER BY created_at DESC LIMIT 1",
        params![prefix, thread_id],
        |row| {
            Ok(ChatMessage {
                message_id: row.get(0)?,
                thread_id: row.get(1)?,
                role: row.get(2)?,
                content: row.get(3)?,
                created_at: row.get(4)?,
            })
        },
    );
    match row {
        Ok(m) => Ok(Some(m)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn context_before(
    conn: &rusqlite::Connection,
    message: &ChatMessage,
    n: i64,
) -> Result<Vec<ChatMessage>> {
    if n == 0 {
        return Ok(Vec::new());
    }
    Ok(
        hq_db::chat::get_messages_page(conn, &message.thread_id, n, Some(&message.message_id))?
            .into_iter()
            .map(|m| m.message)
            .collect(),
    )
}

fn render(mut head: Value, messages: Vec<ChatMessage>, total: i64) -> Value {
    let mut used = 0usize;
    let mut out = Vec::new();
    let mut cut = false;
    for m in messages {
        if used + m.content.len() > MAX_RESTORE_CHARS {
            cut = true;
            break;
        }
        used += m.content.len();
        out.push(json!({
            "message_ref": thread_folds::message_ref(&m.message_id),
            "role": m.role,
            "created_at": m.created_at,
            "content": m.content,
        }));
    }
    let shown = out.len() as i64;
    head["messages"] = json!(out);
    if cut || shown < total {
        head["note"] = json!(format!(
            "showing {shown} of {total} messages; restore a later message ref (m:...) with around to read on"
        ));
    }
    head
}

#[async_trait]
impl HqTool for HistoryRestoreTool {
    fn name(&self) -> &str {
        "history_restore"
    }

    fn description(&self) -> &str {
        "Read the original messages behind a summary of an earlier part of a chat (pass its ref, \
         f:...), or one earlier message with the messages around it (pass m:... and around)."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "ref": {
                    "type": "string",
                    "description": "A fold ref (f:1a2b3c4d) from an \"Earlier in this chat\" summary or session_search, or a message ref (m:1a2b3c4d)."
                },
                "thread_id": {
                    "type": "string",
                    "description": "Only look in this chat thread."
                },
                "around": {
                    "type": "integer",
                    "default": 3,
                    "maximum": 20,
                    "description": "With a message ref: how many messages before and after it to include."
                }
            },
            "required": ["ref"]
        })
    }

    fn category(&self) -> &str {
        "memory"
    }

    fn search_hint(&self) -> Option<&str> {
        Some("restore original chat messages behind a folded summary")
    }

    fn is_read_only(&self) -> bool {
        true
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let Some(reference) = args.get("ref").and_then(|v| v.as_str()) else {
            return Ok(json!({ "error": "missing required field: ref" }));
        };
        let thread_id = args.get("thread_id").and_then(|v| v.as_str());
        let around = args.get("around").and_then(|v| v.as_i64()).unwrap_or(3);
        self.restore(reference, thread_id, around)
    }
}

fn escape_like(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if ch == '\\' || ch == '%' || ch == '_' {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod provider_injection_tests {
    use super::*;
    use serde_json::json;

    /// A fake provider returning canned results: the tool depends only on the trait.
    struct FakeSessionSearchProvider {
        canned: Vec<ChatMessage>,
    }

    #[async_trait]
    impl SessionSearchProvider for FakeSessionSearchProvider {
        async fn search(&self, _filters: SearchFilters) -> Result<SearchOutcome> {
            Ok(SearchOutcome {
                messages: self
                    .canned
                    .iter()
                    .cloned()
                    .map(|message| MessageHit {
                        message_ref: thread_folds::message_ref(&message.message_id),
                        message,
                        folded_into: None,
                    })
                    .collect(),
                summaries: Vec::new(),
            })
        }
    }

    #[tokio::test]
    async fn injected_provider_is_used_instead_of_sqlite() {
        let canned = vec![ChatMessage {
            message_id: "m1".to_string(),
            thread_id: "t1".to_string(),
            role: "assistant".to_string(),
            content: "fake result, no db queried".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
        }];
        let tool = SessionSearchTool::with_provider(Arc::new(FakeSessionSearchProvider { canned }));

        let result = tool.execute(json!({ "query": "anything" })).await.unwrap();
        let results = result["results"].as_array().unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["content"], "fake result, no db queried");
        assert_eq!(result["count"], 1);
    }
}

#[cfg(test)]
mod fold_tests {
    use super::*;
    use hq_db::chat::{add_message, create_thread};
    use hq_db::thread_folds::{NewFold, insert};

    fn folded(db: &Database) -> (String, Vec<String>, String) {
        db.with_conn(|conn| {
            let t = create_thread(conn, "t", "user", "user")?.thread_id;
            let mut ids = Vec::new();
            for (role, text) in [
                ("user", "The deposit is UGX 450,000"),
                ("assistant", "Noted the deposit"),
                ("user", "Pay it on Friday"),
                ("assistant", "Will remind you Friday"),
                ("user", "What time is the meeting"),
                ("assistant", "At 10"),
            ] {
                ids.push(add_message(conn, &t, role, text)?.message_id);
            }
            let fold = insert(
                conn,
                &NewFold {
                    thread_id: &t,
                    tier: 1,
                    first_msg_id: &ids[0],
                    last_msg_id: &ids[3],
                    message_count: 4,
                    summary: "Goal as of this block: rental deposit paid on Friday",
                    source_tokens: 40,
                    summary_tokens: 10,
                    tainted: false,
                    model: "test",
                },
            )?;
            Ok((t, ids, fold.short_ref()))
        })
        .unwrap()
    }

    #[tokio::test]
    async fn search_names_the_block_a_message_was_folded_into() {
        let db = Arc::new(Database::open_memory().unwrap());
        let (_, _, fold_ref) = folded(&db);
        let tool = SessionSearchTool::new(db);
        let out = tool.execute(json!({"query": "deposit"})).await.unwrap();
        let results = out["results"].as_array().unwrap();
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|r| r["folded_into"] == fold_ref));
        assert_eq!(out["summaries"][0]["fold_ref"], fold_ref);
        let out = tool.execute(json!({"query": "meeting"})).await.unwrap();
        assert!(out["results"][0].get("folded_into").is_none());
    }

    #[tokio::test]
    async fn a_meaning_match_finds_a_summary_without_shared_words() {
        let db = Arc::new(Database::open_memory().unwrap());
        let (_, _, fold_ref) = folded(&db);
        db.with_conn(|conn| {
            let fold = thread_folds::find_by_ref(conn, None, &fold_ref)?.unwrap();
            chat_search::store_fold_embedding(conn, &fold, "fake", &[1.0, 0.0])
        })
        .unwrap();
        let embed: QueryEmbedder = Arc::new(|_| Box::pin(async { Ok(vec![0.95, 0.05]) }));
        let tool = SessionSearchTool::with_embedder(db, (embed, "fake".into()));
        let out = tool
            .execute(json!({"query": "landlord money"}))
            .await
            .unwrap();
        assert_eq!(out["summaries"][0]["fold_ref"], fold_ref);
        assert_eq!(out["summaries"][0]["matched_by"], "meaning");
    }

    #[tokio::test]
    async fn restore_returns_a_blocks_originals_and_a_message_with_context() {
        let db = Arc::new(Database::open_memory().unwrap());
        let (t, ids, fold_ref) = folded(&db);
        let tool = HistoryRestoreTool::new(db);
        let out = tool.execute(json!({"ref": fold_ref})).await.unwrap();
        let msgs = out["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 4);
        assert_eq!(msgs[0]["content"], "The deposit is UGX 450,000");

        let mref = thread_folds::message_ref(&ids[4]);
        let out = tool
            .execute(json!({"ref": mref, "around": 1, "thread_id": t}))
            .await
            .unwrap();
        let msgs = out["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[1]["content"], "What time is the meeting");

        let out = tool.execute(json!({"ref": "f:zz"})).await.unwrap();
        assert!(out.get("error").is_some());
        let out = tool
            .execute(json!({"ref": fold_ref, "thread_id": "another"}))
            .await
            .unwrap();
        assert!(out.get("error").is_some(), "a thread filter is honored");
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use hq_db::chat::{add_message, create_thread};
    use serde_json::json;
    use uuid::Uuid;

    #[tokio::test]
    async fn empty_query_errors() -> Result<()> {
        let db = Arc::new(Database::open_memory()?);
        let tool = SessionSearchTool::new(db);
        let result = tool.execute(json!({})).await?;
        assert!(result.get("error").is_some());
        Ok(())
    }

    #[tokio::test]
    async fn finds_message_by_keyword() -> Result<()> {
        let db = Arc::new(Database::open_memory()?);
        let tool = SessionSearchTool::new(db.clone());
        let (thread_id, msg) = db.with_conn(|conn| {
            let t = create_thread(conn, "Search Test", "user", "user")?;
            add_message(conn, &t.thread_id, "user", "hello world")?;
            let m = add_message(conn, &t.thread_id, "assistant", "goodbye world")?;
            Ok((t.thread_id, m))
        })?;

        let result = tool.execute(json!({ "query": "goodbye" })).await?;
        let results = result["results"].as_array().unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["content"], "goodbye world");
        assert_eq!(results[0]["thread_id"], thread_id);
        assert_eq!(results[0]["message_id"], msg.message_id);
        Ok(())
    }

    #[tokio::test]
    async fn filters_by_role_and_date() -> Result<()> {
        let db = Arc::new(Database::open_memory()?);
        let tool = SessionSearchTool::new(db.clone());
        let thread_id = db.with_conn(|conn| {
            let t = create_thread(conn, "Filter Test", "user", "user")?;
            let id = t.thread_id.clone();
            conn.execute(
                "INSERT INTO chat_messages \
                    (message_id, thread_id, role, content, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    Uuid::new_v4().to_string(),
                    id,
                    "user",
                    "searchable old",
                    "2026-07-01T12:00:00Z"
                ],
            )?;
            conn.execute(
                "INSERT INTO chat_messages \
                    (message_id, thread_id, role, content, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    Uuid::new_v4().to_string(),
                    id,
                    "assistant",
                    "searchable new",
                    "2026-07-25T12:00:00Z"
                ],
            )?;
            Ok::<_, anyhow::Error>(id)
        })?;

        let result = tool
            .execute(json!({
                "query": "searchable",
                "role": "assistant"
            }))
            .await?;
        let results = result["results"].as_array().unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["role"], "assistant");
        assert_eq!(results[0]["content"], "searchable new");

        let result = tool
            .execute(json!({
                "query": "searchable",
                "after": "2026-07-20T00:00:00Z"
            }))
            .await?;
        let results = result["results"].as_array().unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["content"], "searchable new");

        let result = tool
            .execute(json!({
                "query": "searchable",
                "before": "2026-07-20T00:00:00Z"
            }))
            .await?;
        let results = result["results"].as_array().unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["content"], "searchable old");

        let result = tool
            .execute(json!({
                "query": "searchable",
                "thread_id": thread_id
            }))
            .await?;
        let results = result["results"].as_array().unwrap();
        assert_eq!(results.len(), 2);

        Ok(())
    }

    #[tokio::test]
    async fn limit_is_respected() -> Result<()> {
        let db = Arc::new(Database::open_memory()?);
        let tool = SessionSearchTool::new(db.clone());
        let _thread_id = db.with_conn(|conn| {
            let t = create_thread(conn, "Limit Test", "user", "user")?;
            let id = t.thread_id.clone();
            for i in 0..25 {
                add_message(conn, &id, "assistant", &format!("keyword {i}"))?;
            }
            Ok::<_, anyhow::Error>(id)
        })?;

        let result = tool
            .execute(json!({
                "query": "keyword",
                "limit": 5
            }))
            .await?;
        let results = result["results"].as_array().unwrap();
        assert_eq!(results.len(), 5);
        assert_eq!(result["count"], 5);

        let result = tool
            .execute(json!({
                "query": "keyword",
                "limit": 200
            }))
            .await?;
        let results = result["results"].as_array().unwrap();
        assert_eq!(results.len(), 25);

        Ok(())
    }
}
