use super::*;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::get;
use tower::ServiceExt;

const TG_TOKEN: &str = "123456789:AAExampleExampleExampleExample12345";
const TG_SECRET: &str = "AAExampleExampleExampleExample12345";
const DC_TOKEN: &str = "MTIzNDU2Nzg5MDEyMzQ1Njc4.GExmpl.ExampleExampleExampleExampleExample12";

fn no_env(_: &str) -> bool {
    false
}

fn vault() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

#[test]
fn token_shapes_are_checked_before_any_call() {
    assert!(verify::check_shape(Platform::Telegram, TG_TOKEN).is_ok());
    assert!(verify::check_shape(Platform::Discord, DC_TOKEN).is_ok());
    for bad in [
        "",
        "123456789",
        "abc:AAExampleExampleExampleExample12345",
        "123456789:short",
        "123456789:AAExample/ExampleExampleExample12345",
        "123456789:AAExampleExampleExampleExample12345 extra",
        DC_TOKEN,
    ] {
        assert!(
            verify::check_shape(Platform::Telegram, bad).is_err(),
            "{bad:?}"
        );
    }
    for bad in ["", "a.b.c", "only.two", TG_TOKEN, &format!("{DC_TOKEN}.x")] {
        assert!(
            verify::check_shape(Platform::Discord, bad).is_err(),
            "{bad:?}"
        );
    }
    assert_eq!(
        clean_token(Platform::Telegram, &format!("  {TG_TOKEN}\n")).unwrap(),
        TG_TOKEN
    );
}

#[test]
fn an_env_pinned_relay_refuses_web_writes_and_says_so() {
    let token_env = |n: &str| n.eq_ignore_ascii_case("HQ_RELAY__TELEGRAM_TOKEN");
    let err = refuse_if_env_locked(Platform::Telegram, &token_env).unwrap_err();
    assert_eq!(err.status(), StatusCode::CONFLICT);
    assert!(err.to_string().contains("HQ_RELAY__TELEGRAM_TOKEN"));
    let enabled_env = |n: &str| n == "HQ_RELAY__DISCORD_ENABLED";
    assert!(refuse_if_env_locked(Platform::Discord, &enabled_env).is_err());
    assert!(refuse_if_env_locked(Platform::Telegram, &enabled_env).is_ok());
    assert!(refuse_if_env_locked(Platform::Discord, &no_env).is_ok());

    let v = vault();
    let relay = RelayConfig::default();
    let status = platform_status(Platform::Telegram, &relay, None, v.path(), &token_env);
    assert_eq!(status["token_from_env"], true);
    assert_eq!(status["enabled_from_env"], false);
}

#[test]
fn save_sets_the_token_and_the_switch_and_keeps_the_rest() {
    let dir = vault();
    let path = dir.path().join("config.yaml");
    std::fs::write(
        &path,
        "default_model: my/model\nrelay:\n  discord_allowed_user_ids: [7]\n",
    )
    .unwrap();
    save_token_at(&path, Platform::Telegram, TG_TOKEN.into()).unwrap();
    let saved = HqConfig::load_from_path(&path).unwrap();
    assert_eq!(saved.relay.telegram_token.as_deref(), Some(TG_TOKEN));
    assert!(saved.relay.telegram_enabled);
    assert!(!saved.relay.discord_enabled);
    assert_eq!(saved.relay.discord_allowed_user_ids, vec![7]);
    assert_eq!(saved.default_model, "my/model");

    disconnect_at(&path, Platform::Telegram).unwrap();
    let off = HqConfig::load_from_path(&path).unwrap();
    assert_eq!(off.relay.telegram_token, None);
    assert!(!off.relay.telegram_enabled);
    assert_eq!(off.default_model, "my/model");
}

#[test]
fn status_reports_a_token_as_set_and_never_contains_it() {
    let v = vault();
    let relay = RelayConfig {
        telegram_token: Some(TG_TOKEN.into()),
        telegram_enabled: true,
        discord_token: Some(DC_TOKEN.into()),
        ..Default::default()
    };
    let verified = Verified {
        username: "example_bot".into(),
        application_id: None,
        message_content: None,
    };
    records::store_identity(v.path(), Platform::Telegram, TG_TOKEN, &verified);
    for platform in PLATFORMS {
        let status = platform_status(platform, &relay, None, v.path(), &no_env);
        let text = status.to_string();
        assert!(
            !text.contains(TG_SECRET) && !text.contains(DC_TOKEN),
            "{text}"
        );
        assert_eq!(status["token_set"], true);
    }
    let tg = platform_status(Platform::Telegram, &relay, None, v.path(), &no_env);
    assert_eq!(tg["bot"]["username"], "example_bot");
    let stored = std::fs::read_to_string(v.path().join("_system/chat-apps.json")).unwrap();
    assert!(!stored.contains(TG_SECRET));
}

#[test]
fn a_remembered_identity_only_matches_its_own_token() {
    let v = vault();
    let verified = Verified {
        username: "example_bot".into(),
        application_id: Some("42".into()),
        message_content: Some(true),
    };
    records::store_identity(v.path(), Platform::Discord, DC_TOKEN, &verified);
    assert!(records::load_identity(v.path(), Platform::Discord, DC_TOKEN).is_some());
    assert!(records::load_identity(v.path(), Platform::Discord, "other").is_none());
    assert!(records::load_identity(v.path(), Platform::Telegram, DC_TOKEN).is_none());
    records::forget(v.path(), Platform::Discord);
    assert!(records::load_identity(v.path(), Platform::Discord, DC_TOKEN).is_none());
}

#[test]
fn needs_restart_compares_the_running_config_with_the_file() {
    let v = vault();
    let running = RelayConfig::default();
    let saved = RelayConfig {
        telegram_token: Some(TG_TOKEN.into()),
        telegram_enabled: true,
        ..Default::default()
    };
    let status = platform_status(
        Platform::Telegram,
        &saved,
        Some(&running),
        v.path(),
        &no_env,
    );
    assert_eq!(status["needs_restart"], true);
    let same = platform_status(Platform::Telegram, &saved, Some(&saved), v.path(), &no_env);
    assert_eq!(same["needs_restart"], false);
    let discord = platform_status(Platform::Discord, &saved, Some(&running), v.path(), &no_env);
    assert_eq!(discord["needs_restart"], false);
}

#[test]
fn a_status_file_from_before_this_process_is_not_current() {
    let v = vault();
    let dir = v.path().join("_system/relay-status");
    std::fs::create_dir_all(&dir).unwrap();
    let started = hq_core::daemon_restart::process_started_at();
    let write = |at: i64| {
        let seen = chrono::DateTime::from_timestamp(at, 0)
            .unwrap()
            .to_rfc3339();
        let body = json!({"status": "connected", "last_seen": seen, "detail": null, "error_message": null});
        std::fs::write(dir.join("telegram.json"), body.to_string()).unwrap();
    };
    write(started - 3600);
    let old = records::relay_status(v.path(), Platform::Telegram, started);
    assert_eq!(old["current"], false);
    write(started + 1);
    let fresh = records::relay_status(v.path(), Platform::Telegram, started);
    assert_eq!(fresh["current"], true);
    assert_eq!(fresh["status"], "connected");
    assert!(records::relay_status(v.path(), Platform::Discord, started).is_null());
}

#[test]
fn pairing_issues_a_code_and_refuses_to_silently_replace_an_owner() {
    let v = vault();
    let relay = RelayConfig::default();
    let code = issue_pairing_code(Platform::Telegram, &relay, v.path(), false, 0).unwrap();
    assert_eq!(code.len(), 11);
    assert!(pairing::has_pending_code(
        v.path(),
        PairPlatform::Telegram,
        1
    ));

    std::fs::write(hq_core::telegram_access::owner_file(v.path()), "77").unwrap();
    let err = issue_pairing_code(Platform::Telegram, &relay, v.path(), false, 2).unwrap_err();
    assert_eq!(err.status(), StatusCode::CONFLICT);
    assert_eq!(hq_core::telegram_access::paired_owner(v.path()), Some(77));

    let fresh = issue_pairing_code(Platform::Telegram, &relay, v.path(), true, 3).unwrap();
    assert_eq!(hq_core::telegram_access::paired_owner(v.path()), None);
    assert_eq!(
        pairing::redeem_pairing_code(v.path(), PairPlatform::Telegram, &fresh, 4),
        Ok(())
    );
}

#[test]
fn discord_pairing_replaces_its_owner_file_too() {
    let v = vault();
    let relay = RelayConfig::default();
    std::fs::create_dir_all(v.path().join("_system")).unwrap();
    std::fs::write(hq_core::discord_access::owner_file(v.path()), "5").unwrap();
    assert!(issue_pairing_code(Platform::Discord, &relay, v.path(), false, 0).is_err());
    issue_pairing_code(Platform::Discord, &relay, v.path(), true, 0).unwrap();
    assert_eq!(hq_core::discord_access::paired_owner(v.path()), None);
}

#[test]
fn pairing_is_refused_when_the_config_decides_who_the_owner_is() {
    let v = vault();
    let tg = RelayConfig {
        telegram_authorized_chat_id: Some(1),
        ..Default::default()
    };
    let err = issue_pairing_code(Platform::Telegram, &tg, v.path(), true, 0).unwrap_err();
    assert_eq!(err.status(), StatusCode::CONFLICT);
    let dc = RelayConfig {
        discord_allowed_user_ids: vec![9],
        ..Default::default()
    };
    assert!(issue_pairing_code(Platform::Discord, &dc, v.path(), true, 0).is_err());
    assert!(!pairing::has_pending_code(
        v.path(),
        PairPlatform::Telegram,
        0
    ));
    assert!(!pairing::has_pending_code(
        v.path(),
        PairPlatform::Discord,
        0
    ));
    let owner = owner_json(Platform::Discord, &dc, v.path());
    assert_eq!(owner["from_config"], true);
}

/// A stand-in for the Telegram and Discord APIs on a loopback port.
async fn fake_api(app: axum::Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    base
}

#[tokio::test]
async fn telegram_check_reads_the_username_and_hides_the_token_on_refusal() {
    let ok = fake_api(axum::Router::new().route(
        &format!("/bot{TG_TOKEN}/getMe"),
        get(|| async {
            Json(
                json!({"ok": true, "result": {"id": 1, "is_bot": true, "username": "example_bot"}}),
            )
        }),
    ))
    .await;
    let v = verify::verify(Platform::Telegram, &ok, TG_TOKEN)
        .await
        .unwrap();
    assert_eq!(v.username, "example_bot");

    let refused = fake_api(axum::Router::new().fallback(|| async {
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({"ok": false, "error_code": 401})),
        )
    }))
    .await;
    let err = verify::verify(Platform::Telegram, &refused, TG_TOKEN)
        .await
        .unwrap_err();
    assert!(err.contains("did not accept"), "{err}");
    assert!(!err.contains(TG_SECRET));

    let down = verify::verify(Platform::Telegram, "http://127.0.0.1:1", TG_TOKEN)
        .await
        .unwrap_err();
    assert!(down.contains("could not reach"), "{down}");
    assert!(!down.contains(TG_SECRET), "{down}");
}

#[tokio::test]
async fn discord_check_reads_the_application_id_and_the_intent_flag() {
    let with_intent = |flags: u64| {
        axum::Router::new()
            .route(
                "/users/@me",
                get(|| async { Json(json!({"id": "42", "username": "example", "bot": true})) }),
            )
            .route(
                "/oauth2/applications/@me",
                get(move || async move { Json(json!({"id": "4242", "flags": flags})) }),
            )
    };
    let on = fake_api(with_intent(1 << 19)).await;
    let v = verify::verify(Platform::Discord, &on, DC_TOKEN)
        .await
        .unwrap();
    assert_eq!(v.username, "example");
    assert_eq!(v.application_id.as_deref(), Some("4242"));
    assert_eq!(v.message_content, Some(true));

    let off = fake_api(with_intent(0)).await;
    let v = verify::verify(Platform::Discord, &off, DC_TOKEN)
        .await
        .unwrap();
    assert_eq!(v.message_content, Some(false));
}

fn router_without_web_token(vault: &Path) -> axum::Router {
    let mut state = WsState::new(vault.to_path_buf(), None);
    state.web_auth_token = None;
    crate::create_router(Arc::new(state))
}

fn write_request(method: &str, path: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json")
        .header("x-hq-client", "web")
        .body(Body::from(body.to_string()))
        .unwrap()
}

#[tokio::test]
async fn every_write_needs_a_web_token() {
    let v = vault();
    let app = router_without_web_token(v.path());
    let token_body = format!(r#"{{"token":"{TG_TOKEN}"}}"#);
    for (method, path, body) in [
        ("POST", "/api/chat-apps/telegram", token_body.as_str()),
        (
            "POST",
            "/api/chat-apps/telegram/verify",
            token_body.as_str(),
        ),
        ("DELETE", "/api/chat-apps/discord", ""),
        ("POST", "/api/chat-apps/discord/pair", "{}"),
        ("POST", "/api/system/restart", ""),
    ] {
        let res = app
            .clone()
            .oneshot(write_request(method, path, body))
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN, "{method} {path}");
    }
    assert!(!pairing::has_pending_code(
        v.path(),
        PairPlatform::Discord,
        0
    ));
    let unknown = app
        .oneshot(write_request("POST", "/api/chat-apps/slack/pair", "{}"))
        .await
        .unwrap();
    assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
}
