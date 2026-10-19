//! Shelving: tidying the task views without touching what agents see. A shelved task or
//! list is hidden from lists that ask for it (`task_list` with `shelf` set to `exclude`) and
//! from nothing else, so the queue, dependencies and links treat it as before.

use anyhow::{Result, bail};
use async_trait::async_trait;
use hq_db::Database;
use hq_db::tasks as t;
use serde_json::{Value, json};
use std::sync::Arc;

use crate::registry::HqTool;
use crate::util::arg_str;

fn id_list(args: &Value) -> Result<Vec<String>> {
    let ids: Vec<String> = args
        .get("ids")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    if ids.is_empty() || ids.len() > t::MAX_SHELVE_BATCH {
        bail!(
            "ids must have between 1 and {} task ids, got {}",
            t::MAX_SHELVE_BATCH,
            ids.len()
        );
    }
    Ok(ids)
}

/// A launched agent shelves only tasks it may use, and never sweeps or shelves a list.
fn check_ids(db: &Database, args: &Value, ids: &[String]) -> Result<()> {
    let caller = crate::harness_session::caller_session(args).map(str::to_string);
    if caller.is_none() {
        return Ok(());
    }
    db.with_conn(|c| {
        for id in ids {
            let task = t::get_task(c, id)?.ok_or_else(|| anyhow::anyhow!("no task '{id}'"))?;
            crate::a2a::check_task_access(c, caller.as_deref(), &task.id)?;
        }
        Ok(())
    })
}

fn refuse_launched_agent(args: &Value) -> Result<()> {
    if crate::harness_session::caller_session(args).is_some() {
        bail!("a launched agent may shelve its own tasks only; ask the owner to do this");
    }
    Ok(())
}

fn outcome_json(out: &t::ShelveOutcome) -> Value {
    json!({ "changed": out.changed, "skipped": out.skipped })
}

pub(super) struct TaskShelveTool {
    pub(super) db: Arc<Database>,
}

#[async_trait]
impl HqTool for TaskShelveTool {
    fn name(&self) -> &str {
        "task_shelve"
    }
    fn description(&self) -> &str {
        "Tidy the owner's task views: hide tasks from the web lists without changing them. A \
         shelved task keeps its status, dependencies, links and place in the queue, and task_get, \
         task_claim and task_update work on it as before. Claiming or reopening a shelved task \
         brings it back. task_list shows shelved tasks unless you pass shelf=exclude. Not a \
         deletion; task_unshelve undoes it. A task someone is working on is skipped."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "ids": { "type": "array", "items": { "type": "string" }, "description": "Internal ids or display ids, at most 500" }
            },
            "required": ["ids"]
        })
    }
    fn category(&self) -> &str {
        "tasks"
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let ids = id_list(&args)?;
        check_ids(&self.db, &args, &ids)?;
        let out = self.db.with_conn(|c| t::shelve_tasks(c, &ids))?;
        Ok(outcome_json(&out))
    }
}

pub(super) struct TaskUnshelveTool {
    pub(super) db: Arc<Database>,
}

#[async_trait]
impl HqTool for TaskUnshelveTool {
    fn name(&self) -> &str {
        "task_unshelve"
    }
    fn description(&self) -> &str {
        "Put shelved tasks back in the owner's task lists. Undoes task_shelve and \
         task_shelve_completed; pass the ids they returned in `changed`."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "ids": { "type": "array", "items": { "type": "string" }, "description": "Internal ids or display ids, at most 500" }
            },
            "required": ["ids"]
        })
    }
    fn category(&self) -> &str {
        "tasks"
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let ids = id_list(&args)?;
        check_ids(&self.db, &args, &ids)?;
        let out = self.db.with_conn(|c| t::unshelve_tasks(c, &ids))?;
        Ok(outcome_json(&out))
    }
}

pub(super) struct TaskShelveCompletedTool {
    pub(super) db: Arc<Database>,
}

#[async_trait]
impl HqTool for TaskShelveCompletedTool {
    fn name(&self) -> &str {
        "task_shelve_completed"
    }
    fn description(&self) -> &str {
        "Shelve every completed task older than `older_than_days` in one step, so the owner's \
         lists show current work. Only finished tasks go: open work, a task with an open sub-task \
         and a task someone holds are never touched. It is a dry run unless you pass \
         dry_run=false, and the reply lists the shelved ids so task_unshelve can undo it. \
         Agents lose nothing: shelved tasks stay in the queue, the graph and dependencies."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "older_than_days": { "type": "integer", "minimum": 1, "maximum": t::MAX_SHELVE_AGE_DAYS, "description": "Completed at least this many days ago" },
                "space_id": { "type": "string", "description": "Only this space" },
                "initiative_id": { "type": "string", "description": "Only this list" },
                "dry_run": { "type": "boolean", "default": true, "description": "Count what would be shelved without shelving it" }
            },
            "required": ["older_than_days"]
        })
    }
    fn category(&self) -> &str {
        "tasks"
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        refuse_launched_agent(&args)?;
        let days = args
            .get("older_than_days")
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| anyhow::anyhow!("older_than_days is required"))?;
        let dry_run = args.get("dry_run").and_then(Value::as_bool).unwrap_or(true);
        let scope = t::SweepScope {
            space_id: Some(arg_str(&args, "space_id")).filter(|s| !s.is_empty()),
            initiative_id: Some(arg_str(&args, "initiative_id")).filter(|s| !s.is_empty()),
        };
        let out = self
            .db
            .with_conn(|c| t::shelve_completed(c, days, &scope, dry_run))?;
        Ok(json!({ "matched": out.matched, "changed": out.changed, "dry_run": out.dry_run }))
    }
}

pub(super) struct InitiativeShelveTool {
    pub(super) db: Arc<Database>,
}

#[async_trait]
impl HqTool for InitiativeShelveTool {
    fn name(&self) -> &str {
        "initiative_shelve"
    }
    fn description(&self) -> &str {
        "Shelve a list so the owner's web view hides it and every task in it, or bring it back \
         with shelved=false. The tasks are not touched: agents still reach them, and working on \
         one (claiming it, reopening it, adding a task to the list) brings the list back. The \
         reply says how many tasks in the list are still open."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "initiative_id": { "type": "string" },
                "shelved": { "type": "boolean", "default": true }
            },
            "required": ["initiative_id"]
        })
    }
    fn category(&self) -> &str {
        "tasks"
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        refuse_launched_agent(&args)?;
        let id = arg_str(&args, "initiative_id");
        if id.is_empty() {
            bail!("initiative_id is required");
        }
        let shelved = args.get("shelved").and_then(Value::as_bool).unwrap_or(true);
        let (list, open) = self
            .db
            .with_conn(|c| t::set_initiative_shelved(c, &id, shelved))?;
        Ok(json!({
            "id": list.id, "name": list.name, "shelved_at": list.shelved_at, "open_tasks": open,
        }))
    }
}
