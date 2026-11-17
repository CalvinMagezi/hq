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
        hq_db::chat::list_threads(
            conn,
            q.limit.unwrap_or(50),
            q.offset.unwrap_or(0),
            Some("web"),
        )
    });
    let running = state
        .active_chat_turns
        .read()
        .await
        .keys()
        .cloned()
        .collect();
    match listed {
        Ok(threads) => Json(ThreadsResponse { threads, running }).into_response(),
        Err(e) => ApiError::internal(e).into_response(),
    }
}

pub(crate) async fn create_thread_handler(
    State(state): State<Arc<WsState>>,
    Json(req): Json<CreateThreadRequest>,
) -> Response {
    match state
        .db
        .with_conn(|conn| hq_db::chat::create_thread(conn, &req.title, "user", "user"))
    {
        Ok(thread) => (StatusCode::CREATED, Json(thread)).into_response(),
        Err(e) => ApiError::internal(e).into_response(),
    }
}

pub(crate) async fn get_thread_messages_handler(
    State(state): State<Arc<WsState>>,
    Path(thread_id): Path<String>,
    Query(q): Query<MessagesQuery>,
) -> Response {
    let limit = q
        .limit
        .unwrap_or(DEFAULT_MESSAGE_PAGE)
        .clamp(1, MAX_MESSAGE_PAGE);
    let before = q.before.as_deref().filter(|b| !b.is_empty());
    match state
        .db
        .with_conn(|conn| hq_db::chat::get_messages_page(conn, &thread_id, limit, before))
    {
        Ok(messages) => Json(messages).into_response(),
        Err(e) => ApiError::internal(e).into_response(),
    }
}

/// How long after an archive or a clear HQ still takes its Undo (resolutions.md A6).
pub(crate) const UNDO_WINDOW_SECS: i64 = 10 * 60;
/// What an Undo past the window, or after a change elsewhere, answers.
pub(crate) const UNDO_TOO_LATE: &str = "changed";

/// `POST /api/threads/{id}/stop`: Stop over HTTP as well as the socket, so a page whose socket dropped
/// can still end a reply. Ends it for every tab, keeping what it wrote.
pub(crate) async fn stop_thread_handler(
    State(state): State<Arc<WsState>>,
    Path(thread_id): Path<String>,
) -> Response {
    crate::ws::stop_chat_turn(&state, &thread_id).await;
    StatusCode::NO_CONTENT.into_response()
}

/// `POST /api/threads/{id}/unarchive`: the Undo of an archive, within the window.
pub(crate) async fn unarchive_thread_handler(
    State(state): State<Arc<WsState>>,
    Path(thread_id): Path<String>,
) -> Response {
    match state
        .db
        .with_conn(|conn| hq_db::chat::unarchive_thread(conn, &thread_id, UNDO_WINDOW_SECS))
    {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => ApiError::Conflict(UNDO_TOO_LATE.into()).into_response(),
        Err(e) => ApiError::internal(e).into_response(),
    }
}

/// The longest name a person may give a chat.
const MAX_RENAME_CHARS: usize = 120;

/// `POST /api/threads/{id}/rename` with `{ "title": "..." }`: the owner names a chat. Every tab hears of it, and a
/// chat renamed this way is never renamed again by HQ.
pub(crate) async fn rename_thread_handler(
    State(state): State<Arc<WsState>>,
    Path(thread_id): Path<String>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> Response {
    let title: String = body
        .get("title")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(MAX_RENAME_CHARS)
        .collect();
    if title.is_empty() {
        return ApiError::bad_request("a chat needs a name").into_response();
    }
    let known = state
        .db
        .with_conn(|conn| hq_db::chat::get_thread(conn, &thread_id))
        .ok()
        .flatten()
        .is_some();
    if !known {
        return ApiError::not_found().into_response();
    }
    match state
        .db
        .with_conn(|conn| hq_db::chat::set_thread_title(conn, &thread_id, &title))
    {
        Ok(()) => {
            state.broadcast(&serde_json::json!({"type": "thread_title", "thread_id": thread_id, "title": title}).to_string());
            axum::Json(serde_json::json!({ "thread_id": thread_id, "title": title }))
                .into_response()
        }
        Err(e) => ApiError::internal(e).into_response(),
    }
}

pub(crate) async fn archive_thread_handler(
    State(state): State<Arc<WsState>>,
    Path(thread_id): Path<String>,
) -> Response {
    match state
        .db
        .with_conn(|conn| hq_db::chat::archive_thread(conn, &thread_id))
    {
        Ok(_) => {
            // The folders the owner allowed for this chat end with it.
            if let Err(e) =
                hq_db::owner_folder_grants::revoke_all(&state.db, &format!("web:{thread_id}"))
            {
                tracing::warn!(%e, "revoking an archived chat's folder allows failed");
            }
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => ApiError::internal(e).into_response(),
    }
}

#[cfg(test)]
mod rename_tests {
    use super::*;

    async fn body(res: Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn the_owner_names_a_chat_and_an_empty_or_unknown_one_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let state = Arc::new(WsState::new(dir.path().to_path_buf(), None));
        let id = state
            .db
            .with_conn(|c| hq_db::chat::create_thread(c, "New chat", "user", "web"))
            .unwrap()
            .thread_id;
        let ok = rename_thread_handler(
            State(state.clone()),
            Path(id.clone()),
            axum::Json(serde_json::json!({ "title": "  Shop   stock\nweek 46 " })),
        )
        .await;
        assert_eq!(ok.status(), StatusCode::OK);
        assert_eq!(body(ok).await["title"], "Shop stock week 46");
        let saved = state
            .db
            .with_conn(|c| hq_db::chat::get_thread(c, &id))
            .unwrap()
            .unwrap()
            .title;
        assert_eq!(saved, "Shop stock week 46");

        let empty = rename_thread_handler(
            State(state.clone()),
            Path(id),
            axum::Json(serde_json::json!({ "title": "   " })),
        )
        .await;
        assert_eq!(empty.status(), StatusCode::BAD_REQUEST);
        let unknown = rename_thread_handler(
            State(state),
            Path("nope".into()),
            axum::Json(serde_json::json!({ "title": "x" })),
        )
        .await;
        assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
    }
}
