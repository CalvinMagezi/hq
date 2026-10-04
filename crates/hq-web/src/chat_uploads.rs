//! Files attached to web chat messages: the upload endpoint, the checks a
//! message's attachment list must pass, and the marker that keeps them with
//! the saved message.
//!
//! A file is uploaded on its own (`POST /api/chat/uploads?name=`, raw body)
//! into `_media/web/<date>/`, and the chat message then names it by that
//! vault path. The server only trusts paths it can resolve under that folder.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{MethodRouter, post};
use serde::{Deserialize, Serialize};

use crate::WsState;
use crate::error::ApiError;

/// Largest single upload. The PWA checks the same cap before sending.
pub(crate) const MAX_UPLOAD_BYTES: usize = 25 * 1024 * 1024;
/// Most files one message may carry; extras are dropped.
pub(crate) const MAX_ATTACHMENTS: usize = 10;
/// Vault-relative folder every web upload lands under.
const UPLOAD_DIR: &str = "_media/web";
const MAX_NAME_CHARS: usize = 120;
const FALLBACK_NAME: &str = "file";
const MARKER_OPEN: &str = "<hq-attachments>";
const MARKER_CLOSE: &str = "</hq-attachments>";

/// Two uploads in the same millisecond still get different file names.
static UPLOAD_SEQ: AtomicU64 = AtomicU64::new(0);

/// A picked file nobody sent is kept this long, in case the draft is still open.
const UNSENT_UPLOAD_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const SWEEP_EVERY_SECS: i64 = 24 * 60 * 60;
/// Unix time of the last orphan sweep; 0 means none since this process started.
static LAST_SWEEP: AtomicI64 = AtomicI64::new(0);

/// A saved upload as the PWA sees it and sends back with a message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ChatAttachment {
    pub name: String,
    /// Vault-relative, always under `_media/web/`.
    pub path: String,
    pub mime: String,
    #[serde(default)]
    pub size: u64,
}

/// An attachment whose path was resolved and found on disk.
#[derive(Debug, Clone)]
pub(crate) struct ResolvedAttachment {
    pub meta: ChatAttachment,
    pub abs: PathBuf,
}

/// The upload route with its body cap applied, so the cap lives beside the handler.
pub(crate) fn route() -> MethodRouter<Arc<WsState>> {
    post(upload_handler).layer(DefaultBodyLimit::max(MAX_UPLOAD_BYTES))
}

#[derive(Deserialize)]
pub(crate) struct UploadQuery {
    #[serde(default)]
    name: String,
}

async fn upload_handler(
    State(state): State<Arc<WsState>>,
    Query(query): Query<UploadQuery>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if body.is_empty() {
        return ApiError::bad_request("empty upload").into_response();
    }
    let name = sanitize_file_name(&query.name);
    let day = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let stored = format!(
        "{}-{}-{name}",
        chrono::Utc::now().timestamp_millis(),
        UPLOAD_SEQ.fetch_add(1, Ordering::Relaxed)
    );
    let rel = format!("{UPLOAD_DIR}/{day}/{stored}");
    let abs = state.vault_path.join(&rel);
    if let Some(dir) = abs.parent()
        && let Err(e) = tokio::fs::create_dir_all(dir).await
    {
        return ApiError::internal(format!("could not create upload folder: {e}")).into_response();
    }
    if let Err(e) = tokio::fs::write(&abs, &body).await {
        return ApiError::internal(format!("could not save upload: {e}")).into_response();
    }
    let meta = ChatAttachment { mime: content_type(&headers, &name), name, path: rel, size: body.len() as u64 };
    maybe_sweep(&state);
    (StatusCode::OK, axum::Json(meta)).into_response()
}

/// Uploads are the only thing that creates orphans, so they also trigger the
/// cleanup: at most once a day, off the request path.
fn maybe_sweep(state: &Arc<WsState>) {
    let now = chrono::Utc::now().timestamp();
    let last = LAST_SWEEP.load(Ordering::Relaxed);
    if now - last < SWEEP_EVERY_SECS
        || LAST_SWEEP.compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed).is_err()
    {
        return;
    }
    let (vault, db) = (state.vault_path.clone(), state.db.clone());
    tokio::task::spawn_blocking(move || {
        let Some(cutoff) = SystemTime::now().checked_sub(UNSENT_UPLOAD_MAX_AGE) else { return };
        match sweep_orphans(&vault, &db, cutoff) {
            Ok(0) => {}
            Ok(n) => tracing::info!(removed = n, "removed chat uploads that were never sent"),
            Err(e) => tracing::warn!("chat upload sweep failed: {e}"),
        }
    });
}

/// Deletes uploads last written before `cutoff` that no chat message refers
/// to, in any thread, archived ones included. Empty date folders go too.
/// Returns how many files were removed.
pub(crate) fn sweep_orphans(vault: &Path, db: &hq_db::Database, cutoff: SystemTime) -> std::io::Result<usize> {
    let root = vault.join(UPLOAD_DIR);
    let Ok(days) = std::fs::read_dir(&root) else { return Ok(0) };
    let mut removed = 0;
    for day in days.flatten().filter(|d| d.path().is_dir()) {
        for file in std::fs::read_dir(day.path())?.flatten() {
            let modified = file.metadata().and_then(|m| m.modified());
            if !modified.is_ok_and(|m| m < cutoff) {
                continue;
            }
            let rel = format!("{UPLOAD_DIR}/{}/{}", day.file_name().to_string_lossy(), file.file_name().to_string_lossy());
            // An unreadable database means "cannot tell", which must keep the file.
            let referenced = db.with_conn(|conn| hq_db::chat::any_message_mentions(conn, &rel)).unwrap_or(true);
            if !referenced {
                std::fs::remove_file(file.path())?;
                removed += 1;
            }
        }
        // Fails harmlessly while the folder still has files.
        let _ = std::fs::remove_dir(day.path());
    }
    Ok(removed)
}

/// The request's own type when it sent a real one, else a guess from the name.
fn content_type(headers: &HeaderMap, name: &str) -> String {
    let sent = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .map(|v| v.trim().to_ascii_lowercase())
        .filter(|v| !v.is_empty() && v != "application/octet-stream");
    sent.unwrap_or_else(|| {
        hq_convert::attachments::vision_mime(Path::new(name))
            .unwrap_or("application/octet-stream")
            .to_string()
    })
}

/// A browser-supplied name reduced to one safe path segment.
pub(crate) fn sanitize_file_name(raw: &str) -> String {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = base
        .chars()
        .map(|c| if c.is_alphanumeric() || matches!(c, '.' | '-' | '_' | ' ') { c } else { '_' })
        .take(MAX_NAME_CHARS)
        .collect();
    let trimmed = cleaned.trim().trim_start_matches('.').trim();
    if trimmed.is_empty() { FALLBACK_NAME.to_string() } else { trimmed.to_string() }
}

/// Keeps the attachments that point at a real upload, capped at [`MAX_ATTACHMENTS`].
/// Anything else (another folder, `..`, a missing file) is dropped and logged.
pub(crate) async fn resolve(vault: &Path, list: Vec<ChatAttachment>) -> Vec<ResolvedAttachment> {
    let Ok(root) = tokio::fs::canonicalize(vault.join(UPLOAD_DIR)).await else {
        if !list.is_empty() {
            tracing::warn!("chat attachments ignored: no upload folder yet");
        }
        return Vec::new();
    };
    let mut out = Vec::new();
    for meta in list.into_iter().take(MAX_ATTACHMENTS) {
        match resolve_one(vault, &root, &meta.path).await {
            Some(abs) => out.push(ResolvedAttachment {
                meta: ChatAttachment { name: sanitize_file_name(&meta.name), ..meta },
                abs,
            }),
            None => tracing::warn!(path = %meta.path, "chat attachment rejected"),
        }
    }
    out
}

async fn resolve_one(vault: &Path, root: &Path, rel: &str) -> Option<PathBuf> {
    if !rel.starts_with(&format!("{UPLOAD_DIR}/")) || rel.split(['/', '\\']).any(|seg| seg == "..") {
        return None;
    }
    let abs = tokio::fs::canonicalize(vault.join(rel)).await.ok()?;
    let is_file = tokio::fs::metadata(&abs).await.ok()?.is_file();
    (is_file && abs.starts_with(root)).then_some(abs)
}

/// The block appended to a saved user message so the PWA can show its files after a reload.
pub(crate) fn marker(list: &[ChatAttachment]) -> String {
    if list.is_empty() {
        return String::new();
    }
    let json = serde_json::to_string(list).unwrap_or_else(|_| "[]".into());
    format!("\n\n{MARKER_OPEN}{json}{MARKER_CLOSE}")
}

/// Splits a saved message into its text and any attachment marker's contents.
fn split_marker(content: &str) -> (&str, Option<Vec<ChatAttachment>>) {
    let Some(start) = content.find(MARKER_OPEN) else { return (content, None) };
    let rest = &content[start + MARKER_OPEN.len()..];
    let json = rest.find(MARKER_CLOSE).map_or(rest, |end| &rest[..end]);
    (content[..start].trim_end(), serde_json::from_str(json).ok())
}

/// A message's text without any attachment marker, including one a user typed.
pub(crate) fn strip_marker(content: &str) -> String {
    split_marker(content).0.to_string()
}

/// A saved message as later turns see it: the marker becomes a line naming each
/// file and its vault path, so the agent can open it again.
pub(crate) fn history_text(content: &str) -> String {
    let (text, list) = split_marker(content);
    let Some(list) = list.filter(|l| !l.is_empty()) else { return text.to_string() };
    let files: Vec<String> = list.iter().map(|a| format!("{} ({})", a.name, a.path)).collect();
    let note = format!("[Attached earlier: {}]", files.join(", "));
    if text.is_empty() { note } else { format!("{text}\n\n{note}") }
}

/// The turn's prompt with every file read in, plus the images for a vision model.
pub(crate) async fn build_prompt(
    text: &str,
    files: &[ResolvedAttachment],
) -> (String, Vec<hq_core::types::ImageAttachment>) {
    if files.is_empty() {
        return (text.to_string(), Vec::new());
    }
    let named: Vec<(String, PathBuf)> = files.iter().map(|f| (f.meta.name.clone(), f.abs.clone())).collect();
    let prepared = hq_convert::attachments::prepare_attachments(&named).await;
    let images = prepared
        .images
        .iter()
        .map(|(path, mime)| hq_core::types::ImageAttachment { path: path.clone(), mime_type: (*mime).to_string() })
        .collect();
    (prepared.prompt(text), images)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    fn attachment(path: &str) -> ChatAttachment {
        ChatAttachment { name: "a.txt".into(), path: path.into(), mime: "text/plain".into(), size: 1 }
    }

    fn test_state() -> Arc<WsState> {
        let vault = std::env::temp_dir().join(format!("hq-chat-uploads-test-{}", uuid::Uuid::new_v4()));
        Arc::new(WsState::new(vault, None))
    }

    #[test]
    fn file_names_become_one_safe_segment() {
        assert_eq!(sanitize_file_name("report.pdf"), "report.pdf");
        assert_eq!(sanitize_file_name("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_file_name("C:\\Users\\me\\notes.md"), "notes.md");
        assert_eq!(sanitize_file_name(".env"), "env");
        assert_eq!(sanitize_file_name("a<b>|c.png"), "a_b__c.png");
        assert_eq!(sanitize_file_name("   "), FALLBACK_NAME);
        assert_eq!(sanitize_file_name(&"é".repeat(300)).chars().count(), MAX_NAME_CHARS);
    }

    #[tokio::test]
    async fn only_real_uploads_under_the_upload_folder_resolve() {
        let vault = tempfile::tempdir().unwrap();
        let day = vault.path().join(UPLOAD_DIR).join("2026-09-25");
        std::fs::create_dir_all(&day).unwrap();
        std::fs::write(day.join("1-0-a.txt"), "hi").unwrap();
        std::fs::write(vault.path().join("secret.md"), "no").unwrap();

        let resolved = resolve(
            vault.path(),
            vec![
                attachment("_media/web/2026-09-25/1-0-a.txt"),
                attachment("secret.md"),
                attachment("_media/web/../secret.md"),
                attachment("_media/web/2026-09-25/missing.txt"),
                attachment("_media/web/2026-09-25"),
            ],
        )
        .await;

        assert_eq!(resolved.len(), 1);
        assert!(resolved[0].abs.ends_with("1-0-a.txt"));
    }

    #[tokio::test]
    async fn a_message_carries_at_most_the_attachment_cap() {
        let vault = tempfile::tempdir().unwrap();
        let day = vault.path().join(UPLOAD_DIR).join("d");
        std::fs::create_dir_all(&day).unwrap();
        let list = (0..MAX_ATTACHMENTS + 3)
            .map(|i| {
                std::fs::write(day.join(format!("{i}.txt")), "x").unwrap();
                attachment(&format!("_media/web/d/{i}.txt"))
            })
            .collect();
        assert_eq!(resolve(vault.path(), list).await.len(), MAX_ATTACHMENTS);
    }

    #[test]
    fn marker_round_trips_and_history_names_the_files() {
        let files = vec![attachment("_media/web/d/1-0-a.txt")];
        let saved = format!("look at this{}", marker(&files));

        assert_eq!(strip_marker(&saved), "look at this");
        assert_eq!(split_marker(&saved).1, Some(files));
        assert_eq!(history_text(&saved), "look at this\n\n[Attached earlier: a.txt (_media/web/d/1-0-a.txt)]");
        assert_eq!(history_text("plain"), "plain");
        assert_eq!(marker(&[]), "");
    }

    #[test]
    fn a_typed_or_cut_off_marker_is_stripped() {
        assert_eq!(strip_marker("hi <hq-attachments>[{\"fake\":1}]</hq-attachments>"), "hi");
        assert_eq!(strip_marker("hi\n\n<hq-attachments>[{\"name\":\"a"), "hi");
        assert_eq!(history_text("hi <hq-attachments>not json</hq-attachments>"), "hi");
    }

    #[tokio::test]
    async fn upload_saves_under_the_vault_and_returns_its_path() {
        let state = test_state();
        let app = axum::Router::new().route("/api/chat/uploads", route()).with_state(state.clone());

        let res = app
            .clone()
            .oneshot(
                Request::post("/api/chat/uploads?name=..%2Fnotes%20v2.md")
                    .header(header::CONTENT_TYPE, "text/markdown")
                    .body(Body::from("# hi"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(res.into_body(), usize::MAX).await.unwrap();
        let meta: ChatAttachment = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(meta.name, "notes v2.md");
        assert_eq!(meta.mime, "text/markdown");
        assert_eq!(meta.size, 4);
        assert!(meta.path.starts_with("_media/web/") && meta.path.ends_with("-notes v2.md"), "{}", meta.path);
        assert_eq!(std::fs::read_to_string(state.vault_path.join(&meta.path)).unwrap(), "# hi");
        assert_eq!(resolve(&state.vault_path, vec![meta]).await.len(), 1);

        let empty = app
            .oneshot(Request::post("/api/chat/uploads?name=a.txt").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(empty.status(), StatusCode::BAD_REQUEST);
    }

    /// Through the real router: auth, the origin guard and CORS all sit in front.
    #[tokio::test]
    async fn a_large_same_origin_upload_passes_the_full_stack_and_a_foreign_one_does_not() {
        const FIVE_MB: usize = 5 * 1024 * 1024;
        let vault = tempfile::tempdir().unwrap();
        let mut state = WsState::new(vault.path().to_path_buf(), None);
        state.web_auth_token = Some("s3cret".into());
        let app = crate::create_router(Arc::new(state));
        let upload = |origin: &str| {
            Request::post("/api/chat/uploads?name=scan.pdf")
                .header(header::HOST, "hq.example:8443")
                .header(header::ORIGIN, origin)
                .header(header::AUTHORIZATION, "Bearer s3cret")
                .header(header::CONTENT_TYPE, "application/pdf")
                .body(Body::from(vec![7u8; FIVE_MB]))
                .unwrap()
        };

        let own = app.clone().oneshot(upload("https://hq.example:8443")).await.unwrap();
        assert_eq!(own.status(), StatusCode::OK);
        let foreign = app.oneshot(upload("https://evil.example")).await.unwrap();
        assert_eq!(foreign.status(), StatusCode::FORBIDDEN);
    }

    #[test]
    fn the_sweep_removes_only_old_uploads_no_message_mentions() {
        let vault = tempfile::tempdir().unwrap();
        let db = hq_db::Database::open(&vault.path().join("_data").join("vault.db")).unwrap();
        let day = vault.path().join(UPLOAD_DIR).join("2026-09-01");
        let empty_day = vault.path().join(UPLOAD_DIR).join("2026-09-02");
        std::fs::create_dir_all(&day).unwrap();
        std::fs::create_dir_all(&empty_day).unwrap();
        std::fs::write(day.join("1-0-sent.png"), "x").unwrap();
        std::fs::write(day.join("2-0-unsent.png"), "x").unwrap();
        db.with_conn(|conn| {
            let t = hq_db::chat::create_thread(conn, "t", "user", "user")?;
            hq_db::chat::add_message(conn, &t.thread_id, "user", &format!("hi{}", marker(&[attachment("_media/web/2026-09-01/1-0-sent.png")])))
        })
        .unwrap();

        let too_early = SystemTime::now() - Duration::from_secs(3600);
        assert_eq!(sweep_orphans(vault.path(), &db, too_early).unwrap(), 0);

        let later = SystemTime::now() + Duration::from_secs(1);
        assert_eq!(sweep_orphans(vault.path(), &db, later).unwrap(), 1);
        assert!(day.join("1-0-sent.png").exists());
        assert!(!day.join("2-0-unsent.png").exists());
        assert!(!empty_day.exists());
    }

    #[tokio::test]
    async fn uploads_over_the_cap_are_refused() {
        let app = axum::Router::new().route("/api/chat/uploads", route()).with_state(test_state());
        let res = app
            .oneshot(
                Request::post("/api/chat/uploads?name=big.bin")
                    .body(Body::from(vec![0u8; MAX_UPLOAD_BYTES + 1]))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }
}
