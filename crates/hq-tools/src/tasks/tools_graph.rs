//! `task_related`: explainable relationships between tasks (FR-069).

use anyhow::{Result, bail};
use async_trait::async_trait;
use hq_db::Database;
use hq_db::task_graph as g;
use serde_json::{Value, json};
use std::sync::Arc;

use crate::registry::HqTool;
use crate::util::arg_str;

const DEFAULT_LIMIT: usize = 10;
const INFERRED_NOTE: &str =
    "Inferred from shared text and tags. Not a declared dependency and not a fact about the work.";

pub(super) struct TaskRelatedTool {
    pub(super) db: Arc<Database>,
}

#[async_trait]
impl HqTool for TaskRelatedTool {
    fn name(&self) -> &str {
        "task_related"
    }
    fn description(&self) -> &str {
        "Find tasks related to one task. Returns two separate lists: `explicit` links someone \
         declared (parent, subtask, depends_on, dependent) and `inferred` similarity with a score \
         and the evidence behind it (shared terms, shared tags, same initiative). Inferred links \
         are hints, never dependencies. One hop, capped at 25. If the similarity index has nothing \
         or fails, `fallback` lists tasks from the same initiative instead. Set rebuild=true to \
         drop and re-derive the whole index (only the derived index changes, never task data)."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Internal id or display id" },
                "limit": { "type": "integer", "description": "Max inferred links (default 10, max 25)" },
                "rebuild": { "type": "boolean", "description": "Rebuild the derived index first", "default": false }
            },
            "required": ["id"]
        })
    }
    fn category(&self) -> &str {
        "tasks"
    }
    // Only the derived index is written; no task, comment or dependency is touched.
    fn is_read_only(&self) -> bool {
        true
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let id = arg_str(&args, "id");
        if id.is_empty() {
            bail!("id is required");
        }
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .map_or(DEFAULT_LIMIT, |n| n as usize)
            .clamp(1, g::MAX_RESULTS);
        // A full rebuild drops and re-derives the index: not something the tasks scope may repeat.
        let rebuild = args
            .get("rebuild")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            && !crate::harness_session::is_tasks_scope(&args);
        self.db.with_conn(move |c| related(c, &id, limit, rebuild))
    }
}

fn related(c: &rusqlite::Connection, id: &str, limit: usize, rebuild: bool) -> Result<Value> {
    let Some(task_id) = g::resolve_task(c, id)? else {
        bail!("no task found for that id");
    };
    let mut out = json!({ "task_id": task_id });
    out["explicit"] = json!(
        g::explicit_links(c, &task_id)?
            .into_iter()
            .map(|l| json!({ "class": "explicit", "kind": l.kind, "task": l.task }))
            .collect::<Vec<_>>()
    );
    let synced = if rebuild {
        g::rebuild(c)
    } else {
        g::sync_to_current(c, g::DEFAULT_SYNC_BUDGET)
    };
    let inferred = match &synced {
        Ok(_) => g::inferred_links(c, &task_id, limit),
        Err(e) => Err(anyhow::anyhow!("{e}")),
    };
    match (synced, inferred) {
        (Ok(report), Ok(links)) => {
            out["index"] = json!(report);
            out["inferred"] = json!(
                links
                    .iter()
                    .map(|l| json!({
                        "class": "inferred", "note": INFERRED_NOTE,
                        "task": l.task, "score": l.score, "evidence": l.evidence,
                    }))
                    .collect::<Vec<_>>()
            );
        }
        (_, Err(e)) | (Err(e), _) => {
            out["inferred"] = json!([]);
            out["warning"] = json!(format!("similarity index unavailable: {e}"));
        }
    }
    if out["inferred"].as_array().is_some_and(Vec::is_empty) {
        out["fallback"] = json!({
            "kind": "same_initiative_listing",
            "tasks": g::fallback_listing(c, &task_id, limit)?,
        });
    }
    Ok(out)
}
