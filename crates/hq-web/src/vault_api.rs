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

fn rel_to_vault(vault: &Path, path: &Path) -> String {
    path.strip_prefix(vault)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// Helper to recursively search a directory for a markdown note matching
/// `stem` (case-insensitive). Never follows symlinks: `DirEntry::file_type`
/// reports the link itself (it does not `stat` through it), so a symlink
/// planted inside the vault pointing outside it is skipped rather than
/// walked into — the vault-boundary check this search exists to respect
/// would otherwise be bypassed entirely, since its result is a raw path
/// handed straight back to the caller.
fn find_note_by_stem(dir: &Path, stem: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if name.starts_with('.') || name.starts_with('_') {
            continue;
        }
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            if let Some(hit) = find_note_by_stem(&path, stem) {
                return Some(hit);
            }
        } else if path.extension().is_some_and(|e| e == "md")
            && path
                .file_stem()
                .is_some_and(|s| s.to_string_lossy().eq_ignore_ascii_case(stem))
        {
            return Some(path);
        }
    }
    None
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
/// 3. Bare note names/wikilinks (e.g. `Plan`) searched in root and `Notebooks/`
///
/// Rejects path traversal (`..`), absolute paths, and external paths.
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
    // 3. Bare note name (no directory separators): look in Notebooks/ recursively
    if !rel.contains('/') && !rel.contains('\\') {
        let stem = rel.strip_suffix(".md").unwrap_or(rel);
        if let Some(hit) = find_note_by_stem(&vault.join("Notebooks"), stem) {
            // Belt-and-suspenders vault-boundary check, independent of
            // `find_note_by_stem`'s own symlink handling: never hand back a
            // path `rel_to_vault` can't express as one truly inside the
            // vault (its `strip_prefix` falls back to `""` for anything
            // else, which would otherwise read as "the vault root").
            let vault_canonical =
                std::fs::canonicalize(vault).unwrap_or_else(|_| vault.to_path_buf());
            let hit_canonical = std::fs::canonicalize(&hit).ok()?;
            if !hit_canonical.starts_with(&vault_canonical) {
                return None;
            }
            let rel_str = rel_to_vault(vault, &hit);
            return Some((hit, rel_str));
        }
    }
    None
}

pub(crate) use hq_core::frontmatter_utils::split_frontmatter;

fn sorted_entries(dir: &Path) -> Vec<std::fs::DirEntry> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .map(|it| {
            it.flatten()
                .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
                .collect()
        })
        .unwrap_or_default();
    entries.sort_by_key(|e| (!e.path().is_dir(), e.file_name()));
    entries
}

fn build_tree(vault: &Path, dir: &Path, name: String) -> Value {
    let children: Vec<Value> = sorted_entries(dir)
        .into_iter()
        .map(|e| {
            let path = e.path();
            let child_name = e.file_name().to_string_lossy().to_string();
            if path.is_dir() {
                build_tree(vault, &path, child_name)
            } else {
                json!({"name": child_name, "path": rel_to_vault(vault, &path), "type": "file"})
            }
        })
        .collect();
    json!({"name": name, "path": rel_to_vault(vault, dir), "type": "dir", "children": children})
}

/// `GET /api/tree?recursive=true[&path=Notebooks]`: the nested tree under `path`.
pub(crate) fn recursive_tree(vault: &Path, root: &str) -> Response {
    let root = if root.is_empty() {
        DEFAULT_TREE_ROOT
    } else {
        root
    };
    let Some(dir) = resolve_in_vault(vault, root) else {
        return ApiError::bad_request("invalid path").into_response();
    };
    let mut tree = build_tree(vault, &dir, root.to_string());
    tree["path"] = json!(root);
    Json(json!({"tree": tree})).into_response()
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
pub(crate) fn read_note(vault: &Path, rel: &str, abs: &Path) -> Response {
    if abs.is_dir() {
        let entries: Vec<Value> = sorted_entries(abs)
            .into_iter()
            .map(|e| {
                let path = e.path();
                json!({
                    "name": e.file_name().to_string_lossy(),
                    "path": rel_to_vault(vault, &path),
                    "isDir": path.is_dir(),
                })
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
        Ok(content) => Json(json!({
            "path": rel, "content": content, "isDir": false, "truncated": size > cap,
        }))
        .into_response(),
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

/// Write `content` to a new file named after `title` in `dir`, adding " N" on collision.
fn create_unique(dir: &Path, title: &str, content: &str) -> std::io::Result<PathBuf> {
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
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut f) => {
                f.write_all(content.as_bytes())?;
                return Ok(path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    unreachable!("the dedupe loop only exits by returning")
}

/// Web writes go straight into FTS so search sees them before the periodic
/// sync does. Tags stay empty here; that sync fills them on its next pass.
fn index_web_note(state: &WsState, abs: &Path, raw: &str) {
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

/// At most this many PDF renders at once: each one starts a browser.
static PDF_SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);

/// `GET /api/note/pdf?path=<note>[&brand=<slug>]`: the note rendered as a PDF download.
pub(crate) async fn note_pdf_handler(
    State(state): State<Arc<WsState>>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    let path_param = params.get("path").cloned().unwrap_or_default();
    if is_rejected_note_ref(&path_param) {
        return ApiError::bad_request("invalid path").into_response();
    }
    let Some((abs, _rel)) = resolve_note_path(&state.vault_path, &path_param) else {
        return ApiError::not_found().into_response();
    };
    let is_text_note = abs
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "md" | "markdown" | "txt"));
    if !is_text_note {
        return ApiError::bad_request("only Markdown notes can be exported as PDF").into_response();
    }
    let brand = match params.get("brand").filter(|b| !b.is_empty()) {
        Some(slug) => match hq_convert::brand::load_brand_kit(&state.vault_path, slug) {
            Ok(kit) => Some(kit),
            Err(e) => return ApiError::bad_request(e.to_string()).into_response(),
        },
        None => None,
    };
    let Ok(_slot) = PDF_SLOTS.acquire().await else {
        return ApiError::internal("pdf export unavailable").into_response();
    };
    let (info, bytes) =
        match hq_convert::note_pdf::export_note_pdf_bytes(&state.vault_path, &abs, brand.as_ref())
            .await
        {
            Ok(done) => done,
            Err(
                e @ (hq_convert::types::ConvertError::NoPdfEngine
                | hq_convert::types::ConvertError::PandocNotFound),
            ) => return ApiError::Unavailable(e.to_string()).into_response(),
            Err(e) => return ApiError::internal(e).into_response(),
        };
    let ascii: String = info
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
    let encoded: String = info
        .title
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect();
    let disposition =
        format!("attachment; filename=\"{ascii}.pdf\"; filename*=UTF-8''{encoded}.pdf");
    (
        axum::http::StatusCode::OK,
        [
            (
                axum::http::header::CONTENT_TYPE,
                "application/pdf".to_string(),
            ),
            (axum::http::header::CONTENT_DISPOSITION, disposition),
            (axum::http::header::CACHE_CONTROL, "no-store".to_string()),
        ],
        axum::body::Body::from(bytes),
    )
        .into_response()
}

/// `POST /api/note/create {folder?, title, content}`: never overwrites an existing note.
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
    let Some(dir) = resolve_in_vault(&state.vault_path, folder) else {
        return ApiError::bad_request("invalid folder").into_response();
    };
    match create_unique(&dir, title, content) {
        Ok(path) => {
            index_web_note(&state, &path, content);
            let rel = rel_to_vault(&state.vault_path, &path);
            Json(json!({"success": true, "path": rel})).into_response()
        }
        Err(e) => ApiError::internal(e).into_response(),
    }
}

/// Replace a note's body, keeping its frontmatter and stamping `modified`.
pub(crate) fn update_preserving_frontmatter(
    existing: &str,
    new_body: &str,
    now_iso: &str,
) -> String {
    let (fm, _) = split_frontmatter(existing);
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
pub(crate) async fn note_update_handler(
    State(state): State<Arc<WsState>>,
    Json(body): Json<Value>,
) -> Response {
    let rel = body.get("path").and_then(Value::as_str).unwrap_or("");
    let content = body.get("content").and_then(Value::as_str).unwrap_or("");
    let Some(abs) = resolve_in_vault(&state.vault_path, rel).filter(|_| !rel.is_empty()) else {
        return ApiError::bad_request("invalid path").into_response();
    };
    let existing = match std::fs::read_to_string(&abs) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return ApiError::not_found().into_response();
        }
        Err(e) => return ApiError::internal(e).into_response(),
    };
    let updated =
        update_preserving_frontmatter(&existing, content, &chrono::Utc::now().to_rfc3339());
    if let Err(e) = hq_vault::notes::write_atomic(&abs, updated.as_bytes()) {
        return ApiError::internal(e).into_response();
    }
    index_web_note(&state, &abs, &updated);
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

fn collect_folders(vault: &Path, dir: &Path, out: &mut Vec<String>) {
    for entry in sorted_entries(dir) {
        let path = entry.path();
        if path.is_dir() && entry.file_name() != "node_modules" {
            out.push(rel_to_vault(vault, &path));
            collect_folders(vault, &path, out);
        }
    }
}

/// `GET /api/vault/folders`: every folder under Notebooks, Inbox first.
pub(crate) async fn folders_handler(State(state): State<Arc<WsState>>) -> Response {
    let mut folders = Vec::new();
    collect_folders(
        &state.vault_path,
        &state.vault_path.join("Notebooks"),
        &mut folders,
    );
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

fn note_meta(vault: &Path, path: &Path) -> Option<NoteMeta> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta.modified().ok()?;
    let head = read_head(path)?;
    let (fm_text, body) = split_frontmatter(&head);
    let fm: serde_yaml::Mapping = fm_text
        .and_then(|t| serde_yaml::from_str(t).ok())
        .unwrap_or_default();
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
        "tags": yaml_str_list(&fm, "tags"),
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

fn collect_notes(vault: &Path, roots: &[&str], cap: usize) -> Vec<NoteMeta> {
    fn walk(vault: &Path, dir: &Path, cap: usize, out: &mut Vec<NoteMeta>) {
        for entry in sorted_entries(dir) {
            if out.len() >= cap {
                return;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            let path = entry.path();
            if path.is_dir() {
                if !SCAN_SKIP.contains(&name.as_str()) {
                    walk(vault, &path, cap, out);
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
        walk(vault, &vault.join(root), cap, &mut notes);
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
pub(crate) fn vault_signals(vault: &Path) -> Value {
    let project = collect_notes(vault, PROJECT_ROOTS, PROJECT_SCAN_CAP);
    let activity = collect_notes(vault, ACTIVITY_ROOTS, ACTIVITY_SCAN_CAP);
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
pub(crate) fn pinned_notes(vault: &Path) -> Vec<Value> {
    collect_notes(vault, PROJECT_ROOTS, usize::MAX)
        .into_iter()
        .filter(|n| n.pinned)
        .take(PINNED_LIMIT)
        .map(|n| n.value)
        .collect()
}

// ponytail: one global cache slot keyed by vault path; per-vault map if one server ever serves several vaults.
static SIGNALS_CACHE: Mutex<Option<(PathBuf, Instant, Value)>> = Mutex::new(None);

/// `GET /api/vault-signals`, cached for a few seconds because it walks the whole vault.
pub(crate) async fn signals_handler(State(state): State<Arc<WsState>>) -> Response {
    let vault = state.vault_path.clone();
    if let Ok(guard) = SIGNALS_CACHE.lock()
        && let Some((path, at, value)) = guard.as_ref()
        && *path == vault
        && at.elapsed() < SIGNALS_TTL
    {
        return Json(value.clone()).into_response();
    }
    let value = match tokio::task::spawn_blocking(move || vault_signals(&vault)).await {
        Ok(v) => v,
        Err(e) => return ApiError::internal(e).into_response(),
    };
    if let Ok(mut guard) = SIGNALS_CACHE.lock() {
        *guard = Some((state.vault_path.clone(), Instant::now(), value.clone()));
    }
    Json(value).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;

    fn vault() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("Notebooks/Inbox")).unwrap();
        dir
    }

    #[tokio::test]
    async fn web_note_writes_are_searchable() {
        let v = vault();
        let state = Arc::new(WsState::new(v.path().to_path_buf(), None));
        let body = json!({"folder": "Notebooks/Inbox", "title": "Plan", "content": "zebra"});
        assert_eq!(
            note_create_handler(State(state.clone()), Json(body))
                .await
                .status(),
            StatusCode::OK
        );
        let edit = json!({"path": "Notebooks/Inbox/Plan.md", "content": "quokka"});
        assert_eq!(
            note_update_handler(State(state.clone()), Json(edit))
                .await
                .status(),
            StatusCode::OK
        );
        let hits = |q: &'static str| {
            state
                .db
                .with_conn(move |c| hq_db::search::keyword_search(c, q, 5))
                .unwrap()
        };
        assert_eq!(hits("quokka")[0].note_path, "Notebooks/Inbox/Plan.md");
        assert!(hits("zebra").is_empty());
    }

    /// A note the index has not seen, found by a partial word through the walk.
    #[tokio::test]
    async fn search_falls_back_to_a_substring_walk() {
        let v = vault();
        std::fs::write(
            v.path().join("Notebooks/Inbox/Trip.md"),
            "packing for london",
        )
        .unwrap();
        let state = Arc::new(WsState::new(v.path().to_path_buf(), None));
        let params = std::collections::HashMap::from([("q".to_string(), "lond".to_string())]);
        let res = crate::api::search_handler(State(state), axum::extract::Query(params)).await;
        let body = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["results"][0]["note_path"], "Notebooks/Inbox/Trip.md");
    }

    #[test]
    fn resolve_rejects_traversal_and_absolute_paths() {
        let v = vault();
        assert!(resolve_in_vault(v.path(), "../etc/passwd").is_none());
        assert!(resolve_in_vault(v.path(), "Notebooks/../../x").is_none());
        assert!(resolve_in_vault(v.path(), "/etc/passwd").is_none());
        assert!(resolve_in_vault(v.path(), "Notebooks/New/Deep.md").is_some());
    }

    #[test]
    fn resolve_note_path_handles_exact_extensionless_and_notebooks() {
        let v = vault();
        let plan_path = v.path().join("Notebooks/Inbox/Plan.md");
        std::fs::write(&plan_path, "Plan body").unwrap();
        let root_note = v.path().join("Readme.md");
        std::fs::write(&root_note, "Readme body").unwrap();

        // 1. Exact path
        let (abs, rel) =
            resolve_note_path(v.path(), "Notebooks/Inbox/Plan.md").expect("exact path");
        assert_eq!(rel, "Notebooks/Inbox/Plan.md");
        assert_eq!(abs, plan_path);

        // 2. Extensionless path
        let (abs, rel) =
            resolve_note_path(v.path(), "Notebooks/Inbox/Plan").expect("extensionless path");
        assert_eq!(rel, "Notebooks/Inbox/Plan.md");
        assert_eq!(abs, plan_path);

        // 3. Root note extensionless
        let (abs, rel) = resolve_note_path(v.path(), "Readme").expect("root extensionless");
        assert_eq!(rel, "Readme.md");
        assert_eq!(abs, root_note);

        // 4. Bare note name in Notebooks subfolder
        let (abs, rel) = resolve_note_path(v.path(), "Plan").expect("bare note in Notebooks");
        assert_eq!(rel, "Notebooks/Inbox/Plan.md");
        assert_eq!(abs, plan_path);

        // 5. Traversal and absolute paths rejected
        assert!(resolve_note_path(v.path(), "../etc/passwd").is_none());
        assert!(resolve_note_path(v.path(), "/etc/passwd").is_none());

        // 6. Missing note returns None
        assert!(resolve_note_path(v.path(), "Nonexistent").is_none());
    }

    /// FR-057: a symlink planted inside `Notebooks/` that points outside the
    /// vault must not let the bare-name search escape it. Before the fix,
    /// `find_note_by_stem` followed `path.is_dir()` through the symlink and
    /// returned the outside file directly, bypassing `resolve_in_vault`'s
    /// canonicalize-and-check entirely.
    #[test]
    fn resolve_note_path_bare_name_does_not_follow_symlink_out_of_vault() {
        let v = vault();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.md"), "top secret").unwrap();
        std::os::unix::fs::symlink(outside.path(), v.path().join("Notebooks/escape")).unwrap();

        assert!(resolve_note_path(v.path(), "secret").is_none());
    }

    /// FR-057: `note_read_handler` distinguishes a malformed/malicious
    /// reference (400) from a well-formed one that just doesn't resolve to a
    /// note (404) using this predicate — it must reject exactly the inputs
    /// `resolve_note_path` itself refuses, and nothing else.
    #[test]
    fn is_rejected_note_ref_flags_traversal_not_missing_notes() {
        assert!(is_rejected_note_ref("../etc/passwd"));
        assert!(is_rejected_note_ref("/etc/passwd"));
        assert!(is_rejected_note_ref("Notebooks/../../x"));
        assert!(is_rejected_note_ref(""));
        assert!(!is_rejected_note_ref("Nonexistent"));
        assert!(!is_rejected_note_ref("Notebooks/Inbox/Plan.md"));
    }

    #[test]
    fn create_unique_dedupes_and_never_overwrites() {
        let v = vault();
        let dir = v.path().join("Notebooks/Inbox");
        let a = create_unique(&dir, "My: Note!", "one").unwrap();
        let b = create_unique(&dir, "My: Note!", "two").unwrap();
        assert_eq!(a.file_name().unwrap(), "My Note.md");
        assert_eq!(b.file_name().unwrap(), "My Note 1.md");
        assert_eq!(std::fs::read_to_string(a).unwrap(), "one");
        assert_eq!(
            create_unique(&dir, "???", "").unwrap().file_name().unwrap(),
            "Untitled.md"
        );
    }

    #[test]
    fn update_keeps_frontmatter_and_stamps_modified() {
        let existing = "---\ntitle: Plan\nmodified: old\ntags: [a]\n---\nold body\n";
        let out = update_preserving_frontmatter(existing, "new body\n", "2026-09-24T00:00:00Z");
        assert!(out.starts_with(
            "---\ntitle: Plan\ntags: [a]\nmodified: '2026-09-24T00:00:00Z'\n---\nnew body"
        ));
        assert!(!out.contains("old body"));
        let bare = update_preserving_frontmatter("no fm", "body", "t");
        assert_eq!(bare, "---\nmodified: 't'\n---\nbody");
    }

    #[test]
    fn split_frontmatter_handles_missing_and_present() {
        assert_eq!(
            split_frontmatter("---\na: 1\n---\nbody"),
            (Some("a: 1\n"), "body")
        );
        assert_eq!(split_frontmatter("plain"), (None, "plain"));
        assert_eq!(
            split_frontmatter("---\nunterminated"),
            (None, "---\nunterminated")
        );
    }

    #[test]
    fn set_pinned_round_trips_without_blank_lines() {
        let note = "---\ntitle: A\n---\nbody\n";
        let pinned = set_pinned(note, true);
        assert_eq!(pinned, "---\ntitle: A\npinned: true\n---\nbody\n");
        assert_eq!(set_pinned(&pinned, false), note);
        assert_eq!(set_pinned("plain", true), "---\npinned: true\n---\nplain");
        assert_eq!(
            set_pinned("---\n\ntitle: A\npinned: true\n---\nb", false),
            "---\ntitle: A\n---\nb"
        );
    }

    #[test]
    fn preview_drops_headings_code_and_wikilinks() {
        let body = "# Title\nSee [[Other Note]] now.\n```\ncode\n```\nEnd.";
        assert_eq!(preview_text(body), "See  now.\nEnd.");
    }

    #[test]
    fn signals_classify_work_review_and_done() {
        let v = vault();
        let nb = v.path().join("Notebooks");
        std::fs::write(nb.join("task.md"), "---\ntype: task\nstatus: open\n---\nx").unwrap();
        std::fs::write(nb.join("done.md"), "---\ntype: task\nstatus: Done\n---\nx").unwrap();
        std::fs::write(nb.join("rev.md"), "---\ntype: task\nstatus: review\n---\nx").unwrap();
        std::fs::write(nb.join("dec.md"), "---\ndecision_needed: true\n---\nx").unwrap();
        std::fs::create_dir_all(v.path().join("_logs")).unwrap();
        std::fs::write(v.path().join("_logs/log.md"), "entry").unwrap();
        let s = vault_signals(v.path());
        let paths = |k: &str| -> Vec<String> {
            let mut p: Vec<String> = s[k]
                .as_array()
                .unwrap()
                .iter()
                .map(|n| n["path"].as_str().unwrap().to_string())
                .collect();
            p.sort();
            p
        };
        assert_eq!(paths("work"), vec!["Notebooks/task.md"]);
        assert_eq!(
            paths("review"),
            vec!["Notebooks/dec.md", "Notebooks/rev.md"]
        );
        assert_eq!(paths("recent").len(), 4);
        assert_eq!(paths("activity"), vec!["_logs/log.md"]);
    }

    #[test]
    fn pinned_reads_only_the_frontmatter() {
        let v = vault();
        let nb = v.path().join("Notebooks");
        std::fs::write(
            nb.join("yes.md"),
            "---\r\ntitle: Kept\r\npinned: true\r\ntags:\r\n  - a\r\n---\r\nbody",
        )
        .unwrap();
        std::fs::write(
            nb.join("hr.md"),
            "---\ntitle: No\n---\ntext\n---\npinned: true\n",
        )
        .unwrap();
        std::fs::write(
            nb.join("dash.md"),
            "---\ntitle: a---b\npinned: true\n---\nx",
        )
        .unwrap();
        let pinned = pinned_notes(v.path());
        let mut titles: Vec<&str> = pinned
            .iter()
            .map(|n| n["title"].as_str().unwrap())
            .collect();
        titles.sort();
        assert_eq!(titles, vec!["Kept", "a---b"]);
        let kept = pinned.iter().find(|n| n["title"] == "Kept").unwrap();
        assert_eq!(kept["tags"], json!(["a"]));
    }

    #[test]
    fn tree_nests_dirs_first_and_skips_dotfiles() {
        let v = vault();
        let nb = v.path().join("Notebooks");
        std::fs::write(nb.join("a.md"), "").unwrap();
        std::fs::write(nb.join(".hidden.md"), "").unwrap();
        std::fs::write(nb.join("Inbox/b.md"), "").unwrap();
        let tree = build_tree(v.path(), &nb, "Notebooks".into());
        let kids = tree["children"].as_array().unwrap();
        assert_eq!(kids.len(), 2);
        assert_eq!(kids[0]["name"], "Inbox");
        assert_eq!(kids[0]["children"][0]["path"], "Notebooks/Inbox/b.md");
        assert_eq!(kids[1]["type"], "file");
    }
}
