//! `POST /api/coder/{action}`: the owner's controls on a coding job, from the Helpers screens.
//! Behind the web credential, so the owner pressing a button is the approval.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use hq_tools::HqTool;
use hq_tools::github::coder::HqCoderTool;
use serde_json::Value;

use crate::WsState;
use crate::error::ApiError;

const CONTROLS: [&str; 5] = ["steer", "stop", "retry", "feedback", "budget"];

pub(crate) async fn control_handler(
    State(state): State<Arc<WsState>>,
    Path(action): Path<String>,
    Json(mut body): Json<Value>,
) -> Response {
    if !CONTROLS.contains(&action.as_str()) {
        return ApiError::bad_request("unknown action").into_response();
    }
    body["action"] = Value::String(action);
    let tool = HqCoderTool::new(Arc::new(state.db.clone()), None);
    match tool.execute(body).await {
        Ok(out) => Json(out).into_response(),
        Err(e) => ApiError::bad_request(e.to_string()).into_response(),
    }
}
