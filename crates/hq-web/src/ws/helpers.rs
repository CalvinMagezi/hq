//! Helper-run events on the chat socket. The tool step alone cannot say when a helper really
//! finishes (a background dispatch returns at once), so the socket also carries the registry's
//! view: `subagent_start` when a run is registered and `subagent_end` when it settles.

use hq_db::Database;
use serde_json::{Value, json};

use crate::helper_runs_api::{now_secs, run_json};

/// The tool whose result names the runs it started.
pub(super) const SPAWN_TOOL: &str = "spawn_subagents";

/// Registry rows for the runs a `spawn_subagents` result names, as the client's JSON shape.
pub(super) fn runs_in_result(db: &Database, result: &str) -> Vec<Value> {
    let ids = hq_agent::agents::run_ids_in_result(result);
    if ids.is_empty() {
        return Vec::new();
    }
    let now = now_secs();
    db.with_conn(|c| {
        Ok(ids
            .iter()
            .filter_map(|id| hq_db::subagent_runs::get(c, id).ok().flatten())
            .map(|row| run_json(&row, now))
            .collect::<Vec<_>>())
    })
    .unwrap_or_default()
}

/// Socket messages for helpers a `spawn_subagents` call just registered: a `subagent_start` for
/// each, then a `subagent_end` for any that already settled (a blocking call).
pub(super) fn start_messages(tid: &Option<String>, runs: &[Value]) -> Vec<String> {
    let mut out = Vec::new();
    for run in runs {
        out.push(json!({"type": "subagent_start", "thread_id": tid, "run": run}).to_string());
        if run["settled_at"].is_number() {
            out.push(end_message(tid, run));
        }
    }
    out
}

pub(super) fn end_message(tid: &Option<String>, run: &Value) -> String {
    json!({"type": "subagent_end", "thread_id": tid, "run_id": run["id"], "state": run["state"], "run": run})
        .to_string()
}

/// A settled background child, read back from the registry so the event carries the final state.
pub(super) fn settled_message(db: &Database, tid: &Option<String>, run_id: &str) -> Option<String> {
    let row = db
        .with_conn(|c| hq_db::subagent_runs::get(c, run_id))
        .ok()
        .flatten()?;
    Some(end_message(tid, &run_json(&row, now_secs())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_core::types::SessionEvent;

    #[test]
    fn a_spawn_result_yields_structured_helpers_and_events() {
        let db = Database::open_memory().unwrap();
        let id = "11111111-1111-4111-8111-111111111111";
        db.with_conn(|c| {
            hq_db::subagent_runs::insert_run(
                c,
                &hq_db::subagent_runs::NewRun {
                    run_id: id.into(),
                    parent_run_id: None,
                    parent_turn_id: None,
                    origin: Some(hq_db::subagent_runs::Origin {
                        platform: "web".into(),
                        chat_id: "th".into(),
                        thread_id: Some("th".into()),
                        identity: None,
                    }),
                    task_id: None,
                    child_id: "t1".into(),
                    role: "researcher".into(),
                    goal: "find it".into(),
                    success_criteria: vec![],
                    required_tools: vec![],
                    detached: true,
                    owner_pid: None,
                    followup_depth: 0,
                },
                now_secs(),
            )
        })
        .unwrap();
        let text = format!("Dispatched 1 child task(s). Run ids: t1={id}. Results arrive later.");
        let runs = runs_in_result(&db, &text);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0]["state"], "queued");

        let tid = Some("th".to_string());
        let msgs = start_messages(&tid, &runs);
        assert_eq!(msgs.len(), 1, "a run still going gets no end event");
        let start: Value = serde_json::from_str(&msgs[0]).unwrap();
        assert_eq!(start["type"], "subagent_start");
        assert_eq!(start["run"]["id"], id);

        let end = settled_message(&db, &tid, id).unwrap();
        let end: Value = serde_json::from_str(&end).unwrap();
        assert_eq!(
            (end["type"].as_str(), end["run_id"].as_str()),
            (Some("subagent_end"), Some(id))
        );

        let step = super::super::session_event_json(
            &SessionEvent::ToolEnd {
                tool_name: SPAWN_TOOL.into(),
                tool_call_id: "c1".into(),
                result: text,
            },
            &tid,
            &runs,
        )
        .unwrap();
        let step: Value = serde_json::from_str(&step).unwrap();
        assert_eq!(step["helpers"][0]["id"], id);
    }
}
