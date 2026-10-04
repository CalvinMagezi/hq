//! `session_search` tool — search past agent/user conversation messages stored
//! in the SQLite `chat_messages` table.

use anyhow::Result;
use async_trait::async_trait;
use hq_db::{Database, chat::ChatMessage};
use rusqlite::params;
use serde_json::{Value, json};
use std::sync::Arc;

use crate::registry::HqTool;

/// Filters for one `session_search` query, already normalized (LIKE-escaped
/// pattern wrapped in `%`, limit clamped to 1..=100) — see `execute`'s
/// argument parsing.
pub struct SearchFilters {
    pub pattern: String,
    pub thread_id: Option<String>,
    pub role: Option<String>,
    pub after: Option<String>,
    pub before: Option<String>,
    pub limit: i64,
}

/// The Provider side of `SessionSearchTool`'s Definition/Consumer split: how
/// a search is actually run. The tool (Consumer) depends only on this trait
/// for results — argument parsing/normalization and the JSON schema stay in
/// the tool. [`SqliteSessionSearchProvider`] (the `chat_messages` table) is
/// the only implementation today.
#[async_trait]
pub trait SessionSearchProvider: Send + Sync {
    async fn search(&self, filters: SearchFilters) -> Result<Vec<ChatMessage>>;
}

pub struct SqliteSessionSearchProvider {
    db: Arc<Database>,
}

#[async_trait]
impl SessionSearchProvider for SqliteSessionSearchProvider {
    async fn search(&self, filters: SearchFilters) -> Result<Vec<ChatMessage>> {
        let sql = "SELECT message_id, thread_id, role, content, created_at \
                     FROM chat_messages \
                    WHERE content LIKE ?1 ESCAPE '\\' \
                      AND (?2 IS NULL OR thread_id = ?2) \
                      AND (?3 IS NULL OR role = ?3) \
                      AND (?4 IS NULL OR created_at > ?4) \
                      AND (?5 IS NULL OR created_at < ?5) \
                    ORDER BY created_at DESC \
                    LIMIT ?6";

        self.db.with_conn(|conn| {
            let mut stmt = conn.prepare(sql)?;
            let rows = stmt.query_map(
                params![
                    filters.pattern,
                    filters.thread_id,
                    filters.role,
                    filters.after,
                    filters.before,
                    filters.limit
                ],
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
                out.push(row?);
            }
            Ok::<_, anyhow::Error>(out)
        })
    }
}

pub struct SessionSearchTool {
    provider: Arc<dyn SessionSearchProvider>,
}

impl SessionSearchTool {
    pub fn new(db: Arc<Database>) -> Self {
        Self {
            provider: Arc::new(SqliteSessionSearchProvider { db }),
        }
    }

    /// Inject a non-default provider (e.g. a fake for tests, or a future
    /// alternate search backend). The Consumer side (this tool's schema and
    /// argument parsing) never needs to change when this does.
    #[cfg(test)]
    pub fn with_provider(provider: Arc<dyn SessionSearchProvider>) -> Self {
        Self { provider }
    }
}

#[async_trait]
impl HqTool for SessionSearchTool {
    fn name(&self) -> &str {
        "session_search"
    }

    fn description(&self) -> &str {
        "Search past agent/user conversation messages by keywords, role, \
         thread/session id, and date range."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Substring to search for in message content."
                },
                "thread_id": {
                    "type": "string",
                    "description": "Filter to a specific chat thread/session id."
                },
                "role": {
                    "type": "string",
                    "description": "Filter by role, e.g. \"user\" or \"assistant\"."
                },
                "after": {
                    "type": "string",
                    "description": "Include messages after this ISO-8601 time."
                },
                "before": {
                    "type": "string",
                    "description": "Include messages before this ISO-8601 time."
                },
                "limit": {
                    "type": "integer",
                    "default": 20,
                    "maximum": 100,
                    "description": "Max results."
                }
            },
            "required": ["query"]
        })
    }

    fn category(&self) -> &str {
        "memory"
    }

    fn search_hint(&self) -> Option<&str> {
        Some("search past conversation messages")
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

        let pattern = format!("%{}%", escape_like(query));
        let thread_id: Option<String> = args
            .get("thread_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let role: Option<String> = args
            .get("role")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let after: Option<String> = args
            .get("after")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let before: Option<String> = args
            .get("before")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let limit = args
            .get("limit")
            .and_then(|v| v.as_i64())
            .unwrap_or(20)
            .clamp(1, 100);

        let results = self
            .provider
            .search(SearchFilters {
                pattern,
                thread_id,
                role,
                after,
                before,
                limit,
            })
            .await?;

        let count = results.len();
        let results_json = results
            .into_iter()
            .map(|m| {
                json!({
                    "message_id": m.message_id,
                    "thread_id": m.thread_id,
                    "role": m.role,
                    "content": m.content,
                    "created_at": m.created_at
                })
            })
            .collect::<Vec<_>>();

        Ok(json!({
            "results": results_json,
            "count": count
        }))
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

    /// A fake `SessionSearchProvider` returning canned results — proves
    /// `SessionSearchTool` genuinely depends only on the `SessionSearchProvider`
    /// trait (the Provider seam), not on `hq_db::Database` directly.
    struct FakeSessionSearchProvider {
        canned: Vec<ChatMessage>,
    }

    #[async_trait]
    impl SessionSearchProvider for FakeSessionSearchProvider {
        async fn search(&self, _filters: SearchFilters) -> Result<Vec<ChatMessage>> {
            Ok(self.canned.clone())
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
