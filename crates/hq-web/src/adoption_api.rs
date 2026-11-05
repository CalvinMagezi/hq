//! `GET /api/adoption` and `GET /api/offers/replay`: the same counts `hq adoption` and
//! `hq offers replay` print, for a phone whose app build cannot be reached with `run-as`
//! (`scripts/phone-hq.sh` reads them over the bridge with a device token). Behind the web token
//! or a device token like every `/api` route; MCP keys and connector tokens never reach them.
//! Counts and catalog wording only, never a message (`docs/security/ADOPTION_DATA.md`).

use std::sync::Arc;

use axum::Json;
use axum::extract::{Query, State};
use axum::response::IntoResponse;
use hq_core::config::HqConfig;
use hq_tools::capabilities::adoption::report;
use hq_tools::capabilities::offers::{DEFAULT_REPLAY_DAYS, ready_detectors, replay};
use hq_tools::capabilities::{Install, Offer, catalog, resolve, used_tools};
use serde::Deserialize;

use crate::WsState;
use crate::error::ApiError;

/// Longest replay window the route reads, so one request cannot scan years of chat.
const MAX_REPLAY_DAYS: i64 = 365;

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// The catalog resolved for a report: a model is taken as given, as in `hq adoption`.
async fn offers(state: &Arc<WsState>) -> Result<Vec<Offer>, ApiError> {
    let config = tokio::task::spawn_blocking(HqConfig::load)
        .await
        .map_err(ApiError::internal)?
        .map_err(|e| ApiError::internal(format!("could not read the HQ config: {e}")))?;
    let (db, registry) = (state.db.clone(), state.registry.clone());
    tokio::task::spawn_blocking(move || {
        let has_tool = |name: &str| registry.as_ref().is_none_or(|r| r.get(name).is_some());
        let install = Install::detect(&config).assuming_model();
        resolve(catalog(), &install, &has_tool, &used_tools(&db))
    })
    .await
    .map_err(ApiError::internal)
}

pub(crate) async fn adoption_handler(
    State(state): State<Arc<WsState>>,
) -> Result<impl IntoResponse, ApiError> {
    let offers = offers(&state).await?;
    let db = state.db.clone();
    let r = tokio::task::spawn_blocking(move || db.with_conn(|c| report(c, now(), &offers)))
        .await
        .map_err(ApiError::internal)?
        .map_err(ApiError::internal)?;
    Ok(Json(r.to_json()))
}

#[derive(Deserialize)]
pub(crate) struct ReplayQuery {
    days: Option<i64>,
}

pub(crate) async fn replay_handler(
    State(state): State<Arc<WsState>>,
    Query(q): Query<ReplayQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let days = q
        .days
        .unwrap_or(DEFAULT_REPLAY_DAYS)
        .clamp(1, MAX_REPLAY_DAYS);
    let ready = ready_detectors(&offers(&state).await?);
    let db = state.db.clone();
    let r = tokio::task::spawn_blocking(move || db.with_conn(|c| replay(c, now(), days, &ready)))
        .await
        .map_err(ApiError::internal)?
        .map_err(ApiError::internal)?;
    Ok(Json(r))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    #[tokio::test]
    async fn the_replay_route_answers_counts_for_both_rule_sets_and_no_text() {
        let vault = tempfile::TempDir::new().unwrap();
        let state = Arc::new(WsState::new(vault.path().to_path_buf(), None));
        state
            .db
            .with_conn(|c| {
                let t = hq_db::chat::create_thread(c, "t", "user", "user")?;
                hq_db::chat::add_message(
                    c,
                    &t.thread_id,
                    "user",
                    "Remind me to pay the private supplier on Friday",
                )
            })
            .unwrap();
        let reply = replay_handler(State(state), Query(ReplayQuery { days: Some(9999) }))
            .await
            .unwrap()
            .into_response();
        let body = to_bytes(reply.into_body(), usize::MAX).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["tuned"]["days"], MAX_REPLAY_DAYS);
        assert_eq!(v["phase1"]["messages"], 1);
        assert!(!String::from_utf8_lossy(&body).contains("supplier"));
    }
}
