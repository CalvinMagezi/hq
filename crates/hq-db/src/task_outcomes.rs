//! Per-task outcome telemetry. Every LLM call recorded through the router
//! writes one row here.

use anyhow::Result;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

/// One record for one LLM call. Populated by `LlmRouter::record_outcome`
/// at every completion site and inserted asynchronously.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskOutcome {
    pub session_id: String,
    pub turn_idx: i64,
    pub model: String,
    pub provider: String,
    /// Serialized `TaskHint::as_str()` ("coding", "planning", ...).
    pub task_hint: String,
    pub latency_ms: i64,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub reasoning_tokens: i64,
    pub cost_usd: f64,
    /// What the provider says it billed, when it said.
    pub provider_cost_usd: Option<f64>,
    /// `provider`, `table`, `unpriced`, `free`, `flat` or `none`. `unpriced` means the cost is unknown.
    pub cost_source: String,
    /// What kind of work the call served (chat, memory, subagent...).
    pub origin: String,
    pub success: bool,
    pub error_class: Option<String>,
    /// Stable id from the source of a call HQ did not make itself (a coding agent's own record),
    /// so recording the same call twice is a no-op.
    pub external_id: Option<String>,
    pub quality_score: Option<f64>,
    pub tool_calls_issued: i64,
    pub tool_calls_succeeded: i64,
    /// Unix epoch seconds.
    pub recorded_at: i64,
}

impl TaskOutcome {
    pub fn now(
        session_id: impl Into<String>,
        turn_idx: i64,
        model: impl Into<String>,
        provider: impl Into<String>,
        task_hint: impl Into<String>,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            turn_idx,
            model: model.into(),
            provider: provider.into(),
            task_hint: task_hint.into(),
            latency_ms: 0,
            input_tokens: None,
            output_tokens: None,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            reasoning_tokens: 0,
            cost_usd: 0.0,
            provider_cost_usd: None,
            cost_source: "table".into(),
            origin: "unknown".into(),
            success: true,
            error_class: None,
            external_id: None,
            quality_score: None,
            tool_calls_issued: 0,
            tool_calls_succeeded: 0,
            recorded_at: current_epoch(),
        }
    }
}

fn current_epoch() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Insert a task_outcomes row. Returns its id.
pub fn insert(conn: &Connection, outcome: &TaskOutcome) -> Result<i64> {
    execute_insert(conn, outcome)?;
    Ok(conn.last_insert_rowid())
}

/// Insert a row that carries an `external_id`, doing nothing if that id is already recorded.
/// Returns whether a row was written.
pub fn insert_if_new(conn: &Connection, outcome: &TaskOutcome) -> Result<bool> {
    Ok(execute_insert(conn, outcome)? > 0)
}

fn execute_insert(conn: &Connection, outcome: &TaskOutcome) -> Result<usize> {
    Ok(conn.execute(
        "INSERT INTO task_outcomes (
            session_id, turn_idx, model, provider, task_hint,
            latency_ms, input_tokens, output_tokens, cost_usd,
            success, error_class, quality_score,
            tool_calls_issued, tool_calls_succeeded, recorded_at,
            cache_read_tokens, cache_write_tokens, reasoning_tokens, cost_source, origin,
            provider_cost_usd, external_id
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15,
                  ?16, ?17, ?18, ?19, ?20, ?21, ?22)
         ON CONFLICT(external_id) WHERE external_id IS NOT NULL DO NOTHING",
        params![
            outcome.session_id,
            outcome.turn_idx,
            outcome.model,
            outcome.provider,
            outcome.task_hint,
            outcome.latency_ms,
            outcome.input_tokens,
            outcome.output_tokens,
            outcome.cost_usd,
            outcome.success as i64,
            outcome.error_class,
            outcome.quality_score,
            outcome.tool_calls_issued,
            outcome.tool_calls_succeeded,
            outcome.recorded_at,
            outcome.cache_read_tokens,
            outcome.cache_write_tokens,
            outcome.reasoning_tokens,
            outcome.cost_source,
            outcome.origin,
            outcome.provider_cost_usd,
            outcome.external_id,
        ],
    )?)
}

/// Compact seed record for router health warm-up at startup.
#[derive(Debug, Clone)]
pub struct OutcomeSeedRow {
    pub provider: String,
    pub task_hint: String,
    pub success: bool,
    pub latency_ms: u64,
}

/// Fetch raw outcome rows for the last `window_secs` seconds, ordered oldest
/// first so the health windows are replayed in chronological order.
/// Used to warm up `LlmRouter` health at session start.
pub fn recent_seed_rows(conn: &Connection, window_secs: i64) -> Result<Vec<OutcomeSeedRow>> {
    let cutoff = current_epoch() - window_secs;
    let mut stmt = conn.prepare(
        "SELECT provider, task_hint, success, latency_ms
         FROM task_outcomes
         WHERE recorded_at >= ?1 AND COALESCE(error_class, '') != 'cancelled'
         ORDER BY recorded_at ASC",
    )?;
    let rows = stmt
        .query_map(params![cutoff], |row| {
            Ok(OutcomeSeedRow {
                provider: row.get(0)?,
                task_hint: row.get(1)?,
                success: row.get::<_, i64>(2)? != 0,
                latency_ms: row.get::<_, i64>(3)? as u64,
            })
        })?
        .collect::<std::result::Result<_, _>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::migrations::run(&conn).unwrap();
        conn
    }

    #[test]
    fn insert_and_read_back() {
        let conn = setup();

        let outcome = TaskOutcome {
            session_id: "sess1".into(),
            turn_idx: 0,
            model: "ollama/gemma4:e4b".into(),
            provider: "ollama".into(),
            task_hint: "coding".into(),
            latency_ms: 1200,
            input_tokens: Some(100),
            output_tokens: Some(200),
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            reasoning_tokens: 0,
            cost_usd: 0.0,
            provider_cost_usd: None,
            cost_source: "free".into(),
            origin: "chat".into(),
            success: true,
            error_class: None,
            external_id: None,
            quality_score: Some(0.7),
            tool_calls_issued: 2,
            tool_calls_succeeded: 2,
            recorded_at: current_epoch(),
        };

        let id = insert(&conn, &outcome).unwrap();
        assert!(id > 0);

        let (model, success): (String, bool) = conn
            .query_row(
                "SELECT model, success FROM task_outcomes WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((model.as_str(), success), ("ollama/gemma4:e4b", true));
    }

    #[test]
    fn a_cancelled_stream_does_not_seed_provider_health() {
        let conn = setup();
        let mut cancelled = TaskOutcome::now("s", 0, "m", "p", "coding");
        cancelled.error_class = Some("cancelled".into());
        insert(&conn, &cancelled).unwrap();
        insert(&conn, &TaskOutcome::now("s", 1, "m", "p", "coding")).unwrap();
        assert_eq!(recent_seed_rows(&conn, 3600).unwrap().len(), 1);
    }

    #[test]
    fn a_call_with_an_external_id_is_recorded_once() {
        let conn = setup();
        let mut o = TaskOutcome::now("s", 0, "m", "claude-code", "harness");
        o.external_id = Some("claude-code:msg_1".into());
        assert!(insert_if_new(&conn, &o).unwrap());
        assert!(!insert_if_new(&conn, &o).unwrap());
        o.external_id = Some("claude-code:msg_2".into());
        assert!(insert_if_new(&conn, &o).unwrap());
        // Rows without an id never collide with each other.
        let plain = TaskOutcome::now("s", 1, "m", "p", "chat");
        assert!(insert_if_new(&conn, &plain).unwrap() && insert_if_new(&conn, &plain).unwrap());
    }
}
