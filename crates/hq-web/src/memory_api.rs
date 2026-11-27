//! The owner's page for what HQ remembers: `/api/memory`.
//!
//! HQ saves short summaries of what the owner says. These routes let the owner's apps list them,
//! search them, correct one, pin one, mark one out of date, keep one that came from a shared
//! channel, and forget one together with everything HQ made from it.
//!
//! They are for the owner's apps only. Nothing in the tools or the MCP gateway names them (a test
//! below reads those crates), so no model, helper or connector can read, change or delete a memory
//! through them, and Lite does not serve them. A memory is deleted for good, so the app is expected
//! to ask first.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use hq_db::memories_owner::{self, ListFilter, MAX_PAGE, MAX_SUMMARY_CHARS, OwnerRow, Standing};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::WsState;
use crate::error::ApiError;

/// How many memories one page holds unless the app asks for another size.
const DEFAULT_PAGE: i64 = 50;

#[derive(Debug, Deserialize)]
pub(crate) struct ListParams {
    /// Words to look for. When set, the other filters are ignored.
    #[serde(default)]
    q: String,
    /// `all` (default), `review`, `outdated`, `unattributed` or `pinned`.
    #[serde(default)]
    filter: String,
    limit: Option<i64>,
    offset: Option<i64>,
}

/// `GET /api/memory?q=&filter=&limit=&offset=`: the memories, newest first, with how many there
/// are of each kind, and how recall is set.
pub(crate) async fn list_handler(
    State(state): State<Arc<WsState>>,
    Query(params): Query<ListParams>,
) -> Response {
    let limit = params.limit.unwrap_or(DEFAULT_PAGE).clamp(1, MAX_PAGE);
    let offset = params.offset.unwrap_or(0).max(0);
    let query = params.q.trim().to_string();
    let filter = ListFilter {
        status: match params.filter.as_str() {
            "review" => Some("review".to_string()),
            "outdated" => Some("superseded".to_string()),
            _ => None,
        },
        unattributed_only: params.filter == "unattributed",
        pinned_only: params.filter == "pinned",
        limit,
        offset,
    };
    let found = state
        .db_blocking(move |conn| {
            let counts = memories_owner::counts(conn)?;
            let items = if query.is_empty() {
                memories_owner::list(conn, &filter)?
            } else {
                memories_owner::search(conn, &query, limit)?
            };
            Ok((counts, items))
        })
        .await;
    match found {
        Ok((counts, items)) => Json(json!({
            "counts": counts,
            "items": items,
            "recall": state.hq_config.as_ref().map(|c| c.daemon.memory_recall),
        }))
        .into_response(),
        Err(e) => ApiError::from(e).into_response(),
    }
}

/// `GET /api/memory/{id}`: one memory.
pub(crate) async fn get_handler(
    State(state): State<Arc<WsState>>,
    Path(id): Path<i64>,
) -> Response {
    match state
        .db_blocking(move |conn| memories_owner::get(conn, id))
        .await
    {
        Ok(Some(row)) => Json(row).into_response(),
        Ok(None) => ApiError::not_found().into_response(),
        Err(e) => ApiError::from(e).into_response(),
    }
}

/// What the app sees after a change: the memory as it now stands.
async fn changed(state: &Arc<WsState>, id: i64, done: bool) -> Response {
    if !done {
        return ApiError::not_found().into_response();
    }
    match state
        .db_blocking(move |conn| memories_owner::get(conn, id))
        .await
    {
        Ok(Some(row)) => Json(json!({"success": true, "item": row})).into_response(),
        Ok(None) => ApiError::not_found().into_response(),
        Err(e) => ApiError::from(e).into_response(),
    }
}

/// `POST /api/memory/{id}/edit {summary}`: replace the text with the owner's own. It becomes
/// theirs: active, from the owner, and marked as edited by them.
pub(crate) async fn edit_handler(
    State(state): State<Arc<WsState>>,
    Path(id): Path<i64>,
    Json(body): Json<Value>,
) -> Response {
    let text = body
        .get("summary")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if text.is_empty() {
        return ApiError::bad_request("write what this memory should say, or forget it instead")
            .into_response();
    }
    if text.chars().count() > MAX_SUMMARY_CHARS {
        return ApiError::bad_request(format!(
            "a memory can be at most {MAX_SUMMARY_CHARS} characters"
        ))
        .into_response();
    }
    match state
        .db_blocking(move |conn| memories_owner::edit(conn, id, &text))
        .await
    {
        Ok(done) => changed(&state, id, done).await,
        Err(e) => ApiError::from(e).into_response(),
    }
}

/// `POST /api/memory/{id}/pin {pinned}`: keep a memory from ever ageing out, or let it age again.
pub(crate) async fn pin_handler(
    State(state): State<Arc<WsState>>,
    Path(id): Path<i64>,
    Json(body): Json<Value>,
) -> Response {
    let Some(pinned) = body.get("pinned").and_then(Value::as_bool) else {
        return ApiError::bad_request("say whether to pin it: {\"pinned\": true}").into_response();
    };
    match state
        .db_blocking(move |conn| memories_owner::set_pinned(conn, id, pinned))
        .await
    {
        Ok(done) => changed(&state, id, done).await,
        Err(e) => ApiError::from(e).into_response(),
    }
}

/// `POST /api/memory/{id}/standing {standing}`: `active` (also vouches for a memory that came
/// from a shared channel), `review` (hold it back) or `outdated` (kept, never brought up).
pub(crate) async fn standing_handler(
    State(state): State<Arc<WsState>>,
    Path(id): Path<i64>,
    Json(body): Json<Value>,
) -> Response {
    let standing = match body.get("standing").and_then(Value::as_str) {
        Some("active") => Standing::Active,
        Some("review") => Standing::Review,
        Some("outdated") => Standing::OutOfDate,
        _ => {
            return ApiError::bad_request("standing must be active, review or outdated")
                .into_response();
        }
    };
    match state
        .db_blocking(move |conn| memories_owner::set_standing(conn, id, standing))
        .await
    {
        Ok(done) => changed(&state, id, done).await,
        Err(e) => ApiError::from(e).into_response(),
    }
}

/// `POST /api/memory/{id}/forget`: delete a memory and everything HQ made from it. Answers what
/// was removed, and `not_reached`, what could not be: the app shows it as it is.
pub(crate) async fn forget_handler(
    State(state): State<Arc<WsState>>,
    Path(id): Path<i64>,
) -> Response {
    let (db, vault) = (state.db.clone(), state.vault_path.clone());
    let done =
        tokio::task::spawn_blocking(move || hq_memory::forget::forget(&db, &vault, id)).await;
    match done {
        Ok(Ok(Some(report))) => Json(json!({
            "success": true,
            "report": report,
            "not_reached": hq_memory::forget::NOT_REACHED,
        }))
        .into_response(),
        Ok(Ok(None)) => ApiError::not_found().into_response(),
        Ok(Err(e)) => ApiError::internal(e).into_response(),
        Err(e) => ApiError::internal(e).into_response(),
    }
}

/// `GET /api/memory/export`: every memory as JSON, for the owner to keep or move.
pub(crate) async fn export_handler(State(state): State<Arc<WsState>>) -> Response {
    let all = state
        .db_blocking(|conn| {
            let mut all: Vec<OwnerRow> = Vec::new();
            loop {
                let page = memories_owner::list(
                    conn,
                    &ListFilter {
                        limit: MAX_PAGE,
                        offset: all.len() as i64,
                        ..ListFilter::default()
                    },
                )?;
                if page.is_empty() {
                    return Ok(all);
                }
                all.extend(page);
            }
        })
        .await;
    match all {
        Ok(items) => Json(json!({"count": items.len(), "items": items})).into_response(),
        Err(e) => ApiError::from(e).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use crate::{WsState, create_router};
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use serde_json::{Value, json};
    use std::sync::Arc;
    use tower::ServiceExt;

    struct App {
        vault: tempfile::TempDir,
        state: Arc<WsState>,
        app: axum::Router,
    }

    impl App {
        fn new() -> Self {
            let vault = tempfile::TempDir::new().unwrap();
            let state = Arc::new(WsState::new(vault.path().to_path_buf(), None));
            let app = create_router(state.clone());
            Self { vault, state, app }
        }

        fn memory(&self, summary: &str) -> i64 {
            self.state
                .db
                .with_conn(|c| {
                    c.execute(
                        "INSERT INTO memories (source, harness, summary, content, entities, topics, importance, created_at)
                         VALUES ('chat-turn', 'web', ?1, ?1, '[]', '[]', 0.5, '2026-06-01T00:00:00+00:00')",
                        [summary],
                    )?;
                    Ok(c.last_insert_rowid())
                })
                .unwrap()
        }

        async fn call(&self, method: &str, uri: &str, body: &str) -> (StatusCode, Value) {
            let request = Request::builder()
                .method(method)
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap();
            let response = self.app.clone().oneshot(request).await.unwrap();
            let status = response.status();
            let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
            (
                status,
                serde_json::from_slice(&bytes).unwrap_or(Value::Null),
            )
        }
    }

    #[tokio::test]
    async fn the_list_shows_the_memories_with_how_many_there_are_of_each_kind() {
        let t = App::new();
        let a = t.memory("Acme delivers on Thursdays");
        let b = t.memory("Group chat moves to Sundays");
        t.state
            .db
            .with_conn(|c| {
                c.execute(
                    "UPDATE memories SET status = 'review', origin = 'unknown_channel' WHERE id = ?1",
                    [b],
                )?;
                Ok(())
            })
            .unwrap();

        let (status, all) = t.call("GET", "/api/memory", "").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(all["items"].as_array().unwrap().len(), 2);
        assert_eq!(all["counts"]["total"], 2);
        assert_eq!(all["counts"]["review"], 1);
        assert_eq!(all["counts"]["unattributed"], 1);
        assert_eq!(all["recall"], "shadow");

        let (_, held) = t.call("GET", "/api/memory?filter=unattributed", "").await;
        assert_eq!(held["items"].as_array().unwrap().len(), 1);
        assert_eq!(held["items"][0]["id"], b);

        let (_, found) = t.call("GET", "/api/memory?q=thursdays", "").await;
        assert_eq!(found["items"][0]["id"], a);
        assert_eq!(found["items"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn one_memory_is_fetched_by_number_and_a_missing_one_is_a_404() {
        let t = App::new();
        let id = t.memory("Acme delivers on Thursdays");
        let (status, row) = t.call("GET", &format!("/api/memory/{id}"), "").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(row["summary"], "Acme delivers on Thursdays");
        assert_eq!(row["origin"], "owner");
        assert_eq!(
            t.call("GET", "/api/memory/9999", "").await.0,
            StatusCode::NOT_FOUND
        );
    }

    #[tokio::test]
    async fn an_edit_makes_the_memory_the_owners_and_refuses_nothing_or_too_much() {
        let t = App::new();
        let id = t.memory("Acme delivers on Tuesdays");
        let uri = format!("/api/memory/{id}/edit");

        let (status, done) = t
            .call(
                "POST",
                &uri,
                r#"{"summary": "  Acme delivers on Thursdays "}"#,
            )
            .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(done["item"]["summary"], "Acme delivers on Thursdays");
        assert_eq!(done["item"]["provenance"], "owner_edited");

        assert_eq!(
            t.call("POST", &uri, r#"{"summary": "   "}"#).await.0,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(t.call("POST", &uri, "{}").await.0, StatusCode::BAD_REQUEST);
        let huge = json!({"summary": "x".repeat(1001)}).to_string();
        assert_eq!(t.call("POST", &uri, &huge).await.0, StatusCode::BAD_REQUEST);
        assert_eq!(
            t.call("POST", "/api/memory/9999/edit", r#"{"summary": "a"}"#)
                .await
                .0,
            StatusCode::NOT_FOUND
        );
    }

    #[tokio::test]
    async fn pinning_and_standing_change_how_a_memory_is_treated() {
        let t = App::new();
        let id = t.memory("Group chat moves to Sundays");
        t.state
            .db
            .with_conn(|c| {
                c.execute(
                    "UPDATE memories SET status = 'review', origin = 'unknown_channel' WHERE id = ?1",
                    [id],
                )?;
                Ok(())
            })
            .unwrap();

        let (_, pinned) = t
            .call(
                "POST",
                &format!("/api/memory/{id}/pin"),
                r#"{"pinned": true}"#,
            )
            .await;
        assert_eq!(pinned["item"]["pinned"], true);
        assert_eq!(
            t.call("POST", &format!("/api/memory/{id}/pin"), "{}")
                .await
                .0,
            StatusCode::BAD_REQUEST
        );

        let uri = format!("/api/memory/{id}/standing");
        let (_, kept) = t.call("POST", &uri, r#"{"standing": "active"}"#).await;
        assert_eq!(
            (
                kept["item"]["status"].as_str(),
                kept["item"]["origin"].as_str()
            ),
            (Some("active"), Some("owner"))
        );
        let (_, old) = t.call("POST", &uri, r#"{"standing": "outdated"}"#).await;
        assert_eq!(old["item"]["status"], "superseded");
        assert_eq!(
            t.call("POST", &uri, r#"{"standing": "deleted"}"#).await.0,
            StatusCode::BAD_REQUEST
        );
    }

    #[tokio::test]
    async fn forgetting_deletes_the_memory_and_says_what_it_could_not_reach() {
        let t = App::new();
        let id = t.memory("Acme delivers on Thursdays");
        let other = t.memory("The loan is due on the 15th");

        let (status, done) = t
            .call("POST", &format!("/api/memory/{id}/forget"), "")
            .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(done["success"], true);
        assert!(done["not_reached"].as_array().unwrap().len() >= 2, "{done}");
        assert_eq!(
            t.call("GET", &format!("/api/memory/{id}"), "").await.0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            t.call("GET", &format!("/api/memory/{other}"), "").await.0,
            StatusCode::OK
        );
        assert_eq!(
            t.call("POST", &format!("/api/memory/{id}/forget"), "")
                .await
                .0,
            StatusCode::NOT_FOUND,
            "a second time there is nothing to forget"
        );
        // Nothing but the fingerprint of the text is kept.
        let kept: i64 = t
            .state
            .db
            .with_conn(|c| {
                Ok(c.query_row("SELECT COUNT(*) FROM memory_tombstones", [], |r| r.get(0))?)
            })
            .unwrap();
        assert_eq!(kept, 1);
        let _ = &t.vault;
    }

    #[tokio::test]
    async fn the_export_lists_everything_past_one_page() {
        let t = App::new();
        for i in 0..205 {
            t.memory(&format!("fact number {i}"));
        }
        let (status, all) = t.call("GET", "/api/memory/export", "").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(all["count"], 205);
        assert_eq!(all["items"].as_array().unwrap().len(), 205);
    }

    #[test]
    fn lite_does_not_serve_what_hq_remembers() {
        assert!(!crate::profile_gate::lite_allows("/api/memory"));
        assert!(!crate::profile_gate::lite_allows("/api/memory/12/forget"));
    }

    /// The owner's memory controls are for the owner's apps only: nothing in the tools, the agent or
    /// the MCP gateway names them, so no model, helper or connector can ask for them.
    #[test]
    fn no_tool_or_mcp_route_reaches_the_owner_only_memory_routes() {
        const NAMES: [&str; 3] = ["api/memory", "memories_owner", "forget::forget"];
        let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        for krate in ["hq-tools/src", "hq-mcp/src", "hq-agent/src"] {
            let mut stack = vec![crates.join(krate)];
            while let Some(dir) = stack.pop() {
                for entry in std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()) {
                    let path = entry.path();
                    if path.is_dir() {
                        stack.push(path);
                    } else if path.extension().is_some_and(|e| e == "rs") {
                        let text = std::fs::read_to_string(&path).unwrap();
                        for name in NAMES {
                            assert!(!text.contains(name), "{} names {name}", path.display());
                        }
                    }
                }
            }
        }
    }
}
