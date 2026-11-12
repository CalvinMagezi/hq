//! The phone's side of a device action: `POST /api/device-actions/{id}/ack`, `.../result` and
//! `GET /api/device-actions/pending`. They sit behind the same sign-in and origin guard as every
//! other `/api` route. The action id in the path is the only handle: it is random, single use,
//! and tied in the store to the chat that made it, so a body never names who is calling.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use chrono::{DateTime, TimeDelta, Utc};
use hq_tools::device_actions::{
    DeviceLink, PhoneClock, Reject, finalize_for, result_is_acceptable_for,
};
use serde_json::{Value, json};

use crate::WsState;

pub(crate) const ACK_PATH: &str = "/api/device-actions/{action_id}/ack";
pub(crate) const RESULT_PATH: &str = "/api/device-actions/{action_id}/result";
pub(crate) const PENDING_PATH: &str = "/api/device-actions/pending";

/// A result body is a few short strings; anything near this size is not one.
const MAX_BODY_BYTES: usize = 16 * 1024;
/// Calls the phone may make per window across all three routes.
const MAX_CALLS_PER_WINDOW: u32 = 60;
const RATE_WINDOW: Duration = Duration::from_secs(60);
/// How far the phone's clock may sit from this server's before the turn counts as having no
/// device: a clock that wrong makes "5 minutes ago" meaningless.
const MAX_CLOCK_SKEW_MINUTES: i64 = 10;

/// What the app puts in a chat frame as `device`.
#[derive(serde::Deserialize, Clone)]
pub(crate) struct DeviceInfo {
    tz: String,
    now_utc: String,
}

/// Fixed-window counter shared by the three routes.
pub(crate) struct RateWindow {
    state: Mutex<(Instant, u32)>,
}

impl Default for RateWindow {
    fn default() -> Self {
        Self {
            state: Mutex::new((Instant::now(), 0)),
        }
    }
}

impl RateWindow {
    fn allow(&self) -> bool {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if s.0.elapsed() >= RATE_WINDOW {
            *s = (Instant::now(), 0);
        }
        s.1 += 1;
        s.1 <= MAX_CALLS_PER_WINDOW
    }
}

pub(crate) fn routes(router: axum::Router<Arc<WsState>>) -> axum::Router<Arc<WsState>> {
    router
        .route(ACK_PATH, post(ack_handler))
        .route(RESULT_PATH, post(result_handler))
        .route(PENDING_PATH, get(pending_handler))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
}

fn refusal(status: StatusCode, reason: &str) -> Response {
    (status, Json(json!({"error": reason}))).into_response()
}

fn rejected(why: Reject) -> Response {
    match why {
        Reject::Unknown => refusal(StatusCode::NOT_FOUND, "unknown_action"),
        Reject::Expired => refusal(StatusCode::GONE, "expired"),
        Reject::Resolved => refusal(StatusCode::CONFLICT, "already_resolved"),
    }
}

fn limited(state: &WsState) -> Option<Response> {
    (!state.device_action_rate.allow()).then(|| refusal(StatusCode::TOO_MANY_REQUESTS, "slow_down"))
}

async fn ack_handler(State(state): State<Arc<WsState>>, Path(action_id): Path<String>) -> Response {
    if let Some(r) = limited(&state) {
        return r;
    }
    match state.device_actions.ack(&action_id) {
        Ok(()) => {
            tracing::info!(%action_id, phase = "acked", "device action");
            Json(json!({"ok": true})).into_response()
        }
        Err(why) => rejected(why),
    }
}

async fn result_handler(
    State(state): State<Arc<WsState>>,
    Path(action_id): Path<String>,
    Json(body): Json<Value>,
) -> Response {
    if let Some(r) = limited(&state) {
        return r;
    }
    let Some(name) = state.device_actions.action_name(&action_id) else {
        return rejected(Reject::Unknown);
    };
    if !result_is_acceptable_for(name, &body) {
        return refusal(StatusCode::BAD_REQUEST, "bad_result");
    }
    let finalize = |body: &Value, payload: &Value| finalize_for(name, body, payload);
    match state.device_actions.settle(&action_id, &body, finalize) {
        Ok(()) => {
            tracing::info!(%action_id, phase = "result", "device action");
            Json(json!({"ok": true})).into_response()
        }
        Err(why) => rejected(why),
    }
}

#[derive(serde::Deserialize)]
struct PendingQuery {
    thread_id: Option<String>,
}

async fn pending_handler(
    State(state): State<Arc<WsState>>,
    Query(q): Query<PendingQuery>,
) -> Response {
    if let Some(r) = limited(&state) {
        return r;
    }
    let actions = state.device_actions.pending(q.thread_id.as_deref());
    Json(json!({"actions": actions})).into_response()
}

/// The link a chat turn gets to the phone that sent it, or `None` when the frame carried no
/// usable `device`: then the device tools are not offered. Logs only why.
pub(crate) fn device_for_turn(
    state: &WsState,
    thread_id: &str,
    device: Option<&DeviceInfo>,
) -> Option<Arc<DeviceLink>> {
    let device = device?;
    let Some(clock) = PhoneClock::parse(&device.tz, &device.now_utc) else {
        tracing::info!(reason = "bad_zone_or_time", "device ignored");
        return None;
    };
    if clock_is_far_off(clock.now(), Utc::now()) {
        tracing::info!(reason = "clock_skew", "device ignored");
        return None;
    }
    let tx = state.tx.clone();
    let emit = Arc::new(move |frame: Value| {
        let _ = tx.send(frame.to_string());
    });
    Some(Arc::new(DeviceLink::new(
        thread_id,
        state.device_actions.clone(),
        emit,
        clock,
    )))
}

fn clock_is_far_off(phone: DateTime<Utc>, server: DateTime<Utc>) -> bool {
    (phone - server).abs() > TimeDelta::minutes(MAX_CLOCK_SKEW_MINUTES)
}

#[cfg(test)]
#[path = "device_actions_api_tests.rs"]
mod device_action_tests;
