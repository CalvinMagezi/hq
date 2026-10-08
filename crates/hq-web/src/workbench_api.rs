//! REST behind the Workbench page: which computers can run agents and where, folder
//! browsing inside each computer's HQ folder, and starting, stopping, resuming,
//! renaming and archiving agents. Folder paths are resolved on the computer that runs
//! the agent, so this server never judges a path by its own operating system.

use axum::{
    Json,
    extract::{Path as AxumPath, Query, State},
    response::{IntoResponse, Response},
};
use hq_db::harness_sessions_registry as registry;
use hq_tools::agent_host::{self, AgentHostError, Host};
use hq_tools::harness_session::{self as harness, GoalText, SpawnRequest};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;

use crate::WsState;
use crate::error::ApiError;

const MAX_LABEL_CHARS: usize = 80;
const MAX_PROMPT_CHARS: usize = 20_000;
const UNKNOWN_HOST: &str = "no such computer";

fn host_by_name(name: &str) -> Result<Host, ApiError> {
    let known = agent_host::all_hosts().map_err(ApiError::internal)?;
    known
        .into_iter()
        .find(|h| h.name() == name)
        .ok_or_else(|| ApiError::NotFound(UNKNOWN_HOST.into()))
}

/// Host calls that can fail on the caller's input (`invalid`) or on the machine.
fn host_error(e: AgentHostError) -> ApiError {
    match &e {
        AgentHostError::Api { code, message } if code == "invalid" => ApiError::BadRequest(message.clone()),
        AgentHostError::Api { code, .. } if code == "unsupported" => {
            ApiError::Conflict("that computer cannot start agents from the web yet; update HQ on it".into())
        }
        err if err.is_unreachable() => ApiError::Unavailable("that computer is unreachable right now".into()),
        _ => {
            tracing::warn!(error = %e, "workbench api: host call failed");
            ApiError::Internal("the computer could not do that; the server log has the details".into())
        }
    }
}

fn join_error(e: tokio::task::JoinError) -> ApiError {
    ApiError::internal(e)
}

fn host_row(h: &Host) -> Value {
    let workspace = h.workspace();
    match (h.version(), workspace) {
        (Ok(version), Ok(workspace)) => {
            json!({ "host": h.name(), "reachable": true, "host_version": version, "workspace": workspace })
        }
        (Ok(version), Err(e)) => {
            json!({ "host": h.name(), "reachable": true, "host_version": version, "workspace": null, "workspace_error": host_error(e).to_string() })
        }
        (Err(e), _) => json!({ "host": h.name(), "reachable": false, "error": host_error(e).to_string() }),
    }
}

/// The computers HQ can start agents on, each with its HQ folder when it has one.
pub(crate) async fn hosts_handler() -> Response {
    let result = tokio::task::spawn_blocking(|| agent_host::all_hosts().map(|hosts| hosts.iter().map(host_row).collect::<Vec<_>>())).await;
    match result {
        Ok(Ok(rows)) => Json(json!({ "hosts": rows, "harnesses": harness::known_harnesses() })).into_response(),
        Ok(Err(e)) => ApiError::internal(e).into_response(),
        Err(e) => ApiError::internal(e).into_response(),
    }
}

#[derive(Deserialize)]
pub(crate) struct DirsQuery {
    path: Option<String>,
}

pub(crate) async fn list_dirs_handler(AxumPath(name): AxumPath<String>, Query(q): Query<DirsQuery>) -> Response {
    let result = tokio::task::spawn_blocking(move || {
        let host = host_by_name(&name)?;
        host.list_dirs(q.path.as_deref()).map_err(host_error)
    })
    .await;
    match result {
        Ok(Ok(listing)) => Json(listing).into_response(),
        Ok(Err(e)) => e.into_response(),
        Err(e) => join_error(e).into_response(),
    }
}

#[derive(Deserialize)]
pub(crate) struct MakeDirBody {
    parent: Option<String>,
    name: String,
}

pub(crate) async fn make_dir_handler(AxumPath(name): AxumPath<String>, Json(body): Json<MakeDirBody>) -> Response {
    let result = tokio::task::spawn_blocking(move || {
        let host = host_by_name(&name)?;
        host.make_dir(body.parent.as_deref(), &body.name).map_err(host_error)
    })
    .await;
    match result {
        Ok(Ok(made)) => Json(made).into_response(),
        Ok(Err(e)) => e.into_response(),
        Err(e) => join_error(e).into_response(),
    }
}

#[derive(Deserialize)]
pub(crate) struct SpawnBody {
    harness: String,
    host: Option<String>,
    /// A folder inside the computer's HQ folder; the HQ folder itself when blank.
    folder: Option<String>,
    /// A new sub-folder to create inside `folder` and run in.
    new_folder: Option<String>,
    prompt: Option<String>,
    label: Option<String>,
}

fn trimmed(v: Option<String>) -> Option<String> {
    v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// Where the agent will run, resolved and confined to the computer's HQ folder by the computer.
fn resolve_folder(host: &Host, folder: Option<&str>, new_folder: Option<&str>) -> Result<PathBuf, ApiError> {
    let resolved = match new_folder {
        Some(name) => host.make_dir(folder, name).map_err(host_error)?,
        None => {
            let listing = host.list_dirs(folder).map_err(host_error)?;
            json!({ "path": listing["path"] })
        }
    };
    resolved["path"]
        .as_str()
        .map(PathBuf::from)
        .ok_or_else(|| ApiError::Internal("the computer did not say which folder it used".into()))
}

/// Start an agent in a folder on a computer. The agent asks before it acts: HQ never
/// launches one in a bypass mode.
pub(crate) async fn spawn_handler(State(state): State<Arc<WsState>>, Json(body): Json<SpawnBody>) -> Response {
    match spawn(&state, body).await {
        Ok(report) => Json(report).into_response(),
        Err(e) => e.into_response(),
    }
}

async fn spawn(state: &Arc<WsState>, body: SpawnBody) -> Result<Value, ApiError> {
    let harness_name = trimmed(Some(body.harness)).ok_or_else(|| ApiError::bad_request("choose which agent to start"))?;
    let prompt = trimmed(body.prompt);
    if prompt.as_deref().is_some_and(|p| p.chars().count() > MAX_PROMPT_CHARS) {
        return Err(ApiError::bad_request(format!("the first message is longer than {MAX_PROMPT_CHARS} characters")));
    }
    let label = trimmed(body.label).unwrap_or_default();
    if label.chars().count() > MAX_LABEL_CHARS {
        return Err(ApiError::bad_request(format!("the name is longer than {MAX_LABEL_CHARS} characters")));
    }
    let host_name = trimmed(body.host);
    let (folder, new_folder) = (trimmed(body.folder), trimmed(body.new_folder));

    let lookup = host_name.clone();
    let cwd = tokio::task::spawn_blocking(move || {
        let host = match lookup.as_deref() {
            Some(name) => host_by_name(name)?,
            None => agent_host::host(None).map_err(ApiError::internal)?,
        };
        let cwd = resolve_folder(&host, folder.as_deref(), new_folder.as_deref())?;
        Ok::<_, ApiError>(cwd)
    })
    .await
    .map_err(join_error)??;

    let db = Arc::new(state.db.clone());
    let report = harness::spawn_with(
        &state.vault_path,
        &db,
        SpawnRequest {
            host: host_name.as_deref(),
            harness: &harness_name,
            prompt: prompt.as_deref(),
            cwd: &cwd,
            label: &label,
            mission_id: None,
            watch: None,
            parent: None,
            goal: GoalText { goal: None, done_criteria: None },
        },
    )
    .await
    .map_err(crate::sessions_api::session_error)?;
    harness::tag_origin(&db, &report, registry::ORIGIN_USER);
    Ok(report)
}

pub(crate) async fn stop_handler(State(state): State<Arc<WsState>>, AxumPath(id): AxumPath<String>) -> Response {
    let db = Arc::new(state.db.clone());
    match tokio::task::spawn_blocking(move || harness::stop(&db, &id)).await {
        Ok(Ok(report)) => Json(report).into_response(),
        Ok(Err(e)) => crate::sessions_api::session_error(e).into_response(),
        Err(e) => ApiError::internal(e).into_response(),
    }
}

#[derive(Deserialize, Default)]
pub(crate) struct ResumeBody {
    prompt: Option<String>,
}

pub(crate) async fn resume_handler(
    State(state): State<Arc<WsState>>,
    AxumPath(id): AxumPath<String>,
    body: Option<Json<ResumeBody>>,
) -> Response {
    let prompt = trimmed(body.and_then(|Json(b)| b.prompt));
    let db = Arc::new(state.db.clone());
    match harness::resume(&state.vault_path, &db, &id, prompt.as_deref()).await {
        Ok(report) => Json(report).into_response(),
        Err(e) => crate::sessions_api::session_error(e).into_response(),
    }
}

#[derive(Deserialize)]
pub(crate) struct RenameBody {
    label: String,
}

pub(crate) async fn rename_handler(
    State(state): State<Arc<WsState>>,
    AxumPath(id): AxumPath<String>,
    Json(body): Json<RenameBody>,
) -> Response {
    let label = body.label.trim().to_string();
    if label.is_empty() || label.chars().count() > MAX_LABEL_CHARS {
        return ApiError::bad_request(format!("a name is 1 to {MAX_LABEL_CHARS} characters")).into_response();
    }
    match state.db.with_conn(|c| registry::set_label(c, &id, &label)) {
        Ok(true) => Json(json!({ "ok": true, "label": label })).into_response(),
        Ok(false) => ApiError::NotFound(format!("no harness session '{id}'")).into_response(),
        Err(e) => ApiError::from(e).into_response(),
    }
}

#[derive(Deserialize)]
pub(crate) struct ArchiveBody {
    archived: bool,
}

pub(crate) async fn archive_handler(
    State(state): State<Arc<WsState>>,
    AxumPath(id): AxumPath<String>,
    Json(body): Json<ArchiveBody>,
) -> Response {
    match state.db.with_conn(|c| registry::set_archived(c, &id, body.archived)) {
        Ok(true) => Json(json!({ "ok": true, "archived": body.archived })).into_response(),
        Ok(false) if state.db.with_conn(|c| registry::get(c, &id)).ok().flatten().is_none() => {
            ApiError::NotFound(format!("no harness session '{id}'")).into_response()
        }
        Ok(false) => ApiError::Conflict("only a session that is not running can be archived".into()).into_response(),
        Err(e) => ApiError::from(e).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use hq_db::harness_sessions_registry::{NewSession, Placement};

    fn test_state() -> Arc<WsState> {
        Arc::new(WsState::new(
            std::env::temp_dir().join(format!("hq-workbench-api-test-{}", uuid::Uuid::new_v4())),
            None,
        ))
    }

    fn seed(state: &WsState, id: &str) {
        state
            .db
            .with_conn(|c| {
                registry::insert(
                    c,
                    &NewSession {
                        id,
                        harness: "claude-code",
                        label: "",
                        cwd: "/repo",
                        mission_id: None,
                        placement: Placement { host: "laptop", agent_name: id, workspace_id: "w1", pane_id: "w1:p1" },
                    },
                )
            })
            .unwrap();
    }

    #[tokio::test]
    async fn renaming_validates_the_name_and_404s_an_unknown_session() {
        let state = test_state();
        seed(&state, "hs-a");
        let rename = |id: &str, label: &str| {
            rename_handler(State(state.clone()), AxumPath(id.into()), Json(RenameBody { label: label.into() }))
        };
        assert_eq!(rename("hs-a", "  ").await.status(), StatusCode::BAD_REQUEST);
        assert_eq!(rename("hs-a", &"x".repeat(MAX_LABEL_CHARS + 1)).await.status(), StatusCode::BAD_REQUEST);
        assert_eq!(rename("hs-none", "Budget").await.status(), StatusCode::NOT_FOUND);
        assert_eq!(rename("hs-a", " Budget ").await.status(), StatusCode::OK);
        let label = state.db.with_conn(|c| Ok(registry::get(c, "hs-a")?.unwrap().label)).unwrap();
        assert_eq!(label, "Budget");
    }

    #[tokio::test]
    async fn archiving_refuses_a_running_session_and_an_unknown_one() {
        let state = test_state();
        seed(&state, "hs-a");
        let archive = |id: &str, archived: bool| {
            archive_handler(State(state.clone()), AxumPath(id.into()), Json(ArchiveBody { archived }))
        };
        assert_eq!(archive("hs-a", true).await.status(), StatusCode::CONFLICT);
        assert_eq!(archive("hs-none", true).await.status(), StatusCode::NOT_FOUND);
        state.db.with_conn(|c| registry::set_status(c, "hs-a", registry::STATUS_STOPPED)).unwrap();
        assert_eq!(archive("hs-a", true).await.status(), StatusCode::OK);
        assert_eq!(archive("hs-a", false).await.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn spawn_asks_which_agent_and_bounds_the_free_text() {
        let state = test_state();
        let body = |harness: &str, prompt: Option<String>, label: Option<String>| SpawnBody {
            harness: harness.into(),
            host: None,
            folder: None,
            new_folder: None,
            prompt,
            label,
        };
        for bad in [
            body("  ", None, None),
            body("claude-code", Some("x".repeat(MAX_PROMPT_CHARS + 1)), None),
            body("claude-code", None, Some("x".repeat(MAX_LABEL_CHARS + 1))),
        ] {
            assert_eq!(spawn_handler(State(state.clone()), Json(bad)).await.status(), StatusCode::BAD_REQUEST);
        }
    }

    #[tokio::test]
    async fn an_unknown_computer_is_a_404_not_a_server_error() {
        let response = list_dirs_handler(AxumPath("no-such-box".into()), Query(DirsQuery { path: None })).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
