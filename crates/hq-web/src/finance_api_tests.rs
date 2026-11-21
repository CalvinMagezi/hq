use super::*;
use axum::body::Body;
use axum::http::Request;
use hq_tools::device_actions::store::{Opened, Timeouts};
use http_body_util::BodyExt;
use tower::ServiceExt;

const TOKEN: &str = "test-web-token";
const SENT: &str = "You have sent UGX 32,000 to <NAME>, 0700000000 on 2026-03-04 10:11:12, fee: 218. Reason: . New balance: 80,455. ID :47152250553.";
const RECEIVED: &str = "You have received UGX 5,000 from <NAME> on 2026-03-05 09:00:00. fee:0. Reason: . New balance: UGX 85,455. ID: 47152250554.";

fn state_in(vault: &std::path::Path) -> Arc<WsState> {
    let mut state = WsState::new(vault.to_path_buf(), None);
    state.web_auth_token = Some(TOKEN.to_string());
    Arc::new(state)
}

fn call(path: &str, body: &str, signed_in: bool) -> Request<Body> {
    let mut b = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json")
        .header("x-hq-client", "web");
    if signed_in {
        b = b.header("authorization", format!("Bearer {TOKEN}"));
    }
    b.body(Body::from(body.to_string())).unwrap()
}

async fn send(app: &axum::Router, req: Request<Body>) -> (StatusCode, String) {
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

fn open(state: &WsState, name: &'static str) -> String {
    let payload = json!({"request_id": "r", "senders": ["MTNMobMoney"], "max_messages": 2000});
    match state
        .device_actions
        .open_named(
            name,
            "t1",
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

fn upload() -> String {
    json!({"messages": [
        {"sender": "MTNMobMoney", "date": 1_772_600_000_000_i64, "body": SENT},
        {"sender": "MTNMobMoney", "date": 1_772_600_100_000_i64, "body": RECEIVED},
        {"sender": "SOMEBANK", "date": 1_772_600_200_000_i64, "body": SENT},
    ]})
    .to_string()
}

fn path(id: &str) -> String {
    format!("/api/finance/import/{id}")
}

#[tokio::test]
async fn the_route_needs_the_web_sign_in() {
    let vault = tempfile::tempdir().unwrap();
    let state = state_in(vault.path());
    let id = open(&state, IMPORT_ACTION_NAME);
    let app = crate::create_router(state);
    let (status, _) = send(&app, call(&path(&id), &upload(), false)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn an_approved_import_writes_the_ledger_and_tells_the_model_only_counts() {
    let vault = tempfile::tempdir().unwrap();
    let state = state_in(vault.path());
    let id = open(&state, IMPORT_ACTION_NAME);
    state.device_actions.ack(&id).unwrap();
    let app = crate::create_router(state.clone());

    let (status, body) = send(&app, call(&path(&id), &upload(), true)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let ledger = std::fs::read_to_string(vault.path().join("_data/finance/ledger.md")).unwrap();
    assert!(ledger.contains("47152250553"), "the ledger holds the row");

    let outcome = state.device_actions.outcome(&id).unwrap();
    assert_eq!(outcome["outcome"], "imported");
    assert_eq!(
        (
            outcome["seen"].as_u64(),
            outcome["ignored"].as_u64(),
            outcome["added"].as_u64()
        ),
        (Some(3), Some(1), Some(2))
    );
    for text in [body, outcome.to_string()] {
        for secret in ["47152250553", "0700000000", "<NAME>", "32,000", "You have"] {
            assert!(
                !text.contains(secret),
                "{secret} reached the response or the model's result"
            );
        }
    }
}

#[tokio::test]
async fn the_id_works_once_only_after_the_phone_acknowledged_and_only_for_this_action() {
    let vault = tempfile::tempdir().unwrap();
    let state = state_in(vault.path());
    let id = open(&state, IMPORT_ACTION_NAME);
    let other = open(&state, hq_tools::calendar::ACTION_NAME);
    state.device_actions.ack(&other).unwrap();
    let app = crate::create_router(state.clone());

    assert_eq!(
        send(&app, call(&path(&id), &upload(), true)).await.0,
        StatusCode::NOT_FOUND,
        "not acknowledged yet"
    );
    assert_eq!(
        send(&app, call(&path(&other), &upload(), true)).await.0,
        StatusCode::NOT_FOUND,
        "another kind of action"
    );
    assert_eq!(
        send(&app, call(&path("0000"), &upload(), true)).await.0,
        StatusCode::NOT_FOUND
    );

    state.device_actions.ack(&id).unwrap();
    assert_eq!(
        send(&app, call(&path(&id), &upload(), true)).await.0,
        StatusCode::OK
    );
    assert_eq!(
        send(&app, call(&path(&id), &upload(), true)).await.0,
        StatusCode::CONFLICT,
        "single use"
    );
    assert!(!vault.path().join("_data/finance/.ledger.md.tmp").exists());
}

#[tokio::test]
async fn a_phone_cannot_widen_the_senders_it_was_told_to_read() {
    let vault = tempfile::tempdir().unwrap();
    let state = state_in(vault.path());
    let id = open(&state, IMPORT_ACTION_NAME);
    state.device_actions.ack(&id).unwrap();
    let app = crate::create_router(state.clone());
    let only_other = json!({"messages": [
        {"sender": "SOMEBANK", "date": 1_772_600_000_000_i64, "body": SENT},
    ]})
    .to_string();
    assert_eq!(
        send(&app, call(&path(&id), &only_other, true)).await.0,
        StatusCode::OK
    );
    let outcome = state.device_actions.outcome(&id).unwrap();
    assert_eq!(
        (outcome["ignored"].as_u64(), outcome["added"].as_u64()),
        (Some(1), Some(0))
    );
}
