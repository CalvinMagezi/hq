//! The sites HQ may fetch in any chat without asking, for the owner to review and remove.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use hq_db::fetch_sites;
use serde_json::json;

use crate::WsState;
use crate::error::ApiError;

pub(crate) async fn list_handler(State(state): State<Arc<WsState>>) -> Response {
    match state.db.with_conn(fetch_sites::trusted) {
        Ok(sites) => Json(json!({ "sites": sites })).into_response(),
        Err(e) => ApiError::internal(e).into_response(),
    }
}

pub(crate) async fn forget_handler(
    State(state): State<Arc<WsState>>,
    Path(host): Path<String>,
) -> Response {
    match state.db.with_conn(|c| fetch_sites::forget(c, &host)) {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(e) => ApiError::internal(e).into_response(),
    }
}
