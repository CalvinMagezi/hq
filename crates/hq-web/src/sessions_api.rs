//! REST for the harness sessions a web chat watches: list them with their
//! task, turn driving on or off, stop watching. The rows come from the
//! registry as of the supervisor's last sweep, so no host is asked per request.

use axum::{
    Json,
    extract::{Path as AxumPath, Query, State},
    response::{IntoResponse, Response},
};
use hq_db::harness_sessions_registry::{self as registry, HarnessSessionRow};
use hq_tools::harness_session::{self as harness, ListFilter, Liveness, NewWatch};
use hq_tools::herdr::{HerdrError, INVALID_KEYS_CODE, validate_keys};
use rusqlite::{Connection, TransactionBehavior};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

use crate::WsState;
use crate::error::ApiError;
use crate::session_driver::broadcast_sync;

fn session_json(conn: &Connection, row: &HarnessSessionRow) -> anyhow::Result<Value> {
    let task = match row.mission_id.as_deref() {
        Some(id) => hq_db::tasks::get_task(conn, id)?,
        None => None,
    };
    Ok(json!({
        "id": row.id,
        "harness": row.harness,
        "label": row.label,
        "host": row.host,
        "agent_name": row.agent_name,
        "owner_thread": row.owner_thread,
        "cwd": row.cwd,
        "status": row.status,
        "agent_status": row.last_agent_status,
        "last_seen_at": row.last_seen_at,
        "drive": row.drive,
        "mode": if row.drive { "drive" } else { "observe" },
        "goal": row.goal,
        "done_criteria": row.done_criteria,
        "drive_blocked_by": registry::goal_gaps(row),
        "drive_off_reason": if row.drive { None } else { row.drive_off_reason.as_deref() },
        "nudges_sent": row.nudges_sent,
        "pending_wake": row.pm_wake,
        "last_driven_at": row.last_driven_at,
        "created_at": row.created_at,
        "task": task.map(|t| json!({
            "id": t.id, "display_id": t.display_id, "title": t.title, "status": t.status,
        })),
    }))
}

pub(crate) async fn list_thread_sessions_handler(
    State(state): State<Arc<WsState>>,
    AxumPath(thread_id): AxumPath<String>,
) -> Response {
    let result = state.db.with_conn(|c| {
        registry::list_for_thread(c, &thread_id)?
            .iter()
            .map(|row| session_json(c, row))
            .collect::<anyhow::Result<Vec<_>>>()
    });
    match result {
        Ok(sessions) => Json(json!({ "sessions": sessions })).into_response(),
        Err(e) => ApiError::from(e).into_response(),
    }
}

#[derive(Deserialize)]
pub(crate) struct DriveBody {
    drive: bool,
}

pub(crate) async fn set_drive_handler(
    State(state): State<Arc<WsState>>,
    AxumPath(id): AxumPath<String>,
    Json(body): Json<DriveBody>,
) -> Response {
    let result = state.db.with_conn(|c| {
        match registry::request_drive(c, &id, body.drive, registry::ACTOR_USER)? {
            registry::DriveChange::NotWatched => {
                return Err(
                    ApiError::NotFound(format!("no watched harness session '{id}'")).into(),
                );
            }
            registry::DriveChange::Refused(gaps) => {
                return Err(ApiError::Conflict(format!(
                    "Drive stays off, HQ only observes: {}",
                    gaps.join("; ")
                ))
                .into());
            }
            registry::DriveChange::Changed(_) => {}
        }
        let row = registry::get(c, &id)?
            .ok_or_else(|| ApiError::NotFound(format!("no harness session '{id}'")))?;
        Ok((session_json(c, &row)?, row.owner_thread))
    });
    match result {
        Ok((session, owner)) => {
            if let Some(thread) = owner {
                broadcast_sync(&state, &thread);
            }
            Json(json!({ "session": session })).into_response()
        }
        Err(e) => ApiError::from(e).into_response(),
    }
}

#[derive(Deserialize)]
pub(crate) struct GoalBody {
    goal: Option<String>,
    done_criteria: Option<String>,
}

/// Record a session's goal and definition of done. A change that no longer
/// passes the drive gate switches a driven session to observing.
pub(crate) async fn set_goal_handler(
    State(state): State<Arc<WsState>>,
    AxumPath(id): AxumPath<String>,
    Json(body): Json<GoalBody>,
) -> Response {
    let result = state.db.with_conn(|c| {
        let update = registry::set_goal(
            c,
            &id,
            body.goal.as_deref(),
            body.done_criteria.as_deref(),
            registry::ACTOR_USER,
        )?
        .ok_or_else(|| ApiError::NotFound(format!("no harness session '{id}'")))?;
        let row = registry::get(c, &id)?
            .ok_or_else(|| ApiError::NotFound(format!("no harness session '{id}'")))?;
        Ok((
            session_json(c, &row)?,
            row.owner_thread,
            update.drive_stopped,
        ))
    });
    match result {
        Ok((session, owner, drive_stopped)) => {
            if let Some(thread) = owner {
                broadcast_sync(&state, &thread);
            }
            Json(json!({ "session": session, "drive_stopped": drive_stopped })).into_response()
        }
        Err(e) => ApiError::from(e).into_response(),
    }
}

pub(crate) async fn unwatch_handler(
    State(state): State<Arc<WsState>>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let result = state.db.with_conn(|c| {
        let row = registry::get(c, &id)?
            .ok_or_else(|| ApiError::NotFound(format!("no harness session '{id}'")))?;
        if row.drive {
            registry::request_drive(c, &id, false, registry::ACTOR_USER)?;
        }
        registry::set_owner(c, &id, None)?;
        Ok(row.owner_thread)
    });
    match result {
        Ok(owner) => {
            // The row no longer names its old chat, so tell that chat directly.
            if let Some(thread) = owner {
                broadcast_sync(&state, &thread);
            }
            Json(json!({ "ok": true })).into_response()
        }
        Err(e) => ApiError::from(e).into_response(),
    }
}

/// Most trailing lines of pane text one request returns.
const MAX_SCREEN_LINES: usize = 500;
const DEFAULT_SCREEN_LINES: usize = 80;
/// Longest prompt the send endpoint accepts.
const MAX_SEND_CHARS: usize = 20_000;
const MAX_SEND_KEYS: usize = 20;

/// `session_json` plus the live fields (`alive`, `reachable`, `agent_status`...).
fn live_session_json(
    conn: &Connection,
    row: &HarnessSessionRow,
    live: Option<&Liveness>,
) -> anyhow::Result<Value> {
    let mut v = session_json(conn, row)?;
    if let (Some(base), Value::Object(extra)) = (v.as_object_mut(), harness::live_fields(row, live))
    {
        base.extend(extra);
    }
    Ok(v)
}

const GENERIC_FAILURE: &str = "the session request failed; the server log has the details";
const HOST_UNREACHABLE: &str = "the session's host is unreachable right now";

/// Turn a harness_session error into the status a client can act on. The 404
/// and 409 cases match messages HQ itself writes (there is no typed error for
/// them yet); anything else is logged and answered generically, because the
/// text can carry ssh or herdr stderr.
fn session_error(e: anyhow::Error) -> ApiError {
    if let Some(api) = e.downcast_ref::<ApiError>() {
        return api.clone();
    }
    match e.downcast_ref::<HerdrError>() {
        Some(HerdrError::Api { code, message }) if code == INVALID_KEYS_CODE => {
            return ApiError::bad_request(message.clone());
        }
        Some(err) if err.is_unreachable() => {
            tracing::warn!(error = %e, "sessions api: host unreachable");
            return ApiError::Unavailable(HOST_UNREACHABLE.into());
        }
        _ => {}
    }
    let msg = e.to_string();
    if msg.contains("no harness session") || msg.starts_with("no task") {
        ApiError::NotFound(msg)
    } else if msg.contains("waiting at a dialog") || msg.contains("is not running") {
        ApiError::Conflict(msg)
    } else {
        tracing::warn!(error = %e, "sessions api: request failed");
        if msg.contains("unreachable") {
            ApiError::Unavailable(HOST_UNREACHABLE.into())
        } else {
            ApiError::Internal(GENERIC_FAILURE.into())
        }
    }
}

fn require_row(state: &WsState, id: &str) -> Result<HarnessSessionRow, ApiError> {
    state
        .db
        .with_conn(|c| registry::get(c, id))
        .map_err(ApiError::from)?
        .ok_or_else(|| ApiError::NotFound(format!("no harness session '{id}'")))
}

#[derive(Deserialize)]
pub(crate) struct ListQuery {
    task_id: Option<String>,
    status: Option<String>,
    host: Option<String>,
}

fn non_empty(v: Option<String>) -> Option<String> {
    v.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// Every session in the registry, with live status and host reachability.
pub(crate) async fn list_all_handler(
    State(state): State<Arc<WsState>>,
    Query(q): Query<ListQuery>,
) -> Response {
    let filter = ListFilter {
        status: non_empty(q.status),
        task: non_empty(q.task_id),
        host: non_empty(q.host),
    };
    let db = Arc::new(state.db.clone());
    let result = tokio::task::spawn_blocking(move || {
        let listed = harness::list_live(&db, &filter)?;
        db.with_conn(|c| {
            listed
                .iter()
                .map(|(row, live)| live_session_json(c, row, live.as_ref()))
                .collect::<anyhow::Result<Vec<_>>>()
        })
    })
    .await;
    match result {
        Ok(Ok(sessions)) => Json(json!({ "sessions": sessions })).into_response(),
        Ok(Err(e)) => session_error(e).into_response(),
        Err(e) => ApiError::internal(e).into_response(),
    }
}

pub(crate) async fn get_session_handler(
    State(state): State<Arc<WsState>>,
    AxumPath(id): AxumPath<String>,
) -> Response {
    let row = match require_row(&state, &id) {
        Ok(row) => row,
        Err(e) => return e.into_response(),
    };
    let db = Arc::new(state.db.clone());
    let result = tokio::task::spawn_blocking(move || {
        let live = (row.status == registry::STATUS_RUNNING)
            .then(|| harness::liveness(&harness::poll_hosts(std::slice::from_ref(&row)), &row));
        db.with_conn(|c| live_session_json(c, &row, live.as_ref()))
    })
    .await;
    match result {
        Ok(Ok(session)) => Json(json!({ "session": session })).into_response(),
        Ok(Err(e)) => session_error(e).into_response(),
        Err(e) => ApiError::internal(e).into_response(),
    }
}

#[derive(Deserialize)]
pub(crate) struct ScreenQuery {
    lines: Option<usize>,
}

/// Recent pane text: live while the agent runs, the supervisor's last snapshot after.
pub(crate) async fn screen_handler(
    State(state): State<Arc<WsState>>,
    AxumPath(id): AxumPath<String>,
    Query(q): Query<ScreenQuery>,
) -> Response {
    if let Err(e) = require_row(&state, &id) {
        return e.into_response();
    }
    let lines = q
        .lines
        .unwrap_or(DEFAULT_SCREEN_LINES)
        .clamp(1, MAX_SCREEN_LINES);
    let db = Arc::new(state.db.clone());
    match tokio::task::spawn_blocking(move || harness::tail_log_shared(&db, &id, lines)).await {
        Ok(Ok(screen)) => Json(screen).into_response(),
        Ok(Err(e)) => session_error(e).into_response(),
        Err(e) => ApiError::internal(e).into_response(),
    }
}

#[derive(Deserialize)]
pub(crate) struct SendBody {
    text: Option<String>,
    keys: Option<Vec<String>>,
}

/// Type a prompt or press keys in a running session, as the user. Text is
/// refused (409) while the agent waits at a dialog; answer it with keys.
pub(crate) async fn send_handler(
    State(state): State<Arc<WsState>>,
    AxumPath(id): AxumPath<String>,
    Json(body): Json<SendBody>,
) -> Response {
    let text = body.text.filter(|t| !t.trim().is_empty());
    let keys = body.keys.filter(|k| !k.is_empty());
    if text.is_some() == keys.is_some() {
        return ApiError::bad_request("pass exactly one of `text` or `keys`").into_response();
    }
    if text
        .as_deref()
        .is_some_and(|t| t.chars().count() > MAX_SEND_CHARS)
    {
        return ApiError::bad_request(format!("text is longer than {MAX_SEND_CHARS} characters"))
            .into_response();
    }
    if keys
        .as_deref()
        .is_some_and(|k| k.len() > MAX_SEND_KEYS || k.iter().any(|key| key.trim().is_empty()))
    {
        return ApiError::bad_request(format!("pass at most {MAX_SEND_KEYS} non-empty keys"))
            .into_response();
    }
    if let Some(Err(message)) = keys.as_deref().map(validate_keys) {
        return ApiError::bad_request(message).into_response();
    }
    if let Err(e) = require_row(&state, &id) {
        return e.into_response();
    }
    let db = Arc::new(state.db.clone());
    let result = tokio::task::spawn_blocking(move || {
        let (report, detail) = match (&text, &keys) {
            (Some(t), _) => (
                harness::send(&db, &id, t, None)?,
                format!("text, {} characters", t.chars().count()),
            ),
            (_, Some(k)) => (
                harness::send_keys(&db, &id, k, None)?,
                format!("keys: {}", k.join(" ")),
            ),
            _ => unreachable!("exactly one of text or keys was checked"),
        };
        // The input is already delivered, so a failed audit row must not turn into an error that invites a resend.
        let audited = db.with_conn(|c| {
            registry::record_event(c, &id, registry::EVENT_SENT, registry::ACTOR_USER, Some(&detail))
        });
        if let Err(e) = audited {
            tracing::warn!(session = %id, error = %e, "sessions api: send was delivered but not audited");
        }
        Ok::<_, anyhow::Error>(report)
    })
    .await;
    match result {
        Ok(Ok(report)) => Json(report).into_response(),
        Ok(Err(e)) => session_error(e).into_response(),
        Err(e) => ApiError::internal(e).into_response(),
    }
}

#[derive(Deserialize, Default)]
pub(crate) struct AdoptBody {
    thread_id: Option<String>,
}

/// Put a session under a chat's watch (observe only; Drive stays a separate
/// switch). Without `thread_id` a new chat is created. Repeating the call, or
/// adopting into the chat that already watches it, changes nothing; a session
/// another chat watches is refused (409) until that chat unwatches it.
pub(crate) async fn adopt_handler(
    State(state): State<Arc<WsState>>,
    AxumPath(id): AxumPath<String>,
    body: Option<Json<AdoptBody>>,
) -> Response {
    let wanted = non_empty(body.and_then(|Json(b)| b.thread_id));
    match state.db.with_conn(|c| adopt(c, &id, wanted.as_deref())) {
        Ok(done) => {
            broadcast_sync(&state, &done.thread_id);
            Json(json!({
                "session": done.session,
                "thread_id": done.thread_id,
                "created_thread": done.created_thread,
                "already_watched": done.already_watched,
            }))
            .into_response()
        }
        Err(e) => ApiError::from(e).into_response(),
    }
}

struct Adopted {
    session: Value,
    thread_id: String,
    created_thread: bool,
    already_watched: bool,
}

fn adopt(conn: &Connection, id: &str, wanted: Option<&str>) -> anyhow::Result<Adopted> {
    // IMMEDIATE takes the write lock up front, so two adopts cannot both see an unowned row.
    let tx = rusqlite::Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let row = registry::get(&tx, id)?
        .ok_or_else(|| ApiError::NotFound(format!("no harness session '{id}'")))?;
    // An unknown thread is a 404 whoever owns the session, so it is checked before ownership.
    if let Some(w) = wanted {
        hq_db::chat::get_thread(&tx, w)?
            .ok_or_else(|| ApiError::NotFound(format!("no chat thread '{w}'")))?;
    }
    let (thread_id, created_thread, already_watched) = match (row.owner_thread.as_deref(), wanted) {
        (Some(owner), Some(w)) if owner != w => {
            return Err(ApiError::Conflict(format!(
                "session '{id}' is watched by another chat; unwatch it there first"
            ))
            .into());
        }
        (Some(owner), _) => (owner.to_string(), false, true),
        (None, Some(w)) => (w.to_string(), false, false),
        (None, None) => {
            let name = if row.label.is_empty() {
                &row.id
            } else {
                &row.label
            };
            let thread =
                hq_db::chat::create_thread(&tx, &format!("Session: {name}"), "user", "user")?;
            (thread.thread_id, true, false)
        }
    };
    if !already_watched {
        harness::start_watch(
            &tx,
            id,
            NewWatch {
                thread: &thread_id,
                drive: false,
                opted_out: false,
            },
        )?;
        let detail = format!("adopted into chat {thread_id}");
        registry::record_event(
            &tx,
            id,
            registry::EVENT_ATTACHED,
            registry::ACTOR_USER,
            Some(&detail),
        )?;
    }
    let row = registry::get(&tx, id)?
        .ok_or_else(|| ApiError::NotFound(format!("no harness session '{id}'")))?;
    let session = session_json(&tx, &row)?;
    tx.commit()?;
    Ok(Adopted {
        session,
        thread_id,
        created_thread,
        already_watched,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use hq_db::harness_sessions_registry::{NewSession, Placement};

    fn test_state() -> Arc<WsState> {
        Arc::new(WsState::new(
            std::env::temp_dir().join(format!("hq-sessions-api-test-{}", uuid::Uuid::new_v4())),
            None,
        ))
    }

    fn seed(state: &WsState, id: &str, thread: Option<&str>) {
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
                        placement: Placement {
                            host: "laptop",
                            agent_name: id,
                            workspace_id: "w1",
                            pane_id: "w1:p1",
                        },
                    },
                )?;
                registry::set_owner(c, id, thread)?;
                Ok(())
            })
            .unwrap();
    }

    async fn body(response: Response) -> Value {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn a_guard_stop_shows_its_reason_in_the_sessions_payload_until_drive_is_back_on() {
        let state = test_state();
        let thread = state
            .db
            .with_conn(|c| Ok(hq_db::chat::create_thread(c, "Work", "user", "user")?.thread_id))
            .unwrap();
        seed(&state, "hs-r", Some(&thread));
        state
            .db
            .with_conn(|c| {
                registry::set_goal(
                    c,
                    "hs-r",
                    Some("Add rate limiting to the login endpoint"),
                    Some("Login returns 429 after 5 failed attempts and cargo test passes"),
                    registry::ACTOR_USER,
                )?;
                registry::request_drive(c, "hs-r", true, registry::ACTOR_USER)?;
                registry::stop_drive(
                    c,
                    "hs-r",
                    registry::ACTOR_GUARD,
                    "The nudge budget is used.",
                )
            })
            .unwrap();
        let sessions = |state: &Arc<WsState>| {
            state
                .db
                .with_conn(|c| {
                    registry::list_for_thread(c, &thread)?
                        .iter()
                        .map(|r| session_json(c, r))
                        .collect::<anyhow::Result<Vec<_>>>()
                })
                .unwrap()
        };

        let off = &sessions(&state)[0];
        assert_eq!(off["drive"], false);
        assert_eq!(off["drive_off_reason"], "The nudge budget is used.");

        state
            .db
            .with_conn(|c| registry::request_drive(c, "hs-r", true, registry::ACTOR_USER))
            .unwrap();
        assert!(sessions(&state)[0]["drive_off_reason"].is_null());
    }

    #[tokio::test]
    async fn global_list_filters_and_marks_unrunning_rows() {
        let state = test_state();
        seed(&state, "hs-a", None);
        state
            .db
            .with_conn(|c| registry::set_status(c, "hs-a", registry::STATUS_STOPPED))
            .unwrap();
        let q = |status: Option<&str>, host: Option<&str>, task: Option<&str>| {
            Query(ListQuery {
                task_id: task.map(Into::into),
                status: status.map(Into::into),
                host: host.map(Into::into),
            })
        };

        let all = body(list_all_handler(State(state.clone()), q(None, None, None)).await).await;
        assert_eq!(all["sessions"][0]["id"], "hs-a");
        assert_eq!(all["sessions"][0]["alive"], false);
        let none =
            body(list_all_handler(State(state.clone()), q(Some("running"), None, None)).await)
                .await;
        assert!(none["sessions"].as_array().unwrap().is_empty());
        let other_host =
            body(list_all_handler(State(state.clone()), q(None, Some("desktop"), None)).await)
                .await;
        assert!(other_host["sessions"].as_array().unwrap().is_empty());
        let bad_task = list_all_handler(State(state.clone()), q(None, None, Some("FR-9999"))).await;
        assert_eq!(bad_task.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn screen_serves_the_stored_snapshot_and_404s_unknown_ids() {
        let state = test_state();
        seed(&state, "hs-s", None);
        let missing = screen_handler(
            State(state.clone()),
            AxumPath("nope".into()),
            Query(ScreenQuery { lines: None }),
        )
        .await;
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
        let shown = screen_handler(
            State(state.clone()),
            AxumPath("hs-s".into()),
            Query(ScreenQuery { lines: Some(5) }),
        )
        .await;
        assert_eq!(shown.status(), StatusCode::OK);
        assert_eq!(body(shown).await["source"], "snapshot");
    }

    #[tokio::test]
    async fn send_wants_exactly_one_of_text_or_keys() {
        let state = test_state();
        seed(&state, "hs-x", None);
        let send = |text: Option<&str>, keys: Option<Vec<&str>>| {
            let body = SendBody {
                text: text.map(Into::into),
                keys: keys.map(|k| k.into_iter().map(Into::into).collect()),
            };
            send_handler(State(state.clone()), AxumPath("hs-x".into()), Json(body))
        };
        assert_eq!(send(None, None).await.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            send(Some("  "), None).await.status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            send(Some("hi"), Some(vec!["enter"])).await.status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            send(None, Some(vec![" "])).await.status(),
            StatusCode::BAD_REQUEST
        );
        for bad in ["--help", "-x", "ctrl+c;ls", "$(id)"] {
            assert_eq!(
                send(None, Some(vec![bad])).await.status(),
                StatusCode::BAD_REQUEST,
                "{bad:?} must not reach herdr"
            );
        }
        let long = "x".repeat(MAX_SEND_CHARS + 1);
        assert_eq!(
            send(Some(&long), None).await.status(),
            StatusCode::BAD_REQUEST
        );
        let unknown = send_handler(
            State(state.clone()),
            AxumPath("nope".into()),
            Json(SendBody {
                text: Some("hi".into()),
                keys: None,
            }),
        )
        .await;
        assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn session_errors_map_to_actionable_statuses() {
        let status = |m: &str| session_error(anyhow::anyhow!(m.to_string())).status();
        assert_eq!(status("no harness session 'x'"), StatusCode::NOT_FOUND);
        assert_eq!(
            status("session x is waiting at a dialog and the text was not sent"),
            StatusCode::CONFLICT
        );
        assert_eq!(
            status("session x is not running (no agent 'a' on host 'h')"),
            StatusCode::CONFLICT
        );
        assert_eq!(
            status("host laptop unreachable"),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }

    #[test]
    fn unrecognised_errors_do_not_leak_stderr_to_the_client() {
        let leaked = "herdr api_error: ssh: connect to host 10.0.0.9 port 22: Connection refused (key /home/hq/.ssh/id)";
        let err = session_error(anyhow::anyhow!(leaked.to_string()));
        assert_eq!(err.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(!err.to_string().contains("10.0.0.9"), "{err}");

        let typed = session_error(anyhow::Error::new(HerdrError::Unreachable {
            host: "laptop".into(),
            detail: "ssh: Permission denied (publickey) for hq@100.64.0.2".into(),
        }));
        assert_eq!(typed.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(!typed.to_string().contains("100.64"), "{typed}");
    }

    #[tokio::test]
    async fn adopt_creates_a_chat_once_and_is_idempotent() {
        let state = test_state();
        seed(&state, "hs-free", None);
        seed(&state, "hs-taken", Some("th-other"));
        let adopt = |id: &str, thread: Option<&str>| {
            adopt_handler(
                State(state.clone()),
                AxumPath(id.into()),
                Some(Json(AdoptBody {
                    thread_id: thread.map(Into::into),
                })),
            )
        };

        let first = body(adopt("hs-free", None).await).await;
        assert_eq!(first["created_thread"], true);
        assert_eq!(
            first["session"]["drive"], false,
            "adopting never turns Drive on"
        );
        let thread = first["thread_id"].as_str().unwrap().to_string();

        let again = body(adopt("hs-free", None).await).await;
        assert_eq!(again["thread_id"], thread.as_str());
        assert_eq!(again["created_thread"], false);
        assert_eq!(again["already_watched"], true);
        let same = body(adopt("hs-free", Some(&thread)).await).await;
        assert_eq!(same["already_watched"], true);

        let elsewhere = state
            .db
            .with_conn(|c| hq_db::chat::create_thread(c, "elsewhere", "user", "user"))
            .unwrap();
        assert_eq!(
            adopt("hs-free", Some(&elsewhere.thread_id)).await.status(),
            StatusCode::CONFLICT
        );
        assert_eq!(
            adopt("hs-free", Some("th-unknown")).await.status(),
            StatusCode::NOT_FOUND,
            "an unknown thread is a 404 even though another chat owns the session"
        );
        assert_eq!(
            adopt("hs-taken", None).await.status(),
            StatusCode::OK,
            "no thread named: reports the current owner"
        );
        assert_eq!(
            adopt("hs-taken", Some(&elsewhere.thread_id)).await.status(),
            StatusCode::CONFLICT
        );
        assert_eq!(
            adopt("hs-taken", Some("th-new")).await.status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(adopt("nope", None).await.status(), StatusCode::NOT_FOUND);

        seed(&state, "hs-two", None);
        assert_eq!(
            adopt("hs-two", Some("no-such-thread")).await.status(),
            StatusCode::NOT_FOUND
        );
        let existing = state
            .db
            .with_conn(|c| hq_db::chat::create_thread(c, "mine", "user", "user"))
            .unwrap();
        let joined = body(adopt("hs-two", Some(&existing.thread_id)).await).await;
        assert_eq!(joined["created_thread"], false);
        assert_eq!(joined["session"]["mode"], "observe");

        let events = state
            .db
            .with_conn(|c| registry::list_events(c, "hs-free", 10))
            .unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|e| e.kind == registry::EVENT_ATTACHED)
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn a_chat_lists_drives_and_unwatches_its_sessions() {
        let state = test_state();
        seed(&state, "hs-mine", Some("th-1"));
        seed(&state, "hs-other", Some("th-2"));
        seed(&state, "hs-loose", None);

        let listed =
            body(list_thread_sessions_handler(State(state.clone()), AxumPath("th-1".into())).await)
                .await;
        let sessions = listed["sessions"].as_array().unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0]["id"], "hs-mine");
        assert_eq!(sessions[0]["drive"], false);
        assert!(sessions[0]["task"].is_null());

        let refused = set_drive_handler(
            State(state.clone()),
            AxumPath("hs-mine".into()),
            Json(DriveBody { drive: true }),
        )
        .await;
        assert_eq!(refused.status(), StatusCode::CONFLICT, "no goal, no drive");
        assert!(
            body(refused).await["error"]
                .as_str()
                .unwrap()
                .contains("goal is missing")
        );

        let goal = GoalBody {
            goal: Some("Add rate limiting to the login endpoint".into()),
            done_criteria: Some("Login returns 429 after 5 failed attempts".into()),
        };
        let set =
            set_goal_handler(State(state.clone()), AxumPath("hs-mine".into()), Json(goal)).await;
        assert_eq!(set.status(), StatusCode::OK);

        let driven = set_drive_handler(
            State(state.clone()),
            AxumPath("hs-mine".into()),
            Json(DriveBody { drive: true }),
        )
        .await;
        assert_eq!(driven.status(), StatusCode::OK);
        assert_eq!(body(driven).await["session"]["drive"], true);

        let refused = set_drive_handler(
            State(state.clone()),
            AxumPath("hs-loose".into()),
            Json(DriveBody { drive: true }),
        )
        .await;
        assert_eq!(
            refused.status(),
            StatusCode::NOT_FOUND,
            "only a watched session can be driven"
        );

        let unwatched = unwatch_handler(State(state.clone()), AxumPath("hs-mine".into())).await;
        assert_eq!(unwatched.status(), StatusCode::OK);
        let listed =
            body(list_thread_sessions_handler(State(state.clone()), AxumPath("th-1".into())).await)
                .await;
        assert!(listed["sessions"].as_array().unwrap().is_empty());
    }
}
