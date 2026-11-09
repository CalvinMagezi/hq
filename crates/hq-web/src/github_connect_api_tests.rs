use super::*;
use axum::body::Body;
use axum::http::Request;
use hq_tools::github::app::Store;
use tempfile::TempDir;
use tower::ServiceExt;

const BASE: &str = "http://127.0.0.1:5678";

fn flow() -> (TempDir, Arc<BrowserFlow>) {
    let dir = TempDir::new().unwrap();
    let client =
        AppClient::new(Store::at(dir.path().join("github")), "http://127.0.0.1:1/").unwrap();
    (dir, Arc::new(BrowserFlow::new(client, BASE.to_string())))
}

async fn get_status(flow: &Arc<BrowserFlow>, uri: &str) -> (StatusCode, axum::http::HeaderMap) {
    let app: Router = browser_routes(flow.clone());
    let response = app
        .oneshot(Request::get(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    (response.status(), response.headers().clone())
}

#[tokio::test]
async fn the_start_page_needs_a_flow_the_owner_began() {
    let (_dir, flow) = flow();
    let (status, _) = get_status(&flow, &format!("{START_PATH}?state=made-up")).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = get_status(&flow, START_PATH).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn the_start_page_opens_for_a_pending_flow_with_its_own_policy() {
    let (_dir, flow) = flow();
    let link = flow.begin();
    let path = link.strip_prefix(BASE).unwrap();
    let (status, headers) = get_status(&flow, path).await;
    assert_eq!(status, StatusCode::OK);
    let csp = headers[header::CONTENT_SECURITY_POLICY].to_str().unwrap();
    assert!(csp.contains("form-action https://github.com"), "{csp}");
    assert_eq!(headers[header::REFERRER_POLICY], "no-referrer");
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
}

#[tokio::test]
async fn a_return_without_a_matching_state_is_refused() {
    let (_dir, flow) = flow();
    flow.begin();
    let uri = format!("{RETURN_PATH}?state=made-up&code=abc");
    let (status, _) = get_status(&flow, &uri).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(flow.client.store().load().is_none());
}

const OWNER_ROUTES: [(&str, &str); 6] = [
    ("GET", "/api/github"),
    ("POST", "/api/github/connect"),
    ("POST", "/api/github/disconnect"),
    ("POST", "/api/github/repos/scan"),
    ("POST", "/api/github/repos/confirm"),
    ("POST", "/api/github/first-task"),
];
const WEB_TOKEN: &str = "s3cret-web-token";

fn guarded_app() -> (TempDir, Router) {
    let vault = TempDir::new().unwrap();
    let mut state = WsState::new(vault.path().to_path_buf(), None);
    state.web_auth_token = Some(WEB_TOKEN.to_string());
    (vault, crate::create_router(Arc::new(state)))
}

fn local(method: &str, uri: &str) -> axum::http::request::Builder {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("host", "127.0.0.1:5678")
}

#[tokio::test]
async fn owner_routes_refuse_a_caller_without_the_web_token() {
    let (_vault, app) = guarded_app();
    for (verb, uri) in OWNER_ROUTES {
        let bare = local(verb, uri).body(Body::empty()).unwrap();
        let response = app.clone().oneshot(bare).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{uri}");
    }
}

#[tokio::test]
async fn an_mcp_key_does_not_open_owner_routes() {
    let (_vault, app) = guarded_app();
    for (verb, uri) in OWNER_ROUTES {
        let keyed = local(verb, uri)
            .header("x-api-key", "an-mcp-or-connector-key")
            .header("authorization", "Bearer an-mcp-or-connector-key")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(keyed).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{uri}");
    }
}

#[tokio::test]
async fn the_browser_routes_are_reachable_but_do_nothing_without_a_flow() {
    let (_vault, app) = guarded_app();
    assert!(!crate::auth::web_path_is_guarded(RETURN_PATH));
    let uri = format!("{RETURN_PATH}?state=made-up&code=abc");
    let request = local("GET", &uri).body(Body::empty()).unwrap();
    let response = app.oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn with_sign_in_off_nothing_can_change_the_connection() {
    let vault = TempDir::new().unwrap();
    let app = crate::create_router(Arc::new(WsState::new(vault.path().to_path_buf(), None)));
    for uri in [
        "/api/github/connect",
        "/api/github/first-task",
        "/api/github/repos/confirm",
    ] {
        let request = local("POST", uri).body(Body::empty()).unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{uri}");
    }
}

#[test]
fn lite_serves_none_of_the_owner_routes() {
    for (_, uri) in OWNER_ROUTES {
        assert!(!crate::profile_gate::lite_allows(uri), "{uri}");
    }
}

async fn call(flow: &Arc<BrowserFlow>, verb: &str, uri: &str, body: Body) -> (StatusCode, Value) {
    let app: Router = owner_routes(flow.clone());
    let request = Request::builder()
        .method(verb)
        .uri(uri)
        .header("content-type", "application/json")
        .body(body)
        .unwrap();
    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 65_536)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn connect_returns_a_link_the_start_page_accepts() {
    let (_dir, flow) = flow();
    let (status, body) = call(&flow, "POST", "/api/github/connect", Body::empty()).await;
    assert_eq!(status, StatusCode::OK);
    let link = body["url"].as_str().unwrap();
    let path = link.strip_prefix(BASE).unwrap();
    assert_eq!(get_status(&flow, path).await.0, StatusCode::OK);
}

#[tokio::test]
async fn status_says_not_connected_and_holds_no_key_material() {
    let (_dir, flow) = flow();
    let (status, body) = call(&flow, "GET", "/api/github", Body::empty()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["standing"], "not_connected");
    assert!(!body.to_string().contains("PRIVATE KEY"));
}

#[tokio::test]
async fn confirming_an_unknown_repository_confirms_nothing() {
    let (_dir, flow) = flow();
    let body = Body::from(r#"{"repo":"o/r"}"#);
    let (status, reply) = call(&flow, "POST", "/api/github/repos/confirm", body).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(reply["confirmed"], false);
}

#[tokio::test]
async fn disconnect_with_nothing_connected_is_harmless() {
    let (_dir, flow) = flow();
    let (status, body) = call(&flow, "POST", "/api/github/disconnect", Body::empty()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["settings_url"], Value::Null);
}

#[tokio::test]
async fn the_install_return_changes_nothing() {
    let (_dir, flow) = flow();
    let (status, _) = get_status(&flow, &format!("{INSTALLED_PATH}?installation_id=1")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(flow.client.store().load().is_none());
}
