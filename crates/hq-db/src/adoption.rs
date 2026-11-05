//! What the owner actually does with HQ, from records it already keeps: their chat messages and
//! the tool calls of their own sessions. Feeds the weekly adoption digest and the capability
//! catalog's "used" marks.
//!
//! Deterministic and local: no model reads the messages, and nothing is sent anywhere.

use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::{BTreeSet, HashMap};

/// The agent name of the owner's own chat sessions (web, phone app, chat apps). Sub-agents,
/// guests and outside MCP clients record other names.
pub const OWNER_AGENT: &str = "hq";
pub const DAY_SECS: i64 = 24 * 3600;
pub const WEEK_SECS: i64 = 7 * DAY_SECS;
/// Most chat messages read for one report.
pub const MAX_MESSAGES: i64 = 20_000;

/// How one tool was used by the owner's sessions in a window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolUse {
    pub calls: u64,
    pub first_at: i64,
}

/// Successful calls made by the owner's own sessions in `[from, until)`, per tool.
pub fn owner_tool_use(
    conn: &Connection,
    from: i64,
    until: i64,
) -> Result<HashMap<String, ToolUse>> {
    let mut stmt = conn.prepare(
        "SELECT tool_name, COUNT(*), MIN(timestamp) FROM tool_usage
          WHERE agent_name = ?1 AND success = 1 AND timestamp >= ?2 AND timestamp < ?3
          GROUP BY tool_name",
    )?;
    let rows = stmt.query_map(params![OWNER_AGENT, from, until], |r| {
        Ok((
            r.get::<_, String>(0)?,
            ToolUse {
                calls: r.get::<_, i64>(1)?.max(0) as u64,
                first_at: r.get(2)?,
            },
        ))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Which of `tools` the owner's own sessions have ever called successfully. One indexed lookup
/// per tool that stops at the first row, so it is cheap enough to run for every session.
pub fn owner_has_used(conn: &Connection, tools: &[&str]) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT 1 FROM tool_usage
          WHERE tool_name = ?1 AND agent_name = ?2 AND success = 1 LIMIT 1",
    )?;
    let mut used = Vec::new();
    for tool in tools {
        if stmt.exists(params![tool, OWNER_AGENT])? {
            used.push((*tool).to_string());
        }
    }
    Ok(used)
}

/// A message in the person's chats, with its time in unix seconds.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UserMessage {
    pub at: i64,
    pub text: String,
    /// An outside MCP client asked it through `hq_ask`; the person did not type it.
    pub from_client: bool,
}

/// How `hq_ask` marks the question it saves (`meta.source.kind`).
const CLIENT_SOURCE_SQL: &str = "COALESCE(json_extract(meta, '$.source.kind'), '') = 'mcp'";

fn parse_time(raw: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|t| t.with_timezone(&Utc).timestamp())
}

fn rfc3339(ts: i64) -> String {
    DateTime::<Utc>::from_timestamp(ts, 0)
        .unwrap_or_default()
        .to_rfc3339()
}

/// When the person first wrote to HQ, if they ever did.
pub fn first_user_message_at(conn: &Connection) -> Result<Option<i64>> {
    let raw: Option<String> = conn
        .query_row(
            &format!(
                "SELECT MIN(created_at) FROM chat_messages WHERE role = 'user' AND NOT {CLIENT_SOURCE_SQL}"
            ),
            [],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    Ok(raw.as_deref().and_then(parse_time))
}

/// User-role messages at or after `since`, oldest first, at most [`MAX_MESSAGES`]. Questions an
/// MCP client asked are included and marked, so a caller decides; [`persons_own`] drops them.
pub fn user_messages_since(conn: &Connection, since: i64) -> Result<Vec<UserMessage>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT created_at, content, {CLIENT_SOURCE_SQL} FROM chat_messages
          WHERE role = 'user' AND created_at >= ?1
          ORDER BY created_at ASC LIMIT ?2"
    ))?;
    let rows = stmt.query_map(params![rfc3339(since), MAX_MESSAGES], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, bool>(2)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (at, text, from_client) = row?;
        // Rows written with another offset compare differently as text; the parsed time decides.
        if let Some(at) = parse_time(&at).filter(|t| *t >= since) {
            out.push(UserMessage {
                at,
                text,
                from_client,
            });
        }
    }
    Ok(out)
}

/// The messages the person typed themselves.
pub fn persons_own(mut messages: Vec<UserMessage>) -> Vec<UserMessage> {
    messages.retain(|m| !m.from_client);
    messages
}

/// Phrases that mean the last answer missed: too long, wrong, or not what was asked. English
/// only for now, and deliberately narrow, so a count that falls means fewer misses rather than
/// different wording.
const CORRECTION_PHRASES: &[&str] = &[
    "shorter",
    "too long",
    "be brief",
    "no, i meant",
    "no i meant",
    "not what i asked",
    "that's wrong",
    "that is wrong",
    "that's not right",
    "that is not right",
    "you misunderstood",
    "i already told you",
    "i already said",
    "you didn't",
    "you did not",
    "wrong answer",
];

/// Whether a message corrects HQ's previous answer.
pub fn is_correction(text: &str) -> bool {
    let lower = text.to_lowercase().replace('\u{2019}', "'");
    CORRECTION_PHRASES.iter().any(|p| lower.contains(p))
}

/// Distinct UTC days with at least one message in the `days` days before `now`.
pub fn active_days(messages: &[UserMessage], now: i64, days: i64) -> usize {
    let from = now - days * DAY_SECS;
    messages
        .iter()
        .filter(|m| m.at >= from && m.at <= now)
        .map(|m| m.at.div_euclid(DAY_SECS))
        .collect::<BTreeSet<_>>()
        .len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;

    const NOW: i64 = 1_800_000_000;

    fn call(c: &Connection, tool: &str, agent: &str, at: i64, success: bool) {
        c.execute(
            "INSERT INTO tool_usage (tool_name, agent_name, session_id, timestamp, success)
             VALUES (?1, ?2, 's', ?3, ?4)",
            params![tool, agent, at, success],
        )
        .unwrap();
    }

    fn say(c: &Connection, role: &str, at: i64, text: &str) {
        c.execute(
            "INSERT OR IGNORE INTO chat_threads (thread_id, title, created_at, updated_at)
             VALUES ('t', 't', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00')",
            [],
        )
        .unwrap();
        c.execute(
            "INSERT INTO chat_messages (message_id, thread_id, role, content, created_at)
             VALUES (?1, 't', ?2, ?3, ?4)",
            params![uuid::Uuid::new_v4().to_string(), role, text, rfc3339(at)],
        )
        .unwrap();
    }

    #[test]
    fn only_the_owners_successful_calls_in_the_window_count() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            call(c, "vault_search", OWNER_AGENT, NOW - 10, true);
            call(c, "vault_search", OWNER_AGENT, NOW - 5, true);
            call(c, "vault_import", OWNER_AGENT, NOW - 5, false);
            call(c, "task_create", "telegram_guest", NOW - 5, true);
            call(c, "task_list", OWNER_AGENT, NOW - WEEK_SECS - 1, true);
            call(c, "task_get", OWNER_AGENT, NOW, true);
            let used = owner_tool_use(c, NOW - WEEK_SECS, NOW)?;
            assert_eq!(used.len(), 1);
            assert_eq!(
                owner_has_used(
                    c,
                    &["vault_search", "vault_import", "task_create", "task_get"]
                )?,
                vec!["vault_search", "task_get"]
            );
            assert_eq!(
                used["vault_search"],
                ToolUse {
                    calls: 2,
                    first_at: NOW - 10
                }
            );
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn messages_come_back_parsed_and_only_the_persons_own() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            assert_eq!(first_user_message_at(c)?, None);
            say(c, "user", NOW - 3 * DAY_SECS, "first");
            say(
                c,
                "assistant",
                NOW - 4 * DAY_SECS,
                "reply before the first?",
            );
            say(c, "user", NOW - 60, "Make it shorter please");
            c.execute(
                "INSERT INTO chat_messages (message_id, thread_id, role, content, created_at, meta)
                 VALUES ('mcp', 't', 'user', 'asked by a client', ?1, ?2)",
                params![
                    rfc3339(NOW - 4 * DAY_SECS),
                    r#"{"source":{"kind":"mcp","caller":"c"}}"#
                ],
            )?;
            assert_eq!(
                first_user_message_at(c)?,
                Some(NOW - 3 * DAY_SECS),
                "a client's question is not the person writing"
            );
            let all = user_messages_since(c, NOW - 5 * DAY_SECS)?;
            assert_eq!(all.len(), 3);
            assert!(all[0].from_client);
            assert_eq!(persons_own(all).len(), 2);
            let recent = user_messages_since(c, NOW - DAY_SECS)?;
            assert_eq!(recent.len(), 1);
            assert_eq!(recent[0].at, NOW - 60);
            assert!(is_correction(&recent[0].text));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn corrections_are_narrow_phrases_not_any_no() {
        assert!(is_correction("No, I meant the other shop"));
        assert!(is_correction("That\u{2019}s wrong"));
        assert!(is_correction("You didn't save it"));
        assert!(!is_correction("No problem, thanks"));
        assert!(!is_correction("Save this as a note"));
    }

    #[test]
    fn active_days_count_distinct_days_in_the_window() {
        let at = |t| UserMessage {
            at: t,
            ..Default::default()
        };
        let day = NOW.div_euclid(DAY_SECS) * DAY_SECS;
        let msgs = vec![
            at(day + 10),
            at(day + 20),
            at(day - DAY_SECS + 5),
            at(day - 10 * DAY_SECS),
        ];
        assert_eq!(active_days(&msgs, day + 30, 7), 2);
        assert_eq!(active_days(&msgs, day + 30, 28), 3);
    }
}
