//! `harness_session_subscribe` and `harness_session_unsubscribe`: any session, of any harness,
//! asks at run time to hear when other sessions finish a turn, block or exit. What it hears is
//! a message on its own task thread, typed into its pane at its next idle (`a2a`), the same
//! path a monitor's `observe` uses. Chains of sessions waking each other stop at
//! `a2a::MAX_RELAY_DEPTH`.

use super::*;
use crate::registry::HqTool;
use crate::util::{arg_str, arg_str_list};
use async_trait::async_trait;

/// The session the subscription is for. A session acts only for itself; the owner names one.
fn observer_of(args: &Value) -> Result<String> {
    let named = Some(arg_str(args, "observer").trim().to_string()).filter(|s| !s.is_empty());
    if connector_label(args).is_some() {
        bail!("a claude.ai connection cannot change subscriptions");
    }
    match (caller_session(args).or(spawned_session(args)), named) {
        (Some(me), Some(other)) if me != other => {
            bail!("a session subscribes only itself; leave out observer")
        }
        (Some(me), _) => Ok(me.to_string()),
        (None, Some(named)) => Ok(named),
        (None, None) => bail!("name the observer: the session that should hear the events"),
    }
}

/// Whether `observer` may watch `target`: its own team, or a session working on the same task.
fn may_watch(c: &rusqlite::Connection, observer: &str, target: &str, task: &str) -> Result<()> {
    if crate::a2a::check_session_access(c, Some(observer), target).is_ok() {
        return Ok(());
    }
    let shares_task = registry::get(c, target)?
        .and_then(|r| r.mission_id)
        .is_some_and(|m| m == task);
    if shares_task {
        return Ok(());
    }
    bail!("session {observer} may only watch sessions of its own team or of its task, not {target}")
}

fn kinds_of(args: &Value) -> Option<Vec<String>> {
    Some(arg_str_list(args, "kinds")).filter(|k| !k.is_empty())
}

/// Records the subscription after every check. Returns what the observer now watches.
pub fn subscribe(db: &Arc<Database>, args: &Value) -> Result<Value> {
    let observer = observer_of(args)?;
    let targets = arg_str_list(args, "session_ids");
    if targets.is_empty() {
        bail!("name the sessions to watch in session_ids");
    }
    let owner = caller_session(args).is_none() && spawned_session(args).is_none();
    let kinds = kinds_of(args);
    db.with_conn(move |c| {
        let row = registry::get(c, &observer)?
            .filter(|r| r.status == registry::STATUS_RUNNING)
            .ok_or_else(|| anyhow::anyhow!("no running session '{observer}'"))?;
        let mut watching = registry::observed_by(c, &observer)?;
        for t in &targets {
            if !watching.contains(t) {
                watching.push(t.clone());
            }
        }
        launch::check_observe(watching.len(), row.mission_id.is_some())?;
        let task = row.mission_id.clone().unwrap_or_default();
        if !owner {
            for t in &targets {
                may_watch(c, &observer, t, &task)?;
            }
        }
        registry::add_observed(c, &observer, &targets, kinds.as_deref())?;
        Ok(json!({ "observer": observer, "watching": registry::observed_by(c, &observer)? }))
    })
}

pub fn unsubscribe(db: &Arc<Database>, args: &Value) -> Result<Value> {
    let observer = observer_of(args)?;
    let targets = arg_str_list(args, "session_ids");
    db.with_conn(move |c| {
        let targets = match targets.is_empty() {
            true => registry::observed_by(c, &observer)?,
            false => targets,
        };
        let removed = registry::remove_observed(c, &observer, &targets)?;
        Ok(json!({
            "observer": observer,
            "removed": removed,
            "watching": registry::observed_by(c, &observer)?,
        }))
    })
}

pub struct HarnessSessionSubscribeTool {
    pub(super) db: Arc<Database>,
}

#[async_trait]
impl HqTool for HarnessSessionSubscribeTool {
    fn name(&self) -> &str {
        "harness_session_subscribe"
    }
    fn description(&self) -> &str {
        "Hear when other harness sessions finish a turn, block at a dialog or exit, without polling: each event is left as a message on your task thread and typed into your session when you are next idle. Works for every harness (claude-code, copilot, antigravity, pi and the rest). A session subscribes itself (it needs a task_id, and may watch its own team or sessions on its task); the owner names the `observer`. `kinds` narrows it to some of finished, blocked and exited (default all three); subscribing again replaces them. Sessions that keep waking each other stop after a few rounds and the owner is told."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "session_ids": { "type": "array", "items": { "type": "string" }, "description": "The sessions to watch" },
                "kinds": { "type": "array", "items": { "type": "string", "enum": registry::OBSERVABLE_KINDS }, "description": "Which events (default all)" },
                "observer": { "type": "string", "description": "Owner only: the session that should hear them. A session is always its own observer." }
            },
            "required": ["session_ids"]
        })
    }
    fn category(&self) -> &str {
        "harness"
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || subscribe(&db, &args)).await?
    }
}

pub struct HarnessSessionUnsubscribeTool {
    pub(super) db: Arc<Database>,
}

#[async_trait]
impl HqTool for HarnessSessionUnsubscribeTool {
    fn name(&self) -> &str {
        "harness_session_unsubscribe"
    }
    fn description(&self) -> &str {
        "Stop hearing about sessions watched with harness_session_subscribe (or given to observe at spawn). Name them in session_ids, or leave it out to stop watching all of them."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "session_ids": { "type": "array", "items": { "type": "string" } },
                "observer": { "type": "string", "description": "Owner only: whose subscriptions to end" }
            }
        })
    }
    fn category(&self) -> &str {
        "harness"
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || unsubscribe(&db, &args)).await?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_db::harness_sessions_registry::{NewSession, Placement};
    use hq_db::tasks as t;

    fn session(db: &Database, id: &str, mission: Option<&str>) {
        db.with_conn(|c| {
            registry::insert(
                c,
                &NewSession {
                    id,
                    harness: "github-copilot",
                    label: "t",
                    cwd: "/t",
                    mission_id: mission,
                    placement: Placement {
                        host: "native",
                        agent_name: id,
                        workspace_id: id,
                        pane_id: id,
                    },
                    purpose: None,
                },
            )
        })
        .unwrap();
    }

    fn world() -> Arc<Database> {
        let db = Arc::new(Database::open_memory().unwrap());
        db.with_conn(|c| {
            t::create_initiative(c, "in-1", "personal", None, "HQ", "hq", "HQ")?;
            for (id, title) in [("tk-1", "shared"), ("tk-2", "other")] {
                let new = t::NewTask {
                    title,
                    created_by: "test",
                    ..Default::default()
                };
                t::create_task(c, id, "in-1", &new)?;
            }
            Ok(())
        })
        .unwrap();
        session(&db, "hs-coder", Some("tk-1"));
        session(&db, "hs-agy", Some("tk-1"));
        session(&db, "hs-stranger", Some("tk-2"));
        session(&db, "hs-loose", None);
        db
    }

    fn as_session(me: &str, args: Value) -> Value {
        let mut args = args;
        args[CALLER_SESSION_ARG] = json!(me);
        args
    }

    #[test]
    fn a_session_watches_a_session_on_its_task_for_the_kinds_it_names() {
        let db = world();
        let args = json!({ "session_ids": ["hs-agy"], "kinds": ["finished", "blocked"] });
        let got = subscribe(&db, &as_session("hs-coder", args)).unwrap();
        assert_eq!(got["watching"], json!(["hs-agy"]));
        let hears = |kind: &str| {
            db.with_conn(|c| registry::running_observers(c, "hs-agy", kind))
                .unwrap()
                .len()
        };
        assert_eq!((hears("blocked"), hears("exited")), (1, 0));

        let gone = unsubscribe(&db, &as_session("hs-coder", json!({}))).unwrap();
        assert_eq!((gone["removed"].as_u64(), hears("blocked")), (Some(1), 0));
    }

    #[test]
    fn a_session_cannot_watch_outside_its_team_and_task_or_act_for_another() {
        let db = world();
        let err = |args: Value| subscribe(&db, &args).unwrap_err().to_string();
        let stranger = as_session("hs-coder", json!({ "session_ids": ["hs-stranger"] }));
        assert!(err(stranger).contains("own team or of its task"));
        let other = as_session(
            "hs-coder",
            json!({ "session_ids": ["hs-agy"], "observer": "hs-agy" }),
        );
        assert!(err(other).contains("subscribes only itself"));
        let taskless = as_session("hs-loose", json!({ "session_ids": ["hs-coder"] }));
        assert!(err(taskless).contains("needs a task_id"));
        let connector = json!({ "session_ids": ["hs-agy"], "observer": "hs-coder", CONNECTOR_ARG: "claude.ai" });
        assert!(err(connector).contains("claude.ai connection"));
        assert!(err(json!({ "session_ids": ["hs-agy"] })).contains("name the observer"));
    }

    #[test]
    fn the_owner_wires_any_pair() {
        let db = world();
        let args = json!({ "session_ids": ["hs-stranger"], "observer": "hs-coder" });
        let got = subscribe(&db, &args).unwrap();
        assert_eq!(got["watching"], json!(["hs-stranger"]));
    }
}
