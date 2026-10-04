use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::WsState;
use crate::error::ApiError;

#[derive(Deserialize)]
pub(crate) struct ListQuery {
    pub(crate) limit: Option<i64>,
    pub(crate) offset: Option<i64>,
}

#[derive(Serialize)]
pub(crate) struct ThreadsResponse {
    pub(crate) threads: Vec<hq_db::chat::ChatThread>,
    /// Threads with a turn still in flight, so a client that reconnects
    /// mid-reply can show them as running instead of losing `turn_end`.
    pub(crate) running: Vec<String>,
}

#[derive(Deserialize)]
pub(crate) struct CreateThreadRequest {
    pub(crate) title: String,
}

/// Largest page a client may ask for, so one request cannot pull a whole long chat.
const MAX_MESSAGE_PAGE: i64 = 200;
const DEFAULT_MESSAGE_PAGE: i64 = 100;

#[derive(Deserialize)]
pub(crate) struct MessagesQuery {
    pub(crate) limit: Option<i64>,
    /// A message id: return the page just older than it.
    pub(crate) before: Option<String>,
}

pub(crate) async fn list_threads_handler(
    State(state): State<Arc<WsState>>,
    Query(q): Query<ListQuery>,
) -> Response {
    // Web chat is its own surface; Telegram/Discord conversations stay there.
    let listed = state.db.with_conn(|conn| {
        hq_db::chat::list_threads(conn, q.limit.unwrap_or(50), q.offset.unwrap_or(0), Some("web"))
    });
    let running = state.active_chat_turns.read().await.keys().cloned().collect();
    match listed {
        Ok(threads) => Json(ThreadsResponse { threads, running }).into_response(),
        Err(e) => ApiError::internal(e).into_response(),
    }
}

pub(crate) async fn create_thread_handler(
    State(state): State<Arc<WsState>>,
    Json(req): Json<CreateThreadRequest>,
) -> Response {
    match state.db.with_conn(|conn| hq_db::chat::create_thread(conn, &req.title, "user", "user")) {
        Ok(thread) => (StatusCode::CREATED, Json(thread)).into_response(),
        Err(e) => ApiError::internal(e).into_response(),
    }
}

pub(crate) async fn get_thread_messages_handler(
    State(state): State<Arc<WsState>>,
    Path(thread_id): Path<String>,
    Query(q): Query<MessagesQuery>,
) -> Response {
    let limit = q.limit.unwrap_or(DEFAULT_MESSAGE_PAGE).clamp(1, MAX_MESSAGE_PAGE);
    let before = q.before.as_deref().filter(|b| !b.is_empty());
    match state.db.with_conn(|conn| hq_db::chat::get_messages_page(conn, &thread_id, limit, before)) {
        Ok(messages) => Json(messages).into_response(),
        Err(e) => ApiError::internal(e).into_response(),
    }
}

pub(crate) async fn archive_thread_handler(
    State(state): State<Arc<WsState>>,
    Path(thread_id): Path<String>,
) -> Response {
    match state.db.with_conn(|conn| hq_db::chat::archive_thread(conn, &thread_id)) {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => ApiError::internal(e).into_response(),
    }
}
