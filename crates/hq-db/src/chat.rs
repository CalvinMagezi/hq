use anyhow::Result;
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatThread {
    pub thread_id: String,
    pub title: String,
    pub thread_type: String,
    pub initiated_by: String,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
    pub last_message_preview: Option<String>,
    pub unread_count: i64,
    pub note_path: Option<String>,
    /// Origin platform: `web` (native Web UI chat), `telegram` or `discord`.
    #[serde(default = "default_platform")]
    pub platform: String,
    /// External chat/channel/contact identifier on the origin platform (e.g. a
    /// Telegram chat_id or a Discord channel_id). `None` for `web` threads.
    #[serde(default)]
    pub external_id: Option<String>,
}

fn default_platform() -> String {
    "web".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub message_id: String,
    pub thread_id: String,
    pub role: String,
    pub content: String,
    pub created_at: String,
}

pub fn create_thread(
    conn: &Connection,
    title: &str,
    initiated_by: &str,
    thread_type: &str,
) -> Result<ChatThread> {
    let thread_id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO chat_threads
            (thread_id, title, thread_type, initiated_by, status, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, 'active', ?5, ?6)",
        params![thread_id, title, thread_type, initiated_by, now, now],
    )?;
    get_thread(conn, &thread_id)?.ok_or_else(|| anyhow::anyhow!("thread not found after create"))
}

pub fn get_thread(conn: &Connection, thread_id: &str) -> Result<Option<ChatThread>> {
    let result = conn
        .query_row(
            "SELECT thread_id, title, thread_type, initiated_by, status,
                    created_at, updated_at, last_message_preview, unread_count, note_path,
                    platform, external_id
               FROM chat_threads WHERE thread_id = ?1",
            params![thread_id],
            |row| {
                Ok(ChatThread {
                    thread_id: row.get(0)?,
                    title: row.get(1)?,
                    thread_type: row.get(2)?,
                    initiated_by: row.get(3)?,
                    status: row.get(4)?,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                    last_message_preview: row.get(7)?,
                    unread_count: row.get(8)?,
                    note_path: row.get(9)?,
                    platform: row.get(10)?,
                    external_id: row.get(11)?,
                })
            },
        )
        .optional()?;
    Ok(result)
}

/// Find the thread bound to a given platform + external chat/channel/contact id, or
/// create one if none exists yet. Idempotent: repeated calls for the same
/// `(platform, external_id)` always return the same thread.
pub fn find_or_create_platform_thread(
    conn: &Connection,
    platform: &str,
    external_id: &str,
    title: &str,
) -> Result<ChatThread> {
    let existing = conn
        .query_row(
            "SELECT thread_id FROM chat_threads WHERE platform = ?1 AND external_id = ?2",
            params![platform, external_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;

    if let Some(thread_id) = existing {
        return get_thread(conn, &thread_id)?
            .ok_or_else(|| anyhow::anyhow!("thread not found after lookup"));
    }

    let thread_id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO chat_threads
            (thread_id, title, thread_type, initiated_by, status, created_at, updated_at,
             platform, external_id)
         VALUES (?1, ?2, 'relay', ?3, 'active', ?4, ?5, ?6, ?7)",
        params![thread_id, title, platform, now, now, platform, external_id],
    )?;
    get_thread(conn, &thread_id)?.ok_or_else(|| anyhow::anyhow!("thread not found after create"))
}

pub fn set_thread_title(conn: &Connection, thread_id: &str, title: &str) -> Result<()> {
    conn.execute(
        "UPDATE chat_threads SET title = ?1 WHERE thread_id = ?2",
        params![title, thread_id],
    )?;
    Ok(())
}

pub fn list_threads(
    conn: &Connection,
    limit: i64,
    offset: i64,
    platform: Option<&str>,
) -> Result<Vec<ChatThread>> {
    let base = "SELECT thread_id, title, thread_type, initiated_by, status,
                created_at, updated_at, last_message_preview, unread_count, note_path,
                platform, external_id
           FROM chat_threads
          WHERE status = 'active'";

    let map_row = |row: &rusqlite::Row| -> rusqlite::Result<ChatThread> {
        Ok(ChatThread {
            thread_id: row.get(0)?,
            title: row.get(1)?,
            thread_type: row.get(2)?,
            initiated_by: row.get(3)?,
            status: row.get(4)?,
            created_at: row.get(5)?,
            updated_at: row.get(6)?,
            last_message_preview: row.get(7)?,
            unread_count: row.get(8)?,
            note_path: row.get(9)?,
            platform: row.get(10)?,
            external_id: row.get(11)?,
        })
    };

    let mut out = Vec::new();
    if let Some(platform) = platform {
        let sql = format!(
            "{base} AND platform = ?1 ORDER BY updated_at DESC, created_at DESC, thread_id DESC LIMIT ?2 OFFSET ?3"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![platform, limit, offset], map_row)?;
        for row in rows {
            out.push(row?);
        }
    } else {
        let sql = format!(
            "{base} ORDER BY updated_at DESC, created_at DESC, thread_id DESC LIMIT ?1 OFFSET ?2"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![limit, offset], map_row)?;
        for row in rows {
            out.push(row?);
        }
    }
    Ok(out)
}

/// A message with its web-chat metadata (tool calls, reasoning), parsed from the `meta` column.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredMessage {
    #[serde(flatten)]
    pub message: ChatMessage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<serde_json::Value>,
}

const PREVIEW_CHARS: usize = 120;

pub fn add_message(
    conn: &Connection,
    thread_id: &str,
    role: &str,
    content: &str,
) -> Result<ChatMessage> {
    add_message_with_meta(conn, thread_id, role, content, None)
}

/// `add_message` plus a JSON `meta` blob stored beside it.
pub fn add_message_with_meta(
    conn: &Connection,
    thread_id: &str,
    role: &str,
    content: &str,
    meta: Option<&serde_json::Value>,
) -> Result<ChatMessage> {
    let message_id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    let preview: String = content.chars().take(PREVIEW_CHARS).collect();
    let meta_text = meta.map(serde_json::Value::to_string);

    conn.execute(
        "INSERT INTO chat_messages (message_id, thread_id, role, content, created_at, meta)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![message_id, thread_id, role, content, now, meta_text],
    )?;

    conn.execute(
        "UPDATE chat_threads SET updated_at = ?1, last_message_preview = ?2
          WHERE thread_id = ?3",
        params![now, preview, thread_id],
    )?;

    Ok(ChatMessage {
        message_id,
        thread_id: thread_id.to_string(),
        role: role.to_string(),
        content: content.to_string(),
        created_at: now,
    })
}

/// The most recent `limit` messages of a thread, oldest first.
pub fn get_messages(conn: &Connection, thread_id: &str, limit: i64) -> Result<Vec<ChatMessage>> {
    let mut stmt = conn.prepare(
        "SELECT message_id, thread_id, role, content, created_at FROM (
            SELECT rowid AS rid, message_id, thread_id, role, content, created_at
              FROM chat_messages
             WHERE thread_id = ?1
             ORDER BY created_at DESC, rowid DESC
             LIMIT ?2
         ) ORDER BY created_at ASC, rid ASC",
    )?;
    let rows = stmt.query_map(params![thread_id, limit], |row| {
        Ok(ChatMessage {
            message_id: row.get(0)?,
            thread_id: row.get(1)?,
            role: row.get(2)?,
            content: row.get(3)?,
            created_at: row.get(4)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// Up to `limit` messages with their meta, oldest first: the newest ones, or
/// with `before` the ones just older than that message. An unknown `before`
/// returns nothing rather than the newest page.
pub fn get_messages_page(
    conn: &Connection,
    thread_id: &str,
    limit: i64,
    before: Option<&str>,
) -> Result<Vec<StoredMessage>> {
    let cursor = match before {
        Some(id) => match message_position(conn, thread_id, id)? {
            Some(pos) => Some(pos),
            None => return Ok(Vec::new()),
        },
        None => None,
    };
    let (created, rowid) = cursor.unzip();
    let mut stmt = conn.prepare(
        "SELECT message_id, thread_id, role, content, created_at, meta FROM (
            SELECT rowid AS rid, message_id, thread_id, role, content, created_at, meta
              FROM chat_messages
             WHERE thread_id = ?1 AND (?2 IS NULL OR (created_at, rowid) < (?2, ?3))
             ORDER BY created_at DESC, rowid DESC
             LIMIT ?4
         ) ORDER BY created_at ASC, rid ASC",
    )?;
    let rows = stmt.query_map(params![thread_id, created, rowid, limit], |row| {
        let meta: Option<String> = row.get(5)?;
        Ok(StoredMessage {
            message: ChatMessage {
                message_id: row.get(0)?,
                thread_id: row.get(1)?,
                role: row.get(2)?,
                content: row.get(3)?,
                created_at: row.get(4)?,
            },
            meta: meta.and_then(|m| serde_json::from_str(&m).ok()),
        })
    })?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(Into::into)
}

/// Where a message sits in its thread's order, or None when it is not in that thread.
fn message_position(
    conn: &Connection,
    thread_id: &str,
    message_id: &str,
) -> Result<Option<(String, i64)>> {
    conn.query_row(
        "SELECT created_at, rowid FROM chat_messages WHERE thread_id = ?1 AND message_id = ?2",
        params![thread_id, message_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
    .map_err(Into::into)
}

/// Deletes a message and every later one in its thread (an edit or a
/// regenerate replaces them). Returns how many went; 0 when the message is
/// not in that thread. The thread's preview falls back to its new last message.
pub fn delete_messages_from(conn: &Connection, thread_id: &str, message_id: &str) -> Result<usize> {
    let Some((created, rowid)) = message_position(conn, thread_id, message_id)? else {
        return Ok(0);
    };
    let deleted = conn.execute(
        "DELETE FROM chat_messages WHERE thread_id = ?1 AND (created_at, rowid) >= (?2, ?3)",
        params![thread_id, created, rowid],
    )?;
    let last: Option<String> = conn
        .query_row(
            "SELECT content FROM chat_messages WHERE thread_id = ?1 ORDER BY created_at DESC, rowid DESC LIMIT 1",
            params![thread_id],
            |row| row.get(0),
        )
        .optional()?;
    let preview = last.map(|c| c.chars().take(PREVIEW_CHARS).collect::<String>());
    conn.execute(
        "UPDATE chat_threads SET last_message_preview = ?1 WHERE thread_id = ?2",
        params![preview, thread_id],
    )?;
    Ok(deleted)
}

/// Whether any chat message, in any thread (archived ones too), contains `needle`.
pub fn any_message_mentions(conn: &Connection, needle: &str) -> Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM chat_messages WHERE instr(content, ?1) > 0)",
        params![needle],
        |row| row.get(0),
    )
    .map_err(Into::into)
}

pub fn archive_thread(conn: &Connection, thread_id: &str) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "UPDATE chat_threads SET status = 'archived', updated_at = ?1
          WHERE thread_id = ?2",
        params![now, thread_id],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;

    #[test]
    fn create_and_get_thread() -> Result<()> {
        let db = Database::open_memory()?;
        db.with_conn(|conn| {
            let t = create_thread(conn, "Test Thread", "user", "user")?;
            assert_eq!(t.title, "Test Thread");
            assert_eq!(t.status, "active");
            assert_eq!(t.thread_type, "user");

            let got = get_thread(conn, &t.thread_id)?.unwrap();
            assert_eq!(got.thread_id, t.thread_id);
            assert_eq!(got.unread_count, 0);
            Ok(())
        })
    }

    #[test]
    fn get_thread_missing_returns_none() -> Result<()> {
        let db = Database::open_memory()?;
        db.with_conn(|conn| {
            let result = get_thread(conn, "nonexistent-id")?;
            assert!(result.is_none());
            Ok(())
        })
    }

    #[test]
    fn list_threads_returns_active_only() -> Result<()> {
        let db = Database::open_memory()?;
        db.with_conn(|conn| {
            let t1 = create_thread(conn, "Thread 1", "user", "user")?;
            let t2 = create_thread(conn, "Thread 2", "user", "user")?;
            archive_thread(conn, &t1.thread_id)?;

            let active = list_threads(conn, 10, 0, None)?;
            assert_eq!(active.len(), 1);
            assert_eq!(active[0].thread_id, t2.thread_id);
            Ok(())
        })
    }

    #[test]
    fn add_and_get_messages_ordered() -> Result<()> {
        let db = Database::open_memory()?;
        db.with_conn(|conn| {
            let t = create_thread(conn, "Chat", "user", "user")?;
            add_message(conn, &t.thread_id, "user", "Hello")?;
            add_message(conn, &t.thread_id, "assistant", "Hi there")?;

            let msgs = get_messages(conn, &t.thread_id, 10)?;
            assert_eq!(msgs.len(), 2);
            assert_eq!(msgs[0].role, "user");
            assert_eq!(msgs[0].content, "Hello");
            assert_eq!(msgs[1].role, "assistant");

            let thread = get_thread(conn, &t.thread_id)?.unwrap();
            assert_eq!(thread.last_message_preview.as_deref(), Some("Hi there"));
            Ok(())
        })
    }

    #[test]
    fn get_messages_returns_newest_when_limited() -> Result<()> {
        let db = Database::open_memory()?;
        db.with_conn(|conn| {
            let t = create_thread(conn, "Long", "user", "user")?;
            for i in 0..5 {
                add_message(conn, &t.thread_id, "user", &format!("m{i}"))?;
            }
            let msgs = get_messages(conn, &t.thread_id, 2)?;
            let contents: Vec<_> = msgs.iter().map(|m| m.content.as_str()).collect();
            assert_eq!(contents, vec!["m3", "m4"]);
            Ok(())
        })
    }

    #[test]
    fn pages_walk_back_through_history_with_meta() -> Result<()> {
        let db = Database::open_memory()?;
        db.with_conn(|conn| {
            let t = create_thread(conn, "Paged", "user", "user")?;
            let mut ids = Vec::new();
            for i in 0..5 {
                ids.push(add_message(conn, &t.thread_id, "user", &format!("m{i}"))?.message_id);
            }
            let meta = serde_json::json!({"tool_steps": [{"id": "c1", "name": "bash"}]});
            add_message_with_meta(conn, &t.thread_id, "assistant", "done", Some(&meta))?;

            let newest = get_messages_page(conn, &t.thread_id, 2, None)?;
            let contents: Vec<_> = newest.iter().map(|m| m.message.content.as_str()).collect();
            assert_eq!(contents, vec!["m4", "done"]);
            assert_eq!(newest[1].meta.as_ref(), Some(&meta));
            assert!(newest[0].meta.is_none());

            let older = get_messages_page(conn, &t.thread_id, 3, Some(&ids[4]))?;
            let contents: Vec<_> = older.iter().map(|m| m.message.content.as_str()).collect();
            assert_eq!(contents, vec!["m1", "m2", "m3"]);
            assert!(get_messages_page(conn, &t.thread_id, 3, Some("missing"))?.is_empty());
            Ok(())
        })
    }

    #[test]
    fn deleting_from_a_message_drops_it_and_everything_after() -> Result<()> {
        let db = Database::open_memory()?;
        db.with_conn(|conn| {
            let t = create_thread(conn, "Edit", "user", "user")?;
            let other = create_thread(conn, "Other", "user", "user")?;
            add_message(conn, &t.thread_id, "user", "keep")?;
            let from = add_message(conn, &t.thread_id, "user", "edit me")?;
            add_message(conn, &t.thread_id, "assistant", "old reply")?;
            add_message(conn, &other.thread_id, "user", "untouched")?;

            assert_eq!(
                delete_messages_from(conn, &other.thread_id, &from.message_id)?,
                0
            );
            assert_eq!(
                delete_messages_from(conn, &t.thread_id, &from.message_id)?,
                2
            );
            let left: Vec<_> = get_messages(conn, &t.thread_id, 10)?
                .into_iter()
                .map(|m| m.content)
                .collect();
            assert_eq!(left, vec!["keep"]);
            let thread = get_thread(conn, &t.thread_id)?.unwrap();
            assert_eq!(thread.last_message_preview.as_deref(), Some("keep"));
            assert_eq!(get_messages(conn, &other.thread_id, 10)?.len(), 1);
            Ok(())
        })
    }

    #[test]
    fn mentions_search_every_thread_including_archived() -> Result<()> {
        let db = Database::open_memory()?;
        db.with_conn(|conn| {
            let t = create_thread(conn, "Old", "user", "user")?;
            add_message(conn, &t.thread_id, "user", "see _media/web/d/1-0-a.png")?;
            archive_thread(conn, &t.thread_id)?;
            assert!(any_message_mentions(conn, "_media/web/d/1-0-a.png")?);
            assert!(!any_message_mentions(conn, "_media/web/d/2-0-b.png")?);
            Ok(())
        })
    }

    #[test]
    fn messages_cascade_delete_with_thread() -> Result<()> {
        let db = Database::open_memory()?;
        db.with_conn(|conn| {
            let t = create_thread(conn, "Temp", "user", "user")?;
            add_message(conn, &t.thread_id, "user", "msg")?;

            conn.execute(
                "DELETE FROM chat_threads WHERE thread_id = ?1",
                params![t.thread_id],
            )?;

            let msgs = get_messages(conn, &t.thread_id, 10)?;
            assert!(msgs.is_empty(), "cascade delete should remove messages");
            Ok(())
        })
    }

    #[test]
    fn archive_thread_hides_from_list() -> Result<()> {
        let db = Database::open_memory()?;
        db.with_conn(|conn| {
            let t = create_thread(conn, "Archive me", "user", "user")?;
            archive_thread(conn, &t.thread_id)?;

            let thread = get_thread(conn, &t.thread_id)?.unwrap();
            assert_eq!(thread.status, "archived");

            let active = list_threads(conn, 10, 0, None)?;
            assert!(active.is_empty());
            Ok(())
        })
    }

    #[test]
    fn create_thread_defaults_to_web_platform() -> Result<()> {
        let db = Database::open_memory()?;
        db.with_conn(|conn| {
            let t = create_thread(conn, "Native chat", "user", "user")?;
            assert_eq!(t.platform, "web");
            assert!(t.external_id.is_none());
            Ok(())
        })
    }

    #[test]
    fn find_or_create_platform_thread_is_idempotent() -> Result<()> {
        let db = Database::open_memory()?;
        db.with_conn(|conn| {
            let t1 = find_or_create_platform_thread(conn, "telegram", "12345", "Telegram: Alice")?;
            let t2 = find_or_create_platform_thread(conn, "telegram", "12345", "Telegram: Alice")?;
            assert_eq!(t1.thread_id, t2.thread_id);
            assert_eq!(t1.platform, "telegram");
            assert_eq!(t1.external_id.as_deref(), Some("12345"));
            Ok(())
        })
    }

    #[test]
    fn find_or_create_platform_thread_no_cross_platform_collision() -> Result<()> {
        let db = Database::open_memory()?;
        db.with_conn(|conn| {
            let telegram = find_or_create_platform_thread(conn, "telegram", "1", "T")?;
            let discord = find_or_create_platform_thread(conn, "discord", "1", "D")?;
            assert_ne!(telegram.thread_id, discord.thread_id);
            Ok(())
        })
    }

    #[test]
    fn list_threads_filters_by_platform() -> Result<()> {
        let db = Database::open_memory()?;
        db.with_conn(|conn| {
            create_thread(conn, "Native", "user", "user")?;
            find_or_create_platform_thread(conn, "telegram", "1", "TG chat")?;
            find_or_create_platform_thread(conn, "discord", "1", "DC channel")?;

            let telegram_only = list_threads(conn, 10, 0, Some("telegram"))?;
            assert_eq!(telegram_only.len(), 1);
            assert_eq!(telegram_only[0].platform, "telegram");

            let all = list_threads(conn, 10, 0, None)?;
            assert_eq!(all.len(), 3);
            Ok(())
        })
    }

    #[test]
    fn list_threads_orders_by_recent_activity_and_stable_tie_break() -> Result<()> {
        let db = Database::open_memory()?;
        db.with_conn(|conn| {
            // Seed two threads with identical updated_at and created_at timestamps.
            let now = "2026-09-28T12:00:00Z";
            conn.execute(
                "INSERT INTO chat_threads (thread_id, title, thread_type, initiated_by, status, created_at, updated_at)
                 VALUES ('thread-aaa', 'Alpha', 'chat', 'user', 'active', ?1, ?1)",
                params![now],
            )?;
            conn.execute(
                "INSERT INTO chat_threads (thread_id, title, thread_type, initiated_by, status, created_at, updated_at)
                 VALUES ('thread-zzz', 'Zulu', 'chat', 'user', 'active', ?1, ?1)",
                params![now],
            )?;

            // Tie-break: updated_at and created_at match, so thread_id DESC dictates 'thread-zzz' before 'thread-aaa'
            let threads = list_threads(conn, 10, 0, None)?;
            assert_eq!(threads.len(), 2);
            assert_eq!(threads[0].thread_id, "thread-zzz");
            assert_eq!(threads[1].thread_id, "thread-aaa");

            // Adding a message to 'thread-aaa' records meaningful activity: updated_at becomes newer
            add_message(conn, "thread-aaa", "user", "Hello Alpha")?;
            let reordered = list_threads(conn, 10, 0, None)?;
            assert_eq!(reordered[0].thread_id, "thread-aaa");
            assert_eq!(reordered[1].thread_id, "thread-zzz");

            // Adding a message to 'thread-zzz' puts it back at the top
            add_message(conn, "thread-zzz", "assistant", "Hello Zulu")?;
            let reordered2 = list_threads(conn, 10, 0, None)?;
            assert_eq!(reordered2[0].thread_id, "thread-zzz");
            assert_eq!(reordered2[1].thread_id, "thread-aaa");

            Ok(())
        })
    }
}
