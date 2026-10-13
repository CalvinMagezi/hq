use super::*;
use axum::body::Body;
use axum::http::Request;
use tower::ServiceExt;

const T0: i64 = 1_000_000;
const TOKEN: &str = "s3cret-web-token";
const HOST: &str = "hq.example.ts.net:8443";
const ORIGIN: &str = "https://hq.example.ts.net:8443";

#[test]
fn a_code_works_once_in_any_spelling() {
    let codes = DeviceCodes::default();
    let (code, expires_at) = codes.issue(T0);
    assert_eq!(code.len(), 11, "XXXXX-XXXXX");
    assert_eq!(expires_at, T0 + CODE_TTL_SECS);
    let typed = format!(" {} ", code.to_lowercase().replace('-', " "));
    assert_eq!(codes.redeem(&typed, "a", T0 + 1), Redeemed::Accepted);
    assert_eq!(codes.redeem(&code, "a", T0 + 2), Redeemed::Refused);
}

#[test]
fn a_code_expires() {
    let codes = DeviceCodes::default();
    let (code, expires_at) = codes.issue(T0);
    assert_eq!(codes.redeem(&code, "a", expires_at), Redeemed::Refused);
}

#[test]
fn a_new_code_replaces_the_old_one() {
    let codes = DeviceCodes::default();
    let (old, _) = codes.issue(T0);
    let (new, _) = codes.issue(T0 + 1);
    assert_eq!(codes.redeem(&old, "a", T0 + 2), Redeemed::Refused);
    assert_eq!(codes.redeem(&new, "a", T0 + 3), Redeemed::Accepted);
}

#[test]
fn wrong_codes_limit_the_client_even_with_the_right_code() {
    let codes = DeviceCodes::default();
    let (code, _) = codes.issue(T0);
    for _ in 0..MAX_FAILURES_PER_CLIENT {
        assert_eq!(codes.redeem("AAAAA-AAAAA", "a", T0), Redeemed::Refused);
    }
    assert_eq!(codes.redeem(&code, "a", T0), Redeemed::Limited);
    let later = T0 + FAILURE_WINDOW_SECS + 1;
    let (code, _) = codes.issue(later);
    assert_eq!(
        codes.redeem(&code, "a", later),
        Redeemed::Accepted,
        "the limit lifts once the window passes"
    );
}

#[test]
fn another_client_is_not_held_by_one_clients_limit() {
    let codes = DeviceCodes::default();
    let (code, _) = codes.issue(T0);
    for _ in 0..MAX_FAILURES_PER_CLIENT {
        codes.redeem("wrong", "a", T0);
    }
    assert_eq!(codes.redeem(&code, "b", T0), Redeemed::Accepted);
}

#[test]
fn the_global_cap_burns_the_code_and_holds_everyone() {
    let codes = DeviceCodes::default();
    let (code, _) = codes.issue(T0);
    for i in 0..MAX_FAILURES_GLOBAL {
        assert_eq!(
            codes.redeem("wrong", &format!("c{i}"), T0),
            Redeemed::Refused
        );
    }
    assert_eq!(codes.redeem(&code, "fresh", T0), Redeemed::Limited);
    assert_eq!(
        codes.redeem(&code, "fresh", T0 + FAILURE_WINDOW_SECS + 1),
        Redeemed::Refused,
        "the burned code stays dead after the window"
    );
}

#[test]
fn tracked_clients_are_bounded() {
    let codes = DeviceCodes::default();
    for i in 0..MAX_TRACKED_CLIENTS + 10 {
        let mut ledger = codes.ledger();
        ledger.record_failure(&format!("c{i}"), T0);
        ledger.failures.clear();
    }
    assert!(codes.ledger().failures_by_client.len() <= MAX_TRACKED_CLIENTS);
}

#[test]
fn the_client_is_the_first_forwarded_address() {
    let mut headers = HeaderMap::new();
    assert_eq!(client_key(&headers), UNKNOWN_CLIENT);
    headers.insert("x-real-ip", "198.51.100.2".parse().unwrap());
    assert_eq!(client_key(&headers), "198.51.100.2");
    headers.insert("x-forwarded-for", "203.0.113.9, 10.0.0.1".parse().unwrap());
    assert_eq!(client_key(&headers), "203.0.113.9");
}

fn router(token: Option<&str>) -> (axum::Router, tempfile::TempDir) {
    let vault = tempfile::TempDir::new().unwrap();
    let mut state = WsState::new(vault.path().to_path_buf(), None);
    state.web_auth_token = token.map(str::to_string);
    state.allowed_origins = vec![ORIGIN.to_string()];
    (crate::create_router(Arc::new(state)), vault)
}

fn post(path: &str) -> axum::http::request::Builder {
    Request::post(path)
        .header("host", HOST)
        .header("origin", ORIGIN)
        .header("x-hq-client", "web")
        .header("content-type", "application/json")
}

async fn send(app: &axum::Router, request: Request<Body>) -> (StatusCode, serde_json::Value) {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or_default())
}

async fn issue(app: &axum::Router) -> String {
    let request = post(DEVICE_CODE_PATH)
        .header("authorization", format!("Bearer {TOKEN}"))
        .body(Body::empty())
        .unwrap();
    let (status, body) = send(app, request).await;
    assert_eq!(status, StatusCode::OK);
    body["code"].as_str().unwrap().to_string()
}

fn exchange(code: &str) -> Request<Body> {
    post(EXCHANGE_PATH)
        .body(Body::from(json!({ "code": code }).to_string()))
        .unwrap()
}

#[tokio::test]
async fn a_signed_in_device_issues_and_a_new_one_exchanges_without_the_token() {
    let (app, _vault) = router(Some(TOKEN));
    let unauthed = post(DEVICE_CODE_PATH).body(Body::empty()).unwrap();
    assert_eq!(send(&app, unauthed).await.0, StatusCode::UNAUTHORIZED);

    let code = issue(&app).await;
    let (status, body) = send(&app, exchange(&code)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["token"], TOKEN);

    let (status, body) = send(&app, exchange(&code)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "a code works once");
    assert!(body.get("token").is_none());
}

#[tokio::test]
async fn wrong_codes_get_429_after_the_limit() {
    let (app, _vault) = router(Some(TOKEN));
    let code = issue(&app).await;
    for _ in 0..MAX_FAILURES_PER_CLIENT {
        assert_eq!(
            send(&app, exchange("AAAAA-AAAAA")).await.0,
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        send(&app, exchange(&code)).await.0,
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn the_exchange_refuses_a_hostile_page_and_a_missing_client_header() {
    let (app, _vault) = router(Some(TOKEN));
    let code = issue(&app).await;
    let hostile = Request::post(EXCHANGE_PATH)
        .header("host", HOST)
        .header("origin", "https://evil.example")
        .header("x-hq-client", "web")
        .header("content-type", "application/json")
        .body(Body::from(json!({ "code": code }).to_string()))
        .unwrap();
    assert_eq!(send(&app, hostile).await.0, StatusCode::FORBIDDEN);
    let headerless = Request::post(EXCHANGE_PATH)
        .header("host", HOST)
        .header("origin", ORIGIN)
        .header("content-type", "application/json")
        .body(Body::from(json!({ "code": code }).to_string()))
        .unwrap();
    assert_eq!(send(&app, headerless).await.0, StatusCode::FORBIDDEN);
    assert_eq!(
        send(&app, exchange(&code)).await.0,
        StatusCode::OK,
        "refused requests never spent the code"
    );
}

#[tokio::test]
async fn without_a_web_token_there_is_nothing_to_hand_out() {
    let (app, _vault) = router(None);
    let issue = Request::post(DEVICE_CODE_PATH)
        .header("host", "127.0.0.1:5678")
        .header("x-hq-client", "web")
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(&app, issue).await.0, StatusCode::NOT_FOUND);
    let trade = Request::post(EXCHANGE_PATH)
        .header("host", "127.0.0.1:5678")
        .header("x-hq-client", "web")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"code":"AAAAA-AAAAA"}"#))
        .unwrap();
    assert_eq!(send(&app, trade).await.0, StatusCode::NOT_FOUND);
}
