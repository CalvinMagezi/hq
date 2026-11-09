//! The browser's side of connecting GitHub (`docs/plans/mobile-helpers.md`). A Custom Tab cannot
//! send the web token, so these routes carry no credential: the random `state` is the only
//! capability, and without a flow an owner started they do nothing.

use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use hq_core::config::HqConfig;
use hq_tools::github::app::{
    AppClient, Flows, INSTALLED_PATH, PAGE_HEADERS, Page, RETURN_PATH, START_PATH, Standing,
    TokenRefused, finished_page, message_page, standing, start_page,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::Notify;

use crate::WsState;
use crate::error::ApiError;

/// The phone app's own link to its GitHub sheet.
const BACK_TO_APP: &str = "hq://helpers/github";

/// One server's connection attempts and the address GitHub sends the browser back to.
pub struct BrowserFlow {
    pub flows: Flows,
    pub client: AppClient,
    pub base: String,
    /// Woken when the browser comes back from the install page, for `hq github connect`.
    pub installed: Notify,
}

impl BrowserFlow {
    pub fn new(client: AppClient, base: String) -> Self {
        Self {
            flows: Flows::default(),
            client,
            base,
            installed: Notify::new(),
        }
    }

    /// Starts a flow and returns the address to open. Owner paths only.
    pub fn begin(&self) -> String {
        format!("{}{START_PATH}?state={}", self.base, self.flows.begin())
    }
}

#[derive(Deserialize)]
struct StartQuery {
    #[serde(default)]
    state: String,
}

#[derive(Deserialize)]
struct ReturnQuery {
    #[serde(default)]
    state: String,
    #[serde(default)]
    code: String,
}

pub fn browser_routes<S>(flow: Arc<BrowserFlow>) -> Router<S> {
    Router::new()
        .route(START_PATH, get(start))
        .route(RETURN_PATH, get(returned))
        .route(INSTALLED_PATH, get(installed))
        .with_state(flow)
}

fn page(status: StatusCode, page: Page) -> Response {
    let mut response = (status, axum::response::Html(page.html)).into_response();
    let headers = response.headers_mut();
    for (name, value) in PAGE_HEADERS {
        headers.insert(
            HeaderName::from_static(name),
            HeaderValue::from_static(value),
        );
    }
    if let Ok(csp) = HeaderValue::from_str(&page.csp) {
        headers.insert(header::CONTENT_SECURITY_POLICY, csp);
    }
    response
}

async fn start(State(flow): State<Arc<BrowserFlow>>, Query(q): Query<StartQuery>) -> Response {
    if !flow.flows.is_pending(&q.state) {
        let refused = message_page(
            "Start again",
            "This link was not started here, or has expired.",
        );
        return page(StatusCode::FORBIDDEN, refused);
    }
    page(StatusCode::OK, start_page(&q.state, &flow.base))
}

async fn returned(State(flow): State<Arc<BrowserFlow>>, Query(q): Query<ReturnQuery>) -> Response {
    match flow.client.finish(&flow.flows, &q.state, &q.code).await {
        Ok(record) => Redirect::to(&record.install_url()).into_response(),
        Err(e) => page(
            StatusCode::FORBIDDEN,
            message_page("GitHub is not connected", &e.to_string()),
        ),
    }
}

/// Changes nothing: anyone can open this address. The owner's own status call reads the
/// installation from GitHub.
async fn installed(State(flow): State<Arc<BrowserFlow>>, headers: HeaderMap) -> Response {
    flow.installed.notify_waiters();
    let on_phone = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ua| ua.contains("Android"));
    let back = on_phone.then_some(("Back to HQ", BACK_TO_APP));
    page(
        StatusCode::OK,
        finished_page(
            "GitHub is connected",
            "Go back to HQ to choose what it should work on.",
            back,
        ),
    )
}

/// The flow for this server, or `None` when no HTTP client could be built. The address is
/// always loopback: on a phone that is HQ's own server, reached by the browser beside it.
pub(crate) fn flow_for(state: &WsState) -> Option<Arc<BrowserFlow>> {
    let port = state
        .hq_config
        .as_deref()
        .map_or_else(|| HqConfig::default().ws_port, |c| c.ws_port);
    let client = AppClient::standard().ok()?;
    Some(Arc::new(BrowserFlow::new(
        client,
        format!("http://127.0.0.1:{port}"),
    )))
}

/// With sign-in off, `/api` answers anything local, including a model's shell, so nothing that
/// changes the connection runs without it. `hq github` is the way in on such an install.
pub(crate) async fn require_sign_in(
    State(state): State<Arc<WsState>>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    if request.method() == axum::http::Method::GET || state.web_auth_enabled_async().await {
        return next.run(request).await;
    }
    let refused =
        json!({"error": "Turn on sign-in for this HQ, or use `hq github` on this machine."});
    (StatusCode::FORBIDDEN, Json(refused)).into_response()
}

/// Behind the web credential, which is the owner's: `/mcp` and its keys never reach these.
pub(crate) fn owner_routes<S>(flow: Arc<BrowserFlow>) -> Router<S> {
    Router::new()
        .route("/api/github", get(status))
        .route("/api/github/connect", post(connect))
        .route("/api/github/disconnect", post(disconnect))
        .route("/api/github/repos/scan", post(scan))
        .route("/api/github/repos/confirm", post(confirm))
        .route("/api/github/first-task", post(confirm_first_task))
        .with_state(flow)
}

fn not_a_refusal(error: anyhow::Error) -> Result<(), ApiError> {
    if error.downcast_ref::<TokenRefused>().is_some() {
        return Ok(());
    }
    Err(ApiError::internal(format!("GitHub check failed: {error}")))
}

/// Where the connection stands. Finds the installation after the person pressed Install, and
/// reads the repositories the first time there are any, so the phone only has to ask.
async fn status(State(flow): State<Arc<BrowserFlow>>) -> Result<Json<Value>, ApiError> {
    let client = &flow.client;
    let waiting = client
        .store()
        .load()
        .is_some_and(|r| r.installation_id.is_none() && !r.needs_reconnect);
    if waiting && let Err(e) = client.refresh_installation().await {
        not_a_refusal(e)?;
    }
    let installed = standing(client.store().load().as_ref()) == Standing::Connected;
    if installed
        && client.repositories().is_empty()
        && let Err(e) = client.scan_repositories().await
    {
        not_a_refusal(e)?;
    }
    Ok(Json(status_body(client)))
}

fn status_body(client: &AppClient) -> Value {
    let record = client.store().load();
    json!({
        "standing": standing(record.as_ref()),
        "owner": record.as_ref().map(|r| r.owner_login.clone()),
        "slug": record.as_ref().map(|r| r.slug.clone()),
        "repository_selection": record.as_ref().and_then(|r| r.repository_selection.clone()),
        "install_url": record.as_ref().map(|r| r.install_url()),
        "settings_url": record.as_ref().map(|r| r.settings_url()),
        "first_task_confirmed": record.as_ref().is_some_and(|r| r.first_task_confirmed),
        "repos": repo_rows(client),
    })
}

fn repo_rows(client: &AppClient) -> Vec<Value> {
    client
        .repositories()
        .iter()
        .map(|r| json!({"flagged": r.flagged(), "picked": r.picked(), "report": r}))
        .collect()
}

async fn connect(State(flow): State<Arc<BrowserFlow>>) -> Json<Value> {
    Json(json!({"url": flow.begin()}))
}

async fn disconnect(State(flow): State<Arc<BrowserFlow>>) -> Result<Json<Value>, ApiError> {
    let settings_url = flow
        .client
        .disconnect()
        .map_err(|e| ApiError::internal(format!("could not remove the key: {e}")))?;
    Ok(Json(json!({"settings_url": settings_url})))
}

async fn confirm_first_task(State(flow): State<Arc<BrowserFlow>>) -> Result<Json<Value>, ApiError> {
    let connected = flow
        .client
        .confirm_first_task()
        .map_err(|e| ApiError::internal(format!("could not save that: {e}")))?;
    Ok(Json(json!({"confirmed": connected})))
}

async fn scan(State(flow): State<Arc<BrowserFlow>>) -> Result<Json<Value>, ApiError> {
    if let Err(e) = flow.client.scan_repositories().await {
        not_a_refusal(e)?;
    }
    Ok(Json(status_body(&flow.client)))
}

#[derive(Deserialize)]
struct ConfirmBody {
    repo: String,
}

async fn confirm(
    State(flow): State<Arc<BrowserFlow>>,
    Json(body): Json<ConfirmBody>,
) -> Result<Json<Value>, ApiError> {
    let known = flow
        .client
        .confirm_repository(&body.repo)
        .map_err(|e| ApiError::internal(format!("could not save that: {e}")))?;
    Ok(Json(json!({"confirmed": known})))
}

#[cfg(test)]
#[path = "github_connect_api_tests.rs"]
mod tests;
