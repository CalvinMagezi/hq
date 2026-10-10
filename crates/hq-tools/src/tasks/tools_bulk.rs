//! Many tasks in one call: breaking an epic into its tasks, or moving a set of tasks together.
//! Each item runs through the same tool a single call would, so every rule, notification and
//! piece of attribution is identical. A bad item fails alone and is reported; the others stand.

use anyhow::{Result, bail};
use async_trait::async_trait;
use serde_json::{Map, Value, json};
use std::collections::{HashMap, HashSet};

use super::tools_lease::onboarding_hint;
use super::tools_task::{TaskCreateTool, TaskUpdateTool};
use crate::registry::HqTool;

/// Most items one bulk call takes.
const MAX_BULK_ITEMS: usize = 100;
/// Marks a reference to an earlier item's `key`: `@epic` becomes that item's display id.
const KEY_REF_PREFIX: char = '@';
/// Fields every item inherits from the call, so the caller sets them once. An item may set its
/// own `lease`, `actor` or names, as it could in a single call.
const INHERITED: &[&str] = &["lease", "actor", "created_by", "author"];
/// What the gateway proved about the caller: the session it is, and the scoped key it came in on.
/// They come only from the call itself: an item or `defaults` carrying one is ignored, so a bulk
/// call can neither act as another session nor drop its scope for its items.
const ATTESTED: &[&str] = &[
    crate::harness_session::CALLER_SESSION_ARG,
    crate::harness_session::TASKS_SCOPE_ARG,
    crate::harness_session::HANDOFF_SCOPE_ARG,
];

fn items_of(args: &Value, field: &str) -> Result<Vec<Value>> {
    let Some(items) = args.get(field).and_then(Value::as_array) else {
        bail!("{field} must be a list");
    };
    if items.is_empty() || items.len() > MAX_BULK_ITEMS {
        bail!("{field} must have between 1 and {MAX_BULK_ITEMS} items, got {}", items.len());
    }
    Ok(items.to_vec())
}

/// One item's arguments: its own fields, then the call's `defaults` and the call's own lease and
/// names where the item set none. The attested session is taken only from the call itself.
fn prepare(args: &Value, defaults: &Map<String, Value>, item: &Map<String, Value>) -> Map<String, Value> {
    let mut spec = item.clone();
    spec.retain(|field, _| !ATTESTED.contains(&field.as_str()));
    for (field, value) in defaults.iter().filter(|(field, _)| !ATTESTED.contains(&field.as_str())) {
        spec.entry(field.clone()).or_insert_with(|| value.clone());
    }
    for key in INHERITED {
        if let (Some(v), false) = (args.get(*key), spec.contains_key(*key)) {
            spec.insert((*key).to_string(), v.clone());
        }
    }
    for key in ATTESTED {
        if let Some(proof) = args.get(*key) {
            spec.insert((*key).to_string(), proof.clone());
        }
    }
    spec
}

/// Replaces `@key` with the display id created for that key, or says which key is unknown.
fn resolve_refs(spec: &mut Map<String, Value>, made: &HashMap<String, String>) -> Result<()> {
    let resolve = |text: &str| -> Result<String> {
        match text.strip_prefix(KEY_REF_PREFIX) {
            None => Ok(text.to_string()),
            Some(key) => made
                .get(key)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("'@{key}' does not name an earlier item that was created")),
        }
    };
    if let Some(Value::String(parent)) = spec.get("parent_id") {
        let resolved = resolve(parent)?;
        spec.insert("parent_id".into(), json!(resolved));
    }
    if let Some(Value::Array(deps)) = spec.get_mut("depends_on") {
        for dep in deps.iter_mut() {
            if let Some(text) = dep.as_str() {
                *dep = json!(resolve(text)?);
            }
        }
    }
    if let Some(Value::Array(links)) = spec.get_mut("links") {
        for link in links.iter_mut().filter(|l| l.get("kind") == Some(&json!("task"))) {
            if let Some(text) = link.get("ref").and_then(Value::as_str) {
                link["ref"] = json!(resolve(text)?);
            }
        }
    }
    Ok(())
}

pub(super) struct TaskCreateManyTool {
    pub(super) inner: TaskCreateTool,
}

#[async_trait]
impl HqTool for TaskCreateManyTool {
    fn name(&self) -> &str {
        "task_create_many"
    }
    fn description(&self) -> &str {
        "Create up to 100 tasks in one call, for breaking a piece of work into its tasks. Each item takes \
         the same fields as task_create plus an optional `key`; a later item can refer to an earlier one \
         as `@key` in parent_id, depends_on or a task link. `defaults` fills fields the items leave out \
         (initiative, space, tags, assignees). Give `external_prefix` and each item gets an external_id \
         from it, so a retried call creates nothing twice. A bad item fails alone and is reported."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "tasks": { "type": "array", "description": "The tasks to create, in order", "items": { "type": "object" } },
                "defaults": { "type": "object", "description": "Fields each task inherits when it does not set them" },
                "external_prefix": { "type": "string", "description": "Makes the call safe to repeat: item n gets external_id `<prefix>-<key or n>`" },
                "lease": { "type": "string", "description": "Your lease token from task_claim, to attribute these to you" },
                "actor": { "type": "string" }
            },
            "required": ["tasks"]
        })
    }
    fn category(&self) -> &str {
        "tasks"
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let items = items_of(&args, "tasks")?;
        let defaults = args.get("defaults").and_then(Value::as_object).cloned().unwrap_or_default();
        let prefix = args.get("external_prefix").and_then(Value::as_str).filter(|p| !p.trim().is_empty());
        // Taken now, before the items run, so the hint reaches this reply and not one inner call's.
        let hint = onboarding_hint(&args);
        let mut made: HashMap<String, String> = HashMap::new();
        let mut keys_seen: HashSet<String> = HashSet::new();
        let mut results = Vec::new();
        for (index, item) in items.iter().enumerate() {
            let Some(item) = item.as_object() else {
                results.push(json!({ "index": index, "ok": false, "error": "each task must be an object" }));
                continue;
            };
            let mut spec = prepare(&args, &defaults, item);
            let key = spec.remove("key").and_then(|k| k.as_str().map(str::to_string));
            if let Some(key) = &key
                && !keys_seen.insert(key.clone())
            {
                results.push(json!({ "index": index, "key": key, "ok": false, "error": format!("key '{key}' is used twice in this call") }));
                continue;
            }
            if let (Some(prefix), false) = (prefix, spec.contains_key("external_id")) {
                spec.insert("external_id".into(), json!(format!("{prefix}-{}", key.clone().unwrap_or_else(|| index.to_string()))));
            }
            let wanted_title = spec.get("title").and_then(Value::as_str).unwrap_or_default().to_string();
            let outcome = match resolve_refs(&mut spec, &made) {
                Ok(()) => self.inner.execute(Value::Object(spec)).await,
                Err(e) => Err(e),
            };
            // A repeat of the same call finds the same tasks. A reused prefix finds someone else's: that is
            // an error, not a parent for the items that follow.
            let outcome = outcome.and_then(|task| {
                let repeated = task.get("deduplicated") == Some(&json!(true));
                match task["title"].as_str() {
                    Some(existing) if repeated && existing != wanted_title.trim() => bail!(
                        "external_id belongs to an existing task titled '{existing}'; use a new external_prefix"
                    ),
                    _ => Ok(task),
                }
            });
            match outcome {
                Ok(task) => {
                    if let (Some(key), Some(display)) = (&key, task["display_id"].as_str()) {
                        made.insert(key.clone(), display.to_string());
                    }
                    results.push(json!({
                        "index": index,
                        "key": key,
                        "ok": true,
                        "id": task["id"],
                        "display_id": task["display_id"],
                        "title": task["title"],
                        "deduplicated": task.get("deduplicated").cloned().unwrap_or(json!(false)),
                    }));
                }
                Err(e) => results.push(json!({ "index": index, "key": key, "ok": false, "error": e.to_string() })),
            }
        }
        let failed = results.iter().filter(|r| r["ok"] == json!(false)).count();
        let mut out = json!({ "created": results.len() - failed, "failed": failed, "results": results });
        if let (Some(hint), Some(obj)) = (hint, out.as_object_mut()) {
            obj.insert("hq_task_protocol".to_string(), hint);
        }
        Ok(out)
    }
}

pub(super) struct TaskUpdateManyTool {
    pub(super) inner: TaskUpdateTool,
}

#[async_trait]
impl HqTool for TaskUpdateManyTool {
    fn name(&self) -> &str {
        "task_update_many"
    }
    fn description(&self) -> &str {
        "Update up to 100 tasks in one call. Each item is a task_update: an `id` and the fields to \
         change. `defaults` fills fields the items leave out. A bad item fails alone and is reported."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "updates": { "type": "array", "description": "Each item has an `id` and the fields to change", "items": { "type": "object" } },
                "defaults": { "type": "object", "description": "Fields each update inherits when it does not set them" },
                "lease": { "type": "string" },
                "actor": { "type": "string" }
            },
            "required": ["updates"]
        })
    }
    fn category(&self) -> &str {
        "tasks"
    }
    async fn execute(&self, args: Value) -> Result<Value> {
        let items = items_of(&args, "updates")?;
        let defaults = args.get("defaults").and_then(Value::as_object).cloned().unwrap_or_default();
        let hint = onboarding_hint(&args);
        let mut results = Vec::new();
        for (index, item) in items.iter().enumerate() {
            let Some(item) = item.as_object() else {
                results.push(json!({ "index": index, "ok": false, "error": "each update must be an object" }));
                continue;
            };
            let spec = prepare(&args, &defaults, item);
            let id = spec.get("id").cloned().unwrap_or(Value::Null);
            match self.inner.execute(Value::Object(spec)).await {
                Ok(task) => results.push(json!({
                    "index": index,
                    "ok": true,
                    "id": id,
                    "status": task["status"],
                    "warnings": task.get("warnings").cloned().unwrap_or(json!([])),
                })),
                Err(e) => results.push(json!({ "index": index, "id": id, "ok": false, "error": e.to_string() })),
            }
        }
        let failed = results.iter().filter(|r| r["ok"] == json!(false)).count();
        let mut out = json!({ "updated": results.len() - failed, "failed": failed, "results": results });
        if let (Some(hint), Some(obj)) = (hint, out.as_object_mut()) {
            obj.insert("hq_task_protocol".to_string(), hint);
        }
        Ok(out)
    }
}
