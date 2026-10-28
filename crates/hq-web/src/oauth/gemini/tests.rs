//! The Gemini profile end to end, in process: a client the owner created, Google's redirect,
//! consent, secret-authenticated token calls, the pinned tools, and the walls between profiles.

use super::*;
use crate::oauth::routes;
use axum::body::Body;
use axum::http::Request;
use base64::Engine;
use hq_tools::registry::{HqTool, ToolRegistry};
use sha2::Digest;
use tower::ServiceExt;

const CONSENT_HOST: &str = "hq.example.ts.net";
const REDIRECT: &str = "https://oauth-redirect.googleusercontent.com/r/hq-test";
const RESOURCE: &str = "https://hq.example.com/mcp/gemini/v1";
const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
const CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
/// SHA-256 of the v1 tool list. A change here changes what Google snapshotted: put the new
/// list on a new version path instead of editing v1.
const V1_TOOLS_SHA256: &str = "fc410b7fb7314223dfb92a8b4106ffedf569e7b33c7d5f0c42aa7ffb5c6ce344";

/// A tool that answers with a fixed value.
struct Fixed(&'static str, Value);

#[async_trait::async_trait]
impl HqTool for Fixed {
    fn name(&self) -> &str {
        self.0
    }
    fn description(&self) -> &str {
        "fixed"
    }
    fn parameters(&self) -> Value {
        json!({})
    }
    async fn execute(&self, args: Value) -> anyhow::Result<Value> {
        if self.1.is_null() {
            return Ok(args);
        }
        Ok(self.1.clone())
    }
}

struct Harness {
    app: axum::Router,
    state: Arc<WsState>,
    _vault: tempfile::TempDir,
}

fn harness(gemini: bool) -> Harness {
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(Fixed(
        "vault_search",
        json!({
            "results": [
                {"path": "Notebooks/a.md", "related": ["_system/MEMORY.md", "Notebooks/b.md"]},
                {"path": "_system/SOUL.md", "related": []}
            ],
            "count": 2
        }),
    )));
    registry.register(Box::new(Fixed(
        "vault_list",
        json!({"paths": ["a.md", "_private.md"], "count": 2}),
    )));
    registry.register(Box::new(Fixed("vault_read", Value::Null)));
    registry.register(Box::new(Fixed("config_manage", json!({"ran": true}))));
    let vault = tempfile::TempDir::new().unwrap();
    let mut state =
        WsState::new(vault.path().to_path_buf(), None).with_registry(Arc::new(registry));
    state.connector = Some(hq_core::config::ConnectorSettings {
        public: "https://hq.example.com".into(),
        consent: format!("https://{CONSENT_HOST}"),
        default_scope: "read_only".into(),
        gemini,
    });
    let state = Arc::new(state);
    let app = routes(axum::Router::new(), &state)
        .route("/mcp", axum::routing::post(crate::mcp_http::mcp_handler))
        .with_state(state.clone());
    Harness {
        app,
        state,
        _vault: vault,
    }
}

async fn send(
    app: &axum::Router,
    req: Request<Body>,
) -> (StatusCode, axum::http::HeaderMap, Value) {
    let res = app.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let headers = res.headers().clone();
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        headers,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn form(path: &str, pairs: &[(&str, &str)], basic: Option<(&str, &str)>) -> Request<Body> {
    let body = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs)
        .finish();
    let mut req = Request::post(path).header("content-type", "application/x-www-form-urlencoded");
    if let Some((id, secret)) = basic {
        let raw = base64::engine::general_purpose::STANDARD.encode(format!("{id}:{secret}"));
        req = req.header("authorization", format!("Basic {raw}"));
    }
    req.body(Body::from(body)).unwrap()
}

fn json_post(path: &str, body: Value, cookie: Option<&str>, bearer: Option<&str>) -> Request<Body> {
    let mut req = Request::post(path).header("content-type", "application/json");
    if let Some(c) = cookie {
        req = req.header("cookie", c.split(';').next().unwrap());
    }
    if let Some(t) = bearer {
        req = req.header("authorization", format!("Bearer {t}"));
    }
    req.body(Body::from(body.to_string())).unwrap()
}

fn authorize_path(client_id: &str, redirect: &str) -> String {
    let q = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs([
            ("response_type", "code"),
            ("client_id", client_id),
            ("redirect_uri", redirect),
            ("code_challenge", CHALLENGE),
            ("code_challenge_method", "S256"),
            ("state", "state-from-google"),
            ("resource", RESOURCE),
        ])
        .finish();
    format!("/oauth/authorize?{q}")
}

async fn authorize(
    h: &Harness,
    client_id: &str,
    redirect: &str,
) -> (StatusCode, Option<String>, Option<String>) {
    let (status, _, _) = send(
        &h.app,
        json_post("/api/oauth/connector/arm", json!({}), None, None),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let req = Request::get(authorize_path(client_id, redirect))
        .header("host", CONSENT_HOST)
        .body(Body::empty())
        .unwrap();
    let (status, headers, _) = send(&h.app, req).await;
    let header = |n: &str| headers.get(n).map(|v| v.to_str().unwrap().to_string());
    (status, header("location"), header("set-cookie"))
}

fn new_client(h: &Harness) -> store::NewClient {
    h.state
        .db
        .with_conn(|c| store::create_client(c, "Pixel", Profile::Gemini, super::super::now()))
        .unwrap()
}

/// Runs the whole flow with the owner choosing `scope`, and returns the token response.
async fn connected(h: &Harness, client: &store::NewClient, scope: &str) -> Value {
    let (status, location, cookie) = authorize(h, &client.id, REDIRECT).await;
    assert_eq!(status, StatusCode::FOUND);
    let id = location
        .unwrap()
        .strip_prefix("/connect?request=")
        .unwrap()
        .to_string();
    let cookie = cookie.unwrap();
    let (status, _, req) = send(
        &h.app,
        Request::get(format!("/api/oauth/requests/{id}"))
            .header("cookie", cookie.split(';').next().unwrap())
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(req["profile"], "gemini");
    assert_eq!(req["client_name"], "Gemini (Pixel)");
    let (status, _, approved) = send(
        &h.app,
        json_post(
            &format!("/api/oauth/requests/{id}/approve"),
            json!({"scope": scope}),
            Some(&cookie),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let redirect = reqwest::Url::parse(approved["redirect"].as_str().unwrap()).unwrap();
    assert!(redirect.as_str().starts_with(REDIRECT));
    let code = redirect
        .query_pairs()
        .find(|(k, _)| k == "code")
        .unwrap()
        .1
        .into_owned();
    let (status, _, tokens) = send(
        &h.app,
        form(
            "/oauth/token",
            &[
                ("grant_type", "authorization_code"),
                ("code", &code),
                ("redirect_uri", REDIRECT),
                ("code_verifier", VERIFIER),
                ("resource", RESOURCE),
            ],
            Some((&client.id, &client.secret)),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{tokens}");
    tokens
}

async fn rpc(
    h: &Harness,
    path: &str,
    token: &str,
    method: &str,
    params: Value,
) -> (StatusCode, axum::http::HeaderMap, Value) {
    send(
        &h.app,
        json_post(
            path,
            json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}),
            None,
            Some(token),
        ),
    )
    .await
}

async fn call_tool(h: &Harness, token: &str, name: &str, args: Value) -> Value {
    let (status, _, body) = rpc(
        h,
        GEMINI_PATH,
        token,
        "tools/call",
        json!({"name": name, "arguments": args}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    body
}

const GEMINI_PATH: &str = hq_core::config::GEMINI_MCP_PATH;

#[tokio::test]
async fn a_gemini_client_connects_read_only_and_reaches_only_the_pinned_tools() {
    let h = harness(true);
    let client = new_client(&h);
    let tokens = connected(&h, &client, "full").await;
    assert_eq!(tokens["token_type"], "Bearer");
    let access = tokens["access_token"].as_str().unwrap();
    let grants = h
        .state
        .db
        .with_conn(|c| store::list_grants(c, false, super::super::now()))
        .unwrap();
    assert_eq!(
        grants[0].grant_scope, "read_only",
        "the owner's pick of full is not kept"
    );
    assert_eq!(grants[0].profile, "gemini");

    let (status, _, init) = rpc(
        &h,
        GEMINI_PATH,
        access,
        "initialize",
        json!({"protocolVersion": "2025-11-25"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(init["result"]["protocolVersion"], "2025-11-25");

    let (_, _, listed) = rpc(&h, GEMINI_PATH, access, "tools/list", json!({})).await;
    let names: Vec<&str> = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, GEMINI_TOOLS);

    let found = call_tool(
        &h,
        access,
        "vault_search",
        json!({"query": "plans", "limit": 500}),
    )
    .await;
    let text = found["result"]["content"][0]["text"].as_str().unwrap();
    let parsed: Value = serde_json::from_str(text).unwrap();
    assert_eq!(parsed["count"], 1);
    assert_eq!(parsed["results"][0]["related"], json!(["Notebooks/b.md"]));
    assert!(!text.contains("_system"), "{text}");

    let listed = call_tool(&h, access, "vault_list", json!({"directory": ""})).await;
    let text = listed["result"]["content"][0]["text"].as_str().unwrap();
    assert!(!text.contains("_private"), "{text}");

    let read = call_tool(
        &h,
        access,
        "vault_read",
        json!({"path": "Notebooks/a.md", "start_line": 2, "extra": "dropped", "_hq_connector": "spoof"}),
    )
    .await;
    let text = read["result"]["content"][0]["text"].as_str().unwrap();
    let ran_with: Value = serde_json::from_str(text).unwrap();
    assert_eq!(ran_with["path"], "Notebooks/a.md");
    assert_eq!(ran_with["start_line"], 2);
    assert!(ran_with.get("extra").is_none());
    assert_ne!(
        ran_with["_hq_connector"], "spoof",
        "identity is the runtime's to set"
    );

    for path in [
        "_system/SOUL.md",
        "Notebooks/../_data/vault.db",
        "/etc/passwd",
        ".git/config",
    ] {
        let refused = call_tool(&h, access, "vault_read", json!({"path": path})).await;
        assert_eq!(refused["result"]["isError"], true, "{path}");
    }
    let refused = call_tool(&h, access, "vault_list", json!({"directory": "_mailboxes"})).await;
    assert_eq!(refused["result"]["isError"], true);
    for name in ["config_manage", "hq_call", "hq_discover"] {
        let refused = call_tool(&h, access, name, json!({})).await;
        assert!(refused.get("error").is_some(), "{name}: {refused}");
    }
}

#[tokio::test]
async fn gemini_refresh_keeps_the_token_and_needs_the_secret() {
    let h = harness(true);
    let client = new_client(&h);
    let tokens = connected(&h, &client, "read_only").await;
    let refresh = tokens["refresh_token"].as_str().unwrap();
    let pairs = [("grant_type", "refresh_token"), ("refresh_token", refresh)];
    let (status, headers, body) = send(&h.app, form("/oauth/token", &pairs, None)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "no client at all: {body}");
    let _ = headers;
    let with_id = [
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh),
        ("client_id", client.id.as_str()),
    ];
    let (status, headers, body) = send(&h.app, form("/oauth/token", &with_id, None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert_eq!(body["error"], "invalid_client");
    assert!(headers.contains_key("www-authenticate"));
    let (status, _, body) = send(
        &h.app,
        form("/oauth/token", &pairs, Some((&client.id, "hqs_wrong"))),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");

    let mut posted = with_id.to_vec();
    posted.push(("client_secret", client.secret.as_str()));
    let (status, _, renewed) = send(&h.app, form("/oauth/token", &posted, None)).await;
    assert_eq!(status, StatusCode::OK, "client_secret_post: {renewed}");
    assert_eq!(
        renewed["refresh_token"], refresh,
        "Google keeps one refresh token"
    );
    let (status, _, again) = send(
        &h.app,
        form("/oauth/token", &pairs, Some((&client.id, &client.secret))),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "client_secret_basic, the same token again: {again}"
    );

    let (status, _, _) = send(
        &h.app,
        form("/oauth/token", &posted, Some((&client.id, &client.secret))),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "two authentication methods at once"
    );
}

#[tokio::test]
async fn revocation_by_token_or_client_ends_the_connection() {
    let h = harness(true);
    let client = new_client(&h);
    let tokens = connected(&h, &client, "read_only").await;
    let access = tokens["access_token"].as_str().unwrap();
    let (status, _, _) = send(
        &h.app,
        form(
            "/oauth/revoke",
            &[("token", tokens["refresh_token"].as_str().unwrap())],
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, headers, _) = rpc(&h, GEMINI_PATH, access, "tools/list", json!({})).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(
        headers["www-authenticate"]
            .to_str()
            .unwrap()
            .contains("oauth-protected-resource/mcp/gemini/v1")
    );

    let tokens = connected(&h, &client, "read_only").await;
    let access = tokens["access_token"].as_str().unwrap();
    let now = super::super::now();
    assert!(
        h.state
            .db
            .with_conn(|c| store::revoke_client(c, &client.id, now))
            .unwrap()
    );
    let (status, _, _) = rpc(&h, GEMINI_PATH, access, "tools/list", json!({})).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _, _) = authorize(&h, &client.id, REDIRECT).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "a revoked client cannot start again"
    );
}

#[tokio::test]
async fn each_profile_refuses_the_others_tokens_and_unknown_credentials() {
    let h = harness(true);
    let client = new_client(&h);
    let tokens = connected(&h, &client, "read_only").await;
    let gemini = tokens["access_token"].as_str().unwrap();
    let (status, _, _) = rpc(&h, "/mcp", gemini, "tools/list", json!({})).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "a Gemini token on Claude's endpoint"
    );

    let claude = h
        .state
        .db
        .with_conn(|c| {
            let now = super::super::now();
            store::arm(c, now + 600)?;
            let new = store::NewRequest {
                client_id: "https://claude.ai/oauth/mcp-oauth-client-metadata",
                client_name: "Claude",
                redirect_uri: "https://claude.ai/api/mcp/auth_callback",
                state: None,
                code_challenge: CHALLENGE,
                resource: "https://hq.example.com/mcp",
                scope: "hq",
                profile: Profile::Claude,
            };
            let store::Pending::Created(id) = store::create_request(c, &new, "b", now)? else {
                anyhow::bail!("not stored");
            };
            let store::Approval::Issued { code, .. } =
                store::approve_request(c, &id, "b", store::GrantScope::ReadOnly, "Claude", now)?
            else {
                anyhow::bail!("not approved");
            };
            let ex = store::CodeExchange {
                code: &code,
                client_id: new.client_id,
                redirect_uri: new.redirect_uri,
                challenge: CHALLENGE,
                resource: None,
            };
            Ok(store::redeem_code(c, &ex, now)?.unwrap())
        })
        .unwrap();
    let (status, _, _) = rpc(
        &h,
        GEMINI_PATH,
        &claude.access_token,
        "tools/list",
        json!({}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "a Claude token on Gemini's endpoint"
    );
    let (status, _, _) = rpc(&h, "/mcp", &claude.access_token, "tools/list", json!({})).await;
    assert_eq!(status, StatusCode::OK, "Claude still reaches its own");

    for junk in ["", "hqo_unknown", "some-api-key"] {
        let (status, _, _) = rpc(&h, GEMINI_PATH, junk, "tools/list", json!({})).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{junk:?}");
    }
}

#[tokio::test]
async fn with_the_profile_off_a_registered_client_gets_nowhere() {
    let h = harness(false);
    let client = new_client(&h);
    let (status, _, _) = authorize(&h, &client.id, REDIRECT).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let pairs = [("grant_type", "refresh_token"), ("refresh_token", "hqr_x")];
    let (status, _, body) = send(
        &h.app,
        form("/oauth/token", &pairs, Some((&client.id, &client.secret))),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    let (status, _, _) = send(
        &h.app,
        json_post(
            GEMINI_PATH,
            json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
            None,
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_gemini_client_must_use_googles_redirect_and_a_claude_client_may_not() {
    let h = harness(true);
    let client = new_client(&h);
    let (status, _, _) = authorize(&h, &client.id, "https://claude.ai/api/mcp/auth_callback").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _, _) = authorize(&h, "hqcl_unknown", REDIRECT).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _, _) = send(&h.app, form("/oauth/register", &[], None)).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "registration is always refused"
    );
}

#[test]
fn every_v1_tool_is_annotated_read_only_and_the_list_is_pinned() {
    let tools = tool_list();
    for t in tools.as_array().unwrap() {
        let a = &t["annotations"];
        assert_eq!(a["readOnlyHint"], true, "{t}");
        assert_eq!(a["destructiveHint"], false, "{t}");
        assert_eq!(a["openWorldHint"], false, "{t}");
        assert!(t["inputSchema"]["type"] == "object", "{t}");
    }
    let digest: String = sha2::Sha256::digest(tools.to_string().as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(digest, V1_TOOLS_SHA256, "the v1 tool list changed");
}

#[test]
fn system_paths_are_recognised() {
    for p in [
        "_system/SOUL.md",
        "a/_threads/x.md",
        ".hidden/x.md",
        "a/../b.md",
        "/abs.md",
        "a//b.md",
        "a\\b.md",
    ] {
        assert!(is_system_path(p), "{p}");
    }
    for p in ["Notebooks/Projects/x.md", "README.md", "a_b/c.md"] {
        assert!(!is_system_path(p), "{p}");
    }
}
