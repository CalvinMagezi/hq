//! The whole connector flow in process, in the shapes claude.ai sent during slice 0.

use super::*;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use hq_tools::registry::{HqTool, ToolRegistry};
use serde_json::{Value, json};
use std::collections::HashMap;
use tower::ServiceExt;

const CLIENT: &str = "https://claude.ai/oauth/mcp-oauth-client-metadata";
const REDIRECT: &str = "https://claude.ai/api/mcp/auth_callback";
const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
const CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

struct Fake(&'static str);

#[async_trait::async_trait]
impl HqTool for Fake {
    fn name(&self) -> &str {
        self.0
    }
    fn description(&self) -> &str {
        "fake"
    }
    fn parameters(&self) -> Value {
        json!({})
    }
    async fn execute(&self, _args: Value) -> anyhow::Result<Value> {
        Ok(json!({"ran": self.0}))
    }
}

fn settings() -> hq_core::config::ConnectorSettings {
    hq_core::config::ConnectorSettings {
        public: "https://hq.example.com".into(),
        consent: "https://hq.example.ts.net".into(),
        default_scope: "read_only".into(),
    }
}

fn app() -> axum::Router {
    super::cimd::remember(super::cimd::ClientMetadata {
        client_id: CLIENT.into(),
        client_name: "Claude".into(),
        redirect_uris: vec![REDIRECT.into()],
    });
    let mut registry = ToolRegistry::new();
    for name in ["vault_search", "config_manage"] {
        registry.register(Box::new(Fake(name)));
    }
    let vault = tempfile::TempDir::new().unwrap();
    let mut state =
        WsState::new(vault.path().to_path_buf(), None).with_registry(Arc::new(registry));
    state.connector = Some(settings());
    let state = Arc::new(state);
    routes(axum::Router::new(), &state)
        .route("/mcp", axum::routing::post(crate::mcp_http::mcp_handler))
        .with_state(state)
}

async fn send(app: &axum::Router, req: Request<Body>) -> (StatusCode, Option<String>, Value) {
    let (status, location, _, body) = send_full(app, req).await;
    (status, location, body)
}

/// Status, `Location`, `Set-Cookie` and the JSON body.
async fn send_full(
    app: &axum::Router,
    req: Request<Body>,
) -> (StatusCode, Option<String>, Option<String>, Value) {
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let header = |name: &str| {
        res.headers()
            .get(name)
            .map(|v| v.to_str().unwrap().to_string())
    };
    let (location, cookie) = (header("location"), header("set-cookie"));
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, location, cookie, body)
}

/// A request carrying the browser's binder cookie.
fn with_cookie(mut req: Request<Body>, cookie: &str) -> Request<Body> {
    let pair = cookie.split(';').next().unwrap().to_string();
    req.headers_mut().insert("cookie", pair.parse().unwrap());
    req
}

/// Arms and runs `/oauth/authorize`; returns the request id and the cookie it set.
async fn authorized(app: &axum::Router) -> (String, String) {
    let (status, _, _) = send(app, post_json("/api/oauth/connector/arm", json!({}))).await;
    assert_eq!(status, StatusCode::OK);
    let (status, location, cookie, _) = send_full(app, get(&authorize_path())).await;
    assert_eq!(status, StatusCode::FOUND);
    let cookie = cookie.expect("a binder cookie");
    for attr in [
        "HttpOnly",
        "Secure",
        "SameSite=Strict",
        "Path=/api/oauth/requests",
    ] {
        assert!(cookie.contains(attr), "{cookie}");
    }
    let id = location.unwrap();
    let id = id.strip_prefix("/connect?request=").unwrap().to_string();
    (id, cookie)
}

fn get(path: &str) -> Request<Body> {
    Request::get(path).body(Body::empty()).unwrap()
}

fn post_json(path: &str, body: Value) -> Request<Body> {
    Request::post(path)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn post_form(path: &str, pairs: &[(&str, &str)]) -> Request<Body> {
    let body = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs)
        .finish();
    Request::post(path)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(body))
        .unwrap()
}

fn authorize_path() -> String {
    let q = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs([
            ("response_type", "code"),
            ("client_id", CLIENT),
            ("redirect_uri", REDIRECT),
            ("code_challenge", CHALLENGE),
            ("code_challenge_method", "S256"),
            ("state", "state-from-claude"),
            ("scope", "hq offline_access"),
            ("resource", "https://hq.example.com/mcp"),
            ("prompt", "consent"),
        ])
        .finish();
    format!("/oauth/authorize?{q}")
}

fn mcp(token: &str, body: Value) -> Request<Body> {
    Request::post("/mcp")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn call(tool: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
           "params": {"name": "hq_call", "arguments": {"tool": tool, "args": {}}}})
}

/// Arms, authorizes and approves with `scope`; returns the code from the redirect.
async fn approved_code(app: &axum::Router, scope: &str) -> String {
    let (id, cookie) = authorized(app).await;
    let shown = with_cookie(get(&format!("/api/oauth/requests/{id}")), &cookie);
    let (status, _, shown) = send(app, shown).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(shown["client_name"], "Claude");
    assert_eq!(shown["redirect_host"], "claude.ai");
    assert_eq!(shown["default_scope"], "read_only");
    assert!(shown["armed_until"].is_i64(), "{shown}");
    let approve = post_json(
        &format!("/api/oauth/requests/{id}/approve"),
        json!({"scope": scope, "label": "Claude on the web"}),
    );
    let (status, _, decided) = send(app, with_cookie(approve, &cookie)).await;
    assert_eq!(status, StatusCode::OK, "{decided}");
    let redirect = url::Url::parse(decided["redirect"].as_str().unwrap()).unwrap();
    assert_eq!(redirect.as_str().split('?').next(), Some(REDIRECT));
    let params: HashMap<_, _> = redirect.query_pairs().into_owned().collect();
    assert_eq!(params["state"], "state-from-claude");
    params["code"].clone()
}

async fn exchange(app: &axum::Router, code: &str) -> (StatusCode, Value) {
    let form = post_form(
        "/oauth/token",
        &[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("client_id", CLIENT),
            ("code_verifier", VERIFIER),
            ("redirect_uri", REDIRECT),
            ("resource", "https://hq.example.com/mcp"),
        ],
    );
    let (status, _, body) = send(app, form).await;
    (status, body)
}

#[tokio::test]
async fn claude_connects_calls_a_tool_in_scope_refreshes_and_is_revoked() {
    let app = app();
    let code = approved_code(&app, "read_only").await;
    let (status, tokens) = exchange(&app, &code).await;
    assert_eq!(status, StatusCode::OK, "{tokens}");
    assert_eq!(tokens["token_type"], "Bearer");
    let access = tokens["access_token"].as_str().unwrap().to_string();
    let refresh = tokens["refresh_token"].as_str().unwrap().to_string();

    let (status, _, ok) = send(&app, mcp(&access, call("vault_search"))).await;
    assert_eq!(status, StatusCode::OK);
    assert!(ok.to_string().contains("vault_search"), "{ok}");
    let (_, _, denied) = send(&app, mcp(&access, call("config_manage"))).await;
    assert!(
        denied["error"]["message"]
            .as_str()
            .unwrap()
            .contains("not permitted"),
        "{denied}"
    );

    let form = post_form(
        "/oauth/token",
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", &refresh),
            ("client_id", CLIENT),
            ("resource", "https://hq.example.com/mcp"),
        ],
    );
    let (status, _, renewed) = send(&app, form).await;
    assert_eq!(status, StatusCode::OK, "{renewed}");
    let new_access = renewed["access_token"].as_str().unwrap().to_string();
    let new_refresh = renewed["refresh_token"].as_str().unwrap().to_string();
    assert_ne!(new_refresh, refresh);

    let (status, _, _) = send(&app, post_form("/oauth/revoke", &[("token", &new_refresh)])).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, _) = send(&app, mcp(&new_access, call("vault_search"))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_code_works_once_and_only_with_its_verifier() {
    let app = app();
    let code = approved_code(&app, "full").await;
    let wrong = post_form(
        "/oauth/token",
        &[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("client_id", CLIENT),
            ("code_verifier", &"x".repeat(43)),
            ("redirect_uri", REDIRECT),
        ],
    );
    let (status, _, body) = send(&app, wrong).await;
    assert_eq!(
        (status, body["error"].as_str()),
        (StatusCode::BAD_REQUEST, Some("invalid_grant"))
    );
    let (status, _) = exchange(&app, &code).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "the failed attempt spent the code"
    );

    let code = approved_code(&app, "full").await;
    assert_eq!(exchange(&app, &code).await.0, StatusCode::OK);
    assert_eq!(exchange(&app, &code).await.0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn nothing_connects_without_arming_and_one_arming_admits_one_client() {
    let app = app();
    let (status, _, _) = send(&app, get(&authorize_path())).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let _ = approved_code(&app, "tasks").await;
    let (status, _, _) = send(&app, get(&authorize_path())).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "approving closed the window");
}

#[tokio::test]
async fn a_denied_request_sends_claude_an_error_and_cannot_be_approved_after() {
    let app = app();
    let (id, cookie) = authorized(&app).await;
    let deny = Request::post(format!("/api/oauth/requests/{id}/deny"))
        .body(Body::empty())
        .unwrap();
    let (status, _, body) = send(&app, with_cookie(deny, &cookie)).await;
    assert_eq!(status, StatusCode::OK);
    let redirect = body["redirect"].as_str().unwrap();
    assert!(redirect.contains("error=access_denied"), "{redirect}");
    let approve = post_json(
        &format!("/api/oauth/requests/{id}/approve"),
        json!({"scope": "full"}),
    );
    let (status, _, _) = send(&app, with_cookie(approve, &cookie)).await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn another_browser_cannot_see_or_decide_the_request() {
    let app = app();
    let (id, cookie) = authorized(&app).await;
    let elsewhere = "hq_oauth_request=someone-else";
    let path = format!("/api/oauth/requests/{id}");
    assert_eq!(
        send(&app, with_cookie(get(&path), elsewhere)).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(send(&app, get(&path)).await.0, StatusCode::NOT_FOUND);
    let approve = post_json(&format!("{path}/approve"), json!({"scope": "full"}));
    assert_eq!(
        send(&app, with_cookie(approve, elsewhere)).await.0,
        StatusCode::CONFLICT
    );
    let mine = send(&app, with_cookie(get(&path), &cookie)).await;
    assert_eq!(
        mine.0,
        StatusCode::OK,
        "still there for the browser that started it"
    );
}

#[tokio::test]
async fn the_kill_switch_ends_every_grant() {
    let app = app();
    let code = approved_code(&app, "full").await;
    let access = exchange(&app, &code).await.1["access_token"]
        .as_str()
        .unwrap()
        .to_string();
    let off = Request::post("/api/oauth/connector/off")
        .body(Body::empty())
        .unwrap();
    let (status, _, body) = send(&app, off).await;
    assert_eq!(
        (status, body["revoked"].as_i64()),
        (StatusCode::OK, Some(1))
    );
    assert_eq!(
        send(&app, mcp(&access, call("vault_search"))).await.0,
        StatusCode::UNAUTHORIZED
    );
    let (_, _, shown) = send(&app, get("/api/oauth/connector")).await;
    assert_eq!(shown["grants"], json!([]));
}

#[tokio::test]
async fn the_server_metadata_matches_what_claude_reads() {
    let app = app();
    let (status, _, m) = send(&app, get(AUTHORIZATION_SERVER_PATH)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(m["issuer"], "https://hq.example.com");
    assert_eq!(
        m["authorization_endpoint"],
        "https://hq.example.ts.net/oauth/authorize"
    );
    assert_eq!(m["token_endpoint"], "https://hq.example.com/oauth/token");
    assert_eq!(m["client_id_metadata_document_supported"], true);
    assert_eq!(m["code_challenge_methods_supported"], json!(["S256"]));
    assert_eq!(m["token_endpoint_auth_methods_supported"], json!(["none"]));
    assert!(m.get("registration_endpoint").is_none());
}
