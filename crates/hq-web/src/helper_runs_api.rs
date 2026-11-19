//! Helper runs (sub-agents a chat delegated to) as the web client reads them: a thread's list,
//! a global search source, and the one JSON shape the socket events reuse.
//!
//! The registry (`hq_db::subagent_runs`) is the source of truth. A run started from a web chat has
//! platform `web` and the thread id as its chat id, so a thread only ever sees its own runs.

use axum::{
    Json,
    extract::{Path, Query, State},
    response::{IntoResponse, Response},
};
use hq_db::subagent_runs::{self as runs, RunRow};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

use crate::WsState;
use crate::error::ApiError;

/// Platform the web chat registers its runs under.
pub(crate) const WEB_PLATFORM: &str = "web";
/// Most runs one request returns.
const MAX_RUNS: i64 = 100;
/// Longest goal or result preview sent to the browser.
const PREVIEW_CHARS: usize = 600;

/// What a person sees: `queued`, `running`, `stalled` (alive on paper, quiet), `done`, `partial`,
/// `blocked`, or the terminal exec state (`failed`, `timed_out`, `cancelled`, `interrupted`,
/// `rejected`). A settled run that finished cleanly reads as `done` only when the parent has not
/// marked it `partial` or `blocked`.
pub(crate) fn run_state(row: &RunRow, now: i64) -> &'static str {
    use runs::Liveness;
    if row.settled_at.is_none() {
        return match (row.exec_status.as_str(), row.liveness(now)) {
            (runs::EXEC_QUEUED, _) => "queued",
            (_, Liveness::Working) => "running",
            _ => "stalled",
        };
    }
    match row.exec_status.as_str() {
        runs::EXEC_COMPLETED => match row.accept_status.as_str() {
            runs::ACCEPT_PARTIAL => "partial",
            runs::ACCEPT_BLOCKED => "blocked",
            _ => "done",
        },
        runs::EXEC_FAILED => "failed",
        runs::EXEC_TIMED_OUT => "timed_out",
        runs::EXEC_BLOCKED => "blocked",
        runs::EXEC_REJECTED => "rejected",
        runs::EXEC_CANCELLED => "cancelled",
        _ => "interrupted",
    }
}

/// The first vault path in a result (`Notebooks/...` or `Exports/...` with an extension): the
/// note or file the helper says it wrote. None when it named no such path.
pub(crate) fn result_link(output: &str) -> Option<String> {
    output
        .split(|c: char| {
            c.is_whitespace()
                || matches!(
                    c,
                    '`' | '(' | ')' | '[' | ']' | '"' | '\'' | '<' | '>' | ','
                )
        })
        .map(|t| t.trim_end_matches(['.', ';', ':']))
        .find(|t| {
            (t.starts_with("Notebooks/") || t.starts_with("Exports/"))
                && !t.contains("..")
                && t.rsplit('/').next().is_some_and(|f| f.contains('.'))
        })
        .map(str::to_string)
}

fn cut(text: &str) -> String {
    hq_core::redact::redact_secrets(&text.chars().take(PREVIEW_CHARS).collect::<String>())
}

/// One run as the client reads it (list, search and socket events share this shape).
pub(crate) fn run_json(row: &RunRow, now: i64) -> Value {
    let link = row
        .output_full
        .as_deref()
        .or(row.output_preview.as_deref())
        .and_then(result_link);
    json!({
        "id": row.run_id,
        "thread_id": row.thread_id,
        "task": row.child_id,
        "role": row.role,
        "goal": cut(&row.goal),
        "state": run_state(row, now),
        "exec_status": row.exec_status,
        "accept_status": row.accept_status,
        "detached": row.detached,
        "started_at": row.started_at,
        "last_activity_at": row.last_activity_at,
        "settled_at": row.settled_at,
        "result_link": link,
        "result_preview": row.output_preview.as_deref().map(cut),
        "error": row.error.as_deref().map(cut),
        "blocker": row.blocker_reason.as_deref().map(cut),
        "missing_deliverables": row.missing_deliverables,
    })
}

pub(crate) fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// `GET /api/threads/{thread_id}/helper-runs`: the runs this chat delegated, newest first.
pub(crate) async fn list_thread_runs_handler(
    State(state): State<Arc<WsState>>,
    Path(thread_id): Path<String>,
) -> Response {
    let filter = runs::ListFilter {
        scope: Some(runs::Origin {
            platform: WEB_PLATFORM.into(),
            chat_id: thread_id,
            thread_id: None,
            identity: None,
        }),
        limit: MAX_RUNS,
        ..Default::default()
    };
    match state.db.with_conn(|c| runs::list(c, &filter)) {
        Ok(rows) => {
            let now = now_secs();
            let list: Vec<Value> = rows.iter().map(|r| run_json(r, now)).collect();
            Json(json!({ "runs": list })).into_response()
        }
        Err(e) => ApiError::internal(e).into_response(),
    }
}

#[derive(Deserialize)]
pub(crate) struct SearchQuery {
    q: Option<String>,
    limit: Option<i64>,
}

/// `GET /api/helper-runs?q=`: web-chat helper runs whose goal, role or result mentions the query,
/// for the global search overlay. Each hit names its thread so the client can open it.
pub(crate) async fn search_runs_handler(
    State(state): State<Arc<WsState>>,
    Query(q): Query<SearchQuery>,
) -> Response {
    let needle = q.q.unwrap_or_default();
    let limit = q.limit.unwrap_or(20).clamp(1, MAX_RUNS);
    match state
        .db
        .with_conn(|c| runs::search(c, WEB_PLATFORM, &needle, limit))
    {
        Ok(rows) => {
            let now = now_secs();
            let list: Vec<Value> = rows.iter().map(|r| run_json(r, now)).collect();
            Json(json!({ "runs": list, "count": list.len() })).into_response()
        }
        Err(e) => ApiError::internal(e).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use runs::{NewRun, Origin, Settlement};

    fn state() -> Arc<WsState> {
        let vault = std::env::temp_dir().join(format!("hq-runs-test-{}", uuid::Uuid::new_v4()));
        Arc::new(WsState::new(vault, None))
    }

    fn seed(state: &WsState, id: &str, thread: &str, goal: &str) {
        let run = NewRun {
            run_id: id.into(),
            parent_run_id: None,
            parent_turn_id: None,
            origin: Some(Origin {
                platform: WEB_PLATFORM.into(),
                chat_id: thread.into(),
                thread_id: Some(thread.into()),
                identity: None,
            }),
            task_id: None,
            child_id: "t1".into(),
            role: "researcher".into(),
            goal: goal.into(),
            success_criteria: vec![],
            required_tools: vec![],
            detached: true,
            owner_pid: None,
            followup_depth: 0,
        };
        state
            .db
            .with_conn(|c| runs::insert_run(c, &run, now_secs()))
            .unwrap();
    }

    #[test]
    fn a_link_is_only_a_vault_path_with_a_file_name() {
        assert_eq!(
            result_link("Wrote `Notebooks/Projects/A/plan.md`."),
            Some("Notebooks/Projects/A/plan.md".into())
        );
        assert_eq!(
            result_link("saved to (Exports/2026-11/report.pdf), done"),
            Some("Exports/2026-11/report.pdf".into())
        );
        assert_eq!(result_link("see Notebooks/Projects/A for more"), None);
        assert_eq!(result_link("Notebooks/../secret.md"), None);
        assert_eq!(result_link("/etc/passwd"), None);
    }

    #[test]
    fn states_follow_execution_and_acceptance() {
        let s = state();
        seed(&s, "r1", "th1", "look it up");
        let now = now_secs();
        let row = s.db.with_conn(|c| runs::get(c, "r1")).unwrap().unwrap();
        assert_eq!(run_state(&row, now), "queued");
        s.db.with_conn(|c| runs::mark_running(c, "r1", now, 600))
            .unwrap();
        let row = s.db.with_conn(|c| runs::get(c, "r1")).unwrap().unwrap();
        assert_eq!(run_state(&row, now), "running");
        let mut settled = row.clone();
        settled.settled_at = Some(now);
        settled.exec_status = runs::EXEC_COMPLETED.into();
        settled.accept_status = runs::ACCEPT_PARTIAL.into();
        assert_eq!(run_state(&settled, now), "partial");
        settled.accept_status = runs::ACCEPT_UNVERIFIED.into();
        assert_eq!(run_state(&settled, now), "done");
        settled.exec_status = runs::EXEC_TIMED_OUT.into();
        assert_eq!(run_state(&settled, now), "timed_out");
    }

    #[tokio::test]
    async fn a_thread_lists_only_its_own_runs_and_search_finds_them() {
        let s = state();
        seed(&s, "r-a", "th-a", "compare vendors");
        seed(&s, "r-b", "th-b", "compare prices");
        let now = now_secs();
        s.db.with_conn(|c| runs::mark_running(c, "r-a", now, 600))
            .unwrap();
        s.db.with_conn(|c| {
            runs::settle(
                c,
                "r-a",
                &Settlement {
                    exec_status: runs::EXEC_COMPLETED.into(),
                    accept_status: runs::ACCEPT_UNVERIFIED.into(),
                    missing_deliverables: vec![],
                    blocker_reason: None,
                    next_action: None,
                    output_full: "Wrote Notebooks/Projects/X/vendors.md".into(),
                    output_preview: "Wrote Notebooks/Projects/X/vendors.md".into(),
                    error: None,
                    resolved_backend: "inprocess".into(),
                },
                now,
                false,
            )
        })
        .unwrap();

        let resp = list_thread_runs_handler(State(s.clone()), Path("th-a".into())).await;
        let body = axum::body::to_bytes(resp.into_body(), 1 << 20)
            .await
            .unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        let list = v["runs"].as_array().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0]["id"], "r-a");
        assert_eq!(list[0]["state"], "done");
        assert_eq!(list[0]["result_link"], "Notebooks/Projects/X/vendors.md");

        let resp = search_runs_handler(
            State(s),
            Query(SearchQuery {
                q: Some("compare".into()),
                limit: None,
            }),
        )
        .await;
        let body = axum::body::to_bytes(resp.into_body(), 1 << 20)
            .await
            .unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["count"], 2);
    }
}
