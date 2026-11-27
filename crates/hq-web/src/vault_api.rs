//! Vault endpoints the PWA needs beyond plain read/write: a recursive tree,
//! directory-aware note reads, safe note creation and frontmatter-preserving
//! updates, the folder list, and the home page's vault signals.

use crate::WsState;
use crate::error::ApiError;
use axum::Json;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

const DEFAULT_TREE_ROOT: &str = "Notebooks";
const DEFAULT_NOTE_FOLDER: &str = "Notebooks/Inbox";
const PREVIEW_CHARS: usize = 150;
const HEAD_BYTES: usize = 4096;
const TEXT_MAX_BYTES: u64 = 500 * 1024;
const LOG_MAX_BYTES: u64 = 50 * 1024;
const BINARY_MAX_BYTES: u64 = 1024 * 1024;
const TEXT_EXTENSIONS: &[&str] = &[
    "md", "json", "ts", "js", "sh", "yaml", "yml", "env", "txt", "csv", "log", "html", "htm",
];
const SCAN_SKIP: &[&str] = &[
    "_data",
    "_embeddings",
    "_archive",
    "_attachments",
    "_media",
    "_metrics",
    "_agent-sessions",
    "_history",
    "_events",
    "_browser",
    "node_modules",
];
const PROJECT_ROOTS: &[&str] = &["Notebooks"];
const ACTIVITY_ROOTS: &[&str] = &["_system", "_logs"];
const PROJECT_SCAN_CAP: usize = 2500;
const ACTIVITY_SCAN_CAP: usize = 300;
const SIGNALS_RECENT: usize = 12;
const SIGNALS_WORK: usize = 20;
const SIGNALS_REVIEW: usize = 20;
const SIGNALS_ACTIVITY: usize = 12;
const PINNED_LIMIT: usize = 50;
const SIGNALS_TTL: Duration = Duration::from_secs(20);
const WORK_TYPES: &[&str] = &["task", "decision"];
const DONE_STATUSES: &[&str] = &["done", "complete", "completed", "archived", "cancelled"];
const REVIEW_STATUSES: &[&str] = &["review", "blocked", "decision_needed"];

/// Resolve a client-supplied vault-relative path, rejecting `..`, absolute paths
/// and symlinks that escape the vault. The target itself need not exist yet.
pub(crate) fn resolve_in_vault(vault: &Path, rel: &str) -> Option<PathBuf> {
    let rel_path = Path::new(rel);
    if rel_path
        .components()
        .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
    {
        return None;
    }
    let joined = vault.join(rel_path);
    let vault_canonical = std::fs::canonicalize(vault).unwrap_or_else(|_| vault.to_path_buf());
    let mut existing = joined.as_path();
    while !existing.exists() {
        existing = existing.parent()?;
    }
    let existing_canonical = std::fs::canonicalize(existing).ok()?;
    existing_canonical
        .starts_with(&vault_canonical)
        .then_some(joined)
}

/// Whether `rel` points into a folder `profile: lite` keeps out of the notes surface: HQ's own
/// (`_system`, `_data`, `_threads`, `_mailboxes` and any other name starting with `_`) and
/// hidden ones (`.git`). The notes the owner wrote live elsewhere. Surrounding whitespace is
/// ignored, because the note resolvers trim the path they are given.
pub fn lite_hides(rel: &str) -> bool {
    Path::new(rel.trim())
        .components()
        .find_map(|c| match c {
            Component::Normal(p) => Some(p.to_string_lossy().into_owned()),
            _ => None,
        })
        .is_some_and(|first| first.starts_with('_') || first.starts_with('.'))
}

/// Whether `abs` (inside `vault`, existing or not) lands in a folder Lite hides once symlinks
/// are resolved, or is the vault root itself. The request string alone is not enough: a
/// symlink in a normal folder can point into `_system`.
pub fn lite_hides_resolved(vault: &Path, abs: &Path) -> bool {
    let Ok(vault) = std::fs::canonicalize(vault) else {
        return true;
    };
    // Resolve the longest existing prefix, then append what does not exist yet.
    let mut existing = abs;
    let mut tail = Vec::new();
    while !existing.exists() {
        match (existing.file_name(), existing.parent()) {
            (Some(name), Some(parent)) => {
                tail.push(name.to_os_string());
                existing = parent;
            }
            _ => return true,
        }
    }
    let Ok(mut resolved) = std::fs::canonicalize(existing) else {
        return true;
    };
    resolved.extend(tail.iter().rev());
    match resolved.strip_prefix(&vault) {
        // The vault root itself holds HQ's folders, so it is not a notes location either.
        Ok(rel) if rel.as_os_str().is_empty() => true,
        Ok(rel) => lite_hides(&rel.to_string_lossy()),
        Err(_) => true,
    }
}

fn rel_to_vault(vault: &Path, path: &Path) -> String {
    path.strip_prefix(vault)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// A note by the name a link gives it, through the resolver every link in the vault uses:
/// a bare name, the end of a path (`Alpha/Plan`), with or without `.md`, any case, with a
/// `#heading` or `|alias` tail ignored. Symlinks are never followed (the walk skips them), so
/// the result is always a path the vault listing itself holds. When two notes share the name,
/// the one nearest the top wins, then the first by path, so the answer does not change
/// between requests.
fn find_note_by_name(vault: &Path, name: &str) -> Option<String> {
    use hq_vault::links::{NoteInput, NoteSet, Resolution, Syntax};
    let name = name.split(['#', '|']).next()?.trim();
    if name.is_empty() {
        return None;
    }
    let walked = hq_vault::walk::walk(vault, false);
    let set = NoteSet::new(
        walked.notes.into_iter().map(|n| NoteInput {
            path: n.rel,
            aliases: Vec::new(),
        }),
        [],
    );
    match set.resolve_target("", Syntax::Wiki, name) {
        Resolution::Unique(path) => Some(path),
        Resolution::Ambiguous(paths) => paths
            .into_iter()
            .min_by_key(|p| (p.matches('/').count(), p.clone())),
        _ => None,
    }
}

/// Whether `rel` is rejected outright as a path-traversal or absolute-path
/// attempt, independent of whether a note actually resolves. Callers use this
/// to distinguish "malformed/malicious reference" (400) from "well-formed
/// reference, no such note" (404) — `resolve_note_path` folds both into
/// `None`, since its own callers (the bare-stem search) don't need the
/// distinction, but the HTTP handler does.
pub(crate) fn is_rejected_note_ref(rel: &str) -> bool {
    let rel = rel.trim();
    rel.is_empty() || rel.starts_with('/') || rel.contains("..")
}

/// Resolves a note reference safely inside the vault.
/// Supports:
/// 1. Exact relative paths (e.g. `Notebooks/Inbox/Plan.md`)
/// 2. Missing `.md` extensions (e.g. `Notebooks/Inbox/Plan`)
/// 3. Names as links write them (e.g. `Plan`, `Alpha/Plan#Goals`), by the shared link resolver
///
/// Rejects path traversal (`..`), absolute paths, and external paths.
/// A folder inside the vault, which `GET /api/note` answers with its listing. The vault root is
/// not one: its listing is `/api/tree`'s.
pub(crate) fn resolve_folder(vault: &Path, rel: &str) -> Option<(PathBuf, String)> {
    let rel = rel.trim().trim_end_matches('/');
    if rel.is_empty() || is_rejected_note_ref(rel) {
        return None;
    }
    let abs = resolve_in_vault(vault, rel)?;
    abs.is_dir().then(|| (abs, rel.to_string()))
}

pub(crate) fn resolve_note_path(vault: &Path, rel: &str) -> Option<(PathBuf, String)> {
    let rel = rel.trim();
    if is_rejected_note_ref(rel) {
        return None;
    }
    // 1. Exact match
    if let Some(abs) = resolve_in_vault(vault, rel)
        && abs.is_file()
    {
        return Some((abs, rel.to_string()));
    }
    // 2. Append .md if no extension
    if Path::new(rel).extension().is_none() {
        let with_md = format!("{rel}.md");
        if let Some(abs) = resolve_in_vault(vault, &with_md)
            && abs.is_file()
        {
            return Some((abs, with_md));
        }
    }
    // 3. A note by the name a link gives it: `Plan`, `Alpha/Plan`, `plan.md`, `Plan#Goals`.
    if let Some(found) = find_note_by_name(vault, rel)
        && let Some(abs) = resolve_in_vault(vault, &found)
        && abs.is_file()
    {
        return Some((abs, found));
    }
    None
}

pub(crate) use hq_core::frontmatter_utils::split_frontmatter;

/// Lite follows no symlinks while walking the notes: one could lead out of them into HQ's own
/// folders, which the request path alone would not show.
fn is_skipped_link(entry: &std::fs::DirEntry, lite: bool) -> bool {
    lite && entry.file_type().is_ok_and(|t| t.is_symlink())
}

fn sorted_entries(dir: &Path, lite: bool) -> Vec<std::fs::DirEntry> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .map(|it| {
            it.flatten()
                .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
                .filter(|e| !is_skipped_link(e, lite))
                .collect()
        })
        .unwrap_or_default();
    entries.sort_by_key(|e| (!e.path().is_dir(), e.file_name()));
    entries
}

fn build_tree(vault: &Path, dir: &Path, name: String, lite: bool) -> Value {
    let children: Vec<Value> = sorted_entries(dir, lite)
        .into_iter()
        .map(|e| {
            let path = e.path();
            let child_name = e.file_name().to_string_lossy().to_string();
            if path.is_dir() {
                build_tree(vault, &path, child_name, lite)
            } else {
                json!({"name": child_name, "path": rel_to_vault(vault, &path), "type": "file"})
            }
        })
        .collect();
    json!({"name": name, "path": rel_to_vault(vault, dir), "type": "dir", "children": children})
}

/// `GET /api/tree?recursive=true[&path=Notebooks]`: the nested tree under `path`.
pub(crate) fn recursive_tree(vault: &Path, root: &str, lite: bool) -> Response {
    let root = if root.is_empty() {
        DEFAULT_TREE_ROOT
    } else {
        root
    };
    let Some(dir) = resolve_in_vault(vault, root) else {
        return ApiError::bad_request("invalid path").into_response();
    };
    // Lite lists folders of notes, never the vault root (which holds HQ's own folders) or
    // anything that resolves into one.
    if lite && (lite_hides(root) || lite_hides_resolved(vault, &dir) || dir_is_root(vault, &dir)) {
        return ApiError::not_found().into_response();
    }
    let mut tree = build_tree(vault, &dir, root.to_string(), lite);
    tree["path"] = json!(root);
    Json(json!({"tree": tree})).into_response()
}

fn dir_is_root(vault: &Path, dir: &Path) -> bool {
    match (std::fs::canonicalize(vault), std::fs::canonicalize(dir)) {
        (Ok(v), Ok(d)) => v == d,
        _ => false,
    }
}

fn read_capped(path: &Path, size: u64, cap: u64) -> std::io::Result<String> {
    use std::io::Read;
    let mut buf = Vec::with_capacity(cap.min(size) as usize);
    std::fs::File::open(path)?.take(cap).read_to_end(&mut buf)?;
    let mut text = String::from_utf8_lossy(&buf).into_owned();
    if size > cap {
        text.push_str(&format!(
            "\n\n...[TRUNCATED: File is {}KB, showing first {}KB]...",
            size / 1024,
            cap / 1024
        ));
    }
    Ok(text)
}

/// Body of `GET /api/note` for a path that exists: directory listing or capped text.
pub(crate) fn read_note(
    db: &hq_db::Database,
    vault: &Path,
    rel: &str,
    abs: &Path,
    lite: bool,
) -> Response {
    if abs.is_dir() {
        let entries: Vec<Value> = sorted_entries(abs, lite)
            .into_iter()
            .map(|e| {
                let path = e.path();
                let mut entry = json!({
                    "name": e.file_name().to_string_lossy(),
                    "path": rel_to_vault(vault, &path),
                    "isDir": path.is_dir(),
                });
                if path.is_file() && path.extension().is_some_and(|x| x == "md") {
                    entry["tags"] = json!(tags_of_file(&path));
                    add_mark(db, vault, &mut entry);
                }
                entry
            })
            .collect();
        return Json(json!({"path": rel, "content": "", "isDir": true, "entries": entries}))
            .into_response();
    }
    let size = match std::fs::metadata(abs) {
        Ok(m) => m.len(),
        Err(e) => return ApiError::internal(e).into_response(),
    };
    let ext = abs
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let is_text = TEXT_EXTENSIONS.contains(&ext.as_str());
    if !is_text && size > BINARY_MAX_BYTES {
        return Json(json!({"path": rel, "content": "", "isDir": false})).into_response();
    }
    let is_log = rel.contains("_logs/") || rel.contains("_jobs/");
    let cap = if is_log {
        LOG_MAX_BYTES
    } else {
        TEXT_MAX_BYTES
    };
    match read_capped(abs, size, cap) {
        Ok(content) => {
            // The version the editor starts from; a save names it as `base` (web wave 4, notes rule 3).
            let hash = (size <= cap).then(|| hq_db::search::content_hash(&content));
            let mut note = json!({
                "path": rel, "content": content, "isDir": false, "truncated": size > cap,
                "hash": hash,
            });
            if ext == "md" {
                note["tags"] = json!(tags_of_file(abs));
                add_mark(db, vault, &mut note);
            }
            Json(note).into_response()
        }
        Err(e) => ApiError::internal(e).into_response(),
    }
}

fn safe_title(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '_' | '-'))
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        "Untitled".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Write `content` to a new file named after `title` in `dir`, adding " N" on collision,
/// holding the per-note lock for the chosen path so a concurrent writer cannot slip in before
/// provenance is recorded.
fn create_unique_locked(
    vault_root: &Path,
    dir: &Path,
    title: &str,
    content: &str,
) -> std::io::Result<(PathBuf, hq_vault::note_lock::NoteGuard)> {
    use std::io::Write;
    std::fs::create_dir_all(dir)?;
    let base = safe_title(title);
    for n in 0.. {
        let name = if n == 0 {
            format!("{base}.md")
        } else {
            format!("{base} {n}.md")
        };
        let path = dir.join(name);
        let rel = rel_to_vault(vault_root, &path);
        let held = hq_vault::note_lock::lock_note(&rel);
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut f) => {
                f.write_all(content.as_bytes())?;
                return Ok((path, held));
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    unreachable!("the dedupe loop only exits by returning")
}

#[cfg(test)]
fn create_unique(dir: &Path, title: &str, content: &str) -> std::io::Result<PathBuf> {
    create_unique_locked(dir, dir, title, content).map(|(p, _)| p)
}

/// Web writes go straight into FTS so search sees them before the periodic
/// sync does. Tags stay empty here; that sync fills them on its next pass.
pub(crate) fn index_web_note(state: &WsState, abs: &Path, raw: &str) {
    let rel = rel_to_vault(&state.vault_path, abs);
    let title = abs
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let body = split_frontmatter(raw).1.to_string();
    if let Err(e) = state
        .db
        .with_conn(move |c| hq_db::search::index_note(c, &rel, &title, &body, ""))
    {
        tracing::warn!(error = %e, "vault: failed to index web note write");
    }
}

/// Record that the owner wrote this note through the web app.
pub(crate) fn record_owner_write(state: &WsState, abs: &Path) {
    let rel = rel_to_vault(&state.vault_path, abs);
    if rel.is_empty() {
        return;
    }
    hq_tools::provenance::Recorder::new(Arc::new(state.db.clone()), state.vault_path.clone())
        .record(&rel, &hq_tools::provenance::Writer::owner());
}

/// The owner pinning a note an agent suggested: the version on disk may still be the
/// agent's (the pin rewrote the same bytes), so the owner accepts it, which is what
/// lets the pin into HQ's context.
pub(crate) fn accept_suggested_pin(state: &WsState, rel: &str, raw: &str) {
    let suggested = state
        .db
        .with_conn(|c| hq_db::provenance::pin_suggested_by(c, rel, raw))
        .ok()
        .flatten();
    if suggested.is_none() {
        return;
    }
    let Ok(vault) = hq_vault::VaultClient::new(state.vault_path.clone()) else {
        return;
    };
    let owner = hq_tools::vault::Reviewer::of_writer(&hq_tools::provenance::Writer::owner(), false);
    if let Err(e) = hq_tools::vault::review_note(
        &state.db,
        &vault,
        rel,
        None,
        hq_db::provenance::ReviewState::Accepted,
        Some("pinned by the owner"),
        &owner,
    ) {
        tracing::warn!(path = rel, error = %e, "could not accept a suggested pin");
    }
}

/// At most this many exports at once: a PDF or PNG of a long note takes real memory.
static EXPORT_SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(4);

/// `GET /api/note/pdf?path=<note>[&brand=<slug>]`: the note rendered as a PDF download.
/// Kept for existing clients; same as `/api/note/export?format=pdf`.
pub(crate) async fn note_pdf_handler(
    State(state): State<Arc<WsState>>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    note_export_response(&state, &params, Some(hq_export::Format::Pdf)).await
}

/// `GET /api/note/export?path=<note>&format=<fmt>[&brand=<slug>]`: the note as a
/// download in any format `hq_export` writes. `format` defaults to `pdf`.
pub(crate) async fn note_export_handler(
    State(state): State<Arc<WsState>>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    note_export_response(&state, &params, None).await
}

async fn note_export_response(
    state: &Arc<WsState>,
    params: &std::collections::HashMap<String, String>,
    fixed_format: Option<hq_export::Format>,
) -> Response {
    let path_param = params.get("path").cloned().unwrap_or_default();
    if is_rejected_note_ref(&path_param) {
        return ApiError::bad_request("invalid path").into_response();
    }
    let format = match fixed_format {
        Some(f) => f,
        None => match params.get("format").filter(|f| !f.is_empty()) {
            Some(raw) => match raw.parse::<hq_export::Format>() {
                Ok(f) => f,
                Err(e) => return ApiError::bad_request(e.to_string()).into_response(),
            },
            None => hq_export::Format::Pdf,
        },
    };
    let Some((abs, _rel)) = resolve_note_path(&state.vault_path, &path_param) else {
        return ApiError::not_found().into_response();
    };
    let is_text_note = abs
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "md" | "markdown" | "txt"));
    if !is_text_note {
        return ApiError::bad_request("only Markdown notes can be exported").into_response();
    }
    let brand = match params.get("brand").filter(|b| !b.is_empty()) {
        Some(slug) => match hq_convert::brand::load_brand_kit(&state.vault_path, slug) {
            Ok(kit) => Some(kit),
            Err(e) => return ApiError::bad_request(e.to_string()).into_response(),
        },
        None => None,
    };
    let languages: Vec<String> = params
        .get("languages")
        .map(|l| {
            l.split(',')
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let Ok(_slot) = EXPORT_SLOTS.acquire().await else {
        return ApiError::internal("export unavailable").into_response();
    };
    #[cfg(feature = "suite")]
    if matches!(format, hq_export::Format::Xlsx | hq_export::Format::Csv)
        && let Some(res) = crate::suite_api::sheet_export_response(&abs, format)
    {
        return res;
    }
    let done =
        match hq_export::export_note(&state.vault_path, &abs, format, brand.as_ref(), &languages)
            .await
        {
            Ok(done) => done,
            // The note simply has nothing of that kind (no tables, no code): the
            // caller asked for something this note cannot give.
            Err(e @ hq_export::ExportError::Unsupported(_)) => {
                return ApiError::bad_request(e.to_string()).into_response();
            }
            Err(e @ hq_export::ExportError::Unavailable(_)) => {
                return ApiError::Unavailable(e.to_string()).into_response();
            }
            Err(e) => return ApiError::internal(e).into_response(),
        };
    let ascii: String = done
        .title
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-");
    let ascii = if ascii.is_empty() {
        "note".to_string()
    } else {
        ascii
    };
    let encoded: String = done
        .title
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect();
    let ext = &done.output.extension;
    let disposition =
        format!("attachment; filename=\"{ascii}.{ext}\"; filename*=UTF-8''{encoded}.{ext}");
    (
        axum::http::StatusCode::OK,
        [
            (
                axum::http::header::CONTENT_TYPE,
                done.output.mime.to_string(),
            ),
            (axum::http::header::CONTENT_DISPOSITION, disposition),
            (axum::http::header::CACHE_CONTROL, "no-store".to_string()),
        ],
        axum::body::Body::from(done.output.bytes),
    )
        .into_response()
}

/// `POST /api/note/create {folder?, title, content, client_id?}`: never overwrites an existing note,
/// and a repeated `client_id` answers with the note it already made.
pub(crate) async fn note_create_handler(
    State(state): State<Arc<WsState>>,
    Json(body): Json<Value>,
) -> Response {
    let folder = body
        .get("folder")
        .and_then(Value::as_str)
        .filter(|f| !f.is_empty())
        .unwrap_or(DEFAULT_NOTE_FOLDER);
    let title = body.get("title").and_then(Value::as_str).unwrap_or("");
    let content = body.get("content").and_then(Value::as_str).unwrap_or("");
    if !state.path_allowed(folder) {
        return ApiError::not_found().into_response();
    }
    let Some(dir) = resolve_in_vault(&state.vault_path, folder) else {
        return ApiError::bad_request("invalid folder").into_response();
    };
    if !state.resolved_allowed(&dir) {
        return ApiError::not_found().into_response();
    }
    let client_id = hq_db::client_ids::usable(body.get("client_id").and_then(Value::as_str));
    if let Some(id) = client_id {
        use hq_db::client_ids::{Claim, KIND_NOTE, claim};
        match state.db.with_conn(|c| claim(c, KIND_NOTE, id)) {
            Ok(Claim::Seen { item_id }) if !item_id.is_empty() => {
                // A resend from a browser's outbox: the note it made already exists.
                return Json(json!({"success": true, "path": item_id, "deduplicated": true}))
                    .into_response();
            }
            Ok(Claim::Seen { .. }) => {
                return ApiError::Conflict("this note is still being made".into()).into_response();
            }
            Ok(Claim::New) => {}
            Err(e) => tracing::warn!("notes: could not check the client id: {e:#}"),
        }
    }
    let created = create_unique_locked(&state.vault_path, &dir, title, content);
    if let Some(id) = client_id {
        use hq_db::client_ids::{KIND_NOTE, record, release};
        let noted = state.db.with_conn(|c| match &created {
            Ok((path, _)) => record(c, KIND_NOTE, id, &rel_to_vault(&state.vault_path, path)),
            Err(_) => release(c, KIND_NOTE, id),
        });
        if let Err(e) = noted {
            tracing::warn!("notes: could not note the client id: {e:#}");
        }
    }
    match created {
        Ok((path, _held)) => {
            index_web_note(&state, &path, content);
            record_owner_write(&state, &path);
            let rel = rel_to_vault(&state.vault_path, &path);
            if let Some(url) = body.get("enrich_url").and_then(Value::as_str) {
                queue_link_capture(&state, &path, &rel, url);
            }
            Json(json!({"success": true, "path": rel})).into_response()
        }
        Err(e) => ApiError::internal(e).into_response(),
    }
}

/// Puts a link the owner shared on the queue for its summary. The note is already saved, so a
/// refused or unqueued link only means it stays a plain note.
fn queue_link_capture(state: &WsState, abs: &Path, rel: &str, url: &str) {
    let on = state
        .hq_config
        .as_deref()
        .is_none_or(|c| c.daemon.link_capture);
    if !on || state.profile().is_lite() || !hq_tools::link_capture::acceptable_url(url) {
        return;
    }
    let Ok(raw) = std::fs::read_to_string(abs) else {
        return;
    };
    let hash = hq_db::search::content_hash(&raw);
    if let Err(e) = state
        .db
        .with_conn(|c| hq_db::link_captures::enqueue(c, rel, url, &hash))
    {
        tracing::warn!(error = %e, "notes: could not queue the shared link");
    }
}

/// Replace a note's body, keeping its frontmatter and stamping `modified`.
pub(crate) fn update_preserving_frontmatter(
    existing: &str,
    new_body: &str,
    now_iso: &str,
) -> String {
    let (fm, _) = split_frontmatter(existing);
    // A client that holds the whole note (the suite editors do) sends its frontmatter back with the
    // body. The note's own frontmatter is the one that stays, so the copy that came with the text is
    // dropped instead of being written as a second block.
    let new_body = split_frontmatter(new_body)
        .0
        .map_or(new_body, |_| split_frontmatter(new_body).1);
    let mut lines: Vec<String> = fm
        .unwrap_or("")
        .lines()
        .filter(|l| !l.starts_with("modified:"))
        .map(str::to_string)
        .collect();
    lines.push(format!("modified: '{now_iso}'"));
    format!("---\n{}\n---\n{new_body}", lines.join("\n"))
}

/// `PUT /api/note {path, content}`: body-only edit that keeps the note's frontmatter.
/// `GET /api/note/versions?path=`: who wrote each version of a note, when, and where its review stands,
/// for About this note (web wave 4, notes R2). The same read as the `note_provenance` tool.
pub(crate) async fn note_versions_handler(
    State(state): State<Arc<WsState>>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    let path = params.get("path").cloned().unwrap_or_default();
    if !state.path_allowed(&path) {
        return ApiError::not_found().into_response();
    }
    let Ok(vault) = hq_vault::VaultClient::new(state.vault_path.clone()) else {
        return ApiError::internal("the notes folder cannot be opened").into_response();
    };
    use hq_tool_api::registry::HqTool;
    let tool =
        hq_tools::vault::NoteProvenanceTool::new(Arc::new(vault), Arc::new(state.db.clone()));
    match tool.execute(json!({ "path": path })).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => ApiError::bad_request(e.to_string()).into_response(),
    }
}

/// `POST /api/note/review {path, verdict, reason}`: the owner's Looks right (`accepted`) or
/// Moves or renames a note as the owner: `{from, to, dry_run?}`. Links to it in other notes are rewritten
/// and the index follows, exactly as the `vault_move` tool does; a dry run only lists who links here. Moving
/// it back undoes it. Both paths must be notes the profile may see, and a path already taken is refused.
pub(crate) async fn note_move_handler(
    State(state): State<Arc<WsState>>,
    Json(body): Json<Value>,
) -> Response {
    let text = |k: &str| {
        body.get(k)
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string()
    };
    let (from, to) = (text("from"), text("to"));
    let dry_run = body
        .get("dry_run")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if from.is_empty() || to.is_empty() {
        return ApiError::bad_request("from and to are both needed").into_response();
    }
    if !state.path_allowed(&from) || !state.path_allowed(&to) {
        return ApiError::not_found().into_response();
    }
    let Ok(vault) = hq_vault::VaultClient::new(state.vault_path.clone()) else {
        return ApiError::internal("the notes folder cannot be opened").into_response();
    };
    match hq_tools::vault_reorg::move_note_value(
        Arc::new(vault),
        Some(Arc::new(state.db.clone())),
        &from,
        &to,
        dry_run,
    )
    .await
    {
        Ok(report) => Json(report).into_response(),
        // The vault's own refusals (a path taken, a protected folder, a missing note) are the person's to fix.
        Err(e) => ApiError::bad_request(e.to_string()).into_response(),
    }
}

/// Moves a folder of notes as the owner: `{from, to, dry_run?}`, both inside Notebooks. The notes keep their names, so
/// links to them keep working, and the index follows. Moving it back undoes it.
pub(crate) async fn folder_move_handler(
    State(state): State<Arc<WsState>>,
    Json(body): Json<Value>,
) -> Response {
    let text = |k: &str| {
        body.get(k)
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .trim_end_matches('/')
            .to_string()
    };
    let (from, to) = (text("from"), text("to"));
    let dry_run = body
        .get("dry_run")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if from.is_empty() || to.is_empty() {
        return ApiError::bad_request("from and to are both needed").into_response();
    }
    if !state.path_allowed(&from) || !state.path_allowed(&to) {
        return ApiError::not_found().into_response();
    }
    let Ok(vault) = hq_vault::VaultClient::new(state.vault_path.clone()) else {
        return ApiError::internal("the notes folder cannot be opened").into_response();
    };
    match hq_tools::vault_reorg::move_folder_value(
        Arc::new(vault),
        Some(Arc::new(state.db.clone())),
        &from,
        &to,
        dry_run,
    )
    .await
    {
        Ok(report) => Json(report).into_response(),
        Err(e) => ApiError::bad_request(e.to_string()).into_response(),
    }
}

/// Something's wrong (`disputed`, with a reason) on the version on disk. It goes through
/// `review_note`, the one place review decisions are applied, as the owner.
pub(crate) async fn note_review_handler(
    State(state): State<Arc<WsState>>,
    Json(body): Json<Value>,
) -> Response {
    let path = body
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    let verdict = match body.get("verdict").and_then(Value::as_str) {
        Some("accepted") => hq_db::provenance::ReviewState::Accepted,
        Some("disputed") => hq_db::provenance::ReviewState::Disputed,
        _ => return ApiError::bad_request("verdict must be accepted or disputed").into_response(),
    };
    let reason = body
        .get("reason")
        .and_then(Value::as_str)
        .map(str::to_string);
    if !state.path_allowed(&path) {
        return ApiError::not_found().into_response();
    }
    let Ok(vault) = hq_vault::VaultClient::new(state.vault_path.clone()) else {
        return ApiError::internal("the notes folder cannot be opened").into_response();
    };
    let db = state.db.clone();
    let reviewed = tokio::task::spawn_blocking(move || {
        hq_tools::vault::review_note(
            &db,
            &vault,
            &path,
            None,
            verdict,
            reason.as_deref(),
            &hq_tools::vault::Reviewer::owner(),
        )
    })
    .await;
    match reviewed {
        Ok(Ok(r)) => Json(json!({
            "path": r.record.path,
            "review_state": r.record.review_state.as_str(),
            "previous_state": r.previous.as_str(),
        }))
        .into_response(),
        Ok(Err(e)) => ApiError::bad_request(e.to_string()).into_response(),
        Err(e) => ApiError::internal(e).into_response(),
    }
}

/// A save that started from an older version: nothing is written, and the editor learns who wrote the
/// newer one and when, so the owner can compare (never a silent overwrite).
// ponytail: the base check and the write are not one atomic step; two saves in the same instant can
// still race. A per-path lock around read and write closes it if that is ever seen.
fn changed_since(state: &WsState, rel: &str, theirs: &str) -> Response {
    let who = state
        .db
        .with_conn(|c| hq_db::provenance::current(c, rel, theirs))
        .ok()
        .flatten();
    let body = json!({
        "changed": true,
        "by": who.as_ref().map(|p| p.source_kind.as_str()),
        "at": who.as_ref().map(|p| p.written_at.clone()),
        "theirs_hash": theirs,
    });
    (axum::http::StatusCode::CONFLICT, Json(body)).into_response()
}

pub(crate) async fn note_update_handler(
    State(state): State<Arc<WsState>>,
    Json(body): Json<Value>,
) -> Response {
    let rel = body.get("path").and_then(Value::as_str).unwrap_or("");
    let content = body.get("content").and_then(Value::as_str).unwrap_or("");
    if !state.path_allowed(rel) {
        return ApiError::not_found().into_response();
    }
    let Some(abs) = resolve_in_vault(&state.vault_path, rel).filter(|_| !rel.is_empty()) else {
        return ApiError::bad_request("invalid path").into_response();
    };
    if !state.resolved_allowed(&abs) {
        return ApiError::not_found().into_response();
    }
    // Read, compare and write happen with no await in between, under the note's lock.
    let _held = hq_vault::note_lock::lock_note(rel);
    let existing = match std::fs::read_to_string(&abs) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return ApiError::not_found().into_response();
        }
        Err(e) => return ApiError::internal(e).into_response(),
    };
    if let Some(base) = body.get("base").and_then(Value::as_str) {
        let theirs = hq_db::search::content_hash(&existing);
        if base != theirs {
            return changed_since(&state, rel, &theirs);
        }
    }
    let updated =
        update_preserving_frontmatter(&existing, content, &chrono::Utc::now().to_rfc3339());
    if let Err(e) = hq_vault::notes::write_atomic(&abs, updated.as_bytes()) {
        return ApiError::internal(e).into_response();
    }
    index_web_note(&state, &abs, &updated);
    record_owner_write(&state, &abs);
    Json(json!({"success": true, "path": rel})).into_response()
}

/// Set or clear `pinned: true` in a note's frontmatter, leaving the rest as is.
pub(crate) fn set_pinned(content: &str, pin: bool) -> String {
    let (fm, body) = split_frontmatter(content);
    let mut lines: Vec<&str> = fm
        .unwrap_or("")
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with("pinned:"))
        .collect();
    if pin {
        lines.push("pinned: true");
    }
    if lines.is_empty() {
        return body.to_string();
    }
    format!("---\n{}\n---\n{body}", lines.join("\n"))
}

fn collect_folders(vault: &Path, dir: &Path, out: &mut Vec<String>, lite: bool) {
    for entry in sorted_entries(dir, lite) {
        let path = entry.path();
        if path.is_dir() && entry.file_name() != "node_modules" {
            out.push(rel_to_vault(vault, &path));
            collect_folders(vault, &path, out, lite);
        }
    }
}

/// `GET /api/vault/folders`: every folder under Notebooks, Inbox first.
pub(crate) async fn folders_handler(State(state): State<Arc<WsState>>) -> Response {
    let (vault, lite) = (state.vault_path.clone(), state.profile().is_lite());
    // A directory walk: keep it off the async runtime.
    let walked = tokio::task::spawn_blocking(move || {
        let mut folders = Vec::new();
        collect_folders(&vault, &vault.join("Notebooks"), &mut folders, lite);
        folders
    })
    .await;
    let mut folders = match walked {
        Ok(folders) => folders,
        Err(e) => return ApiError::internal(e).into_response(),
    };
    folders.retain(|f| f != DEFAULT_NOTE_FOLDER);
    folders.sort();
    folders.insert(0, DEFAULT_NOTE_FOLDER.to_string());
    Json(json!({"folders": folders})).into_response()
}

fn yaml_str(fm: &serde_yaml::Mapping, key: &str) -> Option<String> {
    fm.get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn yaml_true(fm: &serde_yaml::Mapping, key: &str) -> bool {
    matches!(fm.get(key), Some(serde_yaml::Value::Bool(true)))
        || yaml_str(fm, key).as_deref() == Some("true")
}

fn yaml_str_list(fm: &serde_yaml::Mapping, key: &str) -> Vec<String> {
    match fm.get(key) {
        Some(serde_yaml::Value::Sequence(items)) => items
            .iter()
            .filter_map(|v| match v {
                serde_yaml::Value::String(s) => Some(s.clone()),
                serde_yaml::Value::Number(n) => Some(n.to_string()),
                _ => None,
            })
            .collect(),
        _ => yaml_str(fm, key).into_iter().collect(),
    }
}

/// Body text for a preview: drops headings, fenced code and `[[wikilinks]]`.
pub(crate) fn preview_text(body: &str) -> String {
    let mut in_fence = false;
    let mut kept = String::new();
    for line in body.lines() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence || line.trim_start().starts_with('#') {
            continue;
        }
        kept.push_str(line);
        kept.push('\n');
    }
    let mut out = String::new();
    let mut rest = kept.as_str();
    while let Some(start) = rest.find("[[") {
        out.push_str(&rest[..start]);
        rest = match rest[start..].find("]]") {
            Some(end) => &rest[start + end + 2..],
            None => "",
        };
    }
    out.push_str(rest);
    out.trim().chars().take(PREVIEW_CHARS).collect()
}

struct NoteMeta {
    mtime: SystemTime,
    value: Value,
    note_type: Option<String>,
    status: Option<String>,
    visibility: Option<String>,
    decision_needed: bool,
    pinned: bool,
}

fn read_head(path: &Path) -> Option<String> {
    use std::io::Read;
    let mut buf = Vec::with_capacity(HEAD_BYTES);
    std::fs::File::open(path)
        .ok()?
        .take(HEAD_BYTES as u64)
        .read_to_end(&mut buf)
        .ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

/// Most bytes of a note read to find its front matter and tags.
const FRONT_MATTER_CAP: usize = 64 * 1024;
/// Deepest `[` / `{` nesting a front matter may have before it is not parsed.
const FRONT_MATTER_MAX_DEPTH: usize = 8;
/// Largest file hashed to tell whether HQ's record is still the version on disk.
const MARK_HASH_CAP: u64 = 4 * 1024 * 1024;

/// Whether front matter is safe to hand to the YAML parser: small, shallow, and without
/// anchors or aliases, which can expand a few hundred bytes into gigabytes.
fn front_matter_is_plain(text: &str) -> bool {
    if text.len() > FRONT_MATTER_CAP {
        return false;
    }
    let (mut depth, mut deepest) = (0usize, 0usize);
    for c in text.chars() {
        match c {
            '[' | '{' => {
                depth += 1;
                deepest = deepest.max(depth);
            }
            ']' | '}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    deepest <= FRONT_MATTER_MAX_DEPTH
        && !text
            .split(|c: char| c.is_whitespace() || matches!(c, ',' | '[' | ']' | '{' | '}' | ':'))
            .any(|token| token.starts_with('&') || token.starts_with('*'))
}

/// The front matter as a mapping; empty when it is missing, too big, anchored or not YAML.
fn parse_front_matter(text: Option<&str>) -> serde_yaml::Mapping {
    text.filter(|t| front_matter_is_plain(t))
        .and_then(|t| serde_yaml::from_str(t).ok())
        .unwrap_or_default()
}

/// The one reading of a note's `tags` that every list, payload and search filter uses. Read only.
pub(crate) fn tags_of_file(path: &Path) -> Vec<String> {
    use std::io::Read;
    let mut buf = Vec::new();
    let read = std::fs::File::open(path)
        .map(|f| f.take(FRONT_MATTER_CAP as u64 + 1).read_to_end(&mut buf));
    if read.is_err() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&buf);
    yaml_str_list(&parse_front_matter(split_frontmatter(&text).0), "tags")
}

/// Whether `tags` holds `wanted` as a whole tag, ignoring case and a leading `#`.
pub(crate) fn has_tag(tags: &[String], wanted: &str) -> bool {
    let wanted = wanted.trim().trim_start_matches('#').to_lowercase();
    tags.iter()
        .any(|t| t.trim().trim_start_matches('#').to_lowercase() == wanted)
}

/// The hash of the note as it is now, or `None` for a file too large to hash here (it reads as
/// the owner's).
pub(crate) fn file_hash(vault: &Path, rel: &str) -> Option<String> {
    let path = vault.join(rel);
    if std::fs::metadata(&path).ok()?.len() > MARK_HASH_CAP {
        return None;
    }
    std::fs::read_to_string(path)
        .ok()
        .map(|raw| hq_db::search::content_hash(&raw))
}

/// Merges `written_by`, `review` and `why` into a note's JSON. A failed lookup leaves the note
/// as the owner's, with no flag, rather than failing the list. Blocking: call it off the
/// async runtime.
pub(crate) fn add_mark(db: &hq_db::Database, vault: &Path, value: &mut Value) {
    let Some(rel) = value["path"].as_str().map(str::to_string) else {
        return;
    };
    let mark = db
        .with_conn(|c| hq_db::note_marks::mark(c, &rel, || file_hash(vault, &rel)))
        .unwrap_or_else(|_| hq_db::note_marks::NoteMark::you());
    if let (Some(obj), Value::Object(extra)) = (value.as_object_mut(), mark.to_json()) {
        obj.extend(extra);
    }
}

fn note_meta(vault: &Path, path: &Path) -> Option<NoteMeta> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta.modified().ok()?;
    let head = read_head(path)?;
    let (fm_text, body) = split_frontmatter(&head);
    let fm = parse_front_matter(fm_text);
    // A front matter longer than the head read is parsed from the whole file for its tags.
    let tags = if fm_text.is_none() && head.starts_with("---") {
        tags_of_file(path)
    } else {
        yaml_str_list(&fm, "tags")
    };
    let status = yaml_str(&fm, "status").map(|s| s.to_lowercase());
    let decision_needed = yaml_true(&fm, "decision_needed");
    let pinned = yaml_true(&fm, "pinned");
    let title = yaml_str(&fm, "title").unwrap_or_else(|| {
        path.file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default()
    });
    let value = json!({
        "path": rel_to_vault(vault, path),
        "title": title,
        "preview": preview_text(body),
        "tags": tags,
        "mtime": chrono::DateTime::<chrono::Utc>::from(mtime).to_rfc3339(),
        "size": meta.len(),
        "type": yaml_str(&fm, "type"),
        "status": status,
        "domain": yaml_str(&fm, "domain"),
        "primaryAgent": yaml_str(&fm, "primary_agent"),
        "secondaryAgents": yaml_str_list(&fm, "secondary_agents"),
        "createdBy": yaml_str(&fm, "created_by"),
        "updatedBy": yaml_str(&fm, "updated_by"),
        "project": yaml_str(&fm, "project"),
        "visibility": yaml_str(&fm, "visibility"),
        "decisionNeeded": decision_needed,
        "pinned": pinned,
    });
    Some(NoteMeta {
        mtime,
        value,
        note_type: yaml_str(&fm, "type"),
        status,
        visibility: yaml_str(&fm, "visibility"),
        decision_needed,
        pinned,
    })
}

fn collect_notes(vault: &Path, roots: &[&str], cap: usize, lite: bool) -> Vec<NoteMeta> {
    fn walk(vault: &Path, dir: &Path, cap: usize, lite: bool, out: &mut Vec<NoteMeta>) {
        for entry in sorted_entries(dir, lite) {
            if out.len() >= cap {
                return;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            let path = entry.path();
            if path.is_dir() {
                if !SCAN_SKIP.contains(&name.as_str()) {
                    walk(vault, &path, cap, lite, out);
                }
            } else if name.ends_with(".md")
                && let Some(note) = note_meta(vault, &path)
            {
                out.push(note);
            }
        }
    }
    let mut notes = Vec::new();
    for root in roots {
        walk(vault, &vault.join(root), cap, lite, &mut notes);
    }
    notes.sort_by_key(|a| std::cmp::Reverse(a.mtime));
    notes
}

fn is_review(n: &NoteMeta) -> bool {
    if n.visibility.as_deref() == Some("activity") {
        return false;
    }
    n.status
        .as_deref()
        .is_some_and(|s| REVIEW_STATUSES.contains(&s))
        || n.decision_needed
        || n.note_type.as_deref() == Some("review")
}

fn is_work(n: &NoteMeta) -> bool {
    n.visibility.as_deref() != Some("activity")
        && n.note_type
            .as_deref()
            .is_some_and(|t| WORK_TYPES.contains(&t))
        && !n
            .status
            .as_deref()
            .is_some_and(|s| DONE_STATUSES.contains(&s))
        && !is_review(n)
}

/// The home page's recent, work, review and activity buckets.
pub(crate) fn vault_signals(vault: &Path, lite: bool) -> Value {
    let project = collect_notes(vault, PROJECT_ROOTS, PROJECT_SCAN_CAP, lite);
    // The activity bucket reads HQ's own `_system` and `_logs` folders, which Lite does not show.
    let activity = if lite {
        Vec::new()
    } else {
        collect_notes(vault, ACTIVITY_ROOTS, ACTIVITY_SCAN_CAP, lite)
    };
    let pick = |filter: &dyn Fn(&NoteMeta) -> bool, n: usize| -> Vec<Value> {
        project
            .iter()
            .filter(|m| filter(m))
            .take(n)
            .map(|m| m.value.clone())
            .collect()
    };
    json!({
        "recent": pick(&|_| true, SIGNALS_RECENT),
        "work": pick(&is_work, SIGNALS_WORK),
        "review": pick(&is_review, SIGNALS_REVIEW),
        "activity": activity.iter().take(SIGNALS_ACTIVITY).map(|m| m.value.clone()).collect::<Vec<_>>(),
    })
}

/// `GET /api/pinned`: notes with `pinned: true` under Notebooks, newest first.
/// A note an agent pinned carries `suggestedBy` until the owner pins it themselves.
pub(crate) fn pinned_notes(vault: &Path, lite: bool, db: &hq_db::Database) -> Vec<Value> {
    collect_notes(vault, PROJECT_ROOTS, usize::MAX, lite)
        .into_iter()
        .filter(|n| n.pinned)
        .take(PINNED_LIMIT)
        .map(|n| {
            let mut value = n.value;
            let rel = value["path"].as_str().unwrap_or_default().to_string();
            let suggested_by = std::fs::read_to_string(vault.join(&rel))
                .ok()
                .and_then(|raw| {
                    db.with_conn(|c| hq_db::provenance::pin_suggested_by(c, &rel, &raw))
                        .ok()
                        .flatten()
                });
            value["suggestedBy"] = json!(suggested_by);
            add_mark(db, vault, &mut value);
            value
        })
        .collect()
}

// ponytail: one global cache slot keyed by vault path; per-vault map if one server ever serves several vaults.
static SIGNALS_CACHE: Mutex<Option<(PathBuf, bool, Instant, Value)>> = Mutex::new(None);

/// `GET /api/vault-signals`, cached for a few seconds because it walks the whole vault.
pub(crate) async fn signals_handler(State(state): State<Arc<WsState>>) -> Response {
    let vault = state.vault_path.clone();
    let lite = state.profile().is_lite();
    let cached = SIGNALS_CACHE.lock().ok().and_then(|guard| {
        guard.as_ref().and_then(|(path, cached_lite, at, value)| {
            (*path == vault && *cached_lite == lite && at.elapsed() < SIGNALS_TTL)
                .then(|| value.clone())
        })
    });
    if let Some(cached) = cached {
        return marked_signals(state, cached).await;
    }
    let value = match tokio::task::spawn_blocking(move || vault_signals(&vault, lite)).await {
        Ok(v) => v,
        Err(e) => return ApiError::internal(e).into_response(),
    };
    if let Ok(mut guard) = SIGNALS_CACHE.lock() {
        *guard = Some((
            state.vault_path.clone(),
            lite,
            Instant::now(),
            value.clone(),
        ));
    }
    marked_signals(state, value).await
}

async fn marked_signals(state: Arc<WsState>, signals: Value) -> Response {
    match tokio::task::spawn_blocking(move || with_marks(&state, signals)).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => ApiError::internal(e).into_response(),
    }
}

/// The signals' note lists with each note's mark, and `has_new`: whether any note carries a
/// `new` or `changed` mark (the quiet dot on Vault). Marks change when the owner opens a note, so
/// they are added per request and never cached with the walk.
fn with_marks(state: &WsState, mut signals: Value) -> Value {
    for bucket in ["recent", "work", "review", "activity"] {
        if let Some(notes) = signals[bucket].as_array_mut() {
            for note in notes {
                add_mark(&state.db, &state.vault_path, note);
            }
        }
    }
    let has_new = state
        .db
        .with_conn(|c| hq_db::note_marks::any_new(c, |rel| file_hash(&state.vault_path, rel)))
        .unwrap_or(false);
    signals["has_new"] = json!(has_new);
    signals
}

/// `POST /api/note/seen {path}`: the owner opened this note, which clears `new` on the version
/// on disk. Reading a note never does this itself. The path is read as `GET /api/note` reads it
/// (extension and `./` forms resolve to one note), and the note must be a `.md` file.
pub(crate) async fn note_seen_handler(
    State(state): State<Arc<WsState>>,
    Json(body): Json<Value>,
) -> Response {
    let requested = body
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    let requested = requested
        .strip_prefix("./")
        .unwrap_or(requested)
        .to_string();
    if is_rejected_note_ref(&requested) {
        return ApiError::bad_request("invalid path").into_response();
    }
    if !state.path_allowed(&requested) {
        return ApiError::not_found().into_response();
    }
    let worker = state.clone();
    let done = tokio::task::spawn_blocking(move || mark_note_seen(&worker, &requested)).await;
    match done {
        Ok(Ok(())) => Json(json!({"success": true})).into_response(),
        Ok(Err(refusal)) => *refusal,
        Err(e) => ApiError::internal(e).into_response(),
    }
}

fn mark_note_seen(state: &WsState, requested: &str) -> Result<(), Box<Response>> {
    let not_found = || Box::new(ApiError::not_found().into_response());
    let (abs, effective) = resolve_note_path(&state.vault_path, requested).ok_or_else(not_found)?;
    if !effective.ends_with(".md")
        || !state.path_allowed(&effective)
        || !state.resolved_allowed(&abs)
        || !abs.is_file()
    {
        return Err(not_found());
    }
    let too_big = std::fs::metadata(&abs).map_err(|_| not_found())?.len() > MARK_HASH_CAP;
    if too_big {
        let body = Json(json!({"error": "the note is too large to mark as opened"}));
        return Err(Box::new(
            (axum::http::StatusCode::PAYLOAD_TOO_LARGE, body).into_response(),
        ));
    }
    let hash = file_hash(&state.vault_path, &effective).ok_or_else(not_found)?;
    state
        .db
        .with_conn(|c| hq_db::note_marks::mark_seen(c, &effective, &hash))
        .map_err(|e| Box::new(ApiError::internal(e).into_response()))
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod mark_tests;
