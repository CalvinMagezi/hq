//! Native task management REST API.
//!
//! Thin adapter over `hq_db::tasks`, the single shared write surface (the MCP
//! tool layer in `hq-tools::tasks` is the other adapter; both write through
//! the same db functions, so web-originated and agent-originated writes can't
//! drift). The JSON shape, dependency edits and notifications come from
//! `hq_tools::tasks` so both adapters report and notify identically. See
//! docs/plans/native-tasks.

use axum::{
    Json,
    extract::{Path as AxumPath, Query, State},
    response::{IntoResponse, Response},
};
use hq_db::tasks as t;
use hq_tools::tasks::{
    apply_dependency_changes, derive_id_prefix, find_or_create_folder, find_or_create_initiative,
    notify_unblocked, resolve_space_id, slug, task_json, task_json_with_warnings, unblocked_by_transition,
};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use tracing::warn;

use crate::WsState;
use crate::error::ApiError;

/// Lookups hq-web does itself are typed, so their status never depends on
/// message wording. Errors raised inside hq-db stay untyped strings and are
/// classified by `ApiError::from`.
fn not_found(msg: impl Into<String>) -> anyhow::Error {
    ApiError::NotFound(msg.into()).into()
}

fn vanished(id: &str) -> anyhow::Error {
    ApiError::Internal(format!("task {id} vanished after write")).into()
}

/// Mails `notify` (the tags this write introduced) and tells every open web
/// client. An edit that adds no tag mails nobody.
fn notify_and_broadcast(state: &Arc<WsState>, task: &t::Task, event_type: &str, notify: &[String]) {
    // Lite has no agents to route to, and the mailbox folders are hidden from it.
    if !state.profile().is_lite() {
        hq_tools::tasks::notify_tags(&state.vault_path, task, notify);
    }
    state.broadcast(&json!({ "type": event_type, "task": task_json(task) }).to_string());
}

/// Re-broadcasts tasks whose derived fields moved because of another task's
/// write (a parent's sub-task counts, a dependent's `blocked_by`), without
/// re-notifying their tagged agents.
fn broadcast_related(state: &Arc<WsState>, ids: Vec<String>) {
    let related = state.db.with_conn(move |c| {
        let mut tasks = Vec::new();
        for id in ids {
            tasks.extend(t::get_task(c, &id)?);
        }
        Ok::<_, anyhow::Error>(tasks)
    });
    match related {
        Ok(tasks) => {
            for task in tasks {
                state.broadcast(&json!({ "type": "task:updated", "task": task_json(&task) }).to_string());
            }
        }
        Err(e) => warn!(error = %e, "tasks: failed to load related tasks for broadcast"),
    }
}

pub(crate) async fn list_spaces_handler(State(state): State<Arc<WsState>>) -> Response {
    match state.db.with_conn(t::list_spaces) {
        Ok(spaces) => Json(json!({
            "spaces": spaces.iter().map(|s| json!({
                "id": s.id, "name": s.name, "slug": s.slug, "created_at": s.created_at,
            })).collect::<Vec<_>>()
        }))
        .into_response(),
        Err(e) => ApiError::from(e).into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct CreateSpaceBody {
    pub(crate) name: String,
}

pub(crate) async fn create_space_handler(
    State(state): State<Arc<WsState>>,
    Json(body): Json<CreateSpaceBody>,
) -> Response {
    if body.name.trim().is_empty() {
        return ApiError::bad_request("name is required").into_response();
    }
    let id = hq_tools::util::generate_id("sp");
    let space_slug = slug(&body.name);
    match state
        .db
        .with_conn(move |c| t::create_space(c, &id, &body.name, &space_slug))
    {
        Ok(space) => Json(json!({ "id": space.id, "name": space.name, "slug": space.slug })).into_response(),
        Err(e) => ApiError::from(e).into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct ListFoldersParams {
    pub(crate) space_id: Option<String>,
}

pub(crate) async fn list_folders_handler(
    State(state): State<Arc<WsState>>,
    Query(params): Query<ListFoldersParams>,
) -> Response {
    match state.db.with_conn(move |c| t::list_folders(c, params.space_id.as_deref())) {
        Ok(folders) => Json(json!({
            "folders": folders.iter().map(|f| json!({
                "id": f.id, "space_id": f.space_id, "name": f.name, "slug": f.slug,
            })).collect::<Vec<_>>()
        }))
        .into_response(),
        Err(e) => ApiError::from(e).into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct CreateFolderBody {
    pub(crate) space_id: String,
    pub(crate) name: String,
}

pub(crate) async fn create_folder_handler(
    State(state): State<Arc<WsState>>,
    Json(body): Json<CreateFolderBody>,
) -> Response {
    if body.space_id.trim().is_empty() || body.name.trim().is_empty() {
        return ApiError::bad_request("space_id and name are required").into_response();
    }
    let result = state
        .db
        .with_conn(move |c| find_or_create_folder(c, &body.space_id, &body.name));
    match result {
        Ok(folder) => {
            Json(json!({ "id": folder.id, "space_id": folder.space_id, "name": folder.name, "slug": folder.slug }))
                .into_response()
        }
        Err(e) => ApiError::from(e).into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct ListInitiativesParams {
    pub(crate) space_id: Option<String>,
    pub(crate) folder_id: Option<String>,
}

pub(crate) async fn list_initiatives_handler(
    State(state): State<Arc<WsState>>,
    Query(params): Query<ListInitiativesParams>,
) -> Response {
    match state.db.with_conn(move |c| {
        t::list_initiatives(c, params.space_id.as_deref(), params.folder_id.as_deref().map(Some))
    }) {
        Ok(initiatives) => Json(json!({
            "initiatives": initiatives.iter().map(|i| json!({
                "id": i.id, "space_id": i.space_id, "folder_id": i.folder_id, "name": i.name,
                "slug": i.slug, "id_prefix": i.id_prefix,
            })).collect::<Vec<_>>()
        }))
        .into_response(),
        Err(e) => ApiError::from(e).into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct CreateInitiativeBody {
    pub(crate) space_id: String,
    pub(crate) folder: Option<String>,
    pub(crate) name: String,
    pub(crate) id_prefix: Option<String>,
}

pub(crate) async fn create_initiative_handler(
    State(state): State<Arc<WsState>>,
    Json(body): Json<CreateInitiativeBody>,
) -> Response {
    if body.space_id.trim().is_empty() || body.name.trim().is_empty() {
        return ApiError::bad_request("space_id and name are required").into_response();
    }
    let id = hq_tools::util::generate_id("in");
    let initiative_slug = slug(&body.name);
    let result = state.db.with_conn(move |c| {
        let space_id = resolve_space_id(c, &body.space_id)?;
        let space = t::get_space(c, &space_id)?
            .ok_or_else(|| not_found(format!("space '{space_id}' does not exist")))?;
        let folder_id = match &body.folder {
            Some(fname) if !fname.trim().is_empty() => Some(find_or_create_folder(c, &space_id, fname)?.id),
            _ => None,
        };
        let id_prefix = match body.id_prefix {
            Some(p) if !p.trim().is_empty() => p,
            _ => derive_id_prefix(c, &space.slug, &body.name)?,
        };
        t::create_initiative(c, &id, &space.id, folder_id.as_deref(), &body.name, &initiative_slug, &id_prefix)
    });
    match result {
        Ok(initiative) => Json(json!({
            "id": initiative.id, "space_id": initiative.space_id, "folder_id": initiative.folder_id,
            "name": initiative.name, "id_prefix": initiative.id_prefix,
        }))
        .into_response(),
        Err(e) => ApiError::from(e).into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct ListTasksParams {
    pub(crate) space_id: Option<String>,
    pub(crate) initiative_id: Option<String>,
    pub(crate) status: Option<String>,
    pub(crate) tag: Option<String>,
    pub(crate) priority: Option<String>,
    pub(crate) parent_task_id: Option<String>,
    pub(crate) top_level_only: Option<bool>,
    pub(crate) limit: Option<usize>,
    pub(crate) offset: Option<usize>,
}

pub(crate) async fn list_tasks_handler(
    State(state): State<Arc<WsState>>,
    Query(params): Query<ListTasksParams>,
) -> Response {
    let filter = t::TaskFilter {
        space_id: params.space_id,
        initiative_id: params.initiative_id,
        status: params.status,
        tag: params.tag,
        priority: params.priority,
        parent_task_id: match (params.parent_task_id, params.top_level_only) {
            (Some(parent), _) => Some(Some(parent)),
            (None, Some(true)) => Some(None),
            _ => None,
        },
        limit: params.limit,
        offset: params.offset.unwrap_or(0),
    };
    let offset = filter.offset;
    let page = state
        .db
        .with_conn(move |c| Ok((t::list_tasks(c, &filter)?, t::count_tasks(c, &filter)?)));
    match page {
        Ok((tasks, total)) => Json(json!({
            "count": tasks.len(),
            "total": total,
            "offset": offset,
            "has_more": ((offset + tasks.len()) as i64) < total,
            "tasks": tasks.iter().map(task_json).collect::<Vec<_>>()
        }))
        .into_response(),
        Err(e) => ApiError::from(e).into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct CreateTaskBody {
    pub(crate) title: String,
    #[serde(default)]
    pub(crate) description: String,
    pub(crate) initiative_id: Option<String>,
    #[serde(default = "default_space")]
    pub(crate) space_id: String,
    pub(crate) folder: Option<String>,
    #[serde(default = "default_initiative_name")]
    pub(crate) initiative: String,
    pub(crate) priority: Option<String>,
    pub(crate) due_date: Option<String>,
    pub(crate) start_date: Option<String>,
    pub(crate) parent_task_id: Option<String>,
    #[serde(default)]
    pub(crate) depends_on: Vec<String>,
    #[serde(default)]
    pub(crate) tags: Vec<String>,
    #[serde(default = "default_created_by")]
    pub(crate) created_by: String,
    /// Idempotency key, unique per space; a repeat returns the existing task.
    pub(crate) external_id: Option<String>,
}

fn default_space() -> String {
    "personal".to_string()
}
fn default_initiative_name() -> String {
    "Inbox".to_string()
}
fn default_created_by() -> String {
    "unknown".to_string()
}

pub(crate) async fn create_task_handler(
    State(state): State<Arc<WsState>>,
    Json(body): Json<CreateTaskBody>,
) -> Response {
    if body.title.trim().is_empty() {
        return ApiError::bad_request("title is required").into_response();
    }
    let id = hq_tools::util::generate_id("tk");
    let result = state.db.with_conn(move |c| {
        let parent = match &body.parent_task_id {
            Some(pid) => Some(t::get_task(c, pid)?.ok_or_else(|| not_found(format!("parent task {pid} not found")))?),
            None => None,
        };
        let initiative_id = match (&body.initiative_id, &parent) {
            (Some(iid), _) => t::get_initiative(c, iid)?
                .ok_or_else(|| not_found(format!("initiative '{iid}' does not exist")))?
                .id,
            (None, Some(p)) => p.initiative_id.clone(),
            (None, None) => find_or_create_initiative(c, &body.space_id, body.folder.as_deref(), &body.initiative)?.id,
        };
        let (task, created) = t::create_task_dedup(
            c,
            &id,
            &initiative_id,
            &t::NewTask {
                title: &body.title,
                description: &body.description,
                priority: body.priority.as_deref(),
                due_date: body.due_date.as_deref(),
                start_date: body.start_date.as_deref(),
                parent_task_id: body.parent_task_id.as_deref(),
                tags: &body.tags,
                created_by: &body.created_by,
                external_id: body.external_id.as_deref(),
            },
        )?;
        if body.depends_on.is_empty() || !created {
            return Ok((task, created));
        }
        apply_dependency_changes(c, &task.id, &body.depends_on, &[], &body.created_by)?;
        let task = t::get_task(c, &task.id)?.ok_or_else(|| vanished(&task.id))?;
        Ok((task, created))
    });
    match result {
        Ok((task, false)) => {
            let mut out = task_json(&task);
            out["deduplicated"] = true.into();
            Json(out).into_response()
        }
        Ok((task, true)) => {
            notify_and_broadcast(&state, &task, "task:created", &task.tags);
            broadcast_related(&state, task.parent_task_id.iter().cloned().collect());
            Json(task_json(&task)).into_response()
        }
        Err(e) => ApiError::from(e).into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct UpdateTaskBody {
    pub(crate) title: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) status: Option<String>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub(crate) priority: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub(crate) due_date: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub(crate) start_date: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub(crate) parent_task_id: Option<Option<String>>,
    #[serde(default)]
    pub(crate) add_depends_on: Vec<String>,
    #[serde(default)]
    pub(crate) remove_depends_on: Vec<String>,
    pub(crate) tags: Option<Vec<String>>,
    pub(crate) expected_status: Option<String>,
}

/// Distinguishes an absent JSON key (`None`, don't touch) from an explicit
/// `null` (`Some(None)`, clear the field) — the same "field present at all"
/// semantics `hq-tools::tasks` gets for free from raw `serde_json::Value`.
fn deserialize_double_option<'de, D>(deserializer: D) -> Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Some(Option::deserialize(deserializer)?))
}

pub(crate) async fn update_task_handler(
    State(state): State<Arc<WsState>>,
    AxumPath(id): AxumPath<String>,
    Json(body): Json<UpdateTaskBody>,
) -> Response {
    let patch = t::TaskPatch {
        title: body.title,
        description: body.description,
        status: body.status,
        priority: body.priority,
        due_date: body.due_date,
        start_date: body.start_date,
        parent_task_id: body.parent_task_id,
        tags: body.tags,
    };
    let expected_status = body.expected_status;
    let (add_deps, remove_deps) = (body.add_depends_on, body.remove_depends_on);
    // One transaction: the status and the dependency change commit together or
    // not at all, and `previous` is read under the same write lock.
    let result = state.db.with_conn(move |c| {
        t::in_write_tx(c, |c| {
            let previous =
                t::get_task(c, &id)?.ok_or_else(|| not_found("no task found for that id"))?;
            if let Some(expected) = expected_status.as_deref().filter(|e| *e != previous.status) {
                return Err(ApiError::Conflict(format!(
                    "task {id} was not in expected status '{expected}' (claim conflict)"
                ))
                .into());
            }
            let mut task = t::update_task(c, &id, &patch, expected_status.as_deref())?;
            if !add_deps.is_empty() || !remove_deps.is_empty() {
                apply_dependency_changes(c, &task.id, &add_deps, &remove_deps, "web")?;
                task = t::get_task(c, &task.id)?.ok_or_else(|| vanished(&id))?;
            }
            let previous_status = Some(previous.status.as_str());
            let became_ready = task.status == t::STATUS_READY_FOR_REVIEW
                && previous_status != Some(t::STATUS_READY_FOR_REVIEW);
            let unblocked = unblocked_by_transition(c, previous_status, &task)?;
            Ok((task, became_ready, unblocked, previous.parent_task_id, previous.tags))
        })
    });
    match result {
        Ok((task, became_ready_for_review, unblocked, old_parent, previous_tags)) => {
            let added = hq_tools::tasks::added_tags(&previous_tags, &task.tags);
            notify_and_broadcast(&state, &task, "task:updated", &added);
            if !state.profile().is_lite() {
                notify_unblocked(&state.vault_path, &task, &unblocked);
            }
            let mut related: Vec<String> = unblocked.into_iter().map(|u| u.id).collect();
            related.extend(task.parent_task_id.iter().cloned());
            related.extend(old_parent.filter(|p| task.parent_task_id.as_ref() != Some(p)));
            broadcast_related(&state, related);
            if became_ready_for_review {
                hq_tools::tasks::notify_ready_for_review(state.vault_path.clone(), task.clone()).await;
            }
            Json(task_json_with_warnings(&task)).into_response()
        }
        Err(e) => ApiError::from(e).into_response(),
    }
}

/// Lifecycle transitions (entered in_progress / ready_for_review), oldest first.
pub(crate) async fn list_task_events_handler(
    State(state): State<Arc<WsState>>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let result = state.db.with_conn(move |c| {
        let task = t::get_task(c, &id)?.ok_or_else(|| not_found("no task found for that id"))?;
        t::list_task_events(c, &task.id)
    });
    match result {
        Ok(events) => Json(json!({ "events": events })).into_response(),
        Err(e) => ApiError::from(e).into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct DeleteTaskParams {
    #[serde(default)]
    pub(crate) cascade: bool,
}

pub(crate) async fn delete_task_handler(
    State(state): State<Arc<WsState>>,
    AxumPath(id): AxumPath<String>,
    Query(params): Query<DeleteTaskParams>,
) -> Response {
    let target = id.clone();
    let result = state.db.with_conn(move |c| {
        let task = t::get_task(c, &target)?.ok_or_else(|| not_found("no task found for that id"))?;
        let dependents: Vec<String> = t::list_dependents(c, &task.id)?.into_iter().map(|d| d.id).collect();
        let deleted = t::delete_task(c, &task.id, params.cascade)?;
        Ok::<_, anyhow::Error>((task, deleted, dependents))
    });
    let (task, deleted, dependents) = match result {
        Ok(r) => r,
        Err(e) => return ApiError::from(e).into_response(),
    };
    for deleted_id in &deleted {
        let display_id = if *deleted_id == task.id { Some(&task.display_id) } else { None };
        state.broadcast(&json!({ "type": "task:deleted", "id": deleted_id, "display_id": display_id }).to_string());
    }
    let mut related: Vec<String> = dependents.into_iter().filter(|d| !deleted.contains(d)).collect();
    related.extend(task.parent_task_id.iter().cloned());
    broadcast_related(&state, related);
    Json(json!({ "deleted": true, "id": id, "deleted_ids": deleted })).into_response()
}

pub(crate) async fn list_comments_handler(
    State(state): State<Arc<WsState>>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let result = state.db.with_conn(move |c| {
        let task = t::get_task(c, &id)?.ok_or_else(|| not_found("no task found for that id"))?;
        t::list_comments(c, &task.id)
    });
    match result {
        Ok(comments) => Json(json!({
            "comments": comments.iter().map(|c| json!({
                "id": c.id, "author": c.author, "body": c.body, "created_at": c.created_at,
            })).collect::<Vec<_>>()
        }))
        .into_response(),
        Err(e) => ApiError::from(e).into_response(),
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct AddCommentBody {
    pub(crate) body: String,
    #[serde(default = "default_created_by")]
    pub(crate) author: String,
}

pub(crate) async fn add_comment_handler(
    State(state): State<Arc<WsState>>,
    AxumPath(id): AxumPath<String>,
    Json(payload): Json<AddCommentBody>,
) -> Response {
    if payload.body.trim().is_empty() {
        return ApiError::bad_request("body is required").into_response();
    }
    let result = state.db.with_conn(move |c| {
        let task = t::get_task(c, &id)?.ok_or_else(|| not_found("no task found for that id"))?;
        let comment = t::add_comment(c, &task.id, &payload.author, &payload.body, None)?;
        Ok((task, comment))
    });
    match result {
        Ok((task, comment)) => {
            state.broadcast(
                &json!({
                    "type": "task:comment_added",
                    "task_id": task.id,
                    "display_id": task.display_id,
                    "comment": { "id": comment.id, "author": comment.author, "body": comment.body, "created_at": comment.created_at },
                })
                .to_string(),
            );
            Json(json!({
                "id": comment.id, "author": comment.author, "body": comment.body, "created_at": comment.created_at,
            }))
            .into_response()
        }
        Err(e) => {
            warn!(error = %e, "add_comment_handler failed");
            ApiError::from(e).into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;

    #[test]
    fn a_claim_race_from_hq_db_is_still_a_409() {
        let e = anyhow::anyhow!("task tk-1 was not in expected status 'to_do' (claim conflict)");
        assert_eq!(ApiError::from(e).into_response().status(), StatusCode::CONFLICT);
    }
    

    fn test_state() -> Arc<WsState> {
        Arc::new(WsState::new(std::env::temp_dir().join(format!(
            "hq-tasks-api-test-{}",
            uuid::Uuid::new_v4()
        )), None))
    }

    #[tokio::test]
    async fn creating_twice_with_one_external_id_returns_the_first_task() {
        let state = test_state();
        let create = |title: &str| {
            let body = CreateTaskBody {
                title: title.into(),
                description: "".into(),
                initiative_id: None,
                space_id: "personal".into(),
                folder: None,
                initiative: "Inbox".into(),
                priority: None,
                due_date: None,
                start_date: None,
                parent_task_id: None,
                depends_on: vec![],
                tags: vec![],
                created_by: "test".into(),
                external_id: Some("req-1".into()),
            };
            create_task_handler(State(state.clone()), Json(body))
        };

        let first = create("First").await;
        let second = create("Second").await;

        assert_eq!(second.status(), StatusCode::OK);
        let read = |r: Response| async {
            let bytes = axum::body::to_bytes(r.into_body(), usize::MAX).await.unwrap();
            serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()
        };
        let (first, second) = (read(first).await, read(second).await);
        assert_eq!(second["id"], first["id"]);
        assert_eq!(second["title"], "First");
        assert_eq!(second["deduplicated"], true);
        assert!(first.get("deduplicated").is_none());
    }

    #[tokio::test]
    async fn create_list_update_delete_round_trip() {
        let state = test_state();

        let created = create_task_handler(
            State(state.clone()),
            Json(CreateTaskBody {
                title: "Ship it".into(),
                description: "".into(),
                initiative_id: None,
                space_id: "personal".into(),
                folder: None,
                initiative: "Inbox".into(),
                priority: Some("high".into()),
                due_date: None,
                start_date: None,
                parent_task_id: None,
                depends_on: vec![],
                tags: vec!["hq".into()],
                created_by: "test".into(),
                external_id: None,
            }),
        )
        .await;
        assert_eq!(created.status(), StatusCode::OK);

        let list = list_tasks_handler(
            State(state.clone()),
            Query(ListTasksParams {
                space_id: None,
                initiative_id: None,
                status: None,
                tag: Some("hq".into()),
                priority: None,
                parent_task_id: None,
                top_level_only: None,
                limit: None,
                offset: None,
            }),
        )
        .await;
        assert_eq!(list.status(), StatusCode::OK);
    }

    async fn create_titled(state: &Arc<WsState>, title: &str) -> serde_json::Value {
        let body = serde_json::from_value(json!({ "title": title })).unwrap();
        body_json(create_task_handler(State(state.clone()), Json(body)).await).await
    }

    #[tokio::test]
    async fn the_list_reports_its_total_and_pages_on_offset() {
        let state = test_state();
        for n in 0..3 {
            create_titled(&state, &format!("t{n}")).await;
        }
        let page = |limit, offset| {
            let params = ListTasksParams {
                space_id: None,
                initiative_id: None,
                status: None,
                tag: None,
                priority: None,
                parent_task_id: None,
                top_level_only: None,
                limit: Some(limit),
                offset: Some(offset),
            };
            list_tasks_handler(State(state.clone()), Query(params))
        };
        let first = body_json(page(2, 0).await).await;
        assert_eq!((first["count"].as_i64(), first["total"].as_i64()), (Some(2), Some(3)));
        assert_eq!(first["has_more"], json!(true));
        let rest = body_json(page(2, 2).await).await;
        assert_eq!((rest["count"].as_i64(), rest["has_more"].clone()), (Some(1), json!(false)));
    }

    #[tokio::test]
    async fn an_unknown_status_is_a_400_and_a_failed_dependency_undoes_the_status() {
        let state = test_state();
        let id: String = create_titled(&state, "Work").await["id"].as_str().unwrap().into();
        let patch = |value: serde_json::Value| Json(serde_json::from_value::<UpdateTaskBody>(value).unwrap());

        let refused =
            update_task_handler(State(state.clone()), AxumPath(id.clone()), patch(json!({ "status": "doing" }))).await;
        assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
        assert!(body_json(refused).await["error"].as_str().unwrap().contains("ready_for_review"));

        let failed = update_task_handler(
            State(state.clone()),
            AxumPath(id.clone()),
            patch(json!({ "status": "in_progress", "add_depends_on": ["NOPE-999"] })),
        )
        .await;
        assert_eq!(failed.status(), StatusCode::BAD_REQUEST);
        let task = state.db.with_conn(move |c| t::get_task(c, &id)).unwrap().unwrap();
        assert_eq!(task.status, "to_do", "the status write is undone with the failed dependency");
    }

    fn claim_body() -> UpdateTaskBody {
        serde_json::from_value(json!({ "status": "in_progress", "expected_status": "to_do" })).unwrap()
    }

    async fn body_json(response: Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn subtasks_dependencies_and_cascade_delete() {
        let state = test_state();
        let create = |body: serde_json::Value| {
            let state = state.clone();
            async move {
                let response = create_task_handler(State(state), Json(serde_json::from_value(body).unwrap())).await;
                assert_eq!(response.status(), StatusCode::OK);
                body_json(response).await
            }
        };

        let parent = create(json!({ "title": "Launch" })).await;
        let child = create(json!({ "title": "Copy", "parent_task_id": parent["display_id"] })).await;
        assert_eq!(child["parent_task_id"], parent["id"]);
        let dependent = create(json!({ "title": "Publish", "depends_on": [child["id"]] })).await;
        assert_eq!(dependent["blocked_by"], json!([child["display_id"]]));

        let bad_dates: UpdateTaskBody =
            serde_json::from_value(json!({ "start_date": "2026-10-05", "due_date": "2026-10-01" })).unwrap();
        let response =
            update_task_handler(State(state.clone()), AxumPath(child["id"].as_str().unwrap().into()), Json(bad_dates))
                .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let complete: UpdateTaskBody = serde_json::from_value(json!({ "status": "complete" })).unwrap();
        update_task_handler(State(state.clone()), AxumPath(child["id"].as_str().unwrap().into()), Json(complete)).await;
        let dependent_id: String = dependent["id"].as_str().unwrap().into();
        let refreshed = state.db.with_conn(move |c| t::get_task(c, &dependent_id)).unwrap().unwrap();
        assert_eq!(task_json(&refreshed)["blocked_by"], json!([]));

        let parent_id: String = parent["id"].as_str().unwrap().into();
        let refused = delete_task_handler(
            State(state.clone()),
            AxumPath(parent_id.clone()),
            Query(DeleteTaskParams { cascade: false }),
        )
        .await;
        assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
        let deleted =
            delete_task_handler(State(state.clone()), AxumPath(parent_id), Query(DeleteTaskParams { cascade: true }))
                .await;
        assert_eq!(body_json(deleted).await["deleted_ids"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn claim_safe_update_conflict_returns_409() {
        let state = test_state();
        let id = hq_tools::util::generate_id("tk");
        let task = state
            .db
            .with_conn(move |c| {
                let initiative = find_or_create_initiative(c, "personal", None, "Inbox")?;
                t::create_task(
                    c,
                    &id,
                    &initiative.id,
                    &t::NewTask { title: "Claim me", created_by: "test", ..Default::default() },
                )
            })
            .unwrap();

        let first = update_task_handler(
            State(state.clone()),
            AxumPath(task.id.clone()),
            Json(claim_body()),
        )
        .await;
        assert_eq!(first.status(), StatusCode::OK);

        let second = update_task_handler(
            State(state.clone()),
            AxumPath(task.id.clone()),
            Json(claim_body()),
        )
        .await;
        assert_eq!(second.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn claim_exposes_work_start_and_events() {
        let state = test_state();
        let created = create_task_handler(State(state.clone()), Json(serde_json::from_value(json!({ "title": "x" })).unwrap())).await;
        let task = body_json(created).await;
        assert!(task["work_started_at"].is_null());
        let id: String = task["id"].as_str().unwrap().into();

        let claimed = update_task_handler(State(state.clone()), AxumPath(id.clone()), Json(claim_body())).await;
        let claimed = body_json(claimed).await;
        assert_eq!(claimed["work_started_at"].as_str().map(str::len), Some(19));

        let events = body_json(list_task_events_handler(State(state.clone()), AxumPath(id)).await).await;
        assert_eq!(events["events"][0]["event_type"], "entered_in_progress");
        assert_eq!(events["events"][0]["occurred_at"], claimed["work_started_at"]);
    }

    #[tokio::test]
    async fn missing_id_is_404_and_db_fault_is_500() {
        let state = test_state();
        let missing = || AxumPath("tk_does_not_exist".to_string());
        let update = update_task_handler(State(state.clone()), missing(), Json(claim_body())).await;
        assert_eq!(update.status(), StatusCode::NOT_FOUND);
        let delete =
            delete_task_handler(State(state.clone()), missing(), Query(DeleteTaskParams { cascade: false })).await;
        assert_eq!(delete.status(), StatusCode::NOT_FOUND);
        assert_eq!(list_comments_handler(State(state.clone()), missing()).await.status(), StatusCode::NOT_FOUND);
        assert_eq!(list_task_events_handler(State(state.clone()), missing()).await.status(), StatusCode::NOT_FOUND);
        let comment = AddCommentBody { body: "hi".into(), author: "test".into() };
        let added = add_comment_handler(State(state.clone()), missing(), Json(comment)).await;
        assert_eq!(added.status(), StatusCode::NOT_FOUND);

        let created = create_task_handler(State(state.clone()), Json(serde_json::from_value(json!({ "title": "x" })).unwrap())).await;
        let id = body_json(created).await["id"].as_str().unwrap().to_string();
        state.db.with_conn(|c| Ok(c.execute_batch("DROP TABLE task_comments")?)).unwrap();
        let faulted = list_comments_handler(State(state.clone()), AxumPath(id)).await;
        assert_eq!(faulted.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
