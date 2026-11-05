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

/// A scheduled backup counts as overdue this long after it was due.
const BACKUP_GRACE_SECS: i64 = 60 * 60;
const SECS_PER_HOUR: i64 = 3600;

/// What the backup status route says, from the config and the last run's status file.
pub(crate) fn backup_view(
    cfg: &hq_core::config::BackupConfig,
    last: Option<&hq_backup::status::Status>,
    now: chrono::DateTime<chrono::Utc>,
) -> serde_json::Value {
    let configured = cfg.enabled && cfg.dir.is_some();
    let finished = last.and_then(|s| chrono::DateTime::parse_from_rfc3339(&s.finished).ok());
    let last_ok_at = last.filter(|s| s.ok).and(finished);
    let failed = configured && last.is_some_and(|s| !s.ok);
    let due = cfg.every_hours as i64 * SECS_PER_HOUR + BACKUP_GRACE_SECS;
    let overdue = configured
        && last_ok_at.is_none_or(|at| (now - at.with_timezone(&chrono::Utc)).num_seconds() > due);
    json!({
        "configured": configured,
        "last_ok_at": last_ok_at.map(|t| t.to_rfc3339()),
        "failed": failed,
        "overdue": overdue,
        "every_hours": cfg.every_hours,
    })
}

/// `GET /api/backup/status`: whether HQ's scheduled backup is set up, when it last worked, and
/// whether it failed or is overdue (resolutions.md B10). No paths, no keys.
pub(crate) async fn backup_status_handler(State(state): State<Arc<WsState>>) -> Response {
    let cfg = state
        .hq_config
        .as_deref()
        .map(|c| c.backup.clone())
        .unwrap_or_default();
    let last = cfg.dir.as_deref().and_then(hq_backup::status::read);
    Json(backup_view(&cfg, last.as_ref(), chrono::Utc::now())).into_response()
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

    #[test]
    fn backup_is_overdue_or_failed_only_when_set_up() {
        let now = chrono::Utc::now();
        let mut cfg = hq_core::config::BackupConfig::default();
        assert_eq!(backup_view(&cfg, None, now)["overdue"], false, "not set up");
        cfg.enabled = true;
        cfg.dir = Some("/backups".into());
        cfg.every_hours = 24;
        assert_eq!(backup_view(&cfg, None, now)["overdue"], true, "never ran");
        let run = |hours_ago: i64, ok: bool| hq_backup::status::Status {
            finished: (now - chrono::Duration::hours(hours_ago)).to_rfc3339(),
            ok,
            message: String::new(),
            path: None,
            files: 0,
            left_out: 0,
        };
        let fresh = backup_view(&cfg, Some(&run(3, true)), now);
        assert_eq!(fresh["overdue"], false);
        assert_eq!(fresh["failed"], false);
        assert_eq!(
            backup_view(&cfg, Some(&run(72, true)), now)["overdue"],
            true
        );
        assert_eq!(backup_view(&cfg, Some(&run(1, false)), now)["failed"], true);
    }
}
