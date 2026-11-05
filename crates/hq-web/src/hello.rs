//! `GET /api/hello`, open without a sign-in: which HQ answers, which edition and version, and
//! whether it is still starting (docs/plans/ux-run/web/wave-0/02-shared-parts.md rule 7, rules-web.md
//! 2). The install id keys every saved copy in a browser, so one origin can serve two HQs.
//!
//! While HQ opens, [`StartingServer`] holds the web port and answers `/api/hello` with the step it
//! is on and every other `/api` route with 503 `{"starting": true}`; once the full router is ready
//! it takes over on the same listener.

use anyhow::{Context, Result};
use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde_json::{Value, json};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};

use crate::WsState;

pub(crate) const HELLO_PATH: &str = "/api/hello";
const ID_FILE: &str = "hq_id";
/// The oldest web client this server still talks to; raised when a route changes shape.
pub const MIN_CLIENT: u32 = 1;
/// The steps HQ reports while it opens, in the words the phone's start screen uses.
pub const START_STEPS: [&str; 3] = ["Opening your notes and tasks", "Loading HQ", "Almost ready"];

/// This install's id: random, made once, kept in the vault's data folder.
pub fn hq_id(vault_path: &Path) -> Result<String> {
    let dir = vault_path.join("_data");
    let path = dir.join(ID_FILE);
    if let Ok(id) = std::fs::read_to_string(&path) {
        let id = id.trim();
        if !id.is_empty() {
            return Ok(id.to_string());
        }
    }
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let id = uuid::Uuid::new_v4().simple().to_string();
    std::fs::write(&path, &id).with_context(|| format!("writing {}", path.display()))?;
    Ok(id)
}

/// How far HQ has got while it opens.
#[derive(Debug)]
pub struct StartProgress {
    step: AtomicUsize,
    started_at: i64,
}

impl Default for StartProgress {
    fn default() -> Self {
        Self {
            step: AtomicUsize::new(0),
            started_at: chrono::Utc::now().timestamp(),
        }
    }
}

impl StartProgress {
    /// Moves to step `index` of [`START_STEPS`] (0-based); never backwards.
    pub fn at(&self, index: usize) {
        self.step
            .fetch_max(index.min(START_STEPS.len() - 1), Ordering::Relaxed);
    }

    fn view(&self) -> Value {
        let step = self.step.load(Ordering::Relaxed);
        json!({
            "step": step + 1,
            "steps": START_STEPS.len(),
            "step_name": START_STEPS[step],
            "started_at": self.started_at,
        })
    }
}

fn body(hq_id: &str, edition: &str, starting: Option<&StartProgress>) -> Value {
    json!({
        "hq_id": hq_id,
        "edition": edition,
        "version": hq_core::build_info::version(),
        "min_client": MIN_CLIENT,
        "tz": host_zone(),
        "tz_offset_minutes": chrono::Local::now().offset().local_minus_utc() / SECS_PER_MINUTE,
        "starting": starting.map(StartProgress::view),
    })
}

const SECS_PER_MINUTE: i32 = 60;

/// HQ's own time zone as an IANA name, so a page can say "07:00 Nairobi time" for a time HQ
/// keeps in its zone. `None` when the host does not say.
fn host_zone() -> Option<String> {
    iana_time_zone::get_timezone()
        .ok()
        .filter(|z| !z.is_empty())
}

fn edition(profile: hq_core::config::Profile) -> String {
    serde_json::to_value(profile)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// `GET /api/hello` once HQ is up.
pub(crate) async fn hello_handler(State(state): State<Arc<WsState>>) -> axum::response::Response {
    match hq_id(&state.vault_path) {
        Ok(id) => axum::Json(body(&id, &edition(state.profile()), None)).into_response(),
        Err(e) => crate::error::ApiError::internal(e).into_response(),
    }
}

/// The web port while HQ opens: `/api/hello` says how far it got, the rest of `/api` waits.
fn starting_router(id: String, edition: String, progress: Arc<StartProgress>) -> Router {
    let hello = move || {
        let v = body(&id, &edition, Some(&progress));
        async move { axum::Json(v) }
    };
    Router::new()
        .route(HELLO_PATH, axum::routing::get(hello))
        .fallback(|| async {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                axum::Json(json!({ "starting": true, "error": "HQ is still starting." })),
            )
        })
}

/// Holds the web port from the first moment of a start, then hands it to the full router.
pub struct StartingServer {
    current: Arc<RwLock<Router>>,
    pub progress: Arc<StartProgress>,
}

impl StartingServer {
    pub fn new(vault_path: &Path, profile: hq_core::config::Profile) -> Result<Self> {
        let progress = Arc::new(StartProgress::default());
        let first = starting_router(hq_id(vault_path)?, edition(profile), progress.clone());
        Ok(Self {
            current: Arc::new(RwLock::new(first)),
            progress,
        })
    }

    /// The router to serve: every request goes to whichever router is current.
    pub fn router(&self) -> Router {
        let current = self.current.clone();
        Router::new().fallback_service(tower::service_fn(move |req| {
            let router = current.read().expect("router lock").clone();
            tower::ServiceExt::oneshot(router, req)
        }))
    }

    /// HQ is up: the full router takes every request from now on.
    pub fn ready(&self, full: Router) {
        *self.current.write().expect("router lock") = full;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    async fn get(app: &Router, uri: &str) -> (StatusCode, Value) {
        let res = app
            .clone()
            .oneshot(Request::get(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = res.status();
        let bytes = axum::body::to_bytes(res.into_body(), 1 << 16)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    #[test]
    fn hello_names_hqs_time_zone_and_its_offset() {
        let v = body("id", "full", None);
        let offset = chrono::Local::now().offset().local_minus_utc() / SECS_PER_MINUTE;
        assert_eq!(v["tz_offset_minutes"], offset);
        assert!(
            v["tz"].is_null()
                || v["tz"]
                    .as_str()
                    .is_some_and(|z| z.contains('/') || z == "UTC")
        );
    }

    #[test]
    fn the_install_id_is_made_once_and_kept() {
        let dir = tempfile::TempDir::new().unwrap();
        let first = hq_id(dir.path()).unwrap();
        assert_eq!(first.len(), 32);
        assert_eq!(hq_id(dir.path()).unwrap(), first);
        let other = tempfile::TempDir::new().unwrap();
        assert_ne!(
            hq_id(other.path()).unwrap(),
            first,
            "another HQ, another id"
        );
    }

    #[tokio::test]
    async fn while_starting_hello_reports_the_step_and_the_rest_waits_then_hands_over() {
        let dir = tempfile::TempDir::new().unwrap();
        let server = StartingServer::new(dir.path(), hq_core::config::Profile::Full).unwrap();
        let app = server.router();
        server.progress.at(1);
        let (status, v) = get(&app, HELLO_PATH).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["hq_id"], hq_id(dir.path()).unwrap());
        assert_eq!(v["edition"], "full");
        assert_eq!(v["starting"]["step"], 2);
        assert_eq!(v["starting"]["steps"], 3);
        assert_eq!(v["starting"]["step_name"], "Loading HQ");
        let (status, v) = get(&app, "/api/tasks").await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(v["starting"], true);

        server.ready(Router::new().route("/api/tasks", axum::routing::get(|| async { "[]" })));
        let (status, _) = get(&app, "/api/tasks").await;
        assert_eq!(
            status,
            StatusCode::OK,
            "the full router answers on the same service"
        );
    }
}
