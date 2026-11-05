//! `GET /api/status/busy`, behind the web token: what HQ is doing that an update would cut off, so
//! the phone installs only when HQ is idle (docs/plans/ux-run/mobile/wave-0/02-safe-mode.md).
//!
//! `GET /api/status/ai`, behind the web token: whether HQ's AI services are answering, per service
//! the last good and failed call and the failure kind (rules-shared.md 2, decision D16). Fixed
//! classes only: no prompt and no answer text ever reaches this route.

use axum::Json;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use serde_json::json;
use std::sync::Arc;

use crate::WsState;
use crate::error::ApiError;

/// How far back a call still says something about the service now.
const WINDOW_SECS: i64 = 15 * 60;

/// The chain falls back from one service to the next, so HQ's AI is down only when every service
/// it called lately is failing; with no calls at all nothing says it is down.
pub(crate) fn answering(services: &[hq_db::task_outcomes::ServiceStatus]) -> bool {
    services.is_empty() || services.iter().any(|s| !s.failing())
}

pub(crate) async fn status_ai_handler(State(state): State<Arc<WsState>>) -> Response {
    match state
        .db
        .with_conn(|c| hq_db::task_outcomes::service_status(c, WINDOW_SECS))
    {
        Ok(services) => Json(json!({
            "answering": answering(&services),
            "services": services,
        }))
        .into_response(),
        Err(e) => ApiError::internal(e).into_response(),
    }
}

/// `GET /api/status/busy`: `{busy, chat_turns, backup}`.
pub(crate) async fn status_busy_handler(State(state): State<Arc<WsState>>) -> Response {
    let chat_turns = state.active_chat_turns.read().await.len();
    let backup = hq_backup::status::RUNNING.load(std::sync::atomic::Ordering::SeqCst);
    Json(json!({
        "busy": chat_turns > 0 || backup,
        "chat_turns": chat_turns,
        "backup": backup,
    }))
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_db::task_outcomes::ServiceStatus;

    fn service(ok: Option<i64>, failed: Option<i64>) -> ServiceStatus {
        ServiceStatus {
            provider: "p".into(),
            last_ok_at: ok,
            last_failed_at: failed,
            failure: failed.map(|_| "rate_limit".into()),
        }
    }

    #[test]
    fn down_only_when_every_recent_service_is_failing() {
        assert!(answering(&[]), "no calls, nothing says it is down");
        assert!(!answering(&[service(Some(1), Some(2))]));
        assert!(
            answering(&[service(Some(1), Some(2)), service(Some(5), None)]),
            "a fallback answers"
        );
        assert!(
            answering(&[service(Some(3), Some(2))]),
            "it answered after the failure"
        );
    }

    #[tokio::test]
    async fn busy_while_a_backup_runs_and_idle_otherwise() {
        use std::sync::atomic::Ordering;
        let vault = tempfile::TempDir::new().unwrap();
        let state = Arc::new(WsState::new(vault.path().to_path_buf(), None));
        let read = |res: Response| async {
            let bytes = axum::body::to_bytes(res.into_body(), 1 << 16)
                .await
                .unwrap();
            serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()
        };
        let idle = read(status_busy_handler(State(state.clone())).await).await;
        assert_eq!(idle["busy"], false);
        hq_backup::status::RUNNING.store(true, Ordering::SeqCst);
        let busy = read(status_busy_handler(State(state.clone())).await).await;
        hq_backup::status::RUNNING.store(false, Ordering::SeqCst);
        assert_eq!(busy["busy"], true);
        assert_eq!(busy["backup"], true);
    }
}
