use super::*;
use axum::body::Body;
use axum::http::Request;
use tower::ServiceExt;

const T0: i64 = 1_000_000;
const TOKEN: &str = "s3cret-web-token";
const HOST: &str = "hq.example.ts.net:8443";
const ORIGIN: &str = "https://hq.example.ts.net:8443";

/// Held by every test that reaches the handlers: a log line first hit on another thread can be
/// cached as disabled for the capturing test's thread-local subscriber.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

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
    let _serial = SERIAL.lock().await;
    let (app, _vault) = router(Some(TOKEN));
    let unauthed = post(DEVICE_CODE_PATH).body(Body::empty()).unwrap();
    assert_eq!(send(&app, unauthed).await.0, StatusCode::UNAUTHORIZED);

    let code = issue(&app).await;
    let (status, body) = send(&app, exchange(&code)).await;
    assert_eq!(status, StatusCode::OK);
    let device = body["token"].as_str().unwrap();
    assert_ne!(
        device, TOKEN,
        "the exchange hands out a device token, never the shared one"
    );
    assert!(device.starts_with(hq_db::web_auth::DEVICE_TOKEN_PREFIX));
    assert_eq!(list_devices(&app, device).await.0, StatusCode::OK);

    let (status, body) = send(&app, exchange(&code)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "a code works once");
    assert!(body.get("token").is_none());
}

#[tokio::test]
async fn wrong_codes_get_429_after_the_limit() {
    let _serial = SERIAL.lock().await;
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
    let _serial = SERIAL.lock().await;
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
    let _serial = SERIAL.lock().await;
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

fn router_with_state(token: Option<&str>) -> (axum::Router, Arc<WsState>, tempfile::TempDir) {
    let vault = tempfile::TempDir::new().unwrap();
    let mut state = WsState::new(vault.path().to_path_buf(), None);
    state.web_auth_token = token.map(str::to_string);
    state.allowed_origins = vec![ORIGIN.to_string()];
    let state = Arc::new(state);
    (crate::create_router(state.clone()), state, vault)
}

fn authed(request: axum::http::request::Builder, bearer: &str) -> axum::http::request::Builder {
    request.header("authorization", format!("Bearer {bearer}"))
}

async fn list_devices(app: &axum::Router, bearer: &str) -> (StatusCode, serde_json::Value) {
    let request = authed(Request::get(devices::DEVICES_PATH), bearer)
        .header("host", HOST)
        .header("origin", ORIGIN)
        .body(Body::empty())
        .unwrap();
    send(app, request).await
}

async fn rotate(app: &axum::Router, bearer: &str) -> (StatusCode, serde_json::Value) {
    let request = authed(post(devices::ROTATE_PATH), bearer)
        .body(Body::empty())
        .unwrap();
    send(app, request).await
}

async fn sign_out(app: &axum::Router, bearer: &str, id: &str) -> StatusCode {
    let request = authed(Request::delete(format!("/api/auth/devices/{id}")), bearer)
        .header("host", HOST)
        .header("origin", ORIGIN)
        .header("x-hq-client", "web")
        .body(Body::empty())
        .unwrap();
    send(app, request).await.0
}

async fn sign_in_with_code(app: &axum::Router) -> String {
    let code = issue(app).await;
    let (status, body) = send(app, exchange(&code)).await;
    assert_eq!(status, StatusCode::OK);
    body["token"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn a_signed_out_device_is_refused_and_the_others_keep_working() {
    let _serial = SERIAL.lock().await;
    let (app, _vault) = router(Some(TOKEN));
    let phone = sign_in_with_code(&app).await;
    let laptop = sign_in_with_code(&app).await;
    let (status, body) = list_devices(&app, &laptop).await;
    assert_eq!(status, StatusCode::OK);
    let devices = body["devices"].as_array().unwrap();
    assert_eq!(devices.len(), 2);
    assert!(devices.iter().all(|d| d["label"] == CODE_SIGN_IN_LABEL));
    let current: Vec<_> = devices.iter().filter(|d| d["current"] == true).collect();
    assert_eq!(current.len(), 1, "the caller's own row is marked");
    let phone_id = devices.iter().find(|d| d["current"] == false).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    assert_eq!(
        sign_out(&app, &laptop, &phone_id).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(list_devices(&app, &phone).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(list_devices(&app, &laptop).await.0, StatusCode::OK);
    assert_eq!(list_devices(&app, TOKEN).await.0, StatusCode::OK);
    assert_eq!(
        sign_out(&app, &laptop, &phone_id).await,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn the_configured_token_works_until_signed_out_everywhere() {
    let _serial = SERIAL.lock().await;
    let (app, state, _vault) = router_with_state(Some(TOKEN));
    let (status, body) = list_devices(&app, TOKEN).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["operator_link_active"], true);
    assert_eq!(body["this_device_uses_operator_link"], true);
    let phone = sign_in_with_code(&app).await;
    let pending = issue(&app).await;

    let (status, body) = rotate(&app, TOKEN).await;
    assert_eq!(status, StatusCode::OK);
    let kept = body["token"].as_str().unwrap().to_string();
    assert_ne!(kept, TOKEN);

    assert_eq!(list_devices(&app, TOKEN).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(list_devices(&app, &phone).await.0, StatusCode::UNAUTHORIZED);
    let (status, body) = list_devices(&app, &kept).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["operator_link_active"], false);
    assert_eq!(body["devices"].as_array().unwrap().len(), 1);
    assert_eq!(body["devices"][0]["label"], devices::ROTATE_LABEL);
    assert_eq!(
        send(&app, exchange(&pending)).await.0,
        StatusCode::UNAUTHORIZED,
        "a code issued before the rotation is void"
    );
    let ws_ticket = authed(post("/api/ws-ticket"), TOKEN)
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(&app, ws_ticket).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(
        *state.auth_epoch.borrow(),
        1,
        "open sockets are told to close"
    );
}

#[tokio::test]
async fn an_operator_who_sets_a_new_configured_token_turns_it_back_on() {
    let _serial = SERIAL.lock().await;
    let (app, state, _vault) = router_with_state(Some(TOKEN));
    rotate(&app, TOKEN).await;
    assert_eq!(list_devices(&app, TOKEN).await.0, StatusCode::UNAUTHORIZED);
    let mut fresh = WsState::new(state.vault_path.clone(), None);
    fresh.web_auth_token = Some("a-new-operator-token".into());
    fresh.allowed_origins = vec![ORIGIN.to_string()];
    let app = crate::create_router(Arc::new(fresh));
    assert_eq!(
        list_devices(&app, "a-new-operator-token").await.0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn signing_every_device_out_never_opens_the_api() {
    let _serial = SERIAL.lock().await;
    let (app, _vault) = router(Some(TOKEN));
    let (_, body) = rotate(&app, TOKEN).await;
    let kept = body["token"].as_str().unwrap().to_string();
    let (_, body) = list_devices(&app, &kept).await;
    let id = body["devices"][0]["id"].as_str().unwrap().to_string();
    assert_eq!(sign_out(&app, &kept, &id).await, StatusCode::NO_CONTENT);
    for bearer in ["", TOKEN, kept.as_str()] {
        assert_eq!(list_devices(&app, bearer).await.0, StatusCode::UNAUTHORIZED);
    }
    let anonymous = Request::get("/api/settings")
        .header("host", HOST)
        .header("origin", ORIGIN)
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(&app, anonymous).await.0, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn tokens_never_reach_the_logs_the_inbox_or_the_device_list() {
    let _serial = SERIAL.lock().await;
    let (logs, _guard) = hq_core::test_util::capture_logs();
    let (app, state, _vault) = router_with_state(Some(TOKEN));
    let phone = sign_in_with_code(&app).await;
    let (_, body) = rotate(&app, &phone).await;
    let kept = body["token"].as_str().unwrap().to_string();
    let (_, listed) = list_devices(&app, &kept).await;
    let listed = listed.to_string();
    let since = chrono::Utc::now() - chrono::Duration::hours(1);
    let inbox = hq_db::value_items::list_unpushed(&state.db, &since)
        .unwrap()
        .iter()
        .map(|i| format!("{} {}", i.title, i.body))
        .collect::<Vec<_>>()
        .join("\n");
    let out = logs.contents();
    assert!(out.contains("signed out everywhere"), "{out}");
    assert!(inbox.contains("signed out every device"), "{inbox}");
    for secret in [TOKEN, phone.as_str(), kept.as_str()] {
        assert!(!out.contains(secret), "log leaked a token");
        assert!(!inbox.contains(secret), "inbox leaked a token");
        assert!(!listed.contains(secret), "device list leaked a token");
        let hash = hq_db::web_auth::hash_secret(secret);
        assert!(!listed.contains(&hash), "device list leaked a hash");
    }
}

#[tokio::test]
async fn without_a_web_token_the_device_routes_do_not_exist() {
    let _serial = SERIAL.lock().await;
    let (app, _vault) = router(None);
    for request in [
        Request::get(devices::DEVICES_PATH),
        Request::post(devices::ROTATE_PATH).header("x-hq-client", "web"),
        Request::delete("/api/auth/devices/abc").header("x-hq-client", "web"),
    ] {
        let request = request
            .header("host", "127.0.0.1:5678")
            .body(Body::empty())
            .unwrap();
        assert_eq!(send(&app, request).await.0, StatusCode::NOT_FOUND);
    }
}

/// How long a signed-out socket may take to close before the test calls it a leak.
const SOCKET_CLOSE_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

#[tokio::test]
async fn signing_out_everywhere_closes_an_open_chat_socket() {
    use futures::StreamExt;
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let _serial = SERIAL.lock().await;
    let (app, _state, _vault) = router_with_state(Some(TOKEN));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(axum::serve(listener, app.clone()).into_future());

    let mut request = format!("ws://{addr}/ws").into_client_request().unwrap();
    request
        .headers_mut()
        .insert("authorization", format!("Bearer {TOKEN}").parse().unwrap());
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();

    assert_eq!(rotate(&app, TOKEN).await.0, StatusCode::OK);
    let closed = tokio::time::timeout(SOCKET_CLOSE_WAIT, async {
        loop {
            match socket.next().await {
                None | Some(Err(_)) => break,
                Some(Ok(frame)) if frame.is_close() => break,
                Some(Ok(_)) => {}
            }
        }
    })
    .await;
    assert!(
        closed.is_ok(),
        "the socket stayed open after signing out everywhere"
    );
    server.abort();
}

fn seed_push(state: &WsState, endpoint: &str, device: Option<&str>) {
    hq_db::push_subscriptions::upsert(&state.db, endpoint, "k", "a", None, device).unwrap();
}

fn push_endpoints(state: &WsState) -> Vec<String> {
    hq_db::push_subscriptions::list(&state.db)
        .unwrap()
        .into_iter()
        .map(|s| s.endpoint)
        .collect()
}

#[tokio::test]
async fn sign_out_everywhere_ends_connector_grants_the_window_and_push() {
    let _serial = SERIAL.lock().await;
    let (app, state, _vault) = router_with_state(Some(TOKEN));
    let now = chrono::Utc::now().timestamp();
    state
        .db
        .with_conn(|c| {
            c.execute(
                "INSERT INTO mcp_oauth_grants (id, client_id, client_name, label, grant_scope,
                 scope, resource, created_at, expires_at)
                 VALUES ('g1', 'https://claude.example/c', 'Claude', 'Claude', 'full', 'mcp',
                 'https://hq.example.ts.net/mcp', ?1, ?2)",
                rusqlite_params(now),
            )?;
            hq_db::mcp_connector::arm(c, now + 600)
        })
        .unwrap();
    seed_push(&state, "https://push.example.com/thief", None);

    assert_eq!(rotate(&app, TOKEN).await.0, StatusCode::OK);

    let (live, armed) = state
        .db
        .with_conn(|c| {
            Ok((
                hq_db::mcp_connector::list_grants(c, false, now)?.len(),
                hq_db::mcp_connector::armed_until(c, now)?,
            ))
        })
        .unwrap();
    assert_eq!(live, 0, "connector grants are revoked");
    assert_eq!(armed, None, "the approval window is closed");
    assert!(
        push_endpoints(&state).is_empty(),
        "no stranger's browser gets the notice"
    );
}

fn rusqlite_params(now: i64) -> [i64; 2] {
    [now, now + 86_400]
}

#[tokio::test]
async fn signing_a_device_out_drops_its_push_subscriptions_only() {
    let _serial = SERIAL.lock().await;
    let (app, state, _vault) = router_with_state(Some(TOKEN));
    sign_in_with_code(&app).await;
    let (_, body) = list_devices(&app, TOKEN).await;
    let phone_id = body["devices"][0]["id"].as_str().unwrap().to_string();
    // Push is off in this router; `web_push::api` tests that a subscribe stores the device id.
    seed_push(&state, "https://push.example.com/phone", Some(&phone_id));
    seed_push(&state, "https://push.example.com/laptop", None);

    assert_eq!(
        sign_out(&app, TOKEN, &phone_id).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        push_endpoints(&state),
        vec!["https://push.example.com/laptop".to_string()]
    );
}

#[tokio::test]
async fn the_configured_token_is_answered_from_memory_once_known() {
    let _serial = SERIAL.lock().await;
    let (app, state, _vault) = router_with_state(Some(TOKEN));
    assert_eq!(list_devices(&app, TOKEN).await.0, StatusCode::OK);
    state
        .db
        .with_conn(|c| Ok(c.execute_batch("DROP TABLE web_revoked_tokens;")?))
        .unwrap();
    assert_eq!(
        list_devices(&app, TOKEN).await.0,
        StatusCode::OK,
        "a database problem does not sign out a configured token already known to be live"
    );
}

#[tokio::test]
async fn a_rotate_waits_for_an_exchange_in_flight_and_voids_its_code() {
    let _serial = SERIAL.lock().await;
    let (app, state, _vault) = router_with_state(Some(TOKEN));
    let code = issue(&app).await;
    let held = state.auth_lock.lock().await;
    let rotating = tokio::spawn({
        let app = app.clone();
        async move { rotate(&app, TOKEN).await }
    });
    tokio::task::yield_now().await;
    assert!(!rotating.is_finished(), "rotate waits for the lock");
    drop(held);
    let (status, body) = rotating.await.unwrap();
    assert_eq!(status, StatusCode::OK);
    let kept = body["token"].as_str().unwrap().to_string();
    assert_eq!(
        send(&app, exchange(&code)).await.0,
        StatusCode::UNAUTHORIZED
    );
    let (_, body) = list_devices(&app, &kept).await;
    assert_eq!(
        body["devices"].as_array().unwrap().len(),
        1,
        "only the rotating device"
    );
}
