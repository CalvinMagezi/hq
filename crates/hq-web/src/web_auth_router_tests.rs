use super::*;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

async fn status(app: &Router, request: Request<Body>) -> StatusCode {
    app.clone().oneshot(request).await.unwrap().status()
}

/// With sign-in off, a request from this machine is the owner's, and one a proxy forwarded
/// (`tailscale serve` adds `Tailscale-User-Login`, Caddy `X-Forwarded-For`) is refused with
/// a reason. Mutation: drop the `forwarded` check in `web_auth_middleware`; the tailnet
/// request then gets the owner's API.
#[tokio::test]
async fn sign_in_off_serves_this_machine_and_refuses_a_proxy() {
    let vault = tempfile::TempDir::new().unwrap();
    let app = create_router(Arc::new(WsState::new(vault.path().to_path_buf(), None)));
    let local = Request::get("/api/spaces").body(Body::empty()).unwrap();
    assert_ne!(status(&app, local).await, StatusCode::FORBIDDEN);
    for (name, value) in [
        ("tailscale-user-login", "someone@example.com"),
        ("x-forwarded-for", "100.64.0.7"),
        ("forwarded", "for=100.64.0.7"),
    ] {
        let proxied = Request::get("/api/spaces")
            .header(name, value)
            .body(Body::empty())
            .unwrap();
        let res = app.clone().oneshot(proxied).await.unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN, "{name}");
        let body = axum::body::to_bytes(res.into_body(), 1 << 16)
            .await
            .unwrap();
        assert!(
            String::from_utf8_lossy(&body).contains("Set a web token"),
            "{name}"
        );
    }
    let ws = Request::get("/ws")
        .header("x-forwarded-for", "100.64.0.7")
        .body(Body::empty())
        .unwrap();
    assert_eq!(status(&app, ws).await, StatusCode::FORBIDDEN);
    // /health and the web app itself stay reachable, so the page can say what is wrong.
    let health = Request::get("/health")
        .header("x-forwarded-for", "100.64.0.7")
        .body(Body::empty())
        .unwrap();
    assert_eq!(status(&app, health).await, StatusCode::OK);
}

async fn body_json(app: &Router, request: Request<Body>) -> serde_json::Value {
    let res = app.clone().oneshot(request).await.unwrap();
    let body = axum::body::to_bytes(res.into_body(), 1 << 16)
        .await
        .unwrap();
    serde_json::from_slice(&body).unwrap()
}

/// Loopback (the updater, the installers) sees the build; a proxied caller sees only that
/// HQ is up, and a refused `/mcp` call does not learn whether any key is configured.
#[tokio::test]
async fn a_proxied_caller_learns_no_build_details_or_key_setup() {
    let vault = tempfile::TempDir::new().unwrap();
    let app = create_router(Arc::new(WsState::new(vault.path().to_path_buf(), None)));
    let health = |proxied: bool| {
        let req = Request::get("/health");
        let req = if proxied {
            req.header("x-forwarded-for", "100.64.0.7")
        } else {
            req
        };
        req.body(Body::empty()).unwrap()
    };
    let local = body_json(&app, health(false)).await;
    assert_eq!(local["status"], "ok");
    assert!(local.get("git_sha").is_some() && local.get("version").is_some());
    let public = body_json(&app, health(true)).await;
    assert_eq!(
        public,
        serde_json::json!({ "status": "ok", "service": HEALTH_SERVICE })
    );

    let mcp = |proxied: bool| {
        let req = Request::post("/mcp")
            .header("host", "127.0.0.1:5678")
            .header("content-type", "application/json");
        let req = if proxied {
            req.header("x-forwarded-for", "100.64.0.7")
        } else {
            req
        };
        req.body(Body::from(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
        ))
        .unwrap()
    };
    let local = body_json(&app, mcp(false)).await;
    assert!(
        local["error"]["message"]
            .as_str()
            .unwrap()
            .contains("AGENTHQ_API_KEY"),
        "{local}"
    );
    let public = body_json(&app, mcp(true)).await;
    assert_eq!(public["error"]["message"], "Unauthorized", "{public}");
}

/// A change a stolen sign-in could make lands in the owner's inbox naming the device.
#[test]
fn a_settings_change_notice_names_the_device_that_made_it() {
    let vault = tempfile::TempDir::new().unwrap();
    let state = WsState::new(vault.path().to_path_buf(), None);
    let minted = hq_db::web_auth::mint(&state.db, "Work laptop", 0, None).unwrap();
    let device = auth::WebCredential::Device(minted.id);
    state.notify_owner_of_change(Some(&device), "A key was changed", "HQ uses a new key.");
    state.notify_owner_of_change(
        Some(&auth::WebCredential::Configured),
        "A bot was connected",
        "HQ answers as a bot.",
    );
    let items =
        hq_db::value_items::list_by_state(&state.db, hq_core::types::ValueState::Pending).unwrap();
    let body = |title: &str| {
        items
            .iter()
            .find(|i| i.title == title && i.source_task == SETTINGS_CHANGE_SOURCE)
            .map(|i| i.body.clone())
            .unwrap()
    };
    let key = body("A key was changed");
    assert!(key.starts_with("HQ uses a new key."), "{key}");
    assert!(key.contains("\"Work laptop\""), "{key}");
    assert!(key.contains("sign that device out"), "{key}");
    assert!(body("A bot was connected").contains("the original login link"));
}

/// The guard sits inside CORS and in front of every /api route, but never
/// in front of /health, which the deploy rollback check polls.
#[tokio::test]
async fn token_guards_api_but_not_health_or_preflight() {
    let vault = tempfile::TempDir::new().unwrap();
    let mut state = WsState::new(vault.path().to_path_buf(), None);
    state.web_auth_token = Some("s3cret".to_string());
    let app = create_router(Arc::new(state));

    let get = |uri: &str| Request::get(uri).body(Body::empty()).unwrap();
    assert_eq!(
        status(&app, get("/api/spaces")).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(status(&app, get("/health")).await, StatusCode::OK);
    let authed = Request::get("/api/spaces")
        .header("authorization", "Bearer s3cret")
        .body(Body::empty())
        .unwrap();
    assert_ne!(status(&app, authed).await, StatusCode::UNAUTHORIZED);
    let preflight = |origin: &str| {
        Request::options("/api/spaces")
            .header("host", "127.0.0.1:5678")
            .header("origin", origin)
            .header("access-control-request-method", "GET")
            .body(Body::empty())
            .unwrap()
    };
    let own = app
        .clone()
        .oneshot(preflight("http://127.0.0.1:5678"))
        .await
        .unwrap();
    assert_eq!(own.status(), StatusCode::OK);
    assert_eq!(
        own.headers()["access-control-allow-origin"],
        "http://127.0.0.1:5678"
    );
    for refused in ["https://evil.example", "http://localhost:5173"] {
        let hostile = app.clone().oneshot(preflight(refused)).await.unwrap();
        assert!(
            !hostile
                .headers()
                .contains_key("access-control-allow-origin"),
            "{refused} must not be granted read access"
        );
    }
}

/// The default install: sign-in off, loopback bind, nothing in `web_allowed_origins`. A page
/// on another local port (a dev server, a notebook, a downloaded file served with
/// `python -m http.server`) can neither open the chat socket nor read or write the API.
/// HQ's own page and clients that send no `Origin` keep working.
#[tokio::test]
async fn another_local_port_cannot_use_an_unauthenticated_instance() {
    let vault = tempfile::TempDir::new().unwrap();
    let mut state = WsState::new(vault.path().to_path_buf(), None);
    state.allowed_origins = Vec::new();
    let app = create_router(Arc::new(state));
    let socket = |origin: Option<&str>| {
        let mut b = local("GET", "/ws")
            .header("connection", "upgrade")
            .header("upgrade", "websocket")
            .header("sec-websocket-version", "13")
            // RFC 6455's sample key ("the sample nonce"), not a secret.
            .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ=="); // gitleaks:allow
        if let Some(origin) = origin {
            b = b.header("origin", origin);
        }
        b.body(Body::empty()).unwrap()
    };
    let create = |origin: Option<&str>, title: &str| {
        let mut b = local("POST", "/api/note/create").header("content-type", "application/json");
        if let Some(origin) = origin {
            b = b.header("origin", origin);
        }
        b.body(Body::from(format!(r#"{{"title":"{title}","content":""}}"#)))
            .unwrap()
    };

    for foreign in ["http://localhost:3000", "http://127.0.0.1:8888"] {
        assert_eq!(
            status(&app, socket(Some(foreign))).await,
            StatusCode::FORBIDDEN,
            "{foreign} opening /ws"
        );
        assert_eq!(
            status(&app, create(Some(foreign), "pwned")).await,
            StatusCode::FORBIDDEN,
            "{foreign} posting to /api"
        );
    }
    assert!(!vault.path().join("Notebooks/Inbox/pwned.md").exists());

    // Without a real connection the upgrade cannot complete, but it got past the guards.
    for origin in [Some("http://127.0.0.1:5678"), None] {
        assert_ne!(
            status(&app, socket(origin)).await,
            StatusCode::FORBIDDEN,
            "{origin:?} opening /ws"
        );
    }
    assert!(
        status(&app, create(Some("http://127.0.0.1:5678"), "own"))
            .await
            .is_success()
    );
    assert!(status(&app, create(None, "cli")).await.is_success());
}

fn local(method: &str, uri: &str) -> axum::http::request::Builder {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("host", "127.0.0.1:5678")
}

/// Unauthenticated loopback mode: a hostile page can neither change state
/// nor open the chat socket, and a rebound DNS name is refused outright.
#[tokio::test]
async fn hostile_pages_cannot_use_an_unauthenticated_instance() {
    let vault = tempfile::TempDir::new().unwrap();
    let mut state = WsState::new(vault.path().to_path_buf(), None);
    state.allowed_origins = Vec::new();
    let app = create_router(Arc::new(state));

    let post = local("POST", "/api/note/create")
        .header("origin", "https://evil.example")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"title":"pwned","content":""}"#))
        .unwrap();
    assert_eq!(status(&app, post).await, StatusCode::FORBIDDEN);
    assert!(!vault.path().join("Notebooks/Inbox/pwned.md").exists());

    let socket = local("GET", "/ws")
        .header("origin", "https://evil.example")
        .body(Body::empty())
        .unwrap();
    assert_eq!(status(&app, socket).await, StatusCode::FORBIDDEN);

    let rebound = Request::get("/api/spaces")
        .header("host", "rebind.evil.example:5678")
        .body(Body::empty())
        .unwrap();
    assert_eq!(status(&app, rebound).await, StatusCode::FORBIDDEN);

    let own = local("GET", "/api/spaces")
        .header("origin", "http://127.0.0.1:5678")
        .body(Body::empty())
        .unwrap();
    assert_eq!(status(&app, own).await, StatusCode::OK);
}

#[tokio::test]
async fn a_resent_note_or_task_with_its_client_id_is_made_once() {
    let vault = tempfile::TempDir::new().unwrap();
    let app = create_router(Arc::new(WsState::new(vault.path().to_path_buf(), None)));
    let post = |path: &str, body: &str| {
        local("POST", path)
            .header("content-type", "application/json")
            .header("x-hq-client", "web")
            .body(Body::from(body.to_string()))
            .unwrap()
    };
    let note = r#"{"title":"Title","content":"x","client_id":"c-note-1"}"#;
    let first = json_of(&app, post("/api/note/create", note)).await;
    let again = json_of(&app, post("/api/note/create", note)).await;
    assert_eq!(first["path"], again["path"]);
    assert_eq!(again["deduplicated"], true);
    assert!(!vault.path().join("Notebooks/Inbox/Title 2.md").exists());

    let task = r#"{"title":"Buy milk","client_id":"c-task-1"}"#;
    let first = json_of(&app, post("/api/tasks", task)).await;
    let again = json_of(&app, post("/api/tasks", task)).await;
    assert_eq!(first["id"], again["id"]);
    assert_eq!(again["deduplicated"], true);
}

async fn json_of(app: &Router, req: Request<Body>) -> serde_json::Value {
    let res = app.clone().oneshot(req).await.unwrap();
    assert!(res.status().is_success(), "{}", res.status());
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn hello_answers_a_signed_out_browser_and_the_rest_of_the_api_does_not() {
    let vault = tempfile::TempDir::new().unwrap();
    let mut state = WsState::new(vault.path().to_path_buf(), None);
    state.web_auth_token = Some("t".repeat(32));
    let app = create_router(Arc::new(state));
    let get = |path: &str| local("GET", path).body(Body::empty()).unwrap();
    assert_eq!(status(&app, get(hello::HELLO_PATH)).await, StatusCode::OK);
    for refused in ["/api/spaces", "/api/status/ai", "/api/status/busy"] {
        assert_eq!(
            status(&app, get(refused)).await,
            StatusCode::UNAUTHORIZED,
            "{refused}"
        );
    }
}

/// A client holding the web token can check, signed out and without sending it, that the
/// server on the port holds the same token.
#[tokio::test]
async fn hello_proves_it_holds_the_web_token_without_a_sign_in() {
    let vault = tempfile::TempDir::new().unwrap();
    let token = "t".repeat(32);
    let mut state = WsState::new(vault.path().to_path_buf(), None);
    state.web_auth_token = Some(token.clone());
    let app = create_router(Arc::new(state));
    let nonce = hq_core::server_proof::new_nonce();
    let hello = local("GET", &format!("{}?proof={nonce}", hello::HELLO_PATH))
        .body(Body::empty())
        .unwrap();
    let v = json_of(&app, hello).await;
    let proof = v["proof"].as_str().expect("an answer");
    assert!(hq_core::server_proof::verify(&token, &nonce, proof));
    assert!(!v.to_string().contains(&token), "the token never leaves");

    let bad = local("GET", &format!("{}?proof=short", hello::HELLO_PATH))
        .body(Body::empty())
        .unwrap();
    assert_eq!(status(&app, bad).await, StatusCode::BAD_REQUEST);
}

/// Session-changing requests also need a header no other page can add without a failed
/// CORS preflight, a second wall behind the origin check.
#[tokio::test]
async fn session_posts_need_the_client_header() {
    let vault = tempfile::TempDir::new().unwrap();
    let app = create_router(Arc::new(WsState::new(vault.path().to_path_buf(), None)));
    let post = |path: &str, header: bool| {
        let mut b = local("POST", path)
            .header("origin", "http://127.0.0.1:5678")
            .header("content-type", "application/json");
        if header {
            b = b.header("x-hq-client", "web");
        }
        b.body(Body::from("{}")).unwrap()
    };
    let guarded = [
        "send", "adopt", "stop", "resume", "rename", "archive", "drive", "goal", "unwatch",
        "answer",
    ];
    let paths = guarded
        .iter()
        .map(|action| format!("/api/harness-sessions/hs-none/{action}"))
        .chain(["answer", "action"].map(|a| format!("/api/notifications/val_x/{a}")));
    for path in paths {
        assert_eq!(
            status(&app, post(&path, false)).await,
            StatusCode::FORBIDDEN,
            "{path}"
        );
        assert_ne!(
            status(&app, post(&path, true)).await,
            StatusCode::FORBIDDEN,
            "{path}"
        );
    }
    assert_eq!(
        status(&app, post("/api/notifications/push-answer", true)).await,
        StatusCode::NOT_FOUND,
        "a push can no longer answer anything"
    );
    for path in ["/api/harness-sessions", "/api/workbench/hosts/native/dirs"] {
        assert_eq!(
            status(&app, post(path, false)).await,
            StatusCode::FORBIDDEN,
            "{path}"
        );
        assert_ne!(
            status(&app, post(path, true)).await,
            StatusCode::FORBIDDEN,
            "{path}"
        );
    }
    let preflight = Request::options("/api/harness-sessions/hs-none/send")
        .header("origin", "http://127.0.0.1:5678")
        .header("host", "127.0.0.1:5678")
        .header("access-control-request-method", "POST")
        .header("access-control-request-headers", "x-hq-client,content-type")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(preflight).await.unwrap();
    let allowed = res
        .headers()
        .get("access-control-allow-headers")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        !allowed.to_ascii_lowercase().contains("x-hq-client"),
        "CORS must not grant the header: {allowed}"
    );
}

/// URL tokens are refused; the socket opens with a one-time ticket that was
/// minted over the Authorization header.
#[tokio::test]
async fn socket_uses_single_use_tickets_not_url_tokens() {
    let vault = tempfile::TempDir::new().unwrap();
    let mut state = WsState::new(vault.path().to_path_buf(), None);
    state.web_auth_token = Some("s3cret".to_string());
    let state = Arc::new(state);
    let app = create_router(state.clone());

    let url_token = local("GET", "/api/spaces?token=s3cret")
        .body(Body::empty())
        .unwrap();
    assert_eq!(status(&app, url_token).await, StatusCode::UNAUTHORIZED);
    let unauthed_mint = local("POST", "/api/ws-ticket").body(Body::empty()).unwrap();
    assert_eq!(status(&app, unauthed_mint).await, StatusCode::UNAUTHORIZED);

    let mint = local("POST", "/api/ws-ticket")
        .header("authorization", "Bearer s3cret")
        .body(Body::empty())
        .unwrap();
    let res = app.clone().oneshot(mint).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = axum::body::to_bytes(res.into_body(), 4096).await.unwrap();
    let ticket = serde_json::from_slice::<serde_json::Value>(&body).unwrap()["ticket"]
        .as_str()
        .unwrap()
        .to_string();

    let with_ticket = |t: &str| {
        local("GET", &format!("/ws?ticket={t}"))
            .body(Body::empty())
            .unwrap()
    };
    // A plain GET is not an upgrade, so the handler rejects it, but it got past auth.
    assert_ne!(
        status(&app, with_ticket(&ticket)).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        status(&app, with_ticket(&ticket)).await,
        StatusCode::UNAUTHORIZED,
        "replayed"
    );
    let ticket_elsewhere = local(
        "GET",
        &format!("/api/spaces?ticket={}", state.ws_tickets.mint()),
    )
    .body(Body::empty())
    .unwrap();
    assert_eq!(
        status(&app, ticket_elsewhere).await,
        StatusCode::UNAUTHORIZED
    );
}

/// Client routes get the PWA shell (never cached); unknown API paths 404 as JSON.
#[tokio::test]
async fn spa_fallback_serves_shell_but_not_for_api_paths() {
    let vault = tempfile::TempDir::new().unwrap();
    let web = tempfile::TempDir::new().unwrap();
    std::fs::write(web.path().join("index.html"), "<html>shell</html>").unwrap();
    std::fs::write(web.path().join("sw.js"), "//sw").unwrap();
    std::fs::create_dir_all(web.path().join("assets")).unwrap();
    std::fs::write(web.path().join("assets/app-1.js"), "//app ".repeat(200)).unwrap();
    let state = WsState::new(vault.path().to_path_buf(), Some(web.path().to_path_buf()));
    let app = create_router(Arc::new(state));
    let get = |uri: &str| Request::get(uri).body(Body::empty()).unwrap();

    for uri in ["/", "/vault/Notebooks/a.md", "/tasks/123"] {
        let res = app.clone().oneshot(get(uri)).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK, "{uri}");
        assert_eq!(res.headers()["cache-control"], "no-cache", "{uri}");
    }
    let gz = app
        .clone()
        .oneshot(
            Request::get("/assets/app-1.js")
                .header("accept-encoding", "gzip")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        gz.headers()["content-encoding"],
        "gzip",
        "static assets are compressed"
    );
    let sw = app.clone().oneshot(get("/sw.js")).await.unwrap();
    assert_eq!(sw.headers()["cache-control"], "no-cache");
    let asset = app.clone().oneshot(get("/assets/app-1.js")).await.unwrap();
    assert_eq!(asset.status(), StatusCode::OK);
    let chunk = app
        .clone()
        .oneshot(get("/assets/gone-123.js"))
        .await
        .unwrap();
    assert_eq!(chunk.status(), StatusCode::NOT_FOUND);
    for uri in [
        "/api/nope",
        "/ws/nope",
        "/mcp/nope",
        "/hooks/nope",
        "/.well-known/oauth-protected-resource",
    ] {
        let res = app.clone().oneshot(get(uri)).await.unwrap();
        assert_eq!(res.status(), StatusCode::NOT_FOUND, "{uri}");
        assert!(
            res.headers()["content-type"]
                .to_str()
                .unwrap()
                .starts_with("application/json"),
            "{uri}"
        );
    }
}

/// Every vault endpoint that takes a client path refuses to leave the vault.
#[tokio::test]
async fn vault_endpoints_reject_traversal() {
    let vault = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(vault.path().join("Notebooks")).unwrap();
    let app = create_router(Arc::new(WsState::new(vault.path().to_path_buf(), None)));
    let json_req = |method: &str, uri: &str, body: &str| {
        Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    };
    let get = |uri: &str| Request::get(uri).body(Body::empty()).unwrap();
    for req in [
        get("/api/tree?recursive=true&path=../"),
        get("/api/note?path=../etc/passwd"),
        get("/api/note/pdf?path=../etc/passwd"),
        get("/api/note/export?path=../etc/passwd&format=html"),
        json_req(
            "POST",
            "/api/note/create",
            r#"{"folder":"../x","title":"t","content":""}"#,
        ),
        json_req("PUT", "/api/note", r#"{"path":"../x.md","content":""}"#),
        json_req("PUT", "/api/note", r#"{"path":"/etc/hosts","content":""}"#),
    ] {
        let uri = req.uri().to_string();
        assert_eq!(status(&app, req).await, StatusCode::BAD_REQUEST, "{uri}");
    }
    let created = json_req(
        "POST",
        "/api/note/create",
        r#"{"title":"Hello","content":"hi"}"#,
    );
    assert_eq!(status(&app, created).await, StatusCode::OK);
    assert!(vault.path().join("Notebooks/Inbox/Hello.md").exists());
}

#[tokio::test]
async fn note_pdf_endpoint_validates_its_input() {
    let vault = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(vault.path().join("Notebooks")).unwrap();
    std::fs::write(vault.path().join("Notebooks/pic.png"), b"x").unwrap();
    let app = create_router(Arc::new(WsState::new(vault.path().to_path_buf(), None)));
    let get = |uri: &str| Request::get(uri).body(Body::empty()).unwrap();
    assert_eq!(
        status(&app, get("/api/note/pdf")).await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        status(&app, get("/api/note/pdf?path=Missing")).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        status(&app, get("/api/note/pdf?path=Notebooks/pic.png")).await,
        StatusCode::BAD_REQUEST
    );
}

async fn export_vault() -> (tempfile::TempDir, Router) {
    let vault = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(vault.path().join("Notebooks")).unwrap();
    std::fs::write(
        vault.path().join("Notebooks/Plan.md"),
        "---\ntitle: Plan\n---\nHello\n\n## Sites\n\n| a | b |\n|---|---|\n| 1 | 2 |\n",
    )
    .unwrap();
    std::fs::write(vault.path().join("Notebooks/Prose.md"), "Just words.\n").unwrap();
    std::fs::write(vault.path().join("Notebooks/pic.png"), b"x").unwrap();
    let app = create_router(Arc::new(WsState::new(vault.path().to_path_buf(), None)));
    (vault, app)
}

#[tokio::test]
async fn note_export_endpoint_validates_its_input() {
    let (_vault, app) = export_vault().await;
    let get = |uri: &str| Request::get(uri).body(Body::empty()).unwrap();
    assert_eq!(
        status(&app, get("/api/note/export")).await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        status(&app, get("/api/note/export?path=Missing&format=html")).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        status(&app, get("/api/note/export?path=Plan&format=docx-nope")).await,
        StatusCode::BAD_REQUEST,
        "an unknown format is the caller's mistake"
    );
    assert_eq!(
        status(
            &app,
            get("/api/note/export?path=Notebooks/pic.png&format=html")
        )
        .await,
        StatusCode::BAD_REQUEST
    );
    // A note that has nothing of the requested kind is a 400, not a 500.
    for format in ["csv", "xlsx", "json", "code"] {
        let uri = format!("/api/note/export?path=Prose&format={format}");
        assert_eq!(
            status(&app, get(&uri)).await,
            StatusCode::BAD_REQUEST,
            "{uri}"
        );
    }
}

#[tokio::test]
async fn note_export_endpoint_serves_each_format_with_matching_headers() {
    let (_vault, app) = export_vault().await;
    for (format, mime, ext) in [
        ("html", "text/html", "html"),
        ("md", "text/markdown", "md"),
        ("csv", "text/csv", "csv"),
        ("json", "application/json", "json"),
        (
            "xlsx",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            "xlsx",
        ),
        ("ipynb", "application/x-ipynb+json", "ipynb"),
    ] {
        let uri = format!("/api/note/export?path=Plan&format={format}");
        let res = app
            .clone()
            .oneshot(Request::get(&uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK, "{uri}");
        assert!(
            res.headers()["content-type"]
                .to_str()
                .unwrap()
                .starts_with(mime),
            "{uri}"
        );
        let disposition = res.headers()["content-disposition"]
            .to_str()
            .unwrap()
            .to_owned();
        assert!(
            disposition.contains(&format!("Plan.{ext}")),
            "{uri}: {disposition}"
        );
        // A router layer may rewrite no-store to no-cache; either keeps a
        // private note export out of shared caches.
        let cache = res.headers()["cache-control"].to_str().unwrap().to_owned();
        assert!(
            cache.contains("no-store") || cache.contains("no-cache"),
            "{uri}: {cache}"
        );
        let body = axum::body::to_bytes(res.into_body(), 16 * 1024 * 1024)
            .await
            .unwrap();
        assert!(!body.is_empty(), "{uri}");
    }
}

/// The built-in renderer needs no external tool, so this is a normal test.
/// It steps aside when a developer has forced one of the older engines.
#[tokio::test]
async fn note_pdf_endpoint_serves_a_pdf() {
    if std::env::var("HQ_PDF_ENGINE").is_ok_and(|v| !v.trim().is_empty()) {
        return;
    }
    let (_vault, app) = export_vault().await;
    for uri in [
        "/api/note/pdf?path=Plan",
        "/api/note/export?path=Plan&format=pdf",
        "/api/note/export?path=Plan",
    ] {
        let res = app
            .clone()
            .oneshot(Request::get(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK, "{uri}");
        assert_eq!(res.headers()["content-type"], "application/pdf", "{uri}");
        assert!(
            res.headers()["content-disposition"]
                .to_str()
                .unwrap()
                .contains("Plan.pdf")
        );
        let body = axum::body::to_bytes(res.into_body(), 16 * 1024 * 1024)
            .await
            .unwrap();
        assert!(body.starts_with(b"%PDF"), "{uri}");
    }
}
