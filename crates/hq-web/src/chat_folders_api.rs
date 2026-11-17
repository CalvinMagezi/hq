//! The folders the owner allowed for one web chat (`hq_db::owner_folder_grants`), listed in the
//! chat's Details panel with a Revoke button. These routes sit behind the owner's web token; a
//! grant is written only by the owner's answer to an approval (`approve_folder`), and no tool a
//! model can call reaches either.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use serde_json::json;

use crate::WsState;
use crate::error::ApiError;

fn scope_of(thread: &str) -> Option<String> {
    (!thread.is_empty() && !thread.contains(':')).then(|| format!("web:{thread}"))
}

/// `GET /api/chat-folders/{thread}`
pub(crate) async fn list_handler(
    State(state): State<Arc<WsState>>,
    Path(thread): Path<String>,
) -> Response {
    let Some(scope) = scope_of(&thread) else {
        return ApiError::bad_request("not a chat id").into_response();
    };
    match hq_db::owner_folder_grants::list(&state.db, &scope) {
        Ok(folders) => Json(json!({ "folders": folders })).into_response(),
        Err(e) => ApiError::internal(e).into_response(),
    }
}

/// `DELETE /api/chat-folders/{thread}/{id}`: the chat asks again for that folder.
pub(crate) async fn revoke_handler(
    State(state): State<Arc<WsState>>,
    Path((thread, id)): Path<(String, i64)>,
) -> Response {
    let Some(scope) = scope_of(&thread) else {
        return ApiError::bad_request("not a chat id").into_response();
    };
    match hq_db::owner_folder_grants::revoke(&state.db, &scope, id) {
        Ok(true) => Json(json!({ "ok": true })).into_response(),
        Ok(false) => ApiError::not_found().into_response(),
        Err(e) => ApiError::internal(e).into_response(),
    }
}
