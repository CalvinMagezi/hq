//! Recurring tool failures over the last two weeks, grouped by a normalized error signature.
//!
//! Feeds the weekly digest in the owner's inbox. Deterministic: no model reads the messages.

use anyhow::Result;
use rusqlite::{Connection, params};
use std::collections::HashMap;

pub const WEEK_SECS: i64 = 7 * 24 * 3600;
/// Most failure rows read for one digest; a larger fortnight is reported as capped.
pub const MAX_SCANNED: usize = 20_000;
/// Longest signature kept, in characters.
const MAX_SIGNATURE_CHARS: usize = 120;
/// A token at least this long that holds a digit is taken for an id, hash or timestamp.
const MIN_ID_TOKEN_CHARS: usize = 6;
/// LLM failure groups listed per digest.
const MAX_LLM_GROUPS: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureCluster {
    pub tool: String,
    pub signature: String,
    pub this_week: u64,
    pub sessions: u64,
    pub last_week: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmFailureGroup {
    pub provider: String,
    pub error_class: String,
    pub count: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FailureDigest {
    pub calls: u64,
    pub failures: u64,
    pub last_week_calls: u64,
    pub last_week_failures: u64,
    /// Busiest first; only clusters seen this week.
    pub clusters: Vec<FailureCluster>,
    pub llm: Vec<LlmFailureGroup>,
    pub scanned: usize,
    pub capped: bool,
}

/// One failure's shape with the parts that vary between occurrences taken out.
pub fn signature(message: &str) -> String {
    let words: Vec<String> = message
        .split_whitespace()
        .map(|w| normalize_token(&w.to_lowercase()))
        .collect();
    let mut joined = words.join(" ");
    if let Some((cut, _)) = joined.char_indices().nth(MAX_SIGNATURE_CHARS) {
        joined.truncate(cut);
    }
    joined
}

fn normalize_token(token: &str) -> String {
    let trailing: String = token
        .chars()
        .rev()
        .take_while(|c| matches!(c, ',' | '.' | ';' | ':' | ')'))
        .collect();
    let core = &token[..token.len() - trailing.len()];
    let replaced = if core.starts_with(['\'', '"', '`']) {
        "<value>".to_string()
    } else if core.starts_with('/') || core.starts_with("~/") || core.contains(":\\") {
        "<path>".to_string()
    } else if core.chars().count() >= MIN_ID_TOKEN_CHARS && core.chars().any(|c| c.is_ascii_digit())
    {
        "<id>".to_string()
    } else {
        collapse_digits(core)
    };
    replaced + &trailing.chars().rev().collect::<String>()
}

fn collapse_digits(token: &str) -> String {
    let mut out = String::with_capacity(token.len());
    for c in token.chars() {
        let digit = c.is_ascii_digit();
        if !digit {
            out.push(c);
        } else if !out.ends_with('#') {
            out.push('#');
        }
    }
    out
}

/// The digest for the week ending at `now`, compared with the week before it.
pub fn weekly_digest(conn: &Connection, now: i64) -> Result<FailureDigest> {
    let this_start = now - WEEK_SECS;
    let last_start = this_start - WEEK_SECS;
    let (calls, failures) = call_counts(conn, this_start, now)?;
    let (last_week_calls, last_week_failures) = call_counts(conn, last_start, this_start)?;
    let mut stmt = conn.prepare(
        "SELECT tool_name, session_id, timestamp, COALESCE(error_msg, '')
           FROM tool_usage
          WHERE success = 0 AND timestamp >= ?1 AND timestamp < ?2
          ORDER BY timestamp DESC
          LIMIT ?3",
    )?;
    let rows = stmt.query_map(params![last_start, now, MAX_SCANNED as i64 + 1], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, String>(3)?,
        ))
    })?;
    let mut groups: HashMap<(String, String), (u64, u64, Vec<String>)> = HashMap::new();
    let mut scanned = 0;
    for row in rows {
        let (tool, session, at, message) = row?;
        scanned += 1;
        if scanned > MAX_SCANNED {
            break;
        }
        let entry = groups.entry((tool, signature(&message))).or_default();
        if at < this_start {
            entry.1 += 1;
            continue;
        }
        entry.0 += 1;
        if !entry.2.contains(&session) {
            entry.2.push(session);
        }
    }
    let mut clusters: Vec<FailureCluster> = groups
        .into_iter()
        .filter(|(_, (this_week, _, _))| *this_week > 0)
        .map(
            |((tool, signature), (this_week, last_week, sessions))| FailureCluster {
                tool,
                signature,
                this_week,
                sessions: sessions.len() as u64,
                last_week,
            },
        )
        .collect();
    clusters.sort_by(|a, b| {
        b.this_week
            .cmp(&a.this_week)
            .then_with(|| a.tool.cmp(&b.tool))
            .then_with(|| a.signature.cmp(&b.signature))
    });
    Ok(FailureDigest {
        calls,
        failures,
        last_week_calls,
        last_week_failures,
        clusters,
        llm: llm_failures(conn, this_start, now)?,
        scanned: scanned.min(MAX_SCANNED),
        capped: scanned > MAX_SCANNED,
    })
}

fn call_counts(conn: &Connection, from: i64, until: i64) -> Result<(u64, u64)> {
    Ok(conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(success = 0), 0)
           FROM tool_usage WHERE timestamp >= ?1 AND timestamp < ?2",
        params![from, until],
        |r| Ok((r.get::<_, i64>(0)? as u64, r.get::<_, i64>(1)? as u64)),
    )?)
}

fn llm_failures(conn: &Connection, from: i64, until: i64) -> Result<Vec<LlmFailureGroup>> {
    let mut stmt = conn.prepare(
        "SELECT provider, COALESCE(error_class, 'unknown'), COUNT(*) AS n
           FROM task_outcomes
          WHERE success = 0 AND recorded_at >= ?1 AND recorded_at < ?2
            AND COALESCE(error_class, '') != 'cancelled'
          GROUP BY provider, error_class
          ORDER BY n DESC, provider
          LIMIT ?3",
    )?;
    let rows = stmt.query_map(params![from, until, MAX_LLM_GROUPS as i64], |r| {
        Ok(LlmFailureGroup {
            provider: r.get(0)?,
            error_class: r.get(1)?,
            count: r.get::<_, i64>(2)? as u64,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;

    const NOW: i64 = 1_800_000_000;
    const DAY: i64 = 24 * 3600;

    fn fail(c: &Connection, tool: &str, session: &str, at: i64, msg: &str) {
        c.execute(
            "INSERT INTO tool_usage (tool_name, agent_name, session_id, timestamp, success, error_msg)
             VALUES (?1, 'agent', ?2, ?3, 0, ?4)",
            params![tool, session, at, msg],
        )
        .unwrap();
    }

    fn ok(c: &Connection, at: i64) {
        c.execute(
            "INSERT INTO tool_usage (tool_name, agent_name, session_id, timestamp, success)
             VALUES ('vault_read', 'agent', 's', ?1, 1)",
            params![at],
        )
        .unwrap();
    }

    #[test]
    fn the_same_failure_with_different_ids_paths_and_values_is_one_signature() {
        let a = signature("no task found for id tk-1793774644010-def52a.");
        let b = signature("no task found for id tk-1700000000000-aaaaaa.");
        assert_eq!(a, b);
        assert_eq!(a, "no task found for id <id>.");
        assert_eq!(
            signature("/home/user/repo on host 'laptop' is not ready"),
            signature("/home/user/other on host 'desk' is not ready"),
        );
        assert_eq!(
            signature("unknown priority 'medium', expected one of urgent"),
            "unknown priority <value>, expected one of urgent"
        );
        assert_eq!(
            signature("3 sessions running"),
            signature("4 sessions running")
        );
        assert_ne!(
            signature("id is required"),
            signature("task_id is required")
        );
    }

    #[test]
    fn a_long_message_is_cut_on_a_character_boundary() {
        let long = "é".repeat(400);
        assert_eq!(signature(&long).chars().count(), MAX_SIGNATURE_CHARS);
    }

    #[test]
    fn the_digest_groups_this_week_and_compares_with_last_week() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            for (i, session) in ["s1", "s2", "s2"].iter().enumerate() {
                let msg = format!("no task found for id tk-{i}000000");
                fail(c, "task_get", session, NOW - DAY, &msg);
            }
            fail(
                c,
                "task_get",
                "s9",
                NOW - 8 * DAY,
                "no task found for id tk-9999999",
            );
            fail(
                c,
                "vault_find",
                "s3",
                NOW - 2 * DAY,
                "missing required parameter: topic",
            );
            fail(c, "old_tool", "s4", NOW - 9 * DAY, "only last week");
            ok(c, NOW - DAY);
            let d = weekly_digest(c, NOW)?;
            assert_eq!((d.calls, d.failures), (5, 4));
            assert_eq!((d.last_week_calls, d.last_week_failures), (2, 2));
            assert_eq!(
                d.clusters.len(),
                2,
                "a cluster seen only last week is not listed"
            );
            let top = &d.clusters[0];
            assert_eq!(top.tool, "task_get");
            assert_eq!((top.this_week, top.sessions, top.last_week), (3, 2, 1));
            assert_eq!(d.scanned, 6);
            assert!(!d.capped);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn an_empty_table_reports_zero_calls_not_a_clean_week() {
        let db = Database::open_memory().unwrap();
        let d = db.with_conn(|c| weekly_digest(c, NOW)).unwrap();
        assert_eq!(d, FailureDigest::default());
    }
}
