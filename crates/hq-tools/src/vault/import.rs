//! Bringing the vault's non-Markdown files into notes.
//!
//! `vault_inventory` lists the files that are not notes and what can be done
//! with each; `vault_import` converts them (see `hq_convert::ingest`) into
//! Markdown notes with frontmatter that names the source file and its hash.
//!
//! The flow is built to be safe to repeat:
//! - a file that was already imported and has not changed is skipped;
//! - a name that is taken by another note gets a short hash suffix, never an overwrite;
//! - only a note this tool wrote (it has a `source_sha256`) is ever replaced, and only on request;
//! - originals stay where they are unless `archive_originals` is set, and are moved, never deleted.
//!
//! Sources must be inside the vault, so the tool reads nothing else on the host.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::Arc;
use std::time::SystemTime;

use anyhow::{Result, bail};
use async_trait::async_trait;
use hq_convert::ingest::sniff::Support;
use hq_convert::ingest::{self, Capabilities, ConversionReport, ConvertOptions, Kind, OcrMode};
use hq_convert::types::ConvertError;
use hq_db::Database;
use hq_vault::VaultClient;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::{
    names_private_folder, sync_note_index_and_cache, validate_path_in_vault, validate_write_path,
};
use crate::provenance::{Recorder, Writer, record_tool_write};
use crate::registry::HqTool;

/// Where chat uploads and downloaded attachments land. Their files are referenced by
/// conversations and kept as the owner's originals, so this tool never moves them.
const MEDIA_DIR: &str = "_media";
/// Where originals go when `archive_originals` is set.
const ARCHIVE_DIR: &str = "_archive/originals";
const MAX_DEPTH: usize = 24;
const MAX_VISITED: usize = 100_000;
const MAX_BATCH: usize = 200;

// ------------------------------------------------------------------ walking

#[derive(Debug, Clone)]
pub struct Found {
    pub rel: String,
    pub bytes: u64,
    pub modified: SystemTime,
    pub kind: Kind,
}

/// Directories never searched: hidden ones, dependency folders, exports (outputs, not
/// sources) and HQ's own folders. `_media` is the one underscore folder worth searching.
fn skip_dir(name: &str, top_level: bool) -> bool {
    name.starts_with('.')
        || name == "node_modules"
        || name == "_trash"
        || (top_level && (name == "Exports" || (name.starts_with('_') && name != MEDIA_DIR)))
}

/// The kind of a file worth importing, from its name; `None` for notes, data and everything else.
fn candidate_kind(path: &Path) -> Option<Kind> {
    let name = path.file_name()?.to_string_lossy();
    if name.starts_with('.') {
        return None;
    }
    match Kind::from_extension(path.extension()?.to_str()?) {
        Kind::Unknown | Kind::Markdown | Kind::Zip | Kind::Epub => None,
        k => Some(k),
    }
}

/// Calls `visit(rel, abs, metadata)` for every regular file under `start` (a vault-relative
/// folder, or empty for the whole vault), skipping folders [`skip_dir`] names and never following
/// a link. Returns whether the walk was cut short by the size limits.
fn walk_files(
    vault_root: &Path,
    start: &str,
    mut visit: impl FnMut(&str, &Path, &std::fs::Metadata),
) -> bool {
    let mut visited = 0usize;
    let mut stack: Vec<(std::path::PathBuf, String, usize)> = vec![(
        vault_root.join(start.trim_matches('/')),
        start.trim_matches('/').to_string(),
        0,
    )];
    while let Some((dir, rel_dir, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            visited += 1;
            if visited > MAX_VISITED {
                return true;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(meta) = entry.path().symlink_metadata() else {
                continue;
            };
            // A link could lead out of the vault; never follow one.
            if meta.file_type().is_symlink() {
                continue;
            }
            let rel = if rel_dir.is_empty() {
                name.clone()
            } else {
                format!("{rel_dir}/{name}")
            };
            if meta.is_dir() {
                if depth < MAX_DEPTH && !skip_dir(&name, rel_dir.is_empty()) {
                    stack.push((entry.path(), rel, depth + 1));
                }
            } else if meta.is_file() {
                visit(&rel, &entry.path(), &meta);
            }
        }
    }
    false
}

/// Files under `start` (a vault-relative folder, or empty for the whole vault) that could be imported.
/// Returns them, how many other files were passed over, and whether the search was cut short.
pub fn find_candidates(vault_root: &Path, start: &str) -> (Vec<Found>, usize, bool) {
    let mut found = Vec::new();
    let mut other = 0usize;
    let cut = walk_files(vault_root, start, |rel, abs, meta| {
        match candidate_kind(abs) {
            Some(kind) => found.push(Found {
                rel: rel.to_string(),
                bytes: meta.len(),
                modified: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                kind,
            }),
            None => {
                let name = rel.rsplit('/').next().unwrap_or(rel);
                if !name.ends_with(".md") && !name.starts_with('.') {
                    other += 1;
                }
            }
        }
    });
    found.sort_by(|a, b| a.rel.cmp(&b.rel));
    (found, other, cut)
}

// ------------------------------------------------------------------ naming

/// A file name stem made safe and readable for a note name.
fn clean_stem(stem: &str) -> String {
    let mut out = String::new();
    for c in stem.chars() {
        if c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | '(' | ')') {
            out.push(c);
        } else {
            out.push(' ');
        }
    }
    let collapsed = out.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed.trim_matches(|c: char| c == '.' || c == '-' || c == '_' || c == ' ');
    let cut: String = trimmed.chars().take(80).collect();
    if cut.is_empty() {
        "Imported document".to_string()
    } else {
        cut
    }
}

fn file_stem(rel: &str) -> &str {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    name.rsplit_once('.').map_or(name, |(s, _)| s)
}

fn year_month(t: SystemTime) -> String {
    chrono::DateTime::<chrono::Utc>::from(t)
        .format("%Y-%m")
        .to_string()
}

/// Where a file's note goes by default. A file in `Notebooks/` gets its note beside it;
/// anything else goes to `Notebooks/Imports/<year-month of the file>/`, so the path does
/// not change between runs.
pub fn default_note_path(rel: &str, modified: SystemTime, destination: Option<&str>) -> String {
    let stem = clean_stem(file_stem(rel));
    let dir = match destination
        .map(|d| d.trim().trim_matches('/'))
        .filter(|d| !d.is_empty())
    {
        Some(d) => d.to_string(),
        None if rel.starts_with("Notebooks/") => rel
            .rsplit_once('/')
            .map_or("Notebooks", |(d, _)| d)
            .to_string(),
        None => format!("Notebooks/Imports/{}", year_month(modified)),
    };
    format!("{dir}/{stem}.md")
}

fn with_suffix(note_path: &str, suffix: &str) -> String {
    match note_path.strip_suffix(".md") {
        Some(base) => format!("{base}-{suffix}.md"),
        None => format!("{note_path}-{suffix}"),
    }
}

// ----------------------------------------------------------------- settings

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Conflict {
    /// Leave the existing note alone and report it.
    Skip,
    /// Pick another name by adding a short hash of the file.
    Suffix,
    /// Replace a note this tool wrote earlier. Never a note written by hand.
    Overwrite,
}

impl Conflict {
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "skip" => Ok(Self::Skip),
            "suffix" => Ok(Self::Suffix),
            "overwrite" => Ok(Self::Overwrite),
            other => bail!("on_conflict must be skip, suffix or overwrite, not '{other}'"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ImportSettings {
    pub dry_run: bool,
    pub on_conflict: Conflict,
    /// A vault-relative folder for the new notes; see [`default_note_path`] for the default.
    pub destination: Option<String>,
    pub tags: Vec<String>,
    /// Move the original into `_archive/originals/` after its note is written.
    pub archive_originals: bool,
    pub preview_chars: usize,
    /// For a folder: only these formats (as the inventory names them, such as `pdf`, `docx`).
    pub formats: Option<Vec<String>>,
    /// For a folder: also import pictures. Left out by default, because photos, screenshots and
    /// slide renders read by OCR in bulk mostly make notes nobody wants; name one to import it.
    pub include_images: bool,
}

impl Default for ImportSettings {
    fn default() -> Self {
        Self {
            dry_run: false,
            on_conflict: Conflict::Suffix,
            destination: None,
            tags: Vec::new(),
            archive_originals: false,
            preview_chars: 0,
            formats: None,
            include_images: false,
        }
    }
}

// ----------------------------------------------------------------- importer

/// Who is writing, for provenance: a tool call (the arguments say who) or a known writer.
pub enum Caller<'a> {
    Tool(&'a Value),
    Known(Writer),
}

pub struct Importer {
    vault: Arc<VaultClient>,
    db: Option<Arc<Database>>,
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// The notes earlier imports wrote, by the file each came from: `(note path, source hash)`.
/// Found by reading the frontmatter of the vault's notes, so it holds wherever a note was put
/// or moved to, and needs no database.
#[derive(Default)]
struct ImportedIndex {
    by_source: HashMap<String, Vec<(String, String)>>,
    /// Paths a dry run has promised to earlier files in the same batch, so two files that would
    /// get one name are shown with two, as the real run will give them.
    reserved: std::collections::HashSet<String>,
}

impl ImportedIndex {
    fn add(&mut self, source: &str, note: &str, sha: &str) {
        let entry = self.by_source.entry(source.to_string()).or_default();
        entry.retain(|(n, _)| n != note);
        entry.push((note.to_string(), sha.to_string()));
    }

    fn of(&self, source: &str) -> &[(String, String)] {
        self.by_source.get(source).map(Vec::as_slice).unwrap_or(&[])
    }
}

/// `(source, source_sha256)` from the head of a note an import wrote; `None` for any other note.
fn import_origin(abs: &Path) -> Option<(String, String)> {
    use std::io::Read;
    let mut head = Vec::with_capacity(8192);
    std::fs::File::open(abs)
        .ok()?
        .take(8192)
        .read_to_end(&mut head)
        .ok()?;
    let text = String::from_utf8_lossy(&head);
    let rest = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))?;
    let block = &rest[..rest.find("\n---")?];
    let fm: HashMap<String, serde_yaml::Value> = serde_yaml::from_str(block).ok()?;
    let get = |k: &str| fm.get(k).and_then(|v| v.as_str()).map(str::to_string);
    Some((get("source")?, get("source_sha256")?))
}

enum Plan {
    Create(String),
    Update(String),
    Skip {
        path: String,
        reason: String,
        unchanged: bool,
    },
}

impl Importer {
    pub fn new(vault: Arc<VaultClient>, db: Option<Arc<Database>>) -> Self {
        Self { vault, db }
    }

    fn root(&self) -> &Path {
        self.vault.vault_path()
    }

    /// Where an existing note's frontmatter says it came from: (source path, source hash).
    fn origin_of(&self, note_path: &str) -> Option<(Option<String>, Option<String>)> {
        let note = self.vault.read_note(note_path).ok()?;
        let get = |k: &str| {
            note.frontmatter
                .get(k)
                .and_then(|v| v.as_str())
                .map(str::to_string)
        };
        Some((get("source"), get("source_sha256")))
    }

    fn imported_index(&self) -> ImportedIndex {
        let mut index = ImportedIndex::default();
        walk_files(self.root(), "", |rel, abs, _| {
            if rel.ends_with(".md")
                && let Some((source, sha)) = import_origin(abs)
            {
                index.add(&source, rel, &sha);
            }
        });
        index
    }

    fn plan(
        &self,
        rel: &str,
        sha: &str,
        desired: &str,
        conflict: Conflict,
        index: &ImportedIndex,
    ) -> Plan {
        // A note an earlier import wrote for this very file settles it, wherever the note is now.
        let earlier = index.of(rel);
        if let Some((note, _)) = earlier.iter().find(|(_, h)| h == sha) {
            return Plan::Skip {
                path: note.clone(),
                reason: "already imported and unchanged".into(),
                unchanged: true,
            };
        }
        if let Some((note, _)) = earlier.first() {
            return if conflict == Conflict::Overwrite {
                Plan::Update(note.clone())
            } else {
                Plan::Skip {
                    path: note.clone(),
                    reason: "the file changed since it was imported; use on_conflict=overwrite to replace the note (edits made to it will be lost)".into(),
                    unchanged: false,
                }
            };
        }
        let decide = |path: String| -> Option<Plan> {
            if !self.root().join(&path).exists() && !index.reserved.contains(&path) {
                return Some(Plan::Create(path));
            }
            let (src, hash) = self.origin_of(&path).unwrap_or((None, None));
            match (src.as_deref() == Some(rel), hash.as_deref() == Some(sha)) {
                (true, true) => Some(Plan::Skip {
                    path,
                    reason: "already imported and unchanged".into(),
                    unchanged: true,
                }),
                (true, false) if conflict == Conflict::Overwrite => Some(Plan::Update(path)),
                (true, false) => Some(Plan::Skip {
                    reason: "the file changed since it was imported; use on_conflict=overwrite to replace the note (edits made to it will be lost)".into(),
                    path,
                    unchanged: false,
                }),
                (false, _) => None,
            }
        };
        if let Some(plan) = decide(desired.to_string()) {
            return plan;
        }
        // The name belongs to another note.
        match conflict {
            Conflict::Skip => Plan::Skip {
                path: desired.to_string(),
                reason: "another note already has this name".into(),
                unchanged: false,
            },
            Conflict::Suffix => {
                let alt = with_suffix(desired, &sha[..6]);
                decide(alt.clone()).unwrap_or(Plan::Skip {
                    path: alt,
                    reason: "the alternative name is taken by another note too".into(),
                    unchanged: false,
                })
            }
            Conflict::Overwrite => {
                let ours = self
                    .origin_of(desired)
                    .is_some_and(|(_, hash)| hash.is_some());
                if ours {
                    Plan::Update(desired.to_string())
                } else {
                    Plan::Skip {
                        path: desired.to_string(),
                        reason: "that note was not written by an import, so it is not replaced"
                            .into(),
                        unchanged: false,
                    }
                }
            }
        }
    }

    /// Converts one vault file and, unless it is a dry run, writes its note.
    pub async fn import_one(&self, rel: &str, s: &ImportSettings, caller: &Caller<'_>) -> Value {
        self.import_with(rel, s, caller, &mut self.imported_index())
            .await
    }

    async fn import_with(
        &self,
        rel: &str,
        s: &ImportSettings,
        caller: &Caller<'_>,
        index: &mut ImportedIndex,
    ) -> Value {
        match self.try_import(rel, s, caller, index).await {
            Ok(v) => v,
            Err(e) => json!({ "source": rel, "action": "failed", "reason": format!("{e}") }),
        }
    }

    async fn try_import(
        &self,
        rel: &str,
        s: &ImportSettings,
        caller: &Caller<'_>,
        index: &mut ImportedIndex,
    ) -> Result<Value> {
        validate_path_in_vault(self.root(), rel)?;
        if names_private_folder(rel) {
            bail!("files in HQ's own folders (_system, _threads, _data) are not imported");
        }
        let abs = self.root().join(rel);
        if !abs.is_file() {
            bail!("no such file in the vault");
        }
        if abs
            .symlink_metadata()
            .is_ok_and(|m| m.file_type().is_symlink())
        {
            bail!("symbolic links are not followed");
        }
        let kind = candidate_kind(&abs);
        let Some(kind) = kind else {
            bail!("not a file type that imports (notes are already Markdown)");
        };
        let modified = std::fs::metadata(&abs)
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        if let Some(d) = &s.destination {
            if d.trim_matches('/').ends_with(".md") {
                bail!("destination is a folder, not a note path");
            }
            validate_write_path(self.root(), &format!("{}/x.md", d.trim_matches('/')))?;
        }

        let converted = match ingest::convert_file(
            &abs,
            &ConvertOptions {
                ocr: OcrMode::Auto,
                ..ConvertOptions::default()
            },
        )
        .await
        {
            Ok(c) => c,
            Err(ConvertError::UnsupportedFormat(m)) => bail!("cannot convert this file here: {m}"),
            Err(e) => bail!("{e}"),
        };
        let report = &converted.report;
        let desired = default_note_path(rel, modified, s.destination.as_deref());
        let plan = self.plan(rel, &report.source_sha256, &desired, s.on_conflict, index);
        let title = note_title(&converted, rel);
        let body = compose_body(&title, &converted.markdown, report, rel);
        let preview: Option<String> =
            (s.preview_chars > 0).then(|| body.chars().take(s.preview_chars).collect());

        let mut out = json!({
            "source": rel,
            "title": title,
            "format": report.source_format,
            "engine": report.engine,
            "outcome": report.outcome,
            "warnings": report.warning_lines(),
            "stats": report.stats,
        });
        let obj = out.as_object_mut().expect("object");
        if let Some(p) = preview {
            obj.insert("preview".into(), Value::String(p));
        }
        let (path, creating) = match plan {
            Plan::Skip {
                path,
                reason,
                unchanged,
            } => {
                obj.insert("action".into(), json!("skipped"));
                obj.insert("destination".into(), json!(path));
                obj.insert("reason".into(), json!(reason));
                obj.insert("unchanged".into(), json!(unchanged));
                return Ok(out);
            }
            Plan::Create(p) => (p, true),
            Plan::Update(p) => (p, false),
        };
        obj.insert("destination".into(), json!(path));
        index.reserved.insert(path.clone());
        if s.dry_run {
            obj.insert(
                "action".into(),
                json!(if creating {
                    "would_create"
                } else {
                    "would_update"
                }),
            );
            if s.archive_originals {
                obj.insert(
                    "would_archive_to".into(),
                    json!(archive_target(self.root(), rel)),
                );
            }
            return Ok(out);
        }

        let mut tags = vec!["imported".to_string(), report.source_format.clone()];
        tags.extend(s.tags.iter().cloned());
        tags.dedup();
        let note = build_note(&path, &title, body, rel, &converted.report, kind, &tags);
        let vault = self.vault.clone();
        let (note_path, to_write) = (path.clone(), note.clone());
        tokio::task::spawn_blocking(move || vault.write_note(&note_path, &to_write)).await??;
        sync_note_index_and_cache(
            self.db.as_ref(),
            &path,
            &note.content,
            &note.title,
            &note.tags,
        );
        self.record(&path, caller).await;
        // Read it back: the note is only counted once it can be read.
        let back = self.vault.read_note(&path)?;
        if back.content.trim().is_empty() {
            bail!("the note was written but reads back empty");
        }
        obj.insert(
            "action".into(),
            json!(if creating { "created" } else { "updated" }),
        );
        index.add(rel, &path, &report.source_sha256);

        if s.archive_originals {
            match self.archive(rel, &path, caller).await {
                Ok(to) => {
                    obj.insert("archived_to".into(), json!(to));
                }
                Err(e) => {
                    obj.insert("archive_skipped".into(), json!(format!("{e}")));
                }
            }
        }
        Ok(out)
    }

    async fn record(&self, note_path: &str, caller: &Caller<'_>) {
        match caller {
            Caller::Tool(args) => {
                record_tool_write(self.db.as_ref(), self.root(), note_path, args).await
            }
            Caller::Known(writer) => {
                if let Some(db) = &self.db {
                    let recorder = Recorder::new(db.clone(), self.root().to_path_buf());
                    let (path, writer) = (note_path.to_string(), writer.clone());
                    let _ =
                        tokio::task::spawn_blocking(move || recorder.record(&path, &writer)).await;
                }
            }
        }
    }

    /// Moves the original to `_archive/originals/` and points the note's `source` at it.
    async fn archive(&self, rel: &str, note_path: &str, caller: &Caller<'_>) -> Result<String> {
        if rel.starts_with(&format!("{MEDIA_DIR}/")) {
            bail!("files in _media are kept in place: conversations refer to them");
        }
        let target = archive_target(self.root(), rel);
        let (from, to) = (self.root().join(rel), self.root().join(&target));
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::rename(&from, &to)?;
        // Re-point the note at the new location.
        let mut note = self.vault.read_note(note_path)?;
        note.frontmatter
            .insert("source".into(), serde_yaml::Value::String(target.clone()));
        note.frontmatter.insert(
            "original_location".into(),
            serde_yaml::Value::String(rel.to_string()),
        );
        note.modified_at = chrono::Utc::now();
        let vault = self.vault.clone();
        let path = note_path.to_string();
        tokio::task::spawn_blocking(move || vault.write_note(&path, &note)).await??;
        self.record(note_path, caller).await;
        Ok(target)
    }

    // ----------------------------------------------------------- inventory

    /// The files under `start` that are not notes, and what could be done with each.
    pub fn inventory(&self, start: &str, filter: InventoryFilter) -> Result<Value> {
        if !start.is_empty() {
            validate_path_in_vault(self.root(), &format!("{}/x", start.trim_matches('/')))?;
            if names_private_folder(start) {
                bail!("HQ's own folders (_system, _threads, _data) are not searched");
            }
        }
        let caps = ingest::capabilities();
        let index = self.imported_index();
        let (files, other, cut) = find_candidates(self.root(), start);
        let mut entries: Vec<Value> = Vec::new();
        let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
        let mut kinds: BTreeMap<&'static str, usize> = BTreeMap::new();
        let mut total_bytes = 0u64;
        for f in &files {
            let status = self.status_of(f, &caps, &index);
            *counts.entry(status.0).or_default() += 1;
            *kinds.entry(f.kind.label()).or_default() += 1;
            total_bytes += f.bytes;
            if filter
                .statuses
                .as_ref()
                .is_some_and(|s| !s.iter().any(|x| x == status.0))
            {
                continue;
            }
            if filter
                .kinds
                .as_ref()
                .is_some_and(|k| !k.iter().any(|x| x == f.kind.label()))
            {
                continue;
            }
            entries.push(json!({
                "path": f.rel,
                "bytes": f.bytes,
                "modified": chrono::DateTime::<chrono::Utc>::from(f.modified).to_rfc3339(),
                "format": f.kind.label(),
                "status": status.0,
                "support": f.kind.support(&caps).label(),
                "note": status.1,
            }));
        }
        let order = |s: &str| match s {
            "pending" => 0,
            "changed" => 1,
            "blocked" => 2,
            _ => 3,
        };
        entries.sort_by_key(|e| {
            (
                order(e["status"].as_str().unwrap_or("")),
                e["path"].as_str().unwrap_or("").to_string(),
            )
        });
        let matching = entries.len();
        entries.truncate(filter.limit);
        Ok(json!({
            "folder": if start.is_empty() { "(whole vault)" } else { start },
            "files": entries.len(),
            "matching": matching,
            "found": files.len(),
            "total_bytes": total_bytes,
            "by_status": counts,
            "by_format": kinds,
            "other_files_not_listed": other,
            "search_cut_short": cut,
            "can_do_here": {
                "ocr": caps.ocr,
                "pdf_tables": caps.pdf_tables,
                "legacy_office": caps.legacy_office,
            },
            "entries": entries,
            "next": "vault_import with `path`, `paths` or `directory` (dry_run first) converts these into notes",
        }))
    }

    /// `pending`, `imported`, `changed` or `blocked`, and the note or reason that goes with it.
    fn status_of(
        &self,
        f: &Found,
        caps: &Capabilities,
        index: &ImportedIndex,
    ) -> (&'static str, String) {
        let earlier = index.of(&f.rel);
        if !earlier.is_empty() {
            let current = std::fs::read(self.root().join(&f.rel))
                .map(|b| sha256_hex(&b))
                .ok();
            if let Some((note, _)) = earlier.iter().find(|(_, h)| Some(h) == current.as_ref()) {
                return ("imported", note.clone());
            }
        }
        let support = f.kind.support(caps);
        if support != Support::Ready {
            return ("blocked", support.label().to_string());
        }
        match earlier.first() {
            Some((note, _)) => ("changed", note.clone()),
            None => ("pending", String::new()),
        }
    }
}

/// What to import: files named one by one, or whatever is still pending under a folder.
#[derive(Debug, Clone)]
pub enum Source {
    Path(String),
    Paths(Vec<String>),
    /// A vault-relative folder (empty for the whole vault): its pending and changed files.
    Directory(String),
}

impl Importer {
    /// Imports `source` and returns the batch report both the tool and the CLI print:
    /// totals, then one entry per file. `limit` caps a directory batch (at most [`MAX_BATCH`]);
    /// `more_pending` says how many were left for the next call.
    pub async fn run(
        &self,
        source: Source,
        settings: &ImportSettings,
        limit: usize,
        caller: &Caller<'_>,
    ) -> Result<Value> {
        let limit = limit.clamp(1, MAX_BATCH);
        let mut more_pending = 0usize;
        let mut images_left_out = 0usize;
        let sources: Vec<String> = match source {
            Source::Path(p) => vec![p],
            Source::Paths(paths) => {
                if paths.len() > MAX_BATCH {
                    bail!("at most {MAX_BATCH} paths in one call");
                }
                paths
            }
            Source::Directory(dir) => {
                let formats: Option<Vec<String>> = settings
                    .formats
                    .as_ref()
                    .map(|f| f.iter().map(|k| k.trim().to_ascii_lowercase()).collect());
                let inv = self.inventory(
                    &dir,
                    InventoryFilter {
                        statuses: Some(vec!["pending".into(), "changed".into()]),
                        kinds: formats.clone(),
                        limit: 5000,
                    },
                )?;
                let wants_images = settings.include_images
                    || formats
                        .as_ref()
                        .is_some_and(|f| f.iter().any(|k| k == "image"));
                let entries = inv["entries"].as_array().cloned().unwrap_or_default();
                let (images, documents): (Vec<_>, Vec<_>) = entries
                    .iter()
                    .partition(|x| x["format"] == "image" && !wants_images);
                images_left_out = images.len();
                let mut all: Vec<String> = documents
                    .iter()
                    .filter_map(|x| x["path"].as_str().map(str::to_string))
                    .collect();
                more_pending = all.len().saturating_sub(limit);
                all.truncate(limit);
                all
            }
        };

        let mut index = self.imported_index();
        let mut results = Vec::with_capacity(sources.len());
        for rel in &sources {
            results.push(self.import_with(rel, settings, caller, &mut index).await);
        }
        let count = |action: &str| results.iter().filter(|r| r["action"] == action).count();
        let degraded = results
            .iter()
            .filter(|r| r["outcome"] == "degraded")
            .count();
        Ok(json!({
            "dry_run": settings.dry_run,
            "requested": sources.len(),
            "created": count("created"),
            "updated": count("updated"),
            "would_create": count("would_create") + count("would_update"),
            "skipped": count("skipped"),
            "failed": count("failed"),
            "degraded": degraded,
            "more_pending": more_pending,
            "images_left_out": images_left_out,
            "results": results,
        }))
    }
}

#[derive(Debug, Clone)]
pub struct InventoryFilter {
    pub statuses: Option<Vec<String>>,
    pub kinds: Option<Vec<String>>,
    pub limit: usize,
}

fn archive_target(root: &Path, rel: &str) -> String {
    let mut target = format!("{ARCHIVE_DIR}/{rel}");
    let mut n = 2;
    while root.join(&target).exists() {
        let (stem, ext) = rel.rsplit_once('.').unwrap_or((rel, ""));
        target = format!("{ARCHIVE_DIR}/{stem} ({n}).{ext}");
        n += 1;
    }
    target
}

// -------------------------------------------------------------- note content

/// The title the file names, else its first heading, else its file name made readable.
fn note_title(c: &ingest::Converted, rel: &str) -> String {
    if let Some(t) = c.title.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        return t.chars().take(120).collect();
    }
    if let Some(h) = c
        .markdown
        .lines()
        .find(|l| !l.trim().is_empty())
        .and_then(|l| l.strip_prefix("# "))
    {
        let h = h.trim();
        if !h.is_empty() {
            return h.chars().take(120).collect();
        }
    }
    clean_stem(file_stem(rel))
}

fn compose_body(title: &str, markdown: &str, report: &ConversionReport, rel: &str) -> String {
    let mut body = String::new();
    if !markdown.trim_start().starts_with("# ") {
        body.push_str(&format!("# {title}\n\n"));
    }
    body.push_str(markdown.trim_end());
    body.push('\n');
    if !report.warnings.is_empty() {
        body.push_str(&format!(
            "\n---\n\n## Import notes\n\nConverted from `{rel}` ({}, {}).\n\n",
            report.source_format, report.engine
        ));
        for w in &report.warnings {
            body.push_str(&format!("- {}\n", w.detail));
        }
    }
    body
}

fn build_note(
    path: &str,
    title: &str,
    content: String,
    rel: &str,
    report: &ConversionReport,
    kind: Kind,
    tags: &[String],
) -> hq_core::types::Note {
    let mut fm: HashMap<String, serde_yaml::Value> = HashMap::new();
    let text = |s: &str| serde_yaml::Value::String(s.to_string());
    fm.insert("title".into(), text(title));
    fm.insert("noteType".into(), text("imported-document"));
    fm.insert("fileName".into(), text(file_stem(path)));
    fm.insert("source".into(), text(rel));
    fm.insert("source_sha256".into(), text(&report.source_sha256));
    fm.insert("source_format".into(), text(kind.label()));
    fm.insert(
        "converted_at".into(),
        text(&chrono::Utc::now().to_rfc3339()),
    );
    fm.insert("converter".into(), text(&report.engine));
    fm.insert(
        "conversion".into(),
        text(if report.has_loss() { "degraded" } else { "ok" }),
    );
    fm.insert(
        "tags".into(),
        serde_yaml::Value::Sequence(tags.iter().map(|t| text(t)).collect()),
    );
    hq_core::types::Note {
        title: title.to_string(),
        content,
        path: path.to_string(),
        frontmatter: fm,
        note_type: None,
        tags: tags.to_vec(),
        pinned: false,
        source: None,
        embedding_status: None,
        created_at: None,
        updated_at: None,
        modified_at: chrono::Utc::now(),
    }
}

// -------------------------------------------------------------------- tools

fn str_list(args: &Value, key: &str) -> Vec<String> {
    args.get(key)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

pub struct VaultInventoryTool {
    importer: Arc<Importer>,
}

impl VaultInventoryTool {
    pub fn new(importer: Arc<Importer>) -> Self {
        Self { importer }
    }
}

#[async_trait]
impl HqTool for VaultInventoryTool {
    fn name(&self) -> &str {
        "vault_inventory"
    }

    fn description(&self) -> &str {
        "List the files in the vault that are not notes (PDF, Word, Excel, PowerPoint, OpenDocument, RTF, \
         HTML, XML, CSV, text, images) and what can be done with each: whether it is already imported, \
         pending, changed since it was imported, or blocked because this host lacks a tool (OCR, LibreOffice). \
         Read-only. Use it first, then vault_import."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "directory": { "type": "string", "description": "Vault-relative folder to look in (default: the whole vault). Searches subfolders; skips HQ's own folders and Exports." },
                "status": { "type": "array", "items": { "type": "string", "enum": ["pending", "imported", "changed", "blocked"] }, "description": "Only these statuses (default: all)" },
                "format": { "type": "array", "items": { "type": "string" }, "description": "Only these formats, such as pdf or docx" },
                "limit": { "type": "integer", "description": "Most entries to list (default 100, at most 500). Counts always cover everything found." }
            }
        })
    }

    fn category(&self) -> &str {
        "vault"
    }

    fn is_read_only(&self) -> bool {
        true
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let dir = args
            .get("directory")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let filter = InventoryFilter {
            statuses: Some(str_list(&args, "status")).filter(|v| !v.is_empty()),
            kinds: Some(str_list(&args, "format")).filter(|v| !v.is_empty()),
            limit: args
                .get("limit")
                .and_then(Value::as_u64)
                .unwrap_or(100)
                .clamp(1, 500) as usize,
        };
        let importer = self.importer.clone();
        tokio::task::spawn_blocking(move || importer.inventory(&dir, filter)).await?
    }
}

pub struct VaultImportTool {
    importer: Arc<Importer>,
}

impl VaultImportTool {
    pub fn new(importer: Arc<Importer>) -> Self {
        Self { importer }
    }
}

#[async_trait]
impl HqTool for VaultImportTool {
    fn name(&self) -> &str {
        "vault_import"
    }

    fn description(&self) -> &str {
        "Convert files that are inside the vault (PDF, Word, Excel, PowerPoint, OpenDocument, RTF, HTML, \
         XML, CSV, text, scanned pages and photos by OCR) into Markdown notes. Give one `path`, a list of \
         `paths`, or a `directory` (pending files only). Each note gets frontmatter naming its source file \
         and hash, so a repeat run skips what is done. Names that collide get a hash suffix; originals stay \
         put unless archive_originals is set. A directory import is a dry run unless you pass dry_run=false. \
         The result reports, per file, what was kept and what was flattened or read by OCR."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "One vault-relative file to import" },
                "paths": { "type": "array", "items": { "type": "string" }, "description": "Several vault-relative files (at most 200)" },
                "directory": { "type": "string", "description": "A vault-relative folder: every pending file under it (use limit to batch)" },
                "destination": { "type": "string", "description": "Folder for the new notes. Default: beside the file when it is under Notebooks/, else Notebooks/Imports/<year-month>/" },
                "dry_run": { "type": "boolean", "description": "Convert and report without writing. Default false for path/paths, true for directory." },
                "on_conflict": { "type": "string", "enum": ["skip", "suffix", "overwrite"], "description": "When the note name is taken by another note: skip it, add a hash suffix (default), or replace a note an earlier import wrote" },
                "tags": { "type": "array", "items": { "type": "string" }, "description": "Extra tags; imported notes always get 'imported' and their format" },
                "archive_originals": { "type": "boolean", "description": "After the note is written, move the original to _archive/originals/ (never for files in _media). Default false." },
                "preview_chars": { "type": "integer", "description": "Include this many characters of each converted note in the result (default 0, or 600 for a dry run)" },
                "format": { "type": "array", "items": { "type": "string" }, "description": "For directory: only these formats, such as pdf or docx" },
                "include_images": { "type": "boolean", "description": "For directory: also import pictures (photos, screenshots, slide renders). Default false: they are listed as images_left_out, and one can be imported by naming it." },
                "limit": { "type": "integer", "description": "For directory: most files to handle in one call (default 10, at most 200). Call again for the next batch." }
            }
        })
    }

    fn category(&self) -> &str {
        "vault"
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let path = args.get("path").and_then(Value::as_str).map(str::to_string);
        let paths = str_list(&args, "paths");
        let directory = args
            .get("directory")
            .and_then(Value::as_str)
            .map(str::to_string);
        let given = path.is_some() as u8 + !paths.is_empty() as u8 + directory.is_some() as u8;
        if given != 1 {
            bail!("give exactly one of path, paths or directory");
        }
        let dry_run = args
            .get("dry_run")
            .and_then(Value::as_bool)
            .unwrap_or(directory.is_some());
        let settings = ImportSettings {
            dry_run,
            on_conflict: Conflict::parse(
                args.get("on_conflict")
                    .and_then(Value::as_str)
                    .unwrap_or("suffix"),
            )?,
            destination: args
                .get("destination")
                .and_then(Value::as_str)
                .map(str::to_string),
            tags: str_list(&args, "tags"),
            archive_originals: args
                .get("archive_originals")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            preview_chars: args
                .get("preview_chars")
                .and_then(Value::as_u64)
                .map(|n| n.min(5000) as usize)
                .unwrap_or(if dry_run { 600 } else { 0 }),
            formats: Some(str_list(&args, "format")).filter(|f| !f.is_empty()),
            include_images: args
                .get("include_images")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        };
        let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(10) as usize;
        let source = match (path, paths.is_empty(), directory) {
            (Some(p), true, None) => Source::Path(p),
            (None, false, None) => Source::Paths(paths),
            (None, true, Some(d)) => Source::Directory(d),
            _ => unreachable!("exactly one source was checked above"),
        };
        self.importer
            .run(source, &settings, limit, &Caller::Tool(&args))
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    const INVOICE: &[u8] = include_bytes!("../../../hq-convert/tests/fixtures/corpus/invoice.docx");
    const TRACKER: &[u8] = include_bytes!("../../../hq-convert/tests/fixtures/corpus/tracker.xlsx");
    const PAGE: &[u8] = include_bytes!("../../../hq-convert/tests/fixtures/corpus/page.html");

    fn vault() -> (TempDir, Arc<Importer>) {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir_all(dir.path().join("Notebooks")).unwrap();
        let client = Arc::new(VaultClient::new(dir.path().to_path_buf()).unwrap());
        (dir, Arc::new(Importer::new(client, None)))
    }

    fn put(dir: &TempDir, rel: &str, bytes: &[u8]) {
        let p = dir.path().join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, bytes).unwrap();
    }

    async fn run(importer: &Arc<Importer>, args: Value) -> Value {
        VaultImportTool::new(importer.clone())
            .execute(args)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn a_dry_run_converts_and_reports_but_writes_nothing() {
        let (dir, imp) = vault();
        put(&dir, "_media/2026-09-22/Invoice 42.docx", INVOICE);
        let out = run(
            &imp,
            json!({"path": "_media/2026-09-22/Invoice 42.docx", "dry_run": true}),
        )
        .await;
        let r = &out["results"][0];
        assert_eq!(r["action"], "would_create");
        assert!(
            r["destination"]
                .as_str()
                .unwrap()
                .starts_with("Notebooks/Imports/"),
            "{r}"
        );
        assert!(
            r["destination"]
                .as_str()
                .unwrap()
                .ends_with("/Invoice 42.md"),
            "{r}"
        );
        assert!(r["preview"].as_str().unwrap().contains("INVOICE"));
        assert_eq!(r["outcome"], "ok");
        assert!(!dir.path().join("Notebooks/Imports").exists());
    }

    #[tokio::test]
    async fn an_import_writes_a_note_with_frontmatter_naming_its_source() {
        let (dir, imp) = vault();
        put(&dir, "_media/inv.docx", INVOICE);
        let out = run(&imp, json!({"path": "_media/inv.docx"})).await;
        assert_eq!(out["created"], 1, "{out}");
        let dest = out["results"][0]["destination"]
            .as_str()
            .unwrap()
            .to_string();
        let raw = std::fs::read_to_string(dir.path().join(&dest)).unwrap();
        for needle in [
            "source: _media/inv.docx",
            "source_sha256:",
            "noteType: imported-document",
            "conversion: ok",
            "# INVOICE",
            "| Maize seed, 25 kg bag | 10 | 85,000 | 850,000 |",
        ] {
            assert!(raw.contains(needle), "missing {needle:?} in:\n{raw}");
        }
        assert!(raw.contains("imported") && raw.contains("docx"));
        // The original is untouched.
        assert!(dir.path().join("_media/inv.docx").exists());
    }

    #[tokio::test]
    async fn a_repeat_import_skips_unchanged_files_and_flags_changed_ones() {
        let (dir, imp) = vault();
        put(&dir, "Notebooks/Projects/tracker.xlsx", TRACKER);
        let first = run(&imp, json!({"path": "Notebooks/Projects/tracker.xlsx"})).await;
        // Under Notebooks the note sits beside the file.
        assert_eq!(
            first["results"][0]["destination"],
            "Notebooks/Projects/tracker.md"
        );
        let again = run(&imp, json!({"path": "Notebooks/Projects/tracker.xlsx"})).await;
        assert_eq!(again["results"][0]["action"], "skipped");
        assert_eq!(again["results"][0]["unchanged"], true);

        put(&dir, "Notebooks/Projects/tracker.xlsx", INVOICE); // different bytes, same path
        let changed = run(&imp, json!({"path": "Notebooks/Projects/tracker.xlsx"})).await;
        assert_eq!(changed["results"][0]["action"], "skipped");
        assert!(
            changed["results"][0]["reason"]
                .as_str()
                .unwrap()
                .contains("changed")
        );
        let replaced = run(
            &imp,
            json!({"path": "Notebooks/Projects/tracker.xlsx", "on_conflict": "overwrite"}),
        )
        .await;
        assert_eq!(replaced["updated"], 1, "{replaced}");
    }

    const PHOTO: &[u8] = include_bytes!("../../../hq-convert/tests/fixtures/corpus/receipt.jpg");

    fn sources(out: &Value) -> Vec<String> {
        out["results"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["source"].as_str().unwrap().to_string())
            .collect()
    }

    #[tokio::test]
    async fn a_folder_import_leaves_pictures_out_unless_asked_and_a_named_one_always_goes() {
        let (dir, imp) = vault();
        put(&dir, "Notebooks/Docs/Invoice.docx", INVOICE);
        put(&dir, "Notebooks/Docs/photo.jpg", PHOTO);
        let ocr = ingest::capabilities().ocr;

        let plain = run(&imp, json!({"directory": "Notebooks/Docs"})).await;
        assert_eq!(sources(&plain), ["Notebooks/Docs/Invoice.docx"], "{plain}");
        // A host with no OCR lists the picture as blocked, not pending, so nothing is left out.
        assert_eq!(plain["images_left_out"], if ocr { 1 } else { 0 }, "{plain}");

        if ocr {
            let with = run(
                &imp,
                json!({"directory": "Notebooks/Docs", "include_images": true}),
            )
            .await;
            assert_eq!(sources(&with).len(), 2, "{with}");
            let only = run(
                &imp,
                json!({"directory": "Notebooks/Docs", "format": ["image"]}),
            )
            .await;
            assert_eq!(sources(&only), ["Notebooks/Docs/photo.jpg"], "{only}");
        }
        let only_docs = run(
            &imp,
            json!({"directory": "Notebooks/Docs", "format": ["PDF", "docx"]}),
        )
        .await;
        assert_eq!(
            sources(&only_docs),
            ["Notebooks/Docs/Invoice.docx"],
            "{only_docs}"
        );

        // Naming the picture imports it whatever the folder rules say.
        let named = run(
            &imp,
            json!({"path": "Notebooks/Docs/photo.jpg", "dry_run": true}),
        )
        .await;
        assert_eq!(sources(&named), ["Notebooks/Docs/photo.jpg"]);
    }

    #[tokio::test]
    async fn a_dry_run_shows_each_file_the_name_the_real_run_will_give_it() {
        let (dir, imp) = vault();
        // The same document saved in two formats: both want "Notebooks/Docs/Report.md".
        put(&dir, "Notebooks/Docs/Report.docx", INVOICE);
        put(&dir, "Notebooks/Docs/Report.html", PAGE);
        let preview = run(&imp, json!({"directory": "Notebooks/Docs"})).await;
        let planned: Vec<&str> = preview["results"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["destination"].as_str().unwrap())
            .collect();
        assert_eq!(planned.len(), 2);
        assert_ne!(planned[0], planned[1], "{preview}");

        let real = run(
            &imp,
            json!({"directory": "Notebooks/Docs", "dry_run": false}),
        )
        .await;
        let written: Vec<&str> = real["results"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["destination"].as_str().unwrap())
            .collect();
        assert_eq!(planned, written, "the preview matches what was written");
    }

    #[tokio::test]
    async fn an_imported_note_is_recognised_wherever_it_was_moved_or_sent() {
        let (dir, imp) = vault();
        put(&dir, "Notebooks/Projects/tracker.xlsx", TRACKER);
        run(
            &imp,
            json!({"path": "Notebooks/Projects/tracker.xlsx", "destination": "Notebooks/Sheets"}),
        )
        .await;
        assert!(dir.path().join("Notebooks/Sheets/tracker.md").is_file());

        // Asking again for the default place does not make a second copy.
        let again = run(&imp, json!({"path": "Notebooks/Projects/tracker.xlsx"})).await;
        assert_eq!(again["results"][0]["action"], "skipped", "{again}");
        assert_eq!(
            again["results"][0]["destination"],
            "Notebooks/Sheets/tracker.md"
        );

        // Nor after the owner moves the note somewhere else.
        std::fs::create_dir_all(dir.path().join("Notebooks/Finance")).unwrap();
        std::fs::rename(
            dir.path().join("Notebooks/Sheets/tracker.md"),
            dir.path().join("Notebooks/Finance/Tracker 2026.md"),
        )
        .unwrap();
        let inv = imp
            .inventory(
                "",
                InventoryFilter {
                    statuses: None,
                    kinds: None,
                    limit: 10,
                },
            )
            .unwrap();
        assert_eq!(inv["entries"][0]["status"], "imported", "{inv}");
        assert_eq!(
            inv["entries"][0]["note"],
            "Notebooks/Finance/Tracker 2026.md"
        );

        // A changed source updates the note where it now lives.
        put(&dir, "Notebooks/Projects/tracker.xlsx", INVOICE);
        let replaced = run(
            &imp,
            json!({"path": "Notebooks/Projects/tracker.xlsx", "on_conflict": "overwrite"}),
        )
        .await;
        assert_eq!(
            replaced["results"][0]["destination"], "Notebooks/Finance/Tracker 2026.md",
            "{replaced}"
        );
        assert!(!dir.path().join("Notebooks/Projects/tracker.md").exists());
    }

    #[tokio::test]
    async fn a_name_taken_by_another_note_gets_a_hash_suffix_and_a_human_note_is_never_replaced() {
        let (dir, imp) = vault();
        put(&dir, "Notebooks/Docs/Report.docx", INVOICE);
        std::fs::write(
            dir.path().join("Notebooks/Docs/Report.md"),
            "# My own notes\n",
        )
        .unwrap();
        let out = run(&imp, json!({"path": "Notebooks/Docs/Report.docx"})).await;
        let dest = out["results"][0]["destination"].as_str().unwrap();
        assert!(
            dest.starts_with("Notebooks/Docs/Report-") && dest.ends_with(".md"),
            "{dest}"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("Notebooks/Docs/Report.md")).unwrap(),
            "# My own notes\n"
        );

        let refused = run(
            &imp,
            json!({"path": "Notebooks/Docs/Report.docx", "on_conflict": "skip"}),
        )
        .await;
        assert_eq!(refused["results"][0]["action"], "skipped");
    }

    #[tokio::test]
    async fn same_named_files_from_different_days_do_not_collide() {
        let (dir, imp) = vault();
        put(&dir, "_media/2026-09-19/MoMo Statement.docx", INVOICE);
        // Same name, different content: the reader goes by content, so this one reads as a sheet.
        put(&dir, "_media/2026-09-24/MoMo Statement.docx", TRACKER);
        let out = run(&imp, json!({"paths": ["_media/2026-09-19/MoMo Statement.docx", "_media/2026-09-24/MoMo Statement.docx"]})).await;
        assert_eq!(out["created"], 2, "{out}");
        let a = out["results"][0]["destination"].as_str().unwrap();
        let b = out["results"][1]["destination"].as_str().unwrap();
        assert_ne!(a, b);
    }

    #[tokio::test]
    async fn refuses_private_folders_markdown_traversal_and_missing_files() {
        let (dir, imp) = vault();
        put(&dir, "_system/secret.docx", INVOICE);
        put(&dir, "Notebooks/a.md", b"# a");
        for bad in [
            "_system/secret.docx",
            "Notebooks/a.md",
            "../outside.docx",
            "/etc/passwd",
            "Notebooks/nope.pdf",
        ] {
            let out = run(&imp, json!({"path": bad})).await;
            assert_eq!(out["results"][0]["action"], "failed", "{bad}: {out}");
        }
    }

    #[tokio::test]
    async fn archiving_moves_the_original_and_repoints_the_note() {
        let (dir, imp) = vault();
        put(&dir, "Example Deck.xlsx", TRACKER);
        put(&dir, "_media/keep.xlsx", TRACKER);
        let out = run(
            &imp,
            json!({"path": "Example Deck.xlsx", "archive_originals": true}),
        )
        .await;
        let r = &out["results"][0];
        assert_eq!(
            r["archived_to"], "_archive/originals/Example Deck.xlsx",
            "{r}"
        );
        assert!(!dir.path().join("Example Deck.xlsx").exists());
        assert!(
            dir.path()
                .join("_archive/originals/Example Deck.xlsx")
                .exists()
        );
        let note =
            std::fs::read_to_string(dir.path().join(r["destination"].as_str().unwrap())).unwrap();
        assert!(
            note.contains("source: _archive/originals/Example Deck.xlsx"),
            "{note}"
        );
        assert!(note.contains("original_location: Example Deck.xlsx"));

        let kept = run(
            &imp,
            json!({"path": "_media/keep.xlsx", "archive_originals": true}),
        )
        .await;
        assert!(
            kept["results"][0]["archive_skipped"]
                .as_str()
                .unwrap()
                .contains("_media")
        );
        assert!(dir.path().join("_media/keep.xlsx").exists());
    }

    #[tokio::test]
    async fn the_inventory_lists_pending_files_and_skips_system_folders_and_notes() {
        let (dir, imp) = vault();
        put(&dir, "_media/2026-09-22/a.docx", INVOICE);
        put(&dir, "Notebooks/Projects/b.xlsx", TRACKER);
        put(&dir, "page.html", PAGE);
        put(&dir, "_system/hidden.docx", INVOICE);
        put(&dir, "Exports/out.pdf", b"%PDF-1.4");
        put(&dir, ".git/x.docx", INVOICE);
        put(&dir, "Notebooks/note.md", b"# n");
        put(&dir, "Notebooks/data.json", b"{}");
        let inv = VaultInventoryTool::new(imp.clone())
            .execute(json!({}))
            .await
            .unwrap();
        let paths: Vec<&str> = inv["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["path"].as_str().unwrap())
            .collect();
        assert_eq!(
            paths,
            [
                "Notebooks/Projects/b.xlsx",
                "_media/2026-09-22/a.docx",
                "page.html"
            ],
            "{inv}"
        );
        assert_eq!(inv["by_status"]["pending"], 3);
        assert_eq!(inv["other_files_not_listed"], 1);

        run(&imp, json!({"path": "Notebooks/Projects/b.xlsx"})).await;
        let after = VaultInventoryTool::new(imp.clone())
            .execute(json!({"status": ["imported"]}))
            .await
            .unwrap();
        assert_eq!(after["entries"][0]["path"], "Notebooks/Projects/b.xlsx");
        assert_eq!(after["by_status"]["imported"], 1);
    }

    #[tokio::test]
    async fn a_directory_import_is_a_dry_run_by_default_and_batches() {
        let (dir, imp) = vault();
        for i in 0..4 {
            put(&dir, &format!("_media/2026-10-0{i}/doc{i}.docx"), INVOICE);
        }
        let plan = run(&imp, json!({"directory": "_media", "limit": 3})).await;
        assert_eq!(plan["dry_run"], true);
        assert_eq!(plan["would_create"], 3);
        assert_eq!(plan["more_pending"], 1);
        assert!(!dir.path().join("Notebooks/Imports").exists());

        let real = run(
            &imp,
            json!({"directory": "_media", "limit": 3, "dry_run": false}),
        )
        .await;
        assert_eq!(real["created"], 3, "{real}");
        let rest = run(&imp, json!({"directory": "_media", "dry_run": false})).await;
        assert_eq!(rest["created"], 1, "{rest}");
        assert_eq!(rest["more_pending"], 0);
    }

    #[tokio::test]
    async fn exactly_one_source_argument_is_required() {
        let (_dir, imp) = vault();
        let tool = VaultImportTool::new(imp);
        assert!(tool.execute(json!({})).await.is_err());
        assert!(
            tool.execute(json!({"path": "a.docx", "directory": "x"}))
                .await
                .is_err()
        );
    }

    #[test]
    fn note_names_are_readable_and_safe() {
        assert_eq!(clean_stem("MoMo Statement (1)"), "MoMo Statement (1)");
        assert_eq!(clean_stem("photo_6183_AQAD1Q9r"), "photo_6183_AQAD1Q9r");
        assert_eq!(clean_stem("a/b\\c:d*e?"), "a b c d e");
        assert_eq!(clean_stem("..."), "Imported document");
        assert!(clean_stem(&"x".repeat(300)).chars().count() <= 80);
    }
}
