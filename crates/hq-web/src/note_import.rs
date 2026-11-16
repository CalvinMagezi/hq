//! Direct file import for the owner's apps (`POST /api/note/import?name=...`).
//!
//! Files are staged under `Notebooks/Inbox/Imports/` rather than `_archive/originals/`
//! so `Importer` writes the converted note beside them in `Notebooks/` and never touches
//! `_system` or `_archive`.
//!
//! A single response is returned once conversion finishes; no streaming progress is sent
//! because uploads are capped at `Limits::default().max_input_bytes` (64 MiB) and the
//! phone shows a spinner until the answer arrives.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Query, State};
use axum::response::{IntoResponse, Response};
use axum::routing::{MethodRouter, post};
use hq_convert::ingest::sniff::Support;
use hq_convert::ingest::{self, Capabilities, Kind, Limits};
use hq_tools::provenance::Writer;
use hq_tools::vault::{Caller, ImportSettings, Importer, Source};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::WsState;
use crate::error::ApiError;

/// Staging folder inside `Notebooks/` where uploaded originals land before conversion.
const STAGING_DIR: &str = "Notebooks/Inbox/Imports";
/// Cap sanitized file names below the 255-byte filesystem limit, leaving room for `" (N)"`
/// and `write_atomic`'s temporary suffix.
const MAX_NAME_BYTES: usize = 200;
const SUFFIX_RESERVE_BYTES: usize = 16;
const MAX_SKIP_RETRIES: usize = 64;

/// Direct uploads are rare; one lock across staging + `Importer::run` prevents concurrent
/// same-name uploads from racing or cleaning up a file another request reused.
static IMPORT_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Deserialize)]
pub(crate) struct ImportQuery {
    #[serde(default)]
    name: String,
}

fn max_import_bytes() -> usize {
    usize::try_from(Limits::default().max_input_bytes).unwrap_or(usize::MAX)
}

/// The direct import route with the importer's body-size cap applied before reading.
pub(crate) fn route() -> MethodRouter<Arc<WsState>> {
    post(import_handler).layer(DefaultBodyLimit::max(max_import_bytes()))
}

async fn import_handler(
    State(state): State<Arc<WsState>>,
    Query(query): Query<ImportQuery>,
    body: Bytes,
) -> Response {
    let caps = ingest::capabilities();
    import_with_caps(&state, &query.name, body, &caps).await
}

fn is_absolute_or_traversal(raw: &str) -> bool {
    if raw.split(['/', '\\']).any(|seg| seg == "..")
        || raw.starts_with('/')
        || raw.starts_with('\\')
    {
        return true;
    }
    let bytes = raw.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn truncate_utf8_bytes(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

fn sanitize_import_name(raw: &str) -> Result<String, ApiError> {
    if raw.chars().any(char::is_control) || is_absolute_or_traversal(raw) {
        return Err(ApiError::bad_request("invalid file name"));
    }
    let base = raw.rsplit(['/', '\\']).next().unwrap_or(raw).trim();
    let cleaned: String = base
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '.' | '-' | '_' | ' ') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = cleaned.trim().trim_start_matches('.');
    if trimmed.is_empty() {
        return Err(ApiError::bad_request("file name is required"));
    }
    let (stem, ext) = trimmed
        .rsplit_once('.')
        .filter(|(s, e)| !s.is_empty() && !e.is_empty())
        .ok_or_else(|| ApiError::bad_request("file name must have a supported extension"))?;
    let kind = Kind::from_extension(ext);
    if matches!(
        kind,
        Kind::Unknown | Kind::Markdown | Kind::Zip | Kind::Epub
    ) {
        return Err(ApiError::bad_request(format!(
            "unsupported file type: .{ext}"
        )));
    }
    let ext_lower = ext.to_ascii_lowercase();
    // `validate_path_in_vault` rejects any `..` substring, so collapse repeated dots in the stem
    // after `is_absolute_or_traversal` has already refused `..` path segments.
    let mut safe_stem = stem.trim_end().to_string();
    while safe_stem.contains("..") {
        safe_stem = safe_stem.replace("..", ".");
    }
    let max_stem_bytes = MAX_NAME_BYTES
        .saturating_sub(ext_lower.len() + 1 + SUFFIX_RESERVE_BYTES)
        .max(4);
    let short_stem = truncate_utf8_bytes(&safe_stem, max_stem_bytes).trim();
    if short_stem.is_empty() {
        return Err(ApiError::bad_request("file name is required"));
    }
    Ok(format!("{short_stem}.{ext_lower}"))
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Stages `bytes` under `STAGING_DIR`, reusing an identical existing file so `Importer`'s
/// `source`/`source_sha256` idempotency holds without overwriting any different file.
fn stage_file(
    vault_root: &Path,
    file_name: &str,
    bytes: &[u8],
    start_suffix: usize,
) -> Result<(String, PathBuf, bool, usize), ApiError> {
    let incoming_hash = sha256_hex(bytes);
    let (stem, ext) = file_name
        .rsplit_once('.')
        .ok_or_else(|| ApiError::bad_request("file name must have a supported extension"))?;
    let mut suffix = start_suffix.max(1);
    loop {
        let candidate = if suffix == 1 {
            file_name.to_string()
        } else {
            format!("{stem} ({suffix}).{ext}")
        };
        let rel = format!("{STAGING_DIR}/{candidate}");
        let abs = vault_root.join(&rel);
        match abs.symlink_metadata() {
            Ok(meta) => {
                if !meta.file_type().is_symlink()
                    && meta.is_file()
                    && std::fs::read(&abs).is_ok_and(|b| sha256_hex(&b) == incoming_hash)
                {
                    return Ok((rel, abs, false, suffix));
                }
                suffix += 1;
            }
            Err(_) => {
                if let Some(parent) = abs.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| {
                        ApiError::internal(format!("could not create import folder: {e}"))
                    })?;
                }
                hq_vault::notes::write_atomic(&abs, bytes)
                    .map_err(|e| ApiError::internal(format!("could not save file: {e}")))?;
                return Ok((rel, abs, true, suffix));
            }
        }
    }
}

fn cleanup_staged(abs: &Path, staged_new: bool) {
    if staged_new {
        let _ = std::fs::remove_file(abs);
    }
}

async fn import_with_caps(
    state: &Arc<WsState>,
    raw_name: &str,
    body: Bytes,
    caps: &Capabilities,
) -> Response {
    if body.is_empty() {
        return ApiError::bad_request("empty file").into_response();
    }
    let file_name = match sanitize_import_name(raw_name) {
        Ok(name) => name,
        Err(e) => return e.into_response(),
    };
    if !state.path_allowed(STAGING_DIR) {
        return ApiError::not_found().into_response();
    }

    let _guard = IMPORT_LOCK.lock().await;
    let mut start_suffix = 1usize;
    for _ in 0..MAX_SKIP_RETRIES {
        let vault_root = state.vault_path.clone();
        let payload = body.clone();
        let staged_name = file_name.clone();
        let (rel, abs, staged_new, used_suffix) = match tokio::task::spawn_blocking(move || {
            stage_file(&vault_root, &staged_name, &payload, start_suffix)
        })
        .await
        {
            Ok(Ok(res)) => res,
            Ok(Err(e)) => return e.into_response(),
            Err(e) => return ApiError::internal(e).into_response(),
        };

        let support = ingest::sniff(&body, &file_name).kind.support(caps);
        if support != Support::Ready {
            cleanup_staged(&abs, staged_new);
            return ApiError::unprocessable(support.label()).into_response();
        }

        let vault = match hq_vault::VaultClient::new(state.vault_path.clone()) {
            Ok(v) => Arc::new(v),
            Err(e) => {
                cleanup_staged(&abs, staged_new);
                return ApiError::internal(e).into_response();
            }
        };
        let importer = Importer::new(vault, Some(Arc::new(state.db.clone())));
        let report = match importer
            .run(
                Source::Path(rel),
                &ImportSettings::default(),
                1,
                &Caller::Known(Writer::owner()),
            )
            .await
        {
            Ok(r) => r,
            Err(e) => {
                cleanup_staged(&abs, staged_new);
                return ApiError::unprocessable(e.to_string()).into_response();
            }
        };

        let Some(entry) = report["results"].as_array().and_then(|a| a.first()) else {
            cleanup_staged(&abs, staged_new);
            return ApiError::unprocessable("could not import this file").into_response();
        };
        // If the user deleted an earlier staged original and re-uploaded different bytes under
        // the same name, `Importer` skips the stale `source` path; advance to a numeric suffix.
        if entry["action"] == "skipped" && entry["unchanged"].as_bool() != Some(true) {
            cleanup_staged(&abs, staged_new);
            start_suffix = used_suffix + 1;
            continue;
        }
        return build_response(entry, &abs, staged_new);
    }
    ApiError::unprocessable("could not find an available import name").into_response()
}

fn build_response(entry: &Value, abs: &Path, staged_new: bool) -> Response {
    let action = entry["action"].as_str().unwrap_or("failed");
    let status = match action {
        "created" | "updated" => "imported",
        "skipped" if entry["unchanged"].as_bool() == Some(true) => "already_imported",
        _ => {
            cleanup_staged(abs, staged_new);
            let reason = entry["reason"]
                .as_str()
                .unwrap_or("could not import this file");
            return ApiError::unprocessable(reason).into_response();
        }
    };
    let note_path = entry["destination"].as_str().unwrap_or("");
    let title = entry["title"].as_str().unwrap_or("");
    let warnings = entry["warnings"].clone();
    Json(json!({
        "success": true,
        "note_path": note_path,
        "title": title,
        "status": status,
        "warnings": warnings,
    }))
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tls::tests::support::{WEB_TOKEN, world};
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    async fn read_json(res: Response) -> (u16, Value) {
        let status = res.status().as_u16();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        let val = serde_json::from_slice(&bytes).unwrap_or_else(|_| json!({}));
        (status, val)
    }

    async fn post_import(app: &axum::Router, name: &str, bytes: &[u8]) -> (u16, Value) {
        let encoded: String = url::form_urlencoded::byte_serialize(name.as_bytes()).collect();
        let req = Request::builder()
            .method("POST")
            .uri(format!("/api/note/import?name={encoded}"))
            .header("authorization", format!("Bearer {WEB_TOKEN}"))
            .header("content-type", "application/octet-stream")
            .body(Body::from(bytes.to_vec()))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        read_json(res).await
    }

    #[tokio::test]
    async fn imports_html_and_text_and_records_owner_provenance() {
        let w = world();
        let html = b"<html><head><title>Receipt</title></head><body><p>Paid 10</p></body></html>";
        let (status, body) = post_import(&w.router, "receipt.html", html).await;
        assert_eq!(status, 200, "unexpected: {body}");
        assert_eq!(body["success"], true);
        assert_eq!(body["status"], "imported");
        assert_eq!(body["title"], "Receipt");
        assert!(body["warnings"].is_array());
        let note_path = body["note_path"].as_str().unwrap();
        assert!(note_path.starts_with(STAGING_DIR));
        assert!(w.state.vault_path.join(note_path).is_file());

        let history = w
            .state
            .db
            .with_conn(|c| hq_db::provenance::history(c, note_path, 1))
            .unwrap();
        let latest = history.first().expect("provenance recorded");
        assert_eq!(latest.source_kind, hq_db::provenance::SourceKind::Human);

        let (txt_status, txt_body) =
            post_import(&w.router, "notes.txt", b"Plain text note\nSecond line\n").await;
        assert_eq!(txt_status, 200, "unexpected: {txt_body}");
        assert_eq!(txt_body["status"], "imported");
        assert!(
            w.state
                .vault_path
                .join(txt_body["note_path"].as_str().unwrap())
                .is_file()
        );
    }

    #[tokio::test]
    async fn identical_bytes_twice_returns_already_imported_and_makes_one_note() {
        let w = world();
        let bytes = b"Hello from the phone";
        let (s1, b1) = post_import(&w.router, "memo.txt", bytes).await;
        assert_eq!(s1, 200);
        assert_eq!(b1["status"], "imported");
        let first_path = b1["note_path"].as_str().unwrap();

        let (s2, b2) = post_import(&w.router, "memo.txt", bytes).await;
        assert_eq!(s2, 200);
        assert_eq!(b2["status"], "already_imported");
        assert_eq!(b2["note_path"].as_str().unwrap(), first_path);

        let md_count = std::fs::read_dir(w.state.vault_path.join(STAGING_DIR))
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|x| x == "md"))
            .count();
        assert_eq!(md_count, 1);

        // Different bytes with the same name must not overwrite the first staged file.
        let (s3, b3) = post_import(&w.router, "memo.txt", b"Different content").await;
        assert_eq!(s3, 200);
        assert_eq!(b3["status"], "imported");
        assert_ne!(b3["note_path"].as_str().unwrap(), first_path);
        assert!(
            w.state
                .vault_path
                .join(STAGING_DIR)
                .join("memo (2).txt")
                .is_file()
        );
    }

    #[tokio::test]
    async fn rejects_traversal_absolute_nul_and_unsupported_names() {
        let w = world();
        for bad in [
            "",
            "../secret.txt",
            "sub/../secret.txt",
            "/etc/passwd.txt",
            "C:\\temp\\secret.txt",
            "bad\0name.txt",
            "note.md",
            "archive.zip",
            "unknown.xyz",
        ] {
            let (status, _) = post_import(&w.router, bad, b"hello").await;
            assert_eq!(status, 400, "expected 400 for {bad:?}");
        }
        assert!(!w.state.vault_path.join(STAGING_DIR).exists());
    }

    #[tokio::test]
    async fn rejects_oversize_body_with_413() {
        let w = world();
        let too_big = vec![b'a'; max_import_bytes() + 1];
        let (status, _) = post_import(&w.router, "big.txt", &too_big).await;
        assert_eq!(status, 413);
        assert!(!w.state.vault_path.join(STAGING_DIR).exists());
    }

    #[tokio::test]
    async fn failed_conversion_returns_422_and_removes_staged_file() {
        let w = world();
        let (status, body) =
            post_import(&w.router, "broken.docx", b"PK\x03\x04not a valid zip").await;
        assert_eq!(status, 422, "unexpected: {body}");
        assert!(body["error"].as_str().is_some_and(|s| !s.is_empty()));
        assert!(
            !w.state
                .vault_path
                .join(STAGING_DIR)
                .join("broken.docx")
                .exists()
        );
    }

    #[tokio::test]
    async fn image_when_ocr_absent_returns_422_with_blocked_reason_and_cleans_up() {
        let w = world();
        // Minimal 1x1 PNG header bytes so sniff recognises an image.
        const PNG: &[u8] = &[
            0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, b'I', b'H',
            b'D', b'R',
        ];
        let no_ocr = Capabilities {
            ocr: false,
            pdf_tables: false,
            legacy_office: false,
        };
        let res = import_with_caps(&w.state, "photo.png", Bytes::from_static(PNG), &no_ocr).await;
        let (status, body) = read_json(res).await;
        assert_eq!(status, 422, "unexpected: {body}");
        assert_eq!(body["error"], Support::NeedsOcr.label());
        assert!(
            !w.state
                .vault_path
                .join(STAGING_DIR)
                .join("photo.png")
                .exists()
        );
    }

    #[tokio::test]
    async fn long_cjk_name_is_truncated_by_bytes_and_imports() {
        let w = world();
        let cjk_name = format!("{}.txt", "日本語".repeat(40));
        assert!(cjk_name.len() > 255);
        let (status, body) = post_import(&w.router, &cjk_name, b"cjk body").await;
        assert_eq!(status, 200, "unexpected: {body}");
        assert_eq!(body["status"], "imported");
        let note_path = body["note_path"].as_str().unwrap();
        let file_name = note_path.rsplit('/').next().unwrap();
        assert!(file_name.len() <= MAX_NAME_BYTES);
        assert!(w.state.vault_path.join(note_path).is_file());
    }

    #[tokio::test]
    async fn double_dots_inside_name_are_allowed_while_dotdot_segments_are_refused() {
        let w = world();
        let (status, body) = post_import(&w.router, "report..final.txt", b"legit").await;
        assert_eq!(status, 200, "unexpected: {body}");
        assert_eq!(body["status"], "imported");

        for bad in ["../x.txt", "a/../x.txt"] {
            let (s, _) = post_import(&w.router, bad, b"bad").await;
            assert_eq!(s, 400, "expected 400 for {bad:?}");
        }
        assert!(!w.state.vault_path.join("x.txt").exists());
    }

    #[tokio::test]
    async fn concurrent_same_name_different_bytes_produce_two_notes_with_own_contents() {
        let w = world();
        let ((s1, b1), (s2, b2)) = tokio::join!(
            post_import(&w.router, "concurrent.txt", b"first body"),
            post_import(&w.router, "concurrent.txt", b"second body"),
        );
        assert_eq!(s1, 200, "first failed: {b1}");
        assert_eq!(s2, 200, "second failed: {b2}");
        assert_eq!(b1["status"], "imported");
        assert_eq!(b2["status"], "imported");

        let p1 = b1["note_path"].as_str().unwrap();
        let p2 = b2["note_path"].as_str().unwrap();
        assert_ne!(p1, p2);

        let c1 = std::fs::read_to_string(w.state.vault_path.join(p1)).unwrap();
        let c2 = std::fs::read_to_string(w.state.vault_path.join(p2)).unwrap();
        assert!(
            (c1.contains("first body") && c2.contains("second body"))
                || (c1.contains("second body") && c2.contains("first body")),
            "notes did not keep distinct contents: c1={c1:?}, c2={c2:?}"
        );
    }

    #[tokio::test]
    async fn reuploading_different_bytes_after_deleting_staged_original_picks_suffix() {
        let w = world();
        let (s1, b1) = post_import(&w.router, "reupload.txt", b"first version").await;
        assert_eq!(s1, 200, "unexpected: {b1}");
        let p1 = b1["note_path"].as_str().unwrap();

        std::fs::remove_file(w.state.vault_path.join(STAGING_DIR).join("reupload.txt")).unwrap();

        let (s2, b2) = post_import(&w.router, "reupload.txt", b"second version").await;
        assert_eq!(s2, 200, "unexpected: {b2}");
        assert_eq!(b2["status"], "imported");
        let p2 = b2["note_path"].as_str().unwrap();
        assert_ne!(p1, p2);
        let c2 = std::fs::read_to_string(w.state.vault_path.join(p2)).unwrap();
        assert!(c2.contains("second version"));
    }
}
