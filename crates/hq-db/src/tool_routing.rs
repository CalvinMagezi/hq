//! How well the model finds and names tools (`sql/*_tool_routing_events.sql`).
//!
//! Local counters for the offline tool router: searches that found something or nothing, tools
//! loaded on demand, and tool names that did not exist. A row holds a kind, a tool name and a
//! session id, never the words anyone searched for. Nothing here is sent anywhere, and none of it
//! reaches a model prompt.

use anyhow::Result;
use rusqlite::{Connection, params};

/// A search found at least one tool.
pub const SEARCH_HIT: &str = "search_hit";
/// A search found nothing.
pub const SEARCH_MISS: &str = "search_miss";
/// A deferred tool was loaded because a search found it.
pub const ACTIVATION: &str = "activation";
/// A tool name that does not exist was called, and nothing close was offered.
pub const UNKNOWN_TOOL: &str = "unknown_tool";
/// A tool name that does not exist was called, and a close real name was offered.
pub const UNKNOWN_TOOL_HELPED: &str = "unknown_tool_helped";

/// A "tools that may fit" line was added to a turn. The tool name column holds the names it listed.
pub const HINT_SHOWN: &str = "hint_shown";
/// A tool a hint named was loaded afterwards.
pub const HINT_FOLLOWED: &str = "hint_followed";

/// Longest tool name stored. A model can send anything as a tool name.
const MAX_NAME_CHARS: usize = 80;

/// Record one event. Best effort for callers: a failure here must never fail a tool call.
pub fn record(conn: &Connection, session_id: &str, kind: &str, tool_name: &str) -> Result<()> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let name: String = tool_name.chars().take(MAX_NAME_CHARS).collect();
    conn.execute(
        "INSERT INTO tool_routing_events (timestamp, session_id, kind, tool_name)
         VALUES (?1, ?2, ?3, ?4)",
        params![now, session_id, kind, name],
    )?;
    Ok(())
}

/// Counts of each kind of event since a moment.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RoutingSummary {
    pub search_hits: u64,
    pub search_misses: u64,
    pub activations: u64,
    pub unknown_tools: u64,
    pub unknown_tools_helped: u64,
    pub hints_shown: u64,
    pub hints_followed: u64,
}

impl RoutingSummary {
    /// Searches of either outcome.
    pub fn searches(&self) -> u64 {
        self.search_hits + self.search_misses
    }

    /// Wrong tool names of either kind.
    pub fn wrong_names(&self) -> u64 {
        self.unknown_tools + self.unknown_tools_helped
    }

    /// Whether anything was recorded.
    pub fn is_empty(&self) -> bool {
        self.searches() == 0
            && self.activations == 0
            && self.wrong_names() == 0
            && self.hints_shown == 0
    }
}

/// Count the events recorded at or after `since` (Unix seconds).
pub fn summary_since(conn: &Connection, since: i64) -> Result<RoutingSummary> {
    let mut stmt = conn.prepare(
        "SELECT kind, COUNT(*) FROM tool_routing_events WHERE timestamp >= ?1 GROUP BY kind",
    )?;
    let rows = stmt.query_map(params![since], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u64))
    })?;
    let mut out = RoutingSummary::default();
    for row in rows {
        let (kind, n) = row?;
        match kind.as_str() {
            SEARCH_HIT => out.search_hits = n,
            SEARCH_MISS => out.search_misses = n,
            ACTIVATION => out.activations = n,
            UNKNOWN_TOOL => out.unknown_tools = n,
            UNKNOWN_TOOL_HELPED => out.unknown_tools_helped = n,
            HINT_SHOWN => out.hints_shown = n,
            HINT_FOLLOWED => out.hints_followed = n,
            _ => {}
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;

    #[test]
    fn counts_each_kind_and_ignores_older_rows() -> Result<()> {
        let db = Database::open_memory()?;
        db.with_conn(|conn| {
            record(conn, "s1", SEARCH_HIT, "")?;
            record(conn, "s1", SEARCH_HIT, "")?;
            record(conn, "s1", SEARCH_MISS, "")?;
            record(conn, "s1", ACTIVATION, "office_create")?;
            record(conn, "s2", UNKNOWN_TOOL, "no_such_tool")?;
            record(conn, "s2", UNKNOWN_TOOL_HELPED, "vault_serach")?;
            record(conn, "s2", HINT_SHOWN, "task_create")?;
            record(conn, "s2", HINT_FOLLOWED, "task_create")?;
            conn.execute(
                "INSERT INTO tool_routing_events (timestamp, kind) VALUES (1, 'search_miss')",
                [],
            )?;
            let all = summary_since(conn, 100)?;
            assert_eq!(
                all,
                RoutingSummary {
                    search_hits: 2,
                    search_misses: 1,
                    activations: 1,
                    unknown_tools: 1,
                    unknown_tools_helped: 1,
                    hints_shown: 1,
                    hints_followed: 1,
                }
            );
            assert_eq!(all.searches(), 3);
            assert_eq!(all.wrong_names(), 2);
            assert!(!all.is_empty());
            assert!(summary_since(conn, i64::MAX)?.is_empty());
            Ok(())
        })
    }

    #[test]
    fn a_long_tool_name_is_cut_and_no_query_text_is_stored() -> Result<()> {
        let db = Database::open_memory()?;
        db.with_conn(|conn| {
            record(conn, "s", UNKNOWN_TOOL, &"x".repeat(500))?;
            let len: i64 = conn.query_row(
                "SELECT LENGTH(tool_name) FROM tool_routing_events",
                [],
                |r| r.get(0),
            )?;
            assert_eq!(len as usize, MAX_NAME_CHARS);
            let cols: Vec<String> = conn
                .prepare("PRAGMA table_info(tool_routing_events)")?
                .query_map([], |r| r.get::<_, String>(1))?
                .collect::<Result<_, _>>()?;
            assert_eq!(cols, ["id", "timestamp", "session_id", "kind", "tool_name"]);
            Ok(())
        })
    }
}
