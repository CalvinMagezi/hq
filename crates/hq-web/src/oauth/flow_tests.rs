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

/// Hands back the arguments it ran with, markers included.
struct EchoArgs;

#[async_trait::async_trait]
impl HqTool for EchoArgs {
    fn name(&self) -> &str {
        "task_get"
    }
    fn description(&self) -> &str {
        "echo"
    }
    fn parameters(&self) -> Value {
        json!({})
    }
    async fn execute(&self, args: Value) -> anyhow::Result<Value> {
        Ok(args)
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
    let (app, _, vault) = app_and_db();
    // The router's database stays open; the directory goes, as the other tests here expect.
    drop(vault);
    app
}

/// The router, its state for reaching the database and vault, and the vault directory, which
/// lives as long as the caller holds it.
fn app_and_db() -> (axum::Router, Arc<WsState>, tempfile::TempDir) {
    super::cimd::remember(super::cimd::ClientMetadata {
        client_id: CLIENT.into(),
        client_name: "Claude".into(),
        redirect_uris: vec![REDIRECT.into()],
    });
    let mut registry = ToolRegistry::new();
    for name in [
        "vault_search",
        "config_manage",
        "vault_delete",
        "vault_write_note",
        "task_create",
    ] {
        registry.register(Box::new(Fake(name)));
    }
    registry.register(Box::new(EchoArgs));
    let vault = tempfile::TempDir::new().unwrap();
    let mut state =
        WsState::new(vault.path().to_path_buf(), None).with_registry(Arc::new(registry));
    state.connector = Some(settings());
    let state = Arc::new(state);
    let app = routes(axum::Router::new(), &state)
        .route("/mcp", axum::routing::post(crate::mcp_http::mcp_handler))
        .with_state(state.clone());
    (app, state, vault)
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
    let (status, location, cookie, _) = send_full(app, authorize_req()).await;
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

/// The browser's request to the consent origin, as claude.ai sends it there.
fn authorize_req() -> Request<Body> {
    Request::get(authorize_path())
        .header("host", "hq.example.ts.net")
        .body(Body::empty())
        .unwrap()
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
    let (status, _, _) = send(&app, authorize_req()).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let _ = approved_code(&app, "tasks").await;
    let (status, _, _) = send(&app, authorize_req()).await;
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
async fn the_public_name_never_answers_authorize() {
    let app = app();
    send(&app, post_json("/api/oauth/connector/arm", json!({}))).await;
    let public = Request::get(authorize_path())
        .header("host", "hq.example.com")
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(&app, public).await.0, StatusCode::NOT_FOUND);
    assert_eq!(
        send(&app, get(&authorize_path())).await.0,
        StatusCode::NOT_FOUND
    );
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

fn call_with(tool: &str, args: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call",
           "params": {"name": "hq_call", "arguments": {"tool": tool, "args": args}}})
}

async fn full_access(app: &axum::Router) -> String {
    let code = approved_code(app, "full").await;
    exchange(app, &code).await.1["access_token"]
        .as_str()
        .unwrap()
        .to_string()
}

fn text_of(reply: &Value) -> String {
    reply["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

/// The newest approval request for a call, as the owner's inbox holds it.
fn newest_request(state_db: &hq_db::Database) -> hq_core::types::ValueItem {
    let mut items = hq_db::value_items::list_filtered(state_db, None, None, 50).unwrap();
    items
        .retain(|i| i.source_task == "mcp-connector" && i.title.starts_with("Claude wants to run"));
    items.into_iter().next().expect("an approval request")
}

#[tokio::test]
async fn a_high_risk_call_runs_once_per_approval_with_exactly_its_arguments() {
    let (app, state, _vault) = app_and_db();
    let db = &state.db;
    let token = full_access(&app).await;
    let target = json!({"path": "skills/a/SKILL.md"});

    let (_, _, first) = send(&app, mcp(&token, call_with("vault_delete", target.clone()))).await;
    assert!(text_of(&first).contains("Waiting for the owner"), "{first}");
    assert_eq!(first["result"]["isError"], false);
    let ask = newest_request(db);
    assert!(
        ask.body.contains("skills/a/SKILL.md"),
        "the owner sees the real arguments"
    );

    assert!(hq_db::value_items::approve_by_id(db, &ask.id).unwrap());
    let (_, _, ran) = send(&app, mcp(&token, call_with("vault_delete", target.clone()))).await;
    assert!(text_of(&ran).contains("\"ran\""), "{ran}");

    let (_, _, again) = send(&app, mcp(&token, call_with("vault_delete", target.clone()))).await;
    assert!(
        text_of(&again).contains("Waiting"),
        "an approval is spent by one call: {again}"
    );

    let other = json!({"path": "skills/b/SKILL.md"});
    assert!(hq_db::value_items::approve_by_id(db, &newest_request(db).id).unwrap());
    let (_, _, swapped) = send(&app, mcp(&token, call_with("vault_delete", other))).await;
    assert!(
        text_of(&swapped).contains("Waiting"),
        "an approval does not carry to other arguments"
    );
}

#[tokio::test]
async fn a_declined_call_stays_declined_and_ordinary_calls_never_wait() {
    let (app, state, _vault) = app_and_db();
    let db = &state.db;
    let token = full_access(&app).await;
    let (_, _, quick) = send(&app, mcp(&token, call_with("vault_search", json!({})))).await;
    assert!(text_of(&quick).contains("vault_search"), "{quick}");

    let target = json!({"path": "skills/x/SKILL.md"});
    send(&app, mcp(&token, call_with("vault_delete", target.clone()))).await;
    assert!(hq_db::value_items::dismiss_by_id(db, &newest_request(db).id).unwrap());
    let (_, _, no) = send(&app, mcp(&token, call_with("vault_delete", target))).await;
    assert_eq!(no["result"]["isError"], true);
    assert!(text_of(&no).contains("declined"), "{no}");
}

#[tokio::test]
async fn tools_see_the_connection_and_a_busy_connection_is_slowed() {
    let (app, state, _vault) = app_and_db();
    let db = &state.db;
    let token = full_access(&app).await;
    let forged = json!({ hq_tools::harness_session::CONNECTOR_ARG: "owner" });
    let (_, _, echoed) = send(&app, mcp(&token, call_with("task_get", forged))).await;
    let seen: Value = serde_json::from_str(&text_of(&echoed)).unwrap();
    assert_eq!(
        seen[hq_tools::harness_session::CONNECTOR_ARG],
        "Claude on the web"
    );

    let grant = db
        .with_conn(|c| hq_db::mcp_connector::list_grants(c, false, now()))
        .unwrap()[0]
        .id
        .clone();
    db.with_conn(|c| {
        for _ in 0..super::guard::MAX_CALLS_PER_MINUTE {
            hq_db::mcp_connector::record_call(c, &grant, "vault_search", "ok", "h", now())?;
        }
        Ok(())
    })
    .unwrap();
    let (_, _, slowed) = send(&app, mcp(&token, call_with("vault_search", json!({})))).await;
    assert!(text_of(&slowed).contains("Too many calls"), "{slowed}");
    let log = db
        .with_conn(|c| hq_db::mcp_connector::recent_calls(c, Some(&grant), 1))
        .unwrap();
    assert_eq!(log[0].outcome, "rate_limited");
}

#[tokio::test]
async fn connecting_tells_the_owner_at_once() {
    let (app, state, _vault) = app_and_db();
    full_access(&app).await;
    let inbox = hq_core::mailbox::peek_messages(&state.vault_path, "relay").unwrap();
    assert!(
        inbox
            .iter()
            .any(|m| m.content.contains("Claude on the web")),
        "{inbox:?}"
    );
}

#[tokio::test]
async fn approving_while_the_call_waits_lets_it_run() {
    let (app, state, _vault) = app_and_db();
    let token = full_access(&app).await;
    let request = mcp(
        &token,
        call_with("vault_delete", json!({"path": "skills/w/SKILL.md"})),
    );
    let owner = async {
        loop {
            let found = hq_db::value_items::list_filtered(&state.db, None, None, 50)
                .unwrap()
                .into_iter()
                .find(|i| i.title == "Claude wants to run vault_delete");
            if let Some(item) = found {
                assert!(hq_db::value_items::approve_by_id(&state.db, &item.id).unwrap());
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    };
    let ((_, _, ran), ()) = tokio::join!(send(&app, request), owner);
    assert!(text_of(&ran).contains("\"ran\""), "{ran}");
}

#[tokio::test]
async fn a_connection_cannot_flood_the_owner_with_requests() {
    let (app, _state, _vault) = app_and_db();
    let token = full_access(&app).await;
    for i in 0..super::guard::MAX_WAITING_APPROVALS {
        let call = call_with(
            "vault_delete",
            json!({ "path": format!("skills/{i}/SKILL.md") }),
        );
        let (_, _, r) = send(&app, mcp(&token, call)).await;
        assert!(text_of(&r).contains("Waiting"), "{r}");
    }
    let (_, _, refused) = send(
        &app,
        mcp(
            &token,
            call_with("vault_delete", json!({"path": "skills/x/SKILL.md"})),
        ),
    )
    .await;
    assert_eq!(refused["result"]["isError"], true);
    assert!(text_of(&refused).contains("already waiting"), "{refused}");
}

#[tokio::test]
async fn task_writes_never_wait_for_approval_on_any_grant() {
    let (app, state, _vault) = app_and_db();
    let tasks_scope = approved_code(&app, "tasks").await;
    let tasks_token = exchange(&app, &tasks_scope).await.1["access_token"]
        .as_str()
        .unwrap()
        .to_string();
    let full = full_access(&app).await;

    // Tags and assignees, which used to ask first on a full grant, run straight away.
    for token in [&tasks_token, &full] {
        for args in [
            json!({"title": "t", "tags": ["relay"]}),
            json!({"title": "t", "assignees": ["hq"]}),
        ] {
            let (_, _, done) = send(&app, mcp(token, call_with("task_create", args))).await;
            assert!(text_of(&done).contains("task_create"), "{done}");
        }
    }
    let asked = hq_db::value_items::list_filtered(&state.db, None, None, 50)
        .unwrap()
        .into_iter()
        .any(|i| i.title.starts_with("Claude wants to run"));
    assert!(!asked, "the owner was never asked");
}

#[tokio::test]
async fn notes_are_free_to_write_and_trash_but_system_folders_ask_first() {
    let (app, state, _vault) = app_and_db();
    let token = full_access(&app).await;

    // Creating and deleting the owner's own notes runs at once.
    for path in ["Notebooks/a.md", "Inbox/b.md"] {
        for tool in ["vault_write_note", "vault_delete"] {
            let (_, _, r) = send(&app, mcp(&token, call_with(tool, json!({ "path": path })))).await;
            assert!(text_of(&r).contains("\"ran\""), "{tool} {path}: {r}");
        }
    }

    // A system folder waits, and the owner sees the call.
    let (_, _, held) = send(
        &app,
        mcp(
            &token,
            call_with("vault_delete", json!({"path": "_system/SOUL.md"})),
        ),
    )
    .await;
    assert!(text_of(&held).contains("Waiting for the owner"), "{held}");
    assert!(newest_request(&state.db).body.contains("_system/SOUL.md"));
}
