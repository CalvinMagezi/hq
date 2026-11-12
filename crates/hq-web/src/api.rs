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

/// `/health`. On a request a proxy forwarded (anyone `tailscale serve` or Caddy lets in) it says
/// only that HQ is up and that it is HQ, never which build: the updater and the installers poll
/// loopback, where the build details stay. `service` stays so `hq chat --server` can tell HQ
/// from another program before it sends a token.
pub(crate) async fn health_handler(
    State(state): State<Arc<WsState>>,
    headers: axum::http::HeaderMap,
) -> axum::response::Response {
    let db_path = state.vault_path.join("_data").join("vault.db");
    let public = crate::auth::forwarded(&headers);
    if !db_path.exists() {
        let body = if public {
            serde_json::json!({ "status": "unavailable" })
        } else {
            serde_json::json!({ "status": "unavailable", "reason": "vault db missing" })
        };
        return (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(body),
        )
            .into_response();
    }
    if public {
        return axum::Json(serde_json::json!({ "status": "ok", "service": HEALTH_SERVICE }))
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
    // How many notes the search index holds. Left null when the index cannot be read, so a status read never fails for it.
    let notes = state.db.with_conn(hq_db::search::indexed_count).ok();
    axum::Json(serde_json::json!({
        "vault_path": state.vault_path.to_string_lossy(),
        "notes": notes,
    }))
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
    let folder = match search_folder(params.get("folder").map(String::as_str)) {
        Ok(folder) => folder,
        Err(why) => return crate::error::ApiError::bad_request(why).into_response(),
    };
    let prefix = folder.as_ref().map(|f| format!("{NOTEBOOKS}/{f}/"));
    let written_by_hq = match params.get("written_by").map(|w| w.trim()) {
        None | Some("") => false,
        Some("hq") => true,
        Some(_) => {
            return crate::error::ApiError::bad_request("written_by can only be hq")
                .into_response();
        }
    };
    let tag = params
        .get("tag")
        .map(|t| t.trim().trim_start_matches('#').to_string())
        .filter(|t| !t.is_empty());
    // A tag or "written by HQ" filter narrows in the index (a superset), then each hit is checked
    // against the note itself; the index query reads up to FILTERED_INDEX_CAP rows for that.
    let filtered = written_by_hq || tag.is_some();
    let index_limit = if filtered { FILTERED_INDEX_CAP } else { limit };
    let worker = state.clone();
    let (q, t) = (query.clone(), tag.clone());
    let found = tokio::task::spawn_blocking(move || {
        let filter = hq_db::search::SearchFilter {
            prefix: prefix.as_deref(),
            tag: t.as_deref(),
            written_by_hq,
        };
        let hits = worker
            .db
            .with_conn(|c| hq_db::search::keyword_search_filtered(c, &q, index_limit, &filter))
            .unwrap_or_default();
        let any = !hits.is_empty();
        // The index covers Notebooks, but Lite filters what it returns the same way as the
        // note endpoints do, so a hit in a hidden folder never reaches the client.
        let hits: Vec<_> = hits
            .into_iter()
            .filter(|h| {
                worker.path_allowed(&h.note_path)
                    && worker.resolved_allowed(&worker.vault_path.join(h.note_path.trim()))
            })
            .filter(|h| confirms(&worker, &h.note_path, t.as_deref(), written_by_hq))
            .take(limit)
            .collect();
        (any, hits)
    })
    .await
    .unwrap_or_default();
    if found.0 && (!filtered || !found.1.is_empty()) {
        return axum::Json(serde_json::json!({ "results": found.1 })).into_response();
    }
    // The FTS index lags notes written outside the web UI until its 30m sync,
    // and FTS misses partial words, so a miss falls back to a substring walk.
    // It runs off the async runtime so a large vault cannot stall other requests.
    let vault = state.vault_path.clone();
    let needle = query.to_lowercase();
    let lite = state.profile().is_lite();
    let start = match &folder {
        Some(f) => vault.join(NOTEBOOKS).join(f),
        None => vault.join(NOTEBOOKS),
    };
    // Under Lite a folder reached through a link into a hidden folder is not walked at all.
    if !state.resolved_allowed(&start) {
        return axum::Json(serde_json::json!({ "results": [] })).into_response();
    }
    // The filters run on each file before its text is searched, so `limit` counts only matches.
    let worker = state.clone();
    let results = tokio::task::spawn_blocking(move || {
        let accept = |rel: &str| confirms(&worker, rel, tag.as_deref(), written_by_hq);
        let mut results = Vec::new();
        walk_for_substring(&start, &vault, &needle, &mut results, limit, lite, &accept);
        results
    })
    .await
    .unwrap_or_default();
    axum::Json(serde_json::json!({ "results": results })).into_response()
}

const NOTEBOOKS: &str = "Notebooks";
/// Most index rows a filtered search reads before the per-note check picks `limit` of them.
const FILTERED_INDEX_CAP: usize = 500;

/// Whether the note at `rel` carries `tag` (a whole tag) and, when asked, was last written by HQ.
/// Reads the note itself, so it agrees with the lists and payloads.
fn confirms(state: &WsState, rel: &str, tag: Option<&str>, written_by_hq: bool) -> bool {
    if let Some(t) = tag {
        let tags = crate::vault_api::tags_of_file(&state.vault_path.join(rel));
        if !crate::vault_api::has_tag(&tags, t) {
            return false;
        }
    }
    !written_by_hq
        || state
            .db
            .with_conn(|c| {
                hq_db::note_marks::mark(c, rel, || {
                    crate::vault_api::file_hash(&state.vault_path, rel)
                })
            })
            .is_ok_and(|m| m.written_by == hq_db::note_marks::WRITTEN_BY_HQ)
}

/// The folder a search is narrowed to, under the notebooks, or why it is refused: it may not climb
/// (`..`), start at the root (`/`) or use a backslash. An empty value means every folder.
fn search_folder(raw: Option<&str>) -> Result<Option<String>, &'static str> {
    let Some(raw) = raw.map(str::trim).filter(|f| !f.is_empty()) else {
        return Ok(None);
    };
    if raw.starts_with('/') || raw.contains('\\') || raw.split('/').any(|seg| seg == "..") {
        return Err(
            "folder must be a path inside your notes, without .., a leading / or a backslash",
        );
    }
    let folder = raw
        .strip_prefix("Notebooks/")
        .unwrap_or(raw)
        .trim_end_matches('/');
    Ok(Some(folder.to_string()))
}

/// Substring search over the notebooks, skipping `.` and `_` folders.
fn walk_for_substring(
    dir: &std::path::Path,
    root: &std::path::Path,
    needle: &str,
    results: &mut Vec<serde_json::Value>,
    limit: usize,
    lite: bool,
    accept: &dyn Fn(&str) -> bool,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if results.len() >= limit {
            return;
        }
        // Lite does not follow links out of the notes.
        if lite && entry.file_type().is_ok_and(|t| t.is_symlink()) {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            if !name.starts_with('.') && !name.starts_with('_') {
                walk_for_substring(&path, root, needle, results, limit, lite, accept);
            }
        } else if path.extension().is_some_and(|e| e == "md")
            && path
                .strip_prefix(root)
                .is_ok_and(|rel| accept(&rel.to_string_lossy()))
            && let Some(hit) = substring_hit(&path, root, needle)
        {
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

    if !state.path_allowed(&path_param) {
        return ApiError::not_found().into_response();
    }
    let Some(abs) = resolve_in_vault(&state.vault_path, &path_param) else {
        return ApiError::Forbidden("path is outside the vault".to_string()).into_response();
    };
    if !state.resolved_allowed(&abs) {
        return ApiError::not_found().into_response();
    }

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
    if !state.path_allowed(&path_param) {
        return ApiError::not_found().into_response();
    }
    // A folder opened from Notes answers with its listing, under the same checks as a note.
    let resolved = crate::vault_api::resolve_note_path(&state.vault_path, &path_param)
        .or_else(|| crate::vault_api::resolve_folder(&state.vault_path, &path_param));
    let Some((abs, effective_path)) = resolved else {
        return ApiError::not_found().into_response();
    };
    if !state.path_allowed(&effective_path) || !state.resolved_allowed(&abs) {
        return ApiError::not_found().into_response();
    }
    // A folder listing reads every note's head and record, so it runs off the async runtime.
    let (db, vault, lite) = (
        state.db.clone(),
        state.vault_path.clone(),
        state.profile().is_lite(),
    );
    match tokio::task::spawn_blocking(move || {
        crate::vault_api::read_note(&db, &vault, &effective_path, &abs, lite)
    })
    .await
    {
        Ok(response) => response,
        Err(e) => ApiError::internal(e).into_response(),
    }
}

/// The web UI always asks for the recursive tree; it filters client-side.
pub(crate) async fn tree_handler(
    State(state): State<Arc<WsState>>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> axum::response::Response {
    let path_param = params.get("path").cloned().unwrap_or_default();
    if !state.path_allowed(&path_param) {
        return ApiError::not_found().into_response();
    }
    crate::vault_api::recursive_tree(&state.vault_path, &path_param, state.profile().is_lite())
}

pub(crate) async fn pinned_handler(State(state): State<Arc<WsState>>) -> axum::response::Response {
    let vault = state.vault_path.clone();
    let lite = state.profile().is_lite();
    let db = state.db.clone();
    match tokio::task::spawn_blocking(move || crate::vault_api::pinned_notes(&vault, lite, &db))
        .await
    {
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
    if !state.path_allowed(&path_param) {
        return ApiError::not_found().into_response();
    }

    let Some(abs) =
        resolve_in_vault(&state.vault_path, &path_param).filter(|_| !path_param.is_empty())
    else {
        return ApiError::bad_request("invalid path").into_response();
    };
    if !state.resolved_allowed(&abs) {
        return ApiError::not_found().into_response();
    }

    let content = match std::fs::read_to_string(&abs) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return ApiError::not_found().into_response();
        }
        Err(e) => return ApiError::internal(e).into_response(),
    };

    let new_content = crate::vault_api::set_pinned(&content, pin);

    match hq_vault::notes::write_atomic(&abs, new_content.as_bytes()) {
        Ok(_) => {
            crate::vault_api::record_owner_write(&state, &abs);
            if pin {
                crate::vault_api::accept_suggested_pin(&state, &path_param, &new_content);
            }
            axum::Json(serde_json::json!({"success": true})).into_response()
        }
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

#[cfg(test)]
mod search_tests {
    use super::search_folder;

    #[cfg(unix)]
    #[tokio::test]
    async fn under_lite_a_folder_linked_into_a_hidden_one_finds_nothing() {
        use std::sync::Arc;
        let v = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(v.path().join("Notebooks")).unwrap();
        std::fs::create_dir_all(v.path().join("_system")).unwrap();
        std::fs::write(v.path().join("_system/secret.md"), "quokka hidden").unwrap();
        std::os::unix::fs::symlink(v.path().join("_system"), v.path().join("Notebooks/link"))
            .unwrap();
        let state = Arc::new(
            crate::WsState::new(v.path().to_path_buf(), None)
                .with_profile(hq_core::config::Profile::Lite),
        );
        let q = std::collections::HashMap::from([
            ("q".to_string(), "quokka".to_string()),
            ("folder".to_string(), "link".to_string()),
        ]);
        let res = super::search_handler(axum::extract::State(state), axum::extract::Query(q)).await;
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(
            !String::from_utf8_lossy(&bytes).contains("secret"),
            "{}",
            String::from_utf8_lossy(&bytes)
        );
    }

    #[test]
    fn a_folder_that_climbs_or_starts_at_the_root_is_refused() {
        for bad in ["../x", "Projects/../../etc", "/etc", "Projects\\x"] {
            assert!(search_folder(Some(bad)).is_err(), "{bad}");
        }
        assert_eq!(
            search_folder(Some("Projects/Agent-HQ/")),
            Ok(Some("Projects/Agent-HQ".into()))
        );
        assert_eq!(
            search_folder(Some("Notebooks/Projects")),
            Ok(Some("Projects".into()))
        );
        assert_eq!(search_folder(Some("  ")), Ok(None));
        assert_eq!(search_folder(None), Ok(None));
    }
}
