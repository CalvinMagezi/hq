//! `POST /api/system/restart`: apply a config change by restarting HQ, but only when a
//! service manager will start it again. systemd sets `INVOCATION_ID` for every unit it
//! runs; launchd sets `XPC_SERVICE_NAME` to the job label. Anything else (Docker, a plain
//! `hq start` in a terminal) gets a 409 asking the owner to restart HQ by hand.

use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use hq_core::daemon_restart::RestartOutcome;
use serde_json::json;

use crate::WsState;
use crate::error::ApiError;

/// Lets the 202 reach the browser before the process goes away.
const REPLY_FLUSH_DELAY: Duration = Duration::from_millis(500);
const SYSTEMD_ENV: &str = "INVOCATION_ID";
const LAUNCHD_ENV: &str = "XPC_SERVICE_NAME";
pub(crate) const MANUAL_RESTART_HINT: &str = "HQ is not running under a service manager that would start it again, so it cannot restart itself. Restart HQ by hand (stop it and run `hq start all` again) to apply the change.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Supervisor {
    Systemd,
    Launchd(String),
}

/// The service manager that started this process and will start it again, if any.
pub(crate) fn detect_supervisor(
    env: impl Fn(&str) -> Option<String>,
    launchd_label: &str,
) -> Option<Supervisor> {
    if env(SYSTEMD_ENV).is_some_and(|v| !v.trim().is_empty()) {
        return Some(Supervisor::Systemd);
    }
    let label = launchd_label.trim();
    if !label.is_empty() && env(LAUNCHD_ENV).as_deref() == Some(label) {
        return Some(Supervisor::Launchd(label.to_string()));
    }
    None
}

fn launchd_label(state: &WsState) -> String {
    state
        .hq_config
        .as_deref()
        .map(|c| c.self_update.launchd_label.clone())
        .unwrap_or_default()
}

/// This server's supervisor, read from the real process environment.
pub(crate) fn supervisor(state: &WsState) -> Option<Supervisor> {
    detect_supervisor(|name| std::env::var(name).ok(), &launchd_label(state))
}

async fn restart_after_reply(supervisor: Supervisor) {
    tokio::time::sleep(REPLY_FLUSH_DELAY).await;
    match supervisor {
        Supervisor::Systemd => hq_core::daemon_restart::request_restart(),
        Supervisor::Launchd(label) => {
            let outcome = hq_core::daemon_restart::restart_daemon(&label).await;
            if !matches!(outcome, RestartOutcome::Restarted) {
                tracing::warn!(?outcome, "restart: launchd did not restart HQ");
            }
        }
    }
}

pub(crate) async fn restart_handler(
    State(state): State<Arc<WsState>>,
) -> Result<impl IntoResponse, ApiError> {
    crate::chat_apps::require_web_token(&state)?;
    let Some(supervisor) = supervisor(&state) else {
        return Err(ApiError::Conflict(MANUAL_RESTART_HINT.into()));
    };
    tracing::info!(?supervisor, "restart: requested from the web app");
    tokio::spawn(restart_after_reply(supervisor));
    Ok((StatusCode::ACCEPTED, Json(json!({ "restarting": true }))))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_of(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |name| {
            pairs
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| v.to_string())
        }
    }

    #[test]
    fn systemd_is_detected_by_its_invocation_id() {
        let env = env_of(&[(SYSTEMD_ENV, "abc123")]);
        assert_eq!(detect_supervisor(env, ""), Some(Supervisor::Systemd));
        let blank = env_of(&[(SYSTEMD_ENV, " ")]);
        assert_eq!(detect_supervisor(blank, ""), None);
    }

    #[test]
    fn launchd_counts_only_when_it_runs_the_configured_job() {
        let env = env_of(&[(LAUNCHD_ENV, "com.example.hq")]);
        assert_eq!(
            detect_supervisor(env, "com.example.hq"),
            Some(Supervisor::Launchd("com.example.hq".into()))
        );
        let other = env_of(&[(LAUNCHD_ENV, "com.apple.Terminal")]);
        assert_eq!(detect_supervisor(other, "com.example.hq"), None);
        let no_label = env_of(&[(LAUNCHD_ENV, "com.example.hq")]);
        assert_eq!(detect_supervisor(no_label, ""), None);
    }

    #[test]
    fn a_terminal_or_container_has_no_supervisor() {
        assert_eq!(detect_supervisor(env_of(&[]), "com.example.hq"), None);
    }
}
