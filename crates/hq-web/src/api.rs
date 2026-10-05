//! HTTP API route handlers: health, vault status, search, note read, tree,
//! pins, vault assets, and the admin broadcast.

use crate::WsState;
use crate::error::ApiError;
use crate::vault_api::resolve_in_vault;
use axum::extract::State;
use axum::response::IntoResponse;
use std::sync::Arc;

/// Value of `/health`'s `service` field. `hq chat` requires it before it sends
/// a bearer token to whatever answers on a port.
pub const HEALTH_SERVICE: &str = "agent-hq";

pub(crate) async fn health_handler(State(state): State<Arc<WsState>>) -> axum::response::Response {
    let db_path = state.vault_path.join("_data").join("vault.db");
    if !db_path.exists() {
        return (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(serde_json::json!({
                "status": "unavailable",
                "reason": "vault db missing",
            })),
        )
            .into_response();
    }
    axum::Json(serde_json::json!({
        "status": "ok",
        "service": HEALTH_SERVICE,
        "version": hq_core::build_info::version(),
        "git_sha": hq_core::build_info::git_sha(),
        "build_time": hq_core::build_info::build_time(),
        "mcp": state.registry.is_some(),
    }))
    .into_response()
}

pub(crate) async fn vault_status_handler(State(state): State<Arc<WsState>>) -> impl IntoResponse {
    axum::Json(serde_json::json!({ "vault_path": state.vault_path.to_string_lossy() }))
}

pub(crate) async fn search_handler(
    State(state): State<Arc<WsState>>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> axum::response::Response {
    let query = params.get("q").cloned().unwrap_or_default();
    let limit: usize = params
        .get("limit")
        .and_then(|l| l.parse().ok())
        .unwrap_or(20);

    if query.is_empty() {
        return axum::Json(serde_json::json!({"results": [], "error": "no query"})).into_response();
    }

    if let Ok(hits) = state
        .db
        .with_conn(|c| hq_db::search::keyword_search(c, &query, limit))
        && !hits.is_empty()
    {
        return axum::Json(serde_json::json!({ "results": hits })).into_response();
    }
    // The FTS index lags notes written outside the web UI until its 30m sync,
    // and FTS misses partial words, so a miss falls back to a substring walk.
    // It runs off the async runtime so a large vault cannot stall other requests.
    let vault = state.vault_path.clone();
    let needle = query.to_lowercase();
    let results = tokio::task::spawn_blocking(move || {
        let mut results = Vec::new();
        walk_for_substring(
            &vault.join("Notebooks"),
            &vault,
            &needle,
            &mut results,
            limit,
        );
        results
    })
    .await
    .unwrap_or_default();
    axum::Json(serde_json::json!({ "results": results })).into_response()
}

/// Substring search over the notebooks, skipping `.` and `_` folders.
fn walk_for_substring(
    dir: &std::path::Path,
    root: &std::path::Path,
    needle: &str,
    results: &mut Vec<serde_json::Value>,
    limit: usize,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if results.len() >= limit {
            return;
        }
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            if !name.starts_with('.') && !name.starts_with('_') {
                walk_for_substring(&path, root, needle, results, limit);
            }
        } else if let Some(hit) = substring_hit(&path, root, needle) {
            results.push(hit);
        }
    }
}

const SNIPPET_CHARS: usize = 150;

fn substring_hit(
    path: &std::path::Path,
    root: &std::path::Path,
    needle: &str,
) -> Option<serde_json::Value> {
    if path.extension().is_none_or(|e| e != "md") {
        return None;
    }
    let content = std::fs::read_to_string(path).ok()?;
    if !content.to_lowercase().contains(needle) {
        return None;
    }
    let rel = path
        .strip_prefix(root)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let title = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let snippet: String = content
        .lines()
        .find(|l| l.to_lowercase().contains(needle))
        .unwrap_or("")
        .trim()
        .chars()
        .take(SNIPPET_CHARS)
        .collect();
    Some(
        serde_json::json!({"note_path": rel, "title": title, "snippet": snippet, "relevance": 1.0}),
    )
}

/// Types a browser executes or renders as a document when navigated to.
fn is_active_content(mime: &str) -> bool {
    mime.starts_with("image/svg+xml") || mime.starts_with("text/html") || mime.contains("xml")
}

pub(crate) async fn vault_asset_handler(
    State(state): State<Arc<WsState>>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> axum::response::Response {
    let path_param = params.get("path").cloned().unwrap_or_default();
    if path_param.is_empty() {
        return ApiError::bad_request("path is required").into_response();
    }

    let Some(abs) = resolve_in_vault(&state.vault_path, &path_param) else {
        return ApiError::Forbidden("path is outside the vault".to_string()).into_response();
    };

    match std::fs::read(&abs) {
        Ok(data) => {
            let ext = abs
                .extension()
                .and_then(|e| e.to_str())
                .map(str::to_ascii_lowercase);
            let mime = match ext.as_deref() {
                Some("png") => "image/png",
                Some("jpg" | "jpeg") => "image/jpeg",
                Some("gif") => "image/gif",
                Some("svg") => "image/svg+xml",
                Some("html" | "htm") => "text/html; charset=utf-8",
                Some("xhtml") => "application/xhtml+xml",
                Some("xml") => "application/xml",
                Some("pdf") => "application/pdf",
                Some("md") => "text/markdown; charset=utf-8",
                Some("txt" | "csv" | "json" | "yaml" | "yml") => "text/plain; charset=utf-8",
                _ => "application/octet-stream",
            };
            let mut res = (
                axum::http::StatusCode::OK,
                [(axum::http::header::CONTENT_TYPE, mime)],
                axum::body::Body::from(data),
            )
                .into_response();
            if is_active_content(mime) {
                // Vault files can come from anyone who can write the vault. Navigating
                // to one must not run script on HQ's origin; `<img>` and blob previews
                // are unaffected.
                let headers = res.headers_mut();
                headers.insert(
                    axum::http::header::CONTENT_DISPOSITION,
                    axum::http::HeaderValue::from_static("attachment"),
                );
                headers.insert(
                    axum::http::header::CONTENT_SECURITY_POLICY,
                    axum::http::HeaderValue::from_static("sandbox"),
                );
            }
            res
        }
        Err(_) => ApiError::not_found().into_response(),
    }
}

pub(crate) async fn note_read_handler(
    State(state): State<Arc<WsState>>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> axum::response::Response {
    let path_param = params.get("path").cloned().unwrap_or_default();
    if path_param.is_empty() {
        return ApiError::bad_request("path is required").into_response();
    }
    if crate::vault_api::is_rejected_note_ref(&path_param) {
        return ApiError::bad_request("invalid path").into_response();
    }
    let Some((abs, effective_path)) =
        crate::vault_api::resolve_note_path(&state.vault_path, &path_param)
    else {
        return ApiError::not_found().into_response();
    };
    crate::vault_api::read_note(&state.vault_path, &effective_path, &abs)
}

/// The web UI always asks for the recursive tree; it filters client-side.
pub(crate) async fn tree_handler(
    State(state): State<Arc<WsState>>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> axum::response::Response {
    let path_param = params.get("path").cloned().unwrap_or_default();
    crate::vault_api::recursive_tree(&state.vault_path, &path_param)
}

pub(crate) async fn pinned_handler(State(state): State<Arc<WsState>>) -> axum::response::Response {
    let vault = state.vault_path.clone();
    match tokio::task::spawn_blocking(move || crate::vault_api::pinned_notes(&vault)).await {
        Ok(notes) => axum::Json(serde_json::json!({"notes": notes})).into_response(),
        Err(e) => ApiError::internal(e).into_response(),
    }
}

pub(crate) async fn pin_toggle_handler(
    State(state): State<Arc<WsState>>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> axum::response::Response {
    let path_param = body
        .get("path")
        .and_then(|p| p.as_str())
        .unwrap_or("")
        .to_string();
    let pin = body.get("pin").and_then(|p| p.as_bool()).unwrap_or(false);

    let Some(abs) =
        resolve_in_vault(&state.vault_path, &path_param).filter(|_| !path_param.is_empty())
    else {
        return ApiError::bad_request("invalid path").into_response();
    };

    let content = match std::fs::read_to_string(&abs) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return ApiError::not_found().into_response();
        }
        Err(e) => return ApiError::internal(e).into_response(),
    };

    let new_content = crate::vault_api::set_pinned(&content, pin);

    match hq_vault::notes::write_atomic(&abs, new_content.as_bytes()) {
        Ok(_) => axum::Json(serde_json::json!({"success": true})).into_response(),
        Err(e) => ApiError::internal(e).into_response(),
    }
}

/// Fan-out into the WS channel, used by `hq notify-restart` (from
/// the deploy tooling) to warn clients before a CI restart and confirm after.
pub(crate) async fn admin_broadcast_handler(
    State(state): State<Arc<WsState>>,
    axum::Json(body): axum::Json<serde_json::Value>,
) -> impl IntoResponse {
    let Some(msg) = body
        .get("message")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    else {
        return ApiError::bad_request("missing message").into_response();
    };
    state.broadcast(&serde_json::json!({ "type": "system:notice", "message": msg }).to_string());
    axum::Json(serde_json::json!({"ok": true})).into_response()
}
