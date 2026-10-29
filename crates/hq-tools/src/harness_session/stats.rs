//! The numbers "How we will know" in docs/plans/remote-driven-teams.md asks for, read from a
//! task's or a team's event log: how long dialogs waited, how many the owner had to answer and
//! how many a policy or Drive settled, how long hosts were silent, and how much work got done.
//! Counts and times only: no dialog text, so reading them taints nothing.

use super::events::{Reader, Scope, sessions_in};
use super::*;
use crate::registry::HqTool;
use async_trait::async_trait;
use std::collections::BTreeMap;

/// Most events one report reads.
const MAX_EVENTS: usize = 100_000;

fn detail(e: &hq_db::session_events::StreamEvent) -> Value {
    e.detail
        .as_deref()
        .and_then(|d| serde_json::from_str(d).ok())
        .unwrap_or(Value::Null)
}

fn percentile(sorted: &[i64], p: usize) -> Option<i64> {
    if sorted.is_empty() {
        return None;
    }
    Some(sorted[(sorted.len() * p / 100).min(sorted.len() - 1)])
}

/// The report for every session `scope` names that `reader` may see.
pub fn report(db: &Arc<Database>, scope: &Scope, reader: &Reader) -> Result<Value> {
    let sessions = sessions_in(db, scope, reader)?;
    let ids = sessions.clone();
    let events = db.with_conn(move |c| hq_db::session_events::after(c, 0, &ids, MAX_EVENTS))?;
    let kind = |k: &str| events.iter().filter(|e| e.kind == k).count();

    let mut waits: Vec<i64> = events
        .iter()
        .filter(|e| e.kind == "unblocked")
        .filter_map(|e| detail(e)["blocked_ms"].as_i64())
        .collect();
    waits.sort_unstable();

    let mut by_owner = 0;
    let mut automatic: BTreeMap<String, usize> = BTreeMap::new();
    for e in events.iter().filter(|e| e.kind == "answered") {
        let d = detail(e);
        if d["registered"] != true {
            continue;
        }
        match d["by"].as_str().unwrap_or("unknown") {
            "owner" => by_owner += 1,
            other => *automatic.entry(other.to_string()).or_default() += 1,
        }
    }

    let outcome = |o: &str| {
        events
            .iter()
            .filter(|e| e.kind == "policy_decision" && detail(e)["outcome"] == o)
            .count()
    };

    // A host's outage is written on each of its sessions: count it once per host and time.
    let mut outages: BTreeMap<(String, String), i64> = BTreeMap::new();
    for e in events.iter().filter(|e| e.kind == "host_back") {
        let d = detail(e);
        let host = d["host"].as_str().unwrap_or("?").to_string();
        outages
            .entry((host, e.created_at.clone()))
            .or_insert(d["minutes"].as_i64().unwrap_or(0));
    }
    let unreachable: std::collections::BTreeSet<(String, String)> = events
        .iter()
        .filter(|e| e.kind == "host_unreachable")
        .map(|e| {
            (
                detail(e)["host"].as_str().unwrap_or("?").to_string(),
                e.created_at.clone(),
            )
        })
        .collect();

    Ok(json!({
        "sessions": sessions.len(),
        "events": events.len(),
        "dialogs": {
            "blocked": kind("blocked") + kind("question"),
            "questions": kind("question"),
            "answered_by_owner": by_owner,
            "answered_automatically": automatic,
        },
        "waits": {
            "count": waits.len(),
            "median_ms": percentile(&waits, 50),
            "p90_ms": percentile(&waits, 90),
            "max_ms": waits.last(),
            "total_minutes": waits.iter().sum::<i64>() / 60_000,
        },
        "policy": {
            "allowed": outcome("allowed"),
            "denied": outcome("denied"),
            "escalated": outcome("escalated"),
        },
        "hosts": {
            "outages": unreachable.len(),
            "silent_minutes": outages.values().sum::<i64>(),
        },
        "work": {
            "finished_turns": kind("finished"),
            "commits": kind("commit"),
            "exits": kind("exited"),
        },
    }))
}

pub struct HarnessSessionStatsTool {
    pub(super) db: Arc<Database>,
}

#[async_trait]
impl HqTool for HarnessSessionStatsTool {
    fn name(&self) -> &str {
        "harness_session_stats"
    }
    fn description(&self) -> &str {
        "How a team or a task's sessions went, from their event log: dialogs (how many, how many the owner answered, how many a command policy or Drive settled), how long dialogs waited (median, 90th percentile, longest, total), the policy's allows, denies and escalations, host outages and silent minutes, and finished turns, commits and exits. Name session_ids, a task_id or a team, or none for every session you may see. Numbers only."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "session_ids": { "type": "array", "items": { "type": "string" } },
                "task_id": { "type": "string" },
                "team": { "type": "string", "description": "A root session id: every session in its team" }
            }
        })
    }
    fn category(&self) -> &str {
        "harness"
    }
    fn is_read_only(&self) -> bool {
        true
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let scope = Scope::from_args(&args)?;
        let reader = Reader::of(&args);
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || report(&db, &scope, &reader)).await?
    }
}
