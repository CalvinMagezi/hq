use super::*;
use crate::github::app::{AppRecord, Store};
use serde_json::json;
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const REPO: &str = "test-owner/test-repo";

struct Fixture {
    _dir: TempDir,
    server: MockServer,
    app: AppClient,
}

async fn fixture() -> Fixture {
    let dir = TempDir::new().unwrap();
    let store = Store::at(dir.path().join("github"));
    store
        .save_key(include_str!("testdata/test_app_key.pem"))
        .unwrap();
    store
        .save_record(&AppRecord {
            app_id: 1,
            client_id: "Iv-test".into(),
            slug: "hq-coder-abc".into(),
            owner_login: "test-owner".into(),
            owner_id: 42,
            owner_name: None,
            installation_id: Some(9),
            repository_selection: Some("selected".into()),
            needs_reconnect: false,
        })
        .unwrap();
    let server = MockServer::start().await;
    let app = AppClient::new(store, &format!("{}/", server.uri())).unwrap();
    Mock::given(method("POST"))
        .and(path("/app/installations/9/access_tokens"))
        .respond_with(ResponseTemplate::new(201).set_body_json(json!({"token": "ghs_t"})))
        .mount(&server)
        .await;
    Fixture {
        _dir: dir,
        server,
        app,
    }
}

async fn answer(server: &MockServer, at: &str, status: u16, body: Value) {
    Mock::given(method("GET"))
        .and(path(at))
        .respond_with(ResponseTemplate::new(status).set_body_json(body))
        .mount(server)
        .await;
}

async fn repository(f: &Fixture, workflow: Option<&str>, root: Value) {
    let repo = format!("/repos/{REPO}");
    let info = json!({"full_name": REPO, "private": true, "default_branch": "main"});
    answer(&f.server, &repo, 200, info.clone()).await;
    answer(
        &f.server,
        "/installation/repositories",
        200,
        json!({"repositories": [info]}),
    )
    .await;
    answer(&f.server, &format!("{repo}/contents/"), 200, root).await;
    answer(
        &f.server,
        &format!("{repo}/branches/main"),
        200,
        json!({"protected": true}),
    )
    .await;
    let dir = format!("{repo}/contents/.github/workflows");
    let Some(text) = workflow else {
        answer(&f.server, &dir, 404, json!({})).await;
        return;
    };
    answer(&f.server, &dir, 200, json!([{"name": "ci.yml"}])).await;
    Mock::given(method("GET"))
        .and(path(format!("{dir}/ci.yml")))
        .respond_with(ResponseTemplate::new(200).set_body_string(text))
        .mount(&f.server)
        .await;
}

fn names(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|s| s.to_string()).collect()
}

#[test]
fn no_tool_can_start_or_finish_a_connection() {
    let names: Vec<String> = crate::github::create_github_tools(&[])
        .iter()
        .map(|tool| tool.name().to_string())
        .collect();
    assert_eq!(names, ["github_read", "github_clone", "github_write"]);
}

#[test]
fn github_token_alone_is_not_a_secret_worth_flagging() {
    assert!(secret_names("token: ${{ secrets.GITHUB_TOKEN }}").is_empty());
    assert!(secret_names("token: ${{ secrets.github_token }}").is_empty());
}

#[test]
fn other_secrets_are_found_in_every_spelling() {
    assert_eq!(secret_names("a: ${{ secrets.FOO }}"), names(&["FOO"]));
    assert_eq!(secret_names("a: ${{ secrets['BAR'] }}"), names(&["BAR"]));
    assert_eq!(secret_names("    secrets: inherit"), names(&[ALL_SECRETS]));
    assert_eq!(
        secret_names("a: ${{ toJSON(secrets) }}"),
        names(&[ALL_SECRETS])
    );
}

#[tokio::test]
async fn a_repository_using_a_secret_is_flagged_and_left_unpicked() {
    let f = fixture().await;
    repository(&f, Some("k: ${{ secrets.FOO }}"), json!([])).await;
    let reports = f.app.scan_repositories().await.unwrap();
    assert_eq!(reports[0].secrets, ["FOO"]);
    assert!(reports[0].has_ci);
    assert!(reports[0].protected);
    assert!(reports[0].flagged());
    assert!(!reports[0].picked());
    let err = f.app.ensure_picked(REPO).await.unwrap_err();
    assert!(err.to_string().contains("Confirm it"), "{err}");
}

#[tokio::test]
async fn a_preview_host_is_flagged() {
    let f = fixture().await;
    repository(&f, None, json!([{"name": "vercel.json"}, {"name": "src"}])).await;
    let reports = f.app.scan_repositories().await.unwrap();
    assert_eq!(reports[0].previews, ["vercel.json"]);
    assert!(!reports[0].picked());
}

#[tokio::test]
async fn a_plain_repository_is_picked() {
    let f = fixture().await;
    repository(&f, Some("t: ${{ secrets.GITHUB_TOKEN }}"), json!([])).await;
    let reports = f.app.scan_repositories().await.unwrap();
    assert!(reports[0].picked());
    f.app.ensure_picked(REPO).await.unwrap();
}

#[tokio::test]
async fn the_owners_confirmation_picks_a_flagged_repository() {
    let f = fixture().await;
    repository(&f, Some("k: ${{ secrets.FOO }}"), json!([])).await;
    f.app.scan_repositories().await.unwrap();
    assert!(f.app.confirm_repository(REPO).unwrap());
    f.app.ensure_picked(REPO).await.unwrap();
    let again = f.app.scan_repositories().await.unwrap();
    assert!(
        again[0].confirmed,
        "an unchanged repository stays confirmed"
    );
}

#[test]
fn a_new_secret_undoes_a_confirmation() {
    let old = RepoReport {
        repo: REPO.into(),
        private: true,
        default_branch: "main".into(),
        protected: false,
        has_ci: true,
        secrets: vec!["FOO".into()],
        previews: Vec::new(),
        unchecked: false,
        confirmed: true,
    };
    let mut now = old.clone();
    assert!(confirmation_holds(std::slice::from_ref(&old), &now));
    now.secrets.push("BAR".into());
    assert!(!confirmation_holds(&[old], &now));
}

#[tokio::test]
async fn a_repository_hq_could_not_read_is_not_called_safe() {
    let f = fixture().await;
    let repo = format!("/repos/{REPO}");
    let info = json!({"full_name": REPO, "private": true, "default_branch": "main"});
    answer(
        &f.server,
        "/installation/repositories",
        200,
        json!({"repositories": [info]}),
    )
    .await;
    answer(
        &f.server,
        &format!("{repo}/contents/.github/workflows"),
        500,
        json!({}),
    )
    .await;
    let reports = f.app.scan_repositories().await.unwrap();
    assert!(reports[0].unchecked);
    assert!(!reports[0].picked());
}

#[tokio::test]
async fn a_repository_added_after_the_scan_is_read_before_any_work() {
    let f = fixture().await;
    repository(&f, Some("k: ${{ secrets.FOO }}"), json!([])).await;
    assert!(f.app.repositories().is_empty());
    assert!(f.app.ensure_picked(REPO).await.is_err());
    assert_eq!(f.app.repositories().len(), 1);
}

#[tokio::test]
async fn disconnect_forgets_the_repositories() {
    let f = fixture().await;
    repository(&f, None, json!([])).await;
    f.app.scan_repositories().await.unwrap();
    f.app.disconnect().unwrap();
    assert!(f.app.repositories().is_empty());
}

#[tokio::test]
async fn confirming_a_repository_hq_never_saw_does_nothing() {
    let f = fixture().await;
    assert!(!f.app.confirm_repository("test-owner/other").unwrap());
}
