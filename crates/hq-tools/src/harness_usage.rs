//! Token usage of the coding agents HQ's host runs (Claude Code, Codex, Pi, Kimi, OpenCode, Copilot
//! CLI), read from the files each one already writes. Only numbers, model names and ids are read;
//! no prompt or reply text is ever parsed into a value or stored.
//!
//! Every reader turns one record into a [`HarnessUsageEvent`] with a stable `source_id`, so reading
//! a growing file again never counts a call twice. Input follows the ledger's convention: it
//! includes cache-read tokens and excludes cache-write tokens.

use rusqlite::Connection;
use serde_json::Value;

use hq_llm::cost::Usage;

#[derive(Debug, Clone, PartialEq)]
pub struct HarnessUsageEvent {
    pub harness: &'static str,
    /// Unique per call within the harness, so an upsert is idempotent.
    pub source_id: String,
    /// The harness's own session id, to tie the call to an HQ session.
    pub session_ref: String,
    pub cwd: Option<String>,
    pub ts: i64,
    pub model: String,
    pub usage: Usage,
}

fn u32_at(v: &Value, pointer: &str) -> u32 {
    v.pointer(pointer)
        .and_then(Value::as_u64)
        .map_or(0, |n| n.min(u64::from(u32::MAX)) as u32)
}

fn str_at<'a>(v: &'a Value, pointer: &str) -> Option<&'a str> {
    v.pointer(pointer).and_then(Value::as_str).filter(|s| !s.is_empty())
}

/// Ids and model names come from files HQ does not control, so none is stored longer than this.
const MAX_FIELD_CHARS: usize = 128;

fn capped(s: &str) -> String {
    s.chars().take(MAX_FIELD_CHARS).collect()
}

fn parse_ts(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|t| t.timestamp())
        .ok()
        .or_else(|| {
            // SQLite's own `datetime('now')` shape, with or without fractional seconds.
            chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S%.f")
                .ok()
                .map(|t| t.and_utc().timestamp())
        })
}

// ─── Claude Code ────────────────────────────────────────────────────

/// One transcript line. Only `assistant` lines carry usage, and the same `message.id` is written once
/// per content block with identical usage, so the caller keeps the first per id.
pub fn parse_claude_line(line: &str) -> Option<HarnessUsageEvent> {
    let v: Value = serde_json::from_str(line).ok()?;
    if v.get("type")?.as_str()? != "assistant" {
        return None;
    }
    let id = str_at(&v, "/message/id")?.to_string();
    let fresh = u32_at(&v, "/message/usage/input_tokens");
    let cache_read = u32_at(&v, "/message/usage/cache_read_input_tokens");
    Some(HarnessUsageEvent {
        harness: "claude-code",
        source_id: capped(&id),
        session_ref: capped(str_at(&v, "/sessionId")?),
        cwd: str_at(&v, "/cwd").map(str::to_string),
        ts: str_at(&v, "/timestamp").and_then(parse_ts)?,
        model: capped(str_at(&v, "/message/model")?),
        usage: Usage {
            input: fresh.saturating_add(cache_read),
            output: u32_at(&v, "/message/usage/output_tokens"),
            cache_read,
            cache_write: u32_at(&v, "/message/usage/cache_creation_input_tokens"),
            ..Usage::default()
        },
    })
}

// ─── Codex ──────────────────────────────────────────────────────────

/// Codex writes cumulative and per-call counters. Only `last_token_usage` of a `token_count` event
/// is one call; the cumulative total is ignored so nothing is summed twice. `ordinal` is the byte
/// offset of the line, which stays the same when the file grows and keeps two identical calls apart.
pub fn parse_codex_line(
    line: &str,
    ordinal: usize,
    session: &CodexSession,
) -> Option<HarnessUsageEvent> {
    let v: Value = serde_json::from_str(line).ok()?;
    if str_at(&v, "/payload/type")? != "token_count" {
        return None;
    }
    let last = v.pointer("/payload/info/last_token_usage")?;
    Some(HarnessUsageEvent {
        harness: "codex",
        source_id: format!("{}:{ordinal}", session.id),
        session_ref: session.id.clone(),
        cwd: session.cwd.clone(),
        ts: str_at(&v, "/timestamp").and_then(parse_ts)?,
        model: capped(&session.model),
        // Codex input already includes cached input tokens.
        usage: Usage {
            input: u32_at(last, "/input_tokens"),
            output: u32_at(last, "/output_tokens"),
            cache_read: u32_at(last, "/cached_input_tokens"),
            reasoning: u32_at(last, "/reasoning_output_tokens"),
            ..Usage::default()
        },
    })
}

/// What the first lines of a Codex rollout say about the whole session.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CodexSession {
    pub id: String,
    pub cwd: Option<String>,
    pub model: String,
}

impl CodexSession {
    /// Learn the session from a `session_meta` or `turn_context` line; other lines change nothing.
    pub fn observe(&mut self, line: &str) {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            return;
        };
        match v.get("type").and_then(Value::as_str) {
            Some("session_meta") => {
                if let Some(id) = str_at(&v, "/payload/id") {
                    self.id = capped(id);
                }
                self.cwd = str_at(&v, "/payload/cwd").map(str::to_string);
            }
            Some("turn_context") => {
                if let Some(m) = str_at(&v, "/payload/model") {
                    self.model = capped(m);
                }
            }
            _ => {}
        }
    }
}

// ─── Pi ─────────────────────────────────────────────────────────────

/// Pi records its own dollar cost per call, which is used as given.
pub fn parse_pi_line(line: &str, session: &PiSession) -> Option<(HarnessUsageEvent, Option<f64>)> {
    let v: Value = serde_json::from_str(line).ok()?;
    if v.get("type")?.as_str()? != "message" || str_at(&v, "/message/role")? != "assistant" {
        return None;
    }
    let usage = v.pointer("/message/usage")?;
    let cache_read = u32_at(usage, "/cacheRead");
    // A zero is a free or subscription model, not a billed zero.
    let cost = usage
        .pointer("/cost/total")
        .and_then(Value::as_f64)
        .filter(|c| *c > 0.0);
    let event = HarnessUsageEvent {
        harness: "pi",
        source_id: format!("{}:{}", session.id, str_at(&v, "/id")?),
        session_ref: session.id.clone(),
        cwd: session.cwd.clone(),
        ts: str_at(&v, "/timestamp").and_then(parse_ts)?,
        model: capped(str_at(&v, "/message/model")?),
        // Pi reports input net of cache.
        usage: Usage {
            input: u32_at(usage, "/input").saturating_add(cache_read),
            output: u32_at(usage, "/output"),
            cache_read,
            cache_write: u32_at(usage, "/cacheWrite"),
            billed_usd: cost,
            ..Usage::default()
        },
    };
    Some((event, cost))
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PiSession {
    pub id: String,
    pub cwd: Option<String>,
}

impl PiSession {
    pub fn observe(&mut self, line: &str) {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            return;
        };
        if v.get("type").and_then(Value::as_str) == Some("session") {
            if let Some(id) = str_at(&v, "/id") {
                self.id = capped(id);
            }
            self.cwd = str_at(&v, "/cwd").map(str::to_string);
        }
    }
}

// ─── Kimi ───────────────────────────────────────────────────────────

/// A `StatusUpdate` carries one step's usage. The model is not in the record, so the caller passes
/// the one it knows for the session (or an empty string, which the ledger prices as unknown).
pub fn parse_kimi_line(
    line: &str,
    session_ref: &str,
    ordinal: usize,
    model: &str,
) -> Option<HarnessUsageEvent> {
    let v: Value = serde_json::from_str(line).ok()?;
    if str_at(&v, "/message/type")? != "StatusUpdate" {
        return None;
    }
    let usage = v.pointer("/message/payload/token_usage")?;
    let cache_read = u32_at(usage, "/input_cache_read");
    Some(HarnessUsageEvent {
        harness: "kimi",
        source_id: format!("{session_ref}:{ordinal}"),
        session_ref: session_ref.to_string(),
        cwd: None,
        ts: v.get("timestamp").and_then(Value::as_f64)? as i64,
        model: model.to_string(),
        usage: Usage {
            input: u32_at(usage, "/input_other").saturating_add(cache_read),
            output: u32_at(usage, "/output"),
            cache_read,
            cache_write: u32_at(usage, "/input_cache_creation"),
            ..Usage::default()
        },
    })
}

// ─── OpenCode (SQLite) ──────────────────────────────────────────────

/// Assistant messages are one call each. The session's own totals are cumulative and are not read.
pub fn read_opencode(conn: &Connection, after_time_ms: i64) -> rusqlite::Result<Vec<HarnessUsageEvent>> {
    let mut stmt = conn.prepare(
        "SELECT m.id, m.session_id, s.directory, m.time_created,
                json_extract(m.data, '$.modelID'),
                json_extract(m.data, '$.tokens.input'), json_extract(m.data, '$.tokens.output'),
                json_extract(m.data, '$.tokens.reasoning'),
                json_extract(m.data, '$.tokens.cache.read'), json_extract(m.data, '$.tokens.cache.write'),
                json_extract(m.data, '$.cost')
         FROM message m JOIN session s ON s.id = m.session_id
         WHERE json_extract(m.data, '$.role') = 'assistant' AND m.time_created > ?1
           AND (json_extract(m.data, '$.time.completed') IS NOT NULL
                OR json_extract(m.data, '$.tokens.output') > 0)",
    )?;
    let rows = stmt.query_map([after_time_ms], |r| {
        let n = |i: usize| r.get::<_, Option<i64>>(i).map(|v| v.unwrap_or(0).clamp(0, i64::from(u32::MAX)) as u32);
        let cache_read = n(8)?;
        let cost: Option<f64> = r.get(10)?;
        Ok(HarnessUsageEvent {
            harness: "opencode",
            source_id: r.get(0)?,
            session_ref: r.get(1)?,
            cwd: r.get(2)?,
            ts: r.get::<_, i64>(3)? / 1000,
            model: r.get::<_, Option<String>>(4)?.unwrap_or_default(),
            usage: Usage {
                input: n(5)?.saturating_add(cache_read),
                output: n(6)?,
                reasoning: n(7)?,
                cache_read,
                cache_write: n(9)?,
                // A zero cost means free or unpriced there, not a billed zero.
                billed_usd: cost.filter(|c| *c > 0.0),
            },
        })
    })?;
    rows.collect()
}

// ─── Copilot CLI (SQLite) ───────────────────────────────────────────

/// `assistant_usage_events` rows are one call each. Copilot reports AI units, not dollars, so no
/// cost is carried; its `input_tokens` includes both cache reads and writes.
pub fn read_copilot(conn: &Connection, after_id: i64) -> rusqlite::Result<Vec<HarnessUsageEvent>> {
    let mut stmt = conn.prepare(
        "SELECT e.id, e.session_id, s.cwd, e.created_at, e.model, e.input_tokens, e.output_tokens,
                e.cache_read_tokens, e.cache_write_tokens, e.reasoning_tokens
         FROM assistant_usage_events e LEFT JOIN sessions s ON s.id = e.session_id
         WHERE e.id > ?1",
    )?;
    let rows = stmt.query_map([after_id], |r| {
        let n = |i: usize| r.get::<_, Option<i64>>(i).map(|v| v.unwrap_or(0).clamp(0, i64::from(u32::MAX)) as u32);
        let (input, cache_read, cache_write) = (n(5)?, n(7)?, n(8)?);
        let created: String = r.get(3)?;
        Ok(HarnessUsageEvent {
            harness: "github-copilot",
            source_id: r.get::<_, i64>(0)?.to_string(),
            session_ref: r.get(1)?,
            cwd: r.get(2)?,
            ts: parse_ts(&created).unwrap_or_default(),
            model: r.get::<_, Option<String>>(4)?.unwrap_or_default(),
            // Keep the ledger convention: reads included, writes excluded.
            usage: Usage {
                input: input.saturating_sub(cache_write),
                output: n(6)?,
                cache_read,
                cache_write,
                reasoning: n(9)?,
                billed_usd: None,
            },
        })
    })?;
    rows.collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_assistant_lines_become_events_with_cache_folded_into_input() {
        let line = r#"{"type":"assistant","sessionId":"s1","cwd":"/w","timestamp":"2026-10-08T10:00:00Z",
            "message":{"id":"m1","model":"claude-x","content":"IGNORED TEXT","usage":{"input_tokens":2,
            "output_tokens":50,"cache_read_input_tokens":1000,"cache_creation_input_tokens":30}}}"#
            .replace('\n', "");
        let e = parse_claude_line(&line).unwrap();
        assert_eq!((e.source_id.as_str(), e.session_ref.as_str(), e.model.as_str()), ("m1", "s1", "claude-x"));
        assert_eq!((e.usage.input, e.usage.output, e.usage.cache_read, e.usage.cache_write), (1002, 50, 1000, 30));
        assert!(parse_claude_line(r#"{"type":"user","message":{"id":"m"}}"#).is_none());
        assert!(parse_claude_line("not json").is_none());
    }

    #[test]
    fn codex_counts_the_per_call_figure_and_ignores_the_running_total() {
        let mut s = CodexSession::default();
        s.observe(r#"{"type":"session_meta","payload":{"id":"c1","cwd":"/w"}}"#);
        s.observe(r#"{"type":"turn_context","payload":{"model":"gpt-x"}}"#);
        let line = r#"{"timestamp":"2026-10-08T10:00:00Z","type":"event_msg","payload":{"type":"token_count",
            "info":{"total_token_usage":{"input_tokens":99999},"last_token_usage":{"input_tokens":14489,
            "cached_input_tokens":9000,"output_tokens":200,"reasoning_output_tokens":40}}}}"#
            .replace('\n', "");
        let e = parse_codex_line(&line, 7, &s).unwrap();
        assert_eq!((e.source_id.as_str(), e.model.as_str()), ("c1:7", "gpt-x"));
        assert_eq!((e.usage.input, e.usage.cache_read, e.usage.reasoning), (14489, 9000, 40));
        let other = r#"{"timestamp":"2026-10-08T10:00:00Z","payload":{"type":"agent_message"}}"#;
        assert!(parse_codex_line(other, 8, &s).is_none());
    }

    #[test]
    fn pi_keeps_its_own_dollar_figure() {
        let mut s = PiSession::default();
        s.observe(r#"{"type":"session","id":"p1","cwd":"/w"}"#);
        let line = r#"{"type":"message","id":"e1","timestamp":"2026-08-02T09:59:30Z","message":{"role":"assistant",
            "model":"gpt-5.4","usage":{"input":6697,"output":246,"cacheRead":100,"cacheWrite":0,
            "cost":{"total":0.0204}}}}"#
            .replace('\n', "");
        let (e, cost) = parse_pi_line(&line, &s).unwrap();
        assert_eq!(cost, Some(0.0204));
        assert_eq!((e.usage.input, e.usage.billed_usd), (6797, Some(0.0204)));
    }

    #[test]
    fn kimi_status_updates_are_steps_and_other_messages_are_skipped() {
        let line = r#"{"timestamp":1791500000.5,"message":{"type":"StatusUpdate","payload":{"token_usage":
            {"input_other":7689,"output":10,"input_cache_read":9216,"input_cache_creation":5}}}}"#
            .replace('\n', "");
        let e = parse_kimi_line(&line, "k1", 3, "kimi-k3").unwrap();
        assert_eq!((e.source_id.as_str(), e.usage.input, e.usage.cache_write), ("k1:3", 16905, 5));
        assert!(parse_kimi_line(r#"{"message":{"type":"TurnBegin"}}"#, "k1", 4, "").is_none());
    }

    #[test]
    fn opencode_messages_are_read_and_a_zero_cost_is_not_a_billed_zero() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE session (id TEXT, directory TEXT);
             CREATE TABLE message (id TEXT, session_id TEXT, time_created INTEGER, data TEXT);
             INSERT INTO session VALUES ('s1', '/w');
             INSERT INTO message VALUES ('m1','s1',1791500000000,
               '{\"role\":\"assistant\",\"modelID\":\"o\",\"cost\":0,\"tokens\":{\"input\":100,\"output\":5,\"reasoning\":2,\"cache\":{\"read\":50,\"write\":1}}}');
             INSERT INTO message VALUES ('m2','s1',1791500001000,
               '{\"role\":\"assistant\",\"modelID\":\"o\",\"cost\":0.25,\"tokens\":{\"input\":10,\"output\":1,\"cache\":{\"read\":0,\"write\":0}}}');
             INSERT INTO message VALUES ('m3','s1',1791500002000,'{\"role\":\"user\"}');",
        )
        .unwrap();
        let events = read_opencode(&conn, 0).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!((events[0].usage.input, events[0].usage.billed_usd), (150, None));
        assert_eq!(events[1].usage.billed_usd, Some(0.25));
        assert_eq!(read_opencode(&conn, 1_791_500_000_500).unwrap().len(), 1);
    }

    #[test]
    fn copilot_input_loses_the_cache_write_it_included() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (id TEXT, cwd TEXT);
             CREATE TABLE assistant_usage_events (id INTEGER, session_id TEXT, model TEXT, input_tokens INTEGER,
               output_tokens INTEGER, cache_read_tokens INTEGER, cache_write_tokens INTEGER,
               reasoning_tokens INTEGER, created_at TEXT);
             INSERT INTO sessions VALUES ('c1','/w');
             INSERT INTO assistant_usage_events VALUES (1,'c1','m',36401,20,36249,150,0,'2026-10-08T10:00:00Z');",
        )
        .unwrap();
        let e = &read_copilot(&conn, 0).unwrap()[0];
        assert_eq!((e.usage.input, e.usage.cache_read, e.usage.cache_write), (36251, 36249, 150));
        assert!(read_copilot(&conn, 1).unwrap().is_empty());
    }

    #[test]
    fn a_pi_cost_of_zero_is_not_a_billed_zero() {
        let s = PiSession { id: "p1".into(), cwd: None };
        let line = r#"{"type":"message","id":"e1","timestamp":"2026-08-02T09:59:30Z","message":{"role":"assistant","model":"m","usage":{"input":1,"output":1,"cost":{"total":0}}}}"#;
        let (e, cost) = parse_pi_line(line, &s).unwrap();
        assert_eq!((cost, e.usage.billed_usd), (None, None));
    }

    #[test]
    fn opencode_skips_a_message_that_has_not_finished() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE session (id TEXT, directory TEXT);
             CREATE TABLE message (id TEXT, session_id TEXT, time_created INTEGER, data TEXT);
             INSERT INTO session VALUES ('s1', '/w');
             INSERT INTO message VALUES ('m1','s1',1791500000000,'{\"role\":\"assistant\",\"modelID\":\"o\",\"tokens\":{\"input\":0,\"output\":0}}');
             INSERT INTO message VALUES ('m2','s1',1791500001000,'{\"role\":\"assistant\",\"modelID\":\"o\",\"time\":{\"completed\":1791500002000},\"tokens\":{\"input\":5,\"output\":0}}');",
        )
        .unwrap();
        let ids: Vec<String> = read_opencode(&conn, 0).unwrap().into_iter().map(|e| e.source_id).collect();
        assert_eq!(ids, ["m2"]);
    }

    #[test]
    fn a_sqlite_style_timestamp_is_understood() {
        assert_eq!(parse_ts("2026-10-08 10:00:00"), parse_ts("2026-10-08T10:00:00Z"));
        assert!(parse_ts("2026-10-08 10:00:00.250").is_some());
        assert!(parse_ts("garbage").is_none());
    }
}
