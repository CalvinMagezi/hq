//! `/api/reminders`: the Reminders screen's view of the same reminders the `remind_*` tools set.
//! Creating and editing go through those tools, so a time means the same thing from a chat and
//! from the screen, and the same limits apply. Done, snooze and cancel move the row directly and
//! take the waiting ping out of the inbox.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use hq_db::reminders::{self, OUTCOME_DISMISSED, STATUS_ACTIVE, STATUS_DONE};
use hq_tools::reminders::schedule::after_done;
use hq_tools::reminders::tools::{
    ReminderOrigin, create_remind_tools, reminder_json, retire_waiting_ping,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::WsState;
use crate::error::ApiError;

const LIST_LIMIT: usize = 200;

#[derive(Deserialize)]
pub(crate) struct ListQuery {
    /// `active` (default), `done` (finished, stopped or lapsed) or `all`.
    status: Option<String>,
}

#[derive(Deserialize, Default)]
pub(crate) struct SnoozeBody {
    #[serde(default)]
    minutes: Option<i64>,
    /// When to ring again, in epoch seconds. Wins over `minutes`.
    #[serde(default)]
    until: Option<i64>,
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Runs one of the reminder tools as the Reminders screen.
async fn run_tool(state: &WsState, name: &str, args: Value) -> Result<Value, ApiError> {
    let config = hq_core::config::HqConfig::load()
        .map(|c| c.reminders)
        .unwrap_or_default();
    let origin = ReminderOrigin {
        ui: true,
        ..ReminderOrigin::default()
    };
    let tools = create_remind_tools(Arc::new(state.db.clone()), Some(origin), &config);
    let tool = tools
        .iter()
        .find(|t| t.name() == name)
        .ok_or_else(ApiError::not_found)?;
    Ok(tool.execute(args).await?)
}

fn load(state: &WsState, id: &str) -> Result<reminders::Reminder, ApiError> {
    reminders::get(&state.db, id)
        .map_err(ApiError::internal)?
        .ok_or_else(ApiError::not_found)
}

/// Tells open apps the inbox changed, since a waiting ping may have left it.
fn inbox_changed(state: &WsState) {
    state.announce_change(crate::CHANGE_INBOX);
}

pub(crate) async fn list_handler(
    State(state): State<Arc<WsState>>,
    Query(q): Query<ListQuery>,
) -> Result<Response, ApiError> {
    let statuses: &[&str] = match q.status.as_deref().unwrap_or("active") {
        "all" => &[],
        "done" => &[STATUS_DONE, "cancelled", "lapsed"],
        "active" => &[STATUS_ACTIVE],
        other => {
            return Err(ApiError::bad_request(format!(
                "status '{other}' is not active, done or all"
            )));
        }
    };
    let at = now();
    let rows = reminders::list(&state.db, statuses, LIST_LIMIT).map_err(ApiError::internal)?;
    let zone = hq_db::owner_prefs::timezone(&state.db);
    Ok(Json(json!({
        "time_zone": zone.map(|z| z.name()),
        "now": at,
        "reminders": rows.iter().map(|r| reminder_json(r, at)).collect::<Vec<_>>(),
    }))
    .into_response())
}

/// `POST /api/reminders`: `title`, then `at` (local time) or `in_minutes`; optional `tz`,
/// `repeat`, `nudge`, `note`, `task_id`.
pub(crate) async fn create_handler(
    State(state): State<Arc<WsState>>,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let made = run_tool(&state, "remind_create", body).await?;
    Ok((StatusCode::CREATED, Json(made)).into_response())
}

/// `PATCH /api/reminders/{id}`: any of `title`, `note`, `at`, `in_minutes`, `tz`, `repeat`, `nudge`.
pub(crate) async fn patch_handler(
    State(state): State<Arc<WsState>>,
    Path(id): Path<String>,
    Json(mut body): Json<Value>,
) -> Result<Response, ApiError> {
    let map = body
        .as_object_mut()
        .ok_or_else(|| ApiError::bad_request("send a JSON object"))?;
    // The owner marks things done and cancels through their own routes, not through an edit.
    map.remove("done");
    map.insert("id".into(), Value::String(id));
    let updated = run_tool(&state, "remind_update", body).await?;
    inbox_changed(&state);
    Ok(Json(updated).into_response())
}

pub(crate) async fn done_handler(
    State(state): State<Arc<WsState>>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let r = load(&state, &id)?;
    let at = now();
    if !reminders::acknowledge(&state.db, &r.id, at, after_done(&r, at))
        .map_err(ApiError::internal)?
    {
        return Err(ApiError::Conflict(format!(
            "that reminder is already {}",
            r.status
        )));
    }
    retire_waiting_ping(&state.db, &r);
    inbox_changed(&state);
    Ok(Json(reminder_json(&load(&state, &r.id)?, at)).into_response())
}

/// `POST /api/reminders/{id}/snooze` with `{ "minutes": 60 }` or `{ "until": <epoch seconds> }`.
pub(crate) async fn snooze_handler(
    State(state): State<Arc<WsState>>,
    Path(id): Path<String>,
    body: Option<Json<SnoozeBody>>,
) -> Result<Response, ApiError> {
    let r = load(&state, &id)?;
    let at = now();
    let body = body.map(|Json(b)| b).unwrap_or_default();
    let until = crate::reminder_actions::snooze_until(body.minutes, body.until, at)?;
    if !reminders::snooze(&state.db, &r.id, until, at).map_err(ApiError::internal)? {
        return Err(ApiError::Conflict(format!(
            "that reminder is already {}",
            r.status
        )));
    }
    retire_waiting_ping(&state.db, &r);
    inbox_changed(&state);
    Ok(Json(reminder_json(&load(&state, &r.id)?, at)).into_response())
}

pub(crate) async fn delete_handler(
    State(state): State<Arc<WsState>>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let r = load(&state, &id)?;
    let at = now();
    if !reminders::cancel(&state.db, &r.id, at, OUTCOME_DISMISSED).map_err(ApiError::internal)? {
        return Err(ApiError::Conflict(format!(
            "that reminder is already {}",
            r.status
        )));
    }
    retire_waiting_ping(&state.db, &r);
    inbox_changed(&state);
    Ok(StatusCode::NO_CONTENT.into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    fn state() -> (tempfile::TempDir, Arc<WsState>) {
        let dir = tempfile::tempdir().unwrap();
        let state = Arc::new(WsState::new(dir.path().to_path_buf(), None));
        hq_db::owner_prefs::set(
            &state.db,
            hq_db::owner_prefs::OWNER_TIMEZONE,
            "Africa/Kampala",
        )
        .unwrap();
        (dir, state)
    }

    async fn json_of(res: Response) -> Value {
        serde_json::from_slice(&to_bytes(res.into_body(), 1 << 20).await.unwrap()).unwrap()
    }

    #[tokio::test]
    async fn the_screens_routes_create_edit_snooze_finish_and_cancel() {
        let (_dir, state) = state();
        let res = create_handler(
            State(state.clone()),
            Json(json!({"title": "Call Moses", "in_minutes": 60, "nudge": true})),
        )
        .await
        .unwrap();
        assert_eq!(res.status(), StatusCode::CREATED);
        let made = json_of(res).await;
        let id = made["id"].as_str().unwrap().to_string();
        assert_eq!(made["source"], "ui");
        assert_eq!(made["time_zone"], "Africa/Kampala");

        let listed = json_of(
            list_handler(State(state.clone()), Query(ListQuery { status: None }))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(listed["reminders"].as_array().unwrap().len(), 1);
        assert_eq!(listed["time_zone"], "Africa/Kampala");

        let edited = json_of(
            patch_handler(
                State(state.clone()),
                Path(id.clone()),
                Json(json!({"title": "Call Moses now", "done": true})),
            )
            .await
            .unwrap(),
        )
        .await;
        assert_eq!(edited["title"], "Call Moses now");
        assert_eq!(
            edited["status"], "active",
            "an edit cannot finish a reminder"
        );

        let snoozed = json_of(
            snooze_handler(
                State(state.clone()),
                Path(id.clone()),
                Some(Json(SnoozeBody {
                    minutes: Some(120),
                    until: None,
                })),
            )
            .await
            .unwrap(),
        )
        .await;
        assert!(snoozed["due_epoch"].as_i64().unwrap() >= now() + 119 * 60);

        let done = json_of(
            done_handler(State(state.clone()), Path(id.clone()))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(done["status"], "done");
        let again = done_handler(State(state.clone()), Path(id.clone()))
            .await
            .unwrap_err();
        assert_eq!(again.status(), StatusCode::CONFLICT);

        let other = json_of(
            create_handler(
                State(state.clone()),
                Json(json!({"title": "x", "in_minutes": 30})),
            )
            .await
            .unwrap(),
        )
        .await;
        let oid = other["id"].as_str().unwrap().to_string();
        let gone = delete_handler(State(state.clone()), Path(oid.clone()))
            .await
            .unwrap();
        assert_eq!(gone.status(), StatusCode::NO_CONTENT);
        let history = json_of(
            list_handler(
                State(state.clone()),
                Query(ListQuery {
                    status: Some("done".into()),
                }),
            )
            .await
            .unwrap(),
        )
        .await;
        assert_eq!(history["reminders"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn bad_input_is_a_400_and_an_unknown_id_is_a_404() {
        let (_dir, state) = state();
        let bad = create_handler(
            State(state.clone()),
            Json(json!({"title": "x", "at": "2020-01-01T09:00"})),
        )
        .await
        .unwrap_err();
        assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
        let no_time = create_handler(State(state.clone()), Json(json!({"title": "x"})))
            .await
            .unwrap_err();
        assert_eq!(no_time.status(), StatusCode::BAD_REQUEST);
        let missing = done_handler(State(state.clone()), Path("nope".into()))
            .await
            .unwrap_err();
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
        let status = list_handler(
            State(state.clone()),
            Query(ListQuery {
                status: Some("soon".into()),
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(status.status(), StatusCode::BAD_REQUEST);
        let past_snooze = {
            let made = json_of(
                create_handler(
                    State(state.clone()),
                    Json(json!({"title": "x", "in_minutes": 30})),
                )
                .await
                .unwrap(),
            )
            .await;
            snooze_handler(
                State(state.clone()),
                Path(made["id"].as_str().unwrap().into()),
                Some(Json(SnoozeBody {
                    minutes: None,
                    until: Some(now() - 10),
                })),
            )
            .await
            .unwrap_err()
        };
        assert_eq!(past_snooze.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn finishing_a_reminder_takes_its_waiting_ping_out_of_the_inbox() {
        let (_dir, state) = state();
        let made = json_of(
            create_handler(
                State(state.clone()),
                Json(json!({"title": "Pay rent", "in_minutes": 1})),
            )
            .await
            .unwrap(),
        )
        .await;
        let id = made["id"].as_str().unwrap().to_string();
        let ring_at = now() + 3600;
        hq_tools::reminders::fire::fire_due(
            &state.db,
            &hq_core::config::RemindersConfig::default(),
            ring_at,
        )
        .unwrap();
        let waiting = |s: &WsState| {
            hq_db::value_items::list_by_state(&s.db, hq_core::types::ValueState::Pending)
                .unwrap()
                .len()
        };
        // The one-shot is already done after ringing, so use a nudge for the waiting case.
        assert_eq!(waiting(&state), 1);
        let _ = id;
        let nudged = json_of(
            create_handler(
                State(state.clone()),
                Json(json!({"title": "Call back", "in_minutes": 1, "nudge": true})),
            )
            .await
            .unwrap(),
        )
        .await;
        hq_tools::reminders::fire::fire_due(
            &state.db,
            &hq_core::config::RemindersConfig::default(),
            ring_at,
        )
        .unwrap();
        assert_eq!(waiting(&state), 2);
        done_handler(
            State(state.clone()),
            Path(nudged["id"].as_str().unwrap().into()),
        )
        .await
        .unwrap();
        assert_eq!(
            waiting(&state),
            1,
            "the nudge's waiting ping left the inbox"
        );
    }
}
