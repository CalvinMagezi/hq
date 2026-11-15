//! `harness_fleet_status`: one small reply about every session instead of fifty rows of
//! `harness_session_list`. It counts by state, names the blocked sessions and the driven ones,
//! and says how many rows the list leaves out.

use super::*;
use crate::registry::HqTool;
use async_trait::async_trait;

/// Most sessions read from the registry for one report.
const SCAN_LIMIT: usize = 500;
/// Blocked sessions named in a reply, oldest wait first; the count covers all of them.
pub const MAX_BLOCKED_ROWS: usize = 10;
/// Driven sessions named in a reply.
pub const MAX_DRIVEN_ROWS: usize = 20;
/// The kind of event the supervisor logs when a session starts waiting at a dialog.
const BLOCKED_EVENT: &str = "blocked";

pub struct HarnessFleetStatusTool {
    pub(super) db: Arc<Database>,
}

#[async_trait]
impl HqTool for HarnessFleetStatusTool {
    fn name(&self) -> &str {
        "harness_fleet_status"
    }
    fn description(&self) -> &str {
        "A small roll-up of every harness session: counts by registry status and by live agent status, the blocked sessions (id, label, pending dialog kind and number, how long it has waited, task and team), which sessions HQ drives against the cap, and how many sessions harness_session_list leaves out. Call it first when many sessions run, then answer blocked ones with harness_session_pending and harness_session_answer."
    }
    fn parameters(&self) -> Value {
        json!({ "type": "object", "properties": {} })
    }
    fn category(&self) -> &str {
        "harness"
    }
    fn is_read_only(&self) -> bool {
        true
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            let rows = db.with_conn(|c| registry::list(c, None, SCAN_LIMIT))?;
            // A claude.ai connection sees only the sessions it may drive.
            let rows: Vec<HarnessSessionRow> = rows
                .into_iter()
                .filter(|r| connector_may_drive(&db, &args, &r.id).is_ok())
                .collect();
            let running: Vec<HarnessSessionRow> = rows
                .iter()
                .filter(|r| r.status == registry::STATUS_RUNNING)
                .cloned()
                .collect();
            let polled = poll_hosts(&running);
            let cap = agent_host_config().driven_session_cap();
            fleet_status(&db, &rows, &polled, cap)
        })
        .await?
    }
}

/// The report for `rows` (newest first, as the registry lists them) against what the hosts said.
pub fn fleet_status(
    db: &Arc<Database>,
    rows: &[HarnessSessionRow],
    polled: &HostPoll,
    cap: i64,
) -> Result<Value> {
    let mut by_status: std::collections::BTreeMap<&str, i64> = Default::default();
    let mut by_agent: std::collections::BTreeMap<String, i64> = Default::default();
    let mut blocked = Vec::new();
    let mut driven = Vec::new();
    for row in rows {
        *by_status.entry(row.status.as_str()).or_default() += 1;
        if row.status != registry::STATUS_RUNNING {
            continue;
        }
        let live = liveness(polled, row);
        let state = match &live {
            Liveness::Alive(agent) => agent.status.as_str(),
            Liveness::Gone => "gone",
            Liveness::HostUnreachable(_) => "unreachable",
        };
        *by_agent.entry(state.to_string()).or_default() += 1;
        if row.drive {
            driven.push((row, state));
        }
        if let Liveness::Alive(agent) = &live
            && agent.status == AgentStatus::Blocked
        {
            blocked.push((row, agent.clone()));
        }
    }
    let waited = |row: &HarnessSessionRow| {
        db.with_conn(|c| registry::seconds_since_event(c, &row.id, BLOCKED_EVENT))
            .ok()
            .flatten()
    };
    let mut blocked: Vec<_> = blocked
        .into_iter()
        .map(|(row, agent)| (waited(row), row, agent))
        .collect();
    // Longest wait first; a wait nobody recorded sorts last.
    blocked.sort_by_key(|(secs, row, _)| (std::cmp::Reverse(secs.unwrap_or(-1)), row.id.clone()));
    let belongs = |row: &HarnessSessionRow| -> Value {
        let task = row.mission_id.as_deref().and_then(|m| {
            db.with_conn(|c| hq_db::tasks::get_task(c, m))
                .ok()
                .flatten()
                .map(|t| t.display_id)
        });
        let team = team_root(db, rows, row);
        json!({ "task": task, "team": team })
    };
    let shown: Vec<Value> = blocked
        .iter()
        .take(MAX_BLOCKED_ROWS)
        .map(|(secs, row, agent)| {
            let mut v = json!({
                "session_id": row.id,
                "label": row.label,
                "harness": row.harness,
                "pending_kind": agent.pending.as_ref().map(|d| d.kind.clone()),
                "pending_seq": agent.pending_seq,
                "blocked_seconds": secs,
                "driven": row.drive,
            });
            v["belongs_to"] = belongs(row);
            v
        })
        .collect();
    let driven_rows: Vec<Value> = driven
        .iter()
        .take(MAX_DRIVEN_ROWS)
        .map(|(row, state)| {
            json!({
                "session_id": row.id,
                "label": row.label,
                "agent_status": state,
                "nudges_sent": row.nudges_sent,
                "belongs_to": belongs(row),
            })
        })
        .collect();
    let total = rows.len();
    Ok(json!({
        "total_sessions": total,
        "scan_truncated": total >= SCAN_LIMIT,
        "list_shows": SESSION_LIST_LIMIT,
        "list_hides": total.saturating_sub(SESSION_LIST_LIMIT),
        "by_status": by_status,
        "running_by_agent_status": by_agent,
        "blocked": { "count": blocked.len(), "shown": shown.len(), "sessions": shown },
        "driven": {
            "count": driven.len(),
            "cap": cap,
            "shown": driven_rows.len(),
            "sessions": driven_rows,
        },
    }))
}

/// The root of the team a session is in, or `None` for one that stands alone.
fn team_root(
    db: &Arc<Database>,
    rows: &[HarnessSessionRow],
    row: &HarnessSessionRow,
) -> Option<String> {
    if row.parent_session_id.is_some() {
        return db.with_conn(|c| registry::tree_root(c, &row.id)).ok();
    }
    let leads = rows
        .iter()
        .any(|r| r.parent_session_id.as_deref() == Some(row.id.as_str()));
    leads.then(|| row.id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed(db: &Arc<Database>, id: &str, parent: Option<&str>) {
        db.with_conn(|c| {
            registry::insert(
                c,
                &registry::NewSession {
                    id,
                    harness: "claude-code",
                    label: id,
                    cwd: "/w",
                    mission_id: None,
                    placement: registry::Placement {
                        host: "local",
                        agent_name: id,
                        workspace_id: "w1",
                        pane_id: "w1:p1",
                    },
                    purpose: None,
                },
            )?;
            if let Some(p) = parent {
                c.execute(
                    "UPDATE harness_sessions SET parent_session_id = ?2, spawn_depth = 1 WHERE id = ?1",
                    rusqlite::params![id, p],
                )?;
            }
            Ok(())
        })
        .unwrap();
    }

    fn agent(name: &str, status: AgentStatus, pending: Option<&str>) -> AgentInfo {
        AgentInfo {
            name: Some(name.into()),
            kind: "claude-code".into(),
            status,
            pane_id: "w1:p1".into(),
            workspace_id: "w1".into(),
            cwd: "/w".into(),
            title: None,
            state_change_seq: 1,
            launch_pending: false,
            agent_session_id: None,
            exited: false,
            exit_code: None,
            rule: None,
            pending: pending.map(|kind| {
                Box::new(agent_host::PendingDialog {
                    kind: kind.into(),
                    question: None,
                    command: None,
                    cwd: None,
                    options: Vec::new(),
                    truncated: false,
                    text: None,
                })
            }),
            pending_seq: pending.map(|_| 7),
            acp: None,
        }
    }

    fn rows(db: &Arc<Database>) -> Vec<HarnessSessionRow> {
        db.with_conn(|c| registry::list(c, None, SCAN_LIMIT))
            .unwrap()
    }

    #[test]
    fn counts_blocked_driven_and_team_in_one_small_reply() {
        let db = Arc::new(Database::open_memory().unwrap());
        seed(&db, "hs-lead", None);
        seed(&db, "hs-worker", Some("hs-lead"));
        seed(&db, "hs-quiet", None);
        seed(&db, "hs-done", None);
        db.with_conn(|c| {
            registry::set_status(c, "hs-done", registry::STATUS_EXITED)?;
            registry::watch_from_chat(c, "hs-worker", "th-1", true)?;
            c.execute(
                "INSERT INTO harness_session_events (session_id, kind, actor, created_at)
                 VALUES ('hs-worker', 'blocked', 'hq', datetime('now', '-10 minutes'))",
                [],
            )?;
            Ok(())
        })
        .unwrap();
        let polled = HostPoll::from([(
            "local".to_string(),
            Ok(vec![
                agent("hs-lead", AgentStatus::Working, None),
                agent("hs-worker", AgentStatus::Blocked, Some("command")),
                agent("hs-quiet", AgentStatus::Idle, None),
            ]),
        )]);

        let report = fleet_status(&db, &rows(&db), &polled, 8).unwrap();

        assert_eq!(report["total_sessions"], 4);
        assert_eq!(report["by_status"]["running"], 3);
        assert_eq!(report["by_status"]["exited"], 1);
        assert_eq!(report["running_by_agent_status"]["blocked"], 1);
        assert_eq!(report["running_by_agent_status"]["working"], 1);
        assert_eq!(report["running_by_agent_status"]["idle"], 1);
        let blocked = &report["blocked"]["sessions"][0];
        assert_eq!(report["blocked"]["count"], 1);
        assert_eq!(blocked["session_id"], "hs-worker");
        assert_eq!(blocked["pending_kind"], "command");
        assert_eq!(blocked["pending_seq"], 7);
        assert_eq!(blocked["belongs_to"]["team"], "hs-lead");
        assert!(blocked["blocked_seconds"].as_i64().unwrap() >= 600);
        assert_eq!(blocked["driven"], true);
        assert_eq!(report["driven"]["count"], 1);
        assert_eq!(report["driven"]["sessions"][0]["session_id"], "hs-worker");
        assert_eq!(report["driven"]["cap"], 8);
        assert_eq!(report["list_hides"], 0);
    }

    #[test]
    fn says_when_the_list_hides_rows_and_caps_the_blocked_rows() {
        let db = Arc::new(Database::open_memory().unwrap());
        let n = SESSION_LIST_LIMIT + MAX_BLOCKED_ROWS + 5;
        let mut agents = Vec::new();
        for i in 0..n {
            let id = format!("hs-{i:03}");
            seed(&db, &id, None);
            agents.push(agent(&id, AgentStatus::Blocked, Some("question")));
        }
        db.with_conn(|c| {
            c.execute(
                "UPDATE harness_sessions SET drive = 1, owner_thread = 'th-1'",
                [],
            )?;
            Ok(())
        })
        .unwrap();
        let polled = HostPoll::from([("local".to_string(), Ok(agents))]);

        let report = fleet_status(&db, &rows(&db), &polled, 8).unwrap();

        assert_eq!(report["total_sessions"], n);
        assert_eq!(report["list_hides"], n - SESSION_LIST_LIMIT);
        assert_eq!(report["blocked"]["count"], n);
        assert_eq!(report["blocked"]["shown"], MAX_BLOCKED_ROWS);
        assert_eq!(report["driven"]["count"], n);
        assert_eq!(report["driven"]["shown"], MAX_DRIVEN_ROWS);
    }

    #[test]
    fn is_registered_read_only() {
        let db = Arc::new(Database::open_memory().unwrap());
        let tools = super::tools::create_harness_session_tools(db, None, None);
        let tool = tools
            .iter()
            .find(|t| t.name() == "harness_fleet_status")
            .expect("registered next to harness_session_list");
        assert!(tool.is_read_only());
        assert_eq!(tool.category(), "harness");
    }
}
