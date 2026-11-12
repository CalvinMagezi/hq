use super::*;
use axum::body::Body;
use axum::http::Request;
use hq_tools::calendar::ACTION_NAME;
use hq_tools::device_actions::store::{Opened, Timeouts};
use http_body_util::BodyExt;
use tower::ServiceExt;

const TOKEN: &str = "test-web-token";

fn state_in(vault: &std::path::Path) -> Arc<WsState> {
    let mut state = WsState::new(vault.to_path_buf(), None);
    state.web_auth_token = Some(TOKEN.to_string());
    Arc::new(state)
}

fn call(method: &str, path: &str, body: &str, signed_in: bool) -> Request<Body> {
    let mut b = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json")
        .header("x-hq-client", "web");
    if signed_in {
        b = b.header("authorization", format!("Bearer {TOKEN}"));
    }
    b.body(Body::from(body.to_string())).unwrap()
}

async fn send(app: &axum::Router, req: Request<Body>) -> (StatusCode, Value) {
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn open_action(state: &WsState, thread: &str) -> String {
    let payload = json!({"title": "T", "start_local": "2030-01-01T10:00", "end_local": "2030-01-01T11:00",
        "time_zone": "Europe/London", "all_day": false, "duration_defaulted": true});
    match state
        .device_actions
        .open_named(
            ACTION_NAME,
            thread,
            "call",
            &uuid::Uuid::new_v4().to_string(),
            payload,
            Timeouts::default(),
        )
        .ok()
        .unwrap()
    {
        Opened::New { action_id, .. } => action_id,
        Opened::Known { .. } => unreachable!(),
    }
}

fn created() -> String {
    json!({"v": 1, "outcome": "created", "event_ref": "e1", "title": "T",
        "start_local": "2030-01-01T10:00", "end_local": "2030-01-01T11:00",
        "time_zone": "Europe/London", "calendar_label": "Personal"})
    .to_string()
}

#[tokio::test]
async fn the_routes_need_the_web_sign_in() {
    let vault = tempfile::tempdir().unwrap();
    let state = state_in(vault.path());
    let id = open_action(&state, "t1");
    let app = crate::create_router(state);
    for (method, path) in [
        ("POST", format!("/api/device-actions/{id}/ack")),
        ("POST", format!("/api/device-actions/{id}/result")),
        ("GET", PENDING_PATH.to_string()),
    ] {
        let (status, _) = send(&app, call(method, &path, "{}", false)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{method} {path}");
    }
}

#[tokio::test]
async fn ack_then_result_settles_once_and_a_second_use_is_refused() {
    let vault = tempfile::tempdir().unwrap();
    let state = state_in(vault.path());
    let id = open_action(&state, "t1");
    let app = crate::create_router(state.clone());
    let ack = format!("/api/device-actions/{id}/ack");
    let result = format!("/api/device-actions/{id}/result");

    assert_eq!(
        send(&app, call("POST", &ack, "", true)).await.0,
        StatusCode::OK
    );
    assert_eq!(
        send(&app, call("POST", &ack, "", true)).await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        send(&app, call("POST", &result, &created(), true)).await.0,
        StatusCode::OK
    );
    assert_eq!(
        send(&app, call("POST", &result, &created(), true)).await.0,
        StatusCode::CONFLICT
    );
    let outcome = state.device_actions.outcome(&id).unwrap();
    assert_eq!(outcome["outcome"], "created");
    assert_eq!(outcome["saved_to"], "device_calendar");
}

#[tokio::test]
async fn unknown_ids_and_bad_bodies_are_refused_and_an_identity_in_the_body_changes_nothing() {
    let vault = tempfile::tempdir().unwrap();
    let state = state_in(vault.path());
    let id = open_action(&state, "t1");
    let app = crate::create_router(state.clone());
    let unknown = "/api/device-actions/00000000000000000000000000000000";

    assert_eq!(
        send(&app, call("POST", &format!("{unknown}/ack"), "", true))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        send(
            &app,
            call("POST", &format!("{unknown}/result"), &created(), true)
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );

    let result = format!("/api/device-actions/{id}/result");
    for body in [
        r#"{"v":1,"outcome":"device_unreachable"}"#,
        r#"{"v":2,"outcome":"created"}"#,
        r#"{"outcome":"created"}"#,
    ] {
        assert_eq!(
            send(&app, call("POST", &result, body, true)).await.0,
            StatusCode::BAD_REQUEST,
            "{body}"
        );
    }
    // A thread or session named in the body is not read: the action stays bound to "t1".
    let spoof = created().replacen("{", r#"{"thread_id":"t2","session":"other","#, 1);
    assert_eq!(
        send(&app, call("POST", &result, &spoof, true)).await.0,
        StatusCode::OK
    );
    assert!(state.device_actions.pending(Some("t2")).is_empty());
    assert!(state.device_actions.outcome(&id).is_some());
}

#[tokio::test]
async fn a_result_after_the_window_is_gone() {
    let vault = tempfile::tempdir().unwrap();
    let state = state_in(vault.path());
    let id = match state
        .device_actions
        .open_named(
            ACTION_NAME,
            "t1",
            "c",
            "r",
            json!({}),
            Timeouts {
                ack_secs: 0,
                result_secs: 0,
            },
        )
        .ok()
        .unwrap()
    {
        Opened::New { action_id, .. } => action_id,
        Opened::Known { .. } => unreachable!(),
    };
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let app = crate::create_router(state);
    let (status, _) = send(
        &app,
        call("POST", &format!("/api/device-actions/{id}/ack"), "", true),
    )
    .await;
    assert_eq!(status, StatusCode::GONE);
}

#[tokio::test]
async fn pending_lists_open_actions_for_one_chat_with_their_payload() {
    let vault = tempfile::tempdir().unwrap();
    let state = state_in(vault.path());
    let mine = open_action(&state, "t1");
    open_action(&state, "t2");
    let app = crate::create_router(state);
    let (status, body) = send(
        &app,
        call("GET", "/api/device-actions/pending?thread_id=t1", "", true),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let actions = body["actions"].as_array().unwrap();
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0]["action_id"], mine);
    assert_eq!(actions[0]["payload"]["title"], "T");
    let (_, all) = send(&app, call("GET", PENDING_PATH, "", true)).await;
    assert_eq!(all["actions"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn the_routes_are_rate_limited() {
    let vault = tempfile::tempdir().unwrap();
    let state = state_in(vault.path());
    let app = crate::create_router(state);
    let mut last = StatusCode::OK;
    for _ in 0..=MAX_CALLS_PER_WINDOW {
        last = send(&app, call("GET", PENDING_PATH, "", true)).await.0;
    }
    assert_eq!(last, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn a_turn_without_a_usable_device_is_not_offered_the_tool() {
    let vault = tempfile::tempdir().unwrap();
    let state = state_in(vault.path());
    let now = Utc::now().to_rfc3339();
    let info = |tz: &str, now_utc: &str| DeviceInfo {
        tz: tz.into(),
        now_utc: now_utc.into(),
    };
    assert!(
        device_for_turn(&state, "t1", None).is_none(),
        "an older app sends no device"
    );
    assert!(device_for_turn(&state, "t1", Some(&info("Mars/Base", &now))).is_none());
    assert!(device_for_turn(&state, "t1", Some(&info("Europe/London", "soon"))).is_none());
    let stale = (Utc::now() - TimeDelta::minutes(MAX_CLOCK_SKEW_MINUTES + 1)).to_rfc3339();
    assert!(device_for_turn(&state, "t1", Some(&info("Europe/London", &stale))).is_none());
    let ahead = (Utc::now() + TimeDelta::minutes(MAX_CLOCK_SKEW_MINUTES + 1)).to_rfc3339();
    assert!(device_for_turn(&state, "t1", Some(&info("Europe/London", &ahead))).is_none());
    assert!(device_for_turn(&state, "t1", Some(&info("Europe/London", &now))).is_some());
}
