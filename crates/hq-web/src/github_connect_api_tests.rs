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

#[tokio::test]
async fn the_install_return_changes_nothing() {
    let (_dir, flow) = flow();
    let (status, _) = get_status(&flow, &format!("{INSTALLED_PATH}?installation_id=1")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(flow.client.store().load().is_none());
}
