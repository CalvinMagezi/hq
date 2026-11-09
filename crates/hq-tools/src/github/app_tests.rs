use super::*;
use ring::signature::{KeyPair, RSA_PKCS1_2048_8192_SHA256, UnparsedPublicKey};
use tempfile::TempDir;
use wiremock::matchers::{body_json, header_exists, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A throwaway key made for these tests; it signs in nowhere.
const TEST_KEY: &str = include_str!("testdata/test_app_key.pem");
const CLIENT_ID: &str = "Iv-test-client";
const OWNER: &str = "test-owner";
const REPO: &str = "test-owner/test-repo";
const INSTALLATION: u64 = 77;
const TOKEN_PATH: &str = "/app/installations/77/access_tokens";

fn record() -> AppRecord {
    AppRecord {
        app_id: 5,
        client_id: CLIENT_ID.into(),
        slug: "hq-coder-abc".into(),
        owner_login: OWNER.into(),
        owner_id: 42,
        owner_name: None,
        installation_id: Some(INSTALLATION),
        repository_selection: Some("selected".into()),
        needs_reconnect: false,
    }
}

struct Fixture {
    _dir: TempDir,
    server: MockServer,
    app: AppClient,
}

async fn fixture(connected: Option<AppRecord>) -> Fixture {
    let dir = TempDir::new().unwrap();
    let store = Store::at(dir.path().join("github"));
    if let Some(record) = connected {
        store.save_key(TEST_KEY).unwrap();
        store.save_record(&record).unwrap();
    }
    let server = MockServer::start().await;
    let app = AppClient::new(store, &format!("{}/", server.uri())).unwrap();
    Fixture {
        _dir: dir,
        server,
        app,
    }
}

fn refused(err: &anyhow::Error) -> Option<&TokenRefused> {
    err.downcast_ref::<TokenRefused>()
}

#[test]
fn the_manifest_asks_for_exactly_the_planned_permissions_and_no_webhook() {
    let made = manifest("http://127.0.0.1:5678");
    assert_eq!(
        made["default_permissions"],
        json!({"contents": "write", "pull_requests": "write", "actions": "read",
               "checks": "read", "statuses": "read", "metadata": "read"})
    );
    assert_eq!(made["public"], false);
    assert_eq!(made["default_events"], json!([]));
    assert!(made.get("hook_attributes").is_none());
    assert_eq!(
        made["redirect_url"],
        "http://127.0.0.1:5678/github/connect/return"
    );
}

#[test]
fn a_state_works_once_and_a_made_up_one_never() {
    let flows = Flows::default();
    let state = flows.begin();
    assert!(!flows.take("not-a-state"));
    assert!(flows.is_pending(&state));
    assert!(flows.take(&state));
    assert!(!flows.take(&state));
    assert!(!flows.is_pending(&state));
}

#[test]
fn states_are_long_and_different() {
    let flows = Flows::default();
    let (a, b) = (flows.begin(), flows.begin());
    assert_ne!(a, b);
    assert!(a.len() >= 43, "{a}");
}

#[test]
fn an_expired_state_is_refused() {
    let flows = Flows::default();
    let state = flows.begin();
    let past = Instant::now() - FLOW_TTL - Duration::from_secs(1);
    flows.0.lock().unwrap().insert(state.clone(), past);
    assert!(!flows.is_pending(&state));
    assert!(!flows.take(&state));
}

#[test]
fn pending_flows_are_capped() {
    let flows = Flows::default();
    for _ in 0..MAX_PENDING_FLOWS * 3 {
        flows.begin();
    }
    assert_eq!(flows.0.lock().unwrap().len(), MAX_PENDING_FLOWS);
}

#[test]
fn the_start_page_escapes_its_values_and_limits_where_it_can_post() {
    let page = start_page("a\"b<c", "http://127.0.0.1:5678");
    assert!(page.html.contains("state=a&quot;b&lt;c"), "{}", page.html);
    assert!(!page.html.contains("a\"b<c"));
    assert!(page.csp.contains("form-action https://github.com"));
    assert!(!page.csp.contains("unsafe-inline"));
}

#[test]
fn the_jwt_is_signed_by_the_key_and_short_lived() {
    const NOW: u64 = 2_000_000_000;
    let jwt = app_jwt(TEST_KEY, CLIENT_ID, NOW).unwrap();
    let parts: Vec<&str> = jwt.split('.').collect();
    assert_eq!(parts.len(), 3);
    let public = key_pair(TEST_KEY).unwrap().public_key().as_ref().to_vec();
    let signature = URL_SAFE_NO_PAD.decode(parts[2]).unwrap();
    UnparsedPublicKey::new(&RSA_PKCS1_2048_8192_SHA256, public)
        .verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &signature)
        .expect("the signature verifies");
    let claims: Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[1]).unwrap()).unwrap();
    assert_eq!(claims["iss"], CLIENT_ID);
    assert_eq!(claims["iat"], NOW - JWT_BACKDATE_SECS);
    assert_eq!(claims["exp"], NOW + JWT_LIFETIME_SECS);
    const { assert!(JWT_BACKDATE_SECS + JWT_LIFETIME_SECS <= 600) };
}

#[test]
fn a_key_that_is_not_rsa_pem_is_refused() {
    assert!(key_pair("").is_err());
    assert!(
        key_pair("-----BEGIN EC PRIVATE KEY-----\nAAAA\n-----END EC PRIVATE KEY-----").is_err()
    );
    assert!(
        key_pair("-----BEGIN RSA PRIVATE KEY-----\n!!!\n-----END RSA PRIVATE KEY-----").is_err()
    );
}

#[tokio::test]
async fn a_return_without_a_matching_state_never_reaches_github() {
    let f = fixture(None).await;
    let flows = Flows::default();
    flows.begin();
    assert!(f.app.finish(&flows, "made-up", "good-code").await.is_err());
    assert!(f.server.received_requests().await.unwrap().is_empty());
    assert!(f.app.store().load().is_none());
}

#[tokio::test]
async fn a_replayed_return_is_refused() {
    let f = fixture(None).await;
    conversion_ok(&f.server, "good-code", "hq-coder-abc").await;
    let flows = Flows::default();
    let state = flows.begin();
    f.app.finish(&flows, &state, "good-code").await.unwrap();
    assert!(f.app.finish(&flows, &state, "good-code").await.is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn the_key_and_record_are_private_files_and_disconnect_removes_them() {
    use std::os::unix::fs::PermissionsExt;
    let f = fixture(Some(record())).await;
    let root = &f.app.store().root;
    for name in [KEY_FILE, RECORD_FILE] {
        let mode = std::fs::metadata(root.join(name))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, PRIVATE_FILE_MODE, "{name}");
    }
    let dir_mode = std::fs::metadata(root).unwrap().permissions().mode();
    assert_eq!(dir_mode & 0o777, PRIVATE_DIR_MODE);
    let settings = f.app.disconnect().unwrap();
    assert_eq!(
        settings.as_deref(),
        Some("https://github.com/settings/apps/hq-coder-abc")
    );
    assert!(!root.join(KEY_FILE).exists());
    assert!(!root.join(RECORD_FILE).exists());
    assert_eq!(
        standing(f.app.store().load().as_ref()),
        Standing::NotConnected
    );
}

#[tokio::test]
async fn the_record_holds_no_secret() {
    let f = fixture(None).await;
    conversion_ok(&f.server, "good-code", "hq-coder-abc").await;
    f.app.exchange("good-code").await.unwrap();
    let saved = std::fs::read_to_string(f.app.store().root.join(RECORD_FILE)).unwrap();
    assert!(!saved.contains("PRIVATE KEY"));
    assert!(!saved.contains("the-client-secret"));
}

async fn conversion_ok(server: &MockServer, code: &str, slug: &str) {
    let body = json!({
        "id": 5, "client_id": CLIENT_ID, "slug": slug, "pem": TEST_KEY,
        "client_secret": "the-client-secret", "webhook_secret": null,
        "owner": {"login": OWNER, "id": 42, "type": "User"},
    });
    Mock::given(method("POST"))
        .and(path(format!("/app-manifests/{code}/conversions")))
        .respond_with(ResponseTemplate::new(201).set_body_json(body))
        .expect(1)
        .mount(server)
        .await;
}

#[tokio::test]
async fn exchanging_a_code_stores_the_app() {
    let f = fixture(None).await;
    conversion_ok(&f.server, "good-code", "hq-coder-abc").await;
    Mock::given(method("GET"))
        .and(path(format!("/users/{OWNER}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"name": "Test Person"})))
        .mount(&f.server)
        .await;
    let made = f.app.exchange("good-code").await.unwrap();
    assert_eq!(made.slug, "hq-coder-abc");
    assert_eq!(standing(Some(&made)), Standing::NeedsInstall);
    assert_eq!(
        made.install_url(),
        "https://github.com/apps/hq-coder-abc/installations/new"
    );
    assert_eq!(
        made.author(),
        json!({"name": "Test Person", "email": "42+test-owner@users.noreply.github.com"})
    );
    assert_eq!(f.app.store().load(), Some(made));
}

#[tokio::test]
async fn a_malformed_code_never_reaches_github() {
    let f = fixture(None).await;
    for code in ["", "a/b", "../x", "a b"] {
        assert!(f.app.exchange(code).await.is_err(), "{code}");
    }
    assert!(f.server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_refused_code_stores_nothing() {
    let f = fixture(None).await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&f.server)
        .await;
    assert!(f.app.exchange("spent-code").await.is_err());
    assert!(f.app.store().load().is_none());
}

#[tokio::test]
async fn a_slug_hq_cannot_link_to_stores_nothing() {
    let f = fixture(None).await;
    conversion_ok(&f.server, "good-code", "../../evil").await;
    assert!(f.app.exchange("good-code").await.is_err());
    assert!(f.app.store().load().is_none());
}

async fn token_ok(server: &MockServer, step: Step) {
    let wanted = json!({"repositories": ["test-repo"], "permissions": step.permissions()});
    Mock::given(method("POST"))
        .and(path(TOKEN_PATH))
        .and(header_exists("authorization"))
        .and(body_json(wanted))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"token": "ghs_test"})))
        .expect(1)
        .mount(server)
        .await;
}

#[tokio::test]
async fn a_token_covers_one_repository_and_only_the_steps_permissions() {
    for step in [Step::Fetch, Step::Publish, Step::ReadCi] {
        let f = fixture(Some(record())).await;
        token_ok(&f.server, step).await;
        let token = f.app.token(REPO, step).await.unwrap();
        assert_eq!(token.secret(), "ghs_test");
    }
}

#[test]
fn no_step_asks_for_a_permission_outside_its_job() {
    assert_eq!(Step::Fetch.permissions(), json!({"contents": "read"}));
    for step in [Step::Fetch, Step::Publish, Step::ReadCi] {
        let asked = step.permissions();
        for never in [
            "workflows",
            "administration",
            "secrets",
            "environments",
            "deployments",
        ] {
            assert!(asked.get(never).is_none(), "{step:?} asked for {never}");
        }
    }
}

#[test]
fn a_token_cannot_be_printed_by_accident() {
    let token = AppToken("ghs_secret".into());
    assert!(!format!("{token:?}").contains("ghs_secret"));
}

#[tokio::test]
async fn a_repository_of_another_owner_is_refused_without_asking_github() {
    let f = fixture(Some(record())).await;
    let err = f
        .app
        .token("someone-else/test-repo", Step::Fetch)
        .await
        .unwrap_err();
    assert!(matches!(refused(&err), Some(TokenRefused::NotCovered(_))));
    assert!(f.server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_deleted_app_says_reconnect_and_stops_asking() {
    let f = fixture(Some(record())).await;
    Mock::given(method("POST"))
        .and(path(TOKEN_PATH))
        .respond_with(
            ResponseTemplate::new(401)
                .set_body_json(json!({"message": "A JSON web token could not be decoded"})),
        )
        .expect(1)
        .mount(&f.server)
        .await;
    let first = f.app.token(REPO, Step::Fetch).await.unwrap_err();
    assert_eq!(refused(&first), Some(&TokenRefused::Reconnect));
    let second = f.app.token(REPO, Step::Fetch).await.unwrap_err();
    assert_eq!(refused(&second), Some(&TokenRefused::Reconnect));
    assert_eq!(standing(f.app.store().load().as_ref()), Standing::Reconnect);
}

#[tokio::test]
async fn a_wrong_clock_is_not_a_deleted_app() {
    let f = fixture(Some(record())).await;
    Mock::given(method("POST"))
        .and(path(TOKEN_PATH))
        .respond_with(ResponseTemplate::new(401).set_body_json(
            json!({"message": "'Expiration time' claim ('exp') is too far in the future"}),
        ))
        .mount(&f.server)
        .await;
    let err = f.app.token(REPO, Step::Fetch).await.unwrap_err();
    assert!(err.to_string().contains("clock"), "{err}");
    assert_eq!(standing(f.app.store().load().as_ref()), Standing::Connected);
}

#[tokio::test]
async fn an_uninstalled_app_says_so() {
    let f = fixture(Some(record())).await;
    Mock::given(method("POST"))
        .and(path(TOKEN_PATH))
        .respond_with(ResponseTemplate::new(404))
        .mount(&f.server)
        .await;
    let err = f.app.token(REPO, Step::Fetch).await.unwrap_err();
    assert_eq!(refused(&err), Some(&TokenRefused::NotInstalled));
    assert_eq!(
        standing(f.app.store().load().as_ref()),
        Standing::NeedsInstall
    );
}

#[tokio::test]
async fn a_repository_the_person_did_not_pick_is_not_covered() {
    let f = fixture(Some(record())).await;
    Mock::given(method("POST"))
        .and(path(TOKEN_PATH))
        .respond_with(ResponseTemplate::new(422))
        .mount(&f.server)
        .await;
    let err = f.app.token(REPO, Step::Fetch).await.unwrap_err();
    assert!(matches!(refused(&err), Some(TokenRefused::NotCovered(_))));
}

#[tokio::test]
async fn the_installation_comes_from_github_and_matches_the_owner() {
    let mut start = record();
    start.installation_id = None;
    let f = fixture(Some(start)).await;
    let listed = json!([
        {"id": 1, "account": {"login": "someone-else"}, "repository_selection": "all"},
        {"id": INSTALLATION, "account": {"login": OWNER}, "repository_selection": "selected"},
    ]);
    Mock::given(method("GET"))
        .and(path("/app/installations"))
        .respond_with(ResponseTemplate::new(200).set_body_json(listed))
        .mount(&f.server)
        .await;
    let found = f.app.refresh_installation().await.unwrap();
    assert_eq!(found.installation_id, Some(INSTALLATION));
    assert_eq!(found.repository_selection.as_deref(), Some("selected"));
    assert_eq!(standing(Some(&found)), Standing::Connected);
}

#[tokio::test]
async fn with_nothing_connected_no_token_is_asked_for() {
    let f = fixture(None).await;
    let err = f.app.token(REPO, Step::Fetch).await.unwrap_err();
    assert_eq!(refused(&err), Some(&TokenRefused::NotConnected));
    assert!(f.server.received_requests().await.unwrap().is_empty());
}
