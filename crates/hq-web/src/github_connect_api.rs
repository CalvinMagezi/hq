//! The browser's side of connecting GitHub (`docs/plans/mobile-helpers.md`). A Custom Tab cannot
//! send the web token, so these routes carry no credential: the random `state` is the only
//! capability, and without a flow an owner started they do nothing.

use std::sync::Arc;

use axum::Router;
use axum::extract::{Query, State};
use axum::http::{HeaderName, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use hq_tools::github::app::{
    AppClient, Flows, INSTALLED_PATH, PAGE_HEADERS, Page, RETURN_PATH, START_PATH, message_page,
    start_page,
};
use serde::Deserialize;
use tokio::sync::Notify;

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
async fn installed(State(flow): State<Arc<BrowserFlow>>) -> Response {
    flow.installed.notify_waiters();
    page(
        StatusCode::OK,
        message_page(
            "GitHub is connected",
            "You can close this page and go back to HQ.",
        ),
    )
}

#[cfg(test)]
#[path = "github_connect_api_tests.rs"]
mod tests;
