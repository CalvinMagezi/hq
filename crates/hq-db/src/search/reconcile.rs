//! Keeps the derived search state (FTS rows, sync state, note cache, embeddings, link rows) in
//! step with the Markdown files on disk.
//!
//! The files are the source of truth. Every function here re-reads the file at the moment it
//! acts and works from what it finds, never from a payload handed to it. That makes each one
//! safe to repeat and to run in any order: a periodic scan and a filesystem event that arrive
//! for the same path converge on the same state.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

use anyhow::Result;
use rusqlite::Connection;

use super::fts::{extract_tags_from_frontmatter, index_note, strip_frontmatter};

/// The vault folder the index covers.
pub const NOTEBOOKS_DIR: &str = "Notebooks";

/// Marks the hash algorithm, so a future change is told apart from a content change.
const HASH_PREFIX: &str = "b3:";

/// Hash of a note's raw text, as stored in `sync_state.content_hash`.
///
/// BLAKE3 rather than `DefaultHasher`, whose output Rust does not promise to keep across
/// releases: a toolchain upgrade must not mark every note changed.
pub fn content_hash(raw: &str) -> String {
    format!("{HASH_PREFIX}{}", blake3::hash(raw.as_bytes()).to_hex())
}

/// What `reconcile_path` did for one path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathOutcome {
    /// The file was new or changed and has been (re)indexed.
    Indexed,
    /// The index already matched the file.
    Unchanged,
    /// The file is gone (or no longer a plain note) and its rows were removed.
    Removed,
    /// The path is outside what the index covers; nothing was touched.
    Ignored,
}

/// What one full pass of `sync_index` found.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SyncReport {
    pub indexed: usize,
    pub removed: usize,
    pub unchanged: usize,
    /// Files that exist but could not be read or indexed. Their rows were left as they were.
    pub failed: usize,
    /// Directories that could not be listed. Rows under them were not removed.
    pub unlisted_dirs: usize,
}

// ─── What the index covers ───────────────────────────────────────────────────

/// Directories the scan never enters: hidden ones (`.git`, `.obsidian`) and the trash.
fn is_excluded_dir(name: &str) -> bool {
    name.starts_with('.') || name == "_trash"
}

/// Plain notes only. Hidden files cover editor swap and lock files (`.foo.md.swp`, `.#foo.md`);
/// other editors' temp names do not end in `.md`.
fn is_indexable_file_name(name: &str) -> bool {
    !name.starts_with('.')
        && name != "_meta.md"
        && Path::new(name).extension().is_some_and(|e| e == "md")
}

/// The canonical, `/`-separated form of `rel` if the index covers it, else `None`.
///
/// Rejects anything that is not a plain relative path under `Notebooks/`: absolute paths,
/// `..`, excluded directories, unsupported file names.
pub fn indexable_rel_path(rel: &str) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    for component in Path::new(rel).components() {
        match component {
            Component::Normal(p) => parts.push(p.to_str()?),
            Component::CurDir => {}
            _ => return None,
        }
    }
    let (file, dirs) = parts.split_last()?;
    let (root, inner) = dirs.split_first()?;
    if *root != NOTEBOOKS_DIR
        || inner.iter().any(|d| is_excluded_dir(d))
        || !is_indexable_file_name(file)
    {
        return None;
    }
    Some(parts.join("/"))
}

/// The canonical, `/`-separated form of a folder `rel` if the index covers it, else `None`.
/// `Notebooks` itself is covered.
pub fn indexable_dir_path(rel: &str) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    for component in Path::new(rel).components() {
        match component {
            Component::Normal(p) => parts.push(p.to_str()?),
            Component::CurDir => {}
            _ => return None,
        }
    }
    let (root, inner) = parts.split_first()?;
    if *root != NOTEBOOKS_DIR || inner.iter().any(|d| is_excluded_dir(d)) {
        return None;
    }
    Some(parts.join("/"))
}

/// Whether a directory below `Notebooks/` on the way to `rel` is a symlink. The scan never
/// follows links, so a path reached through one must not be indexed by a single-path call either.
fn crosses_symlink(vault_path: &Path, rel: &str) -> bool {
    let mut path = vault_path.join(NOTEBOOKS_DIR);
    for part in rel.split('/').skip(1) {
        path.push(part);
        match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_symlink() => return true,
            Ok(_) => {}
            Err(_) => return false,
        }
    }
    false
}

fn rel_string(path: &Path, vault_path: &Path) -> String {
    path.strip_prefix(vault_path)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

// ─── Walking the vault ───────────────────────────────────────────────────────

struct Walk {
    /// Absolute path and vault-relative path of every indexable note, sorted by the latter.
    files: Vec<(PathBuf, String)>,
    /// Vault-relative directories that could not be fully listed.
    unlisted: Vec<String>,
}

/// Collect the notes under `notebooks_dir`. Symlinks are skipped (files and directories), so a
/// link can neither escape the vault nor loop; hidden entries and the trash are skipped.
fn walk_markdown(notebooks_dir: &Path, vault_path: &Path) -> Walk {
    let mut walk = Walk {
        files: Vec::new(),
        unlisted: Vec::new(),
    };
    let mut stack = vec![notebooks_dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) => {
                tracing::warn!(dir = %dir.display(), error = %e, "vault scan: cannot list directory");
                walk.unlisted.push(rel_string(&dir, vault_path));
                continue;
            }
        };
        let mut complete = true;
        for entry in entries {
            let Ok(entry) = entry else {
                complete = false;
                continue;
            };
            let Ok(file_type) = entry.file_type() else {
                complete = false;
                continue;
            };
            if file_type.is_symlink() {
                continue;
            }
            let file_name = entry.file_name();
            let Some(name) = file_name.to_str() else {
                continue;
            };
            if file_type.is_dir() {
                if !is_excluded_dir(name) {
                    stack.push(entry.path());
                }
            } else if file_type.is_file() && is_indexable_file_name(name) {
                let path = entry.path();
                let rel = rel_string(&path, vault_path);
                walk.files.push((path, rel));
            }
        }
        if !complete {
            walk.unlisted.push(rel_string(&dir, vault_path));
        }
    }
    walk.files.sort_by(|a, b| a.1.cmp(&b.1));
    walk
}

impl Walk {
    fn empty() -> Self {
        Self {
            files: Vec::new(),
            unlisted: Vec::new(),
        }
    }
}

/// Walk one folder below `Notebooks/`. A folder that is gone, was replaced by a file or a link,
/// or is reached through a link counts as empty; one that cannot be inspected counts as unlisted.
fn walk_subtree(vault_path: &Path, rel_dir: &str) -> Walk {
    if crosses_symlink(vault_path, rel_dir) {
        return Walk::empty();
    }
    let dir = vault_path.join(rel_dir);
    match std::fs::symlink_metadata(&dir) {
        Ok(meta) if meta.file_type().is_dir() => walk_markdown(&dir, vault_path),
        Ok(_) => Walk::empty(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Walk::empty(),
        Err(e) => {
            tracing::warn!(dir = %dir.display(), error = %e, "vault scan: cannot inspect directory");
            Walk {
                files: Vec::new(),
                unlisted: vec![rel_dir.to_string()],
            }
        }
    }
}

/// Every file `rebuild_index` and `sync_index` would index, as `(absolute, vault-relative)`.
pub(super) fn indexable_files(notebooks_dir: &Path) -> Vec<(PathBuf, String)> {
    let vault_path = notebooks_dir.parent().unwrap_or(notebooks_dir);
    walk_markdown(notebooks_dir, vault_path).files
}

fn under_unlisted_dir(path: &str, unlisted: &[String]) -> bool {
    unlisted.iter().any(|dir| {
        path.strip_prefix(dir.as_str())
            .is_some_and(|rest| rest.starts_with('/'))
    })
}

// ─── Removing a path ─────────────────────────────────────────────────────────

fn in_savepoint<T>(conn: &Connection, work: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
    conn.execute_batch("SAVEPOINT vault_reconcile")?;
    match work(conn) {
        Ok(value) => {
            conn.execute_batch("RELEASE vault_reconcile")?;
            Ok(value)
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK TO vault_reconcile; RELEASE vault_reconcile");
            Err(e)
        }
    }
}

/// Delete everything derived from a note's text. Returns how many rows went.
pub(super) fn delete_content_rows(conn: &Connection, path: &str) -> Result<usize> {
    in_savepoint(conn, |conn| {
        let mut removed = 0;
        removed += conn.execute("DELETE FROM notes_fts WHERE path = ?1", [path])?;
        removed += conn.execute("DELETE FROM embeddings WHERE note_path = ?1", [path])?;
        removed += conn.execute("DELETE FROM sync_state WHERE path = ?1", [path])?;
        removed += conn.execute("DELETE FROM vault_cache WHERE path = ?1", [path])?;
        Ok(removed)
    })
}

/// Remove every trace of a deleted note: its search state and its link rows.
///
/// For a note that is gone. A move uses `remove_note` instead, which leaves link rows alone.
pub fn remove_path_everywhere(conn: &Connection, path: &str) -> Result<()> {
    in_savepoint(conn, |conn| {
        delete_content_rows(conn, path)?;
        conn.execute(
            "DELETE FROM graph_links WHERE source_path = ?1 OR target_path = ?1",
            [path],
        )?;
        Ok(())
    })
}

// ─── Reconciling one file ────────────────────────────────────────────────────

fn known_hash(conn: &Connection, rel: &str) -> Result<Option<String>> {
    use rusqlite::OptionalExtension;
    Ok(conn
        .query_row(
            "SELECT content_hash FROM sync_state WHERE path = ?1",
            [rel],
            |row| row.get(0),
        )
        .optional()?)
}

fn file_mtime_secs(path: &Path) -> Option<i64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    let since_epoch = modified.duration_since(std::time::UNIX_EPOCH).ok()?;
    i64::try_from(since_epoch.as_secs()).ok()
}

/// Bring the index in step with one file whose path is already known to be indexable.
///
/// `known` is the hash recorded in `sync_state`, passed in so a full scan loads it once.
fn reconcile_file(
    conn: &Connection,
    abs: &Path,
    rel: &str,
    known: Option<&str>,
) -> Result<PathOutcome> {
    let raw = match std::fs::read_to_string(abs) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let removed = delete_content_rows(conn, rel)?;
            return Ok(if removed > 0 {
                PathOutcome::Removed
            } else {
                PathOutcome::Unchanged
            });
        }
        // The file exists but cannot be read (permissions, not valid UTF-8, mid-write). Keep
        // what the index has rather than treating a read failure as a deletion.
        Err(e) => return Err(e.into()),
    };
    let hash = content_hash(&raw);
    if known == Some(hash.as_str()) {
        return Ok(PathOutcome::Unchanged);
    }

    let title = abs
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    let content = strip_frontmatter(&raw);
    let tags = extract_tags_from_frontmatter(&raw);
    let mtime = file_mtime_secs(abs);
    let now = chrono::Utc::now().timestamp();
    in_savepoint(conn, |conn| {
        index_note(conn, rel, &title, &content, &tags)?;
        conn.execute(
            "INSERT INTO sync_state (path, content_hash, modified_at, synced_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(path) DO UPDATE SET
               content_hash = excluded.content_hash,
               modified_at = excluded.modified_at,
               synced_at = excluded.synced_at",
            rusqlite::params![rel, hash, mtime.unwrap_or(now), now],
        )?;
        Ok(())
    })?;

    // The cache only saves re-reading the file later, and carries the file's own mtime so
    // "recent notes" orders by when the note changed, not by when it was indexed.
    let token_count = ((raw.len() * 2).div_ceil(7)) as i64;
    if let Err(e) = crate::vault_cache::write_note_cached_at(
        conn,
        rel,
        &raw,
        &title,
        None,
        Some(token_count),
        mtime,
    ) {
        tracing::debug!(path = %rel, error = %e, "vault_cache write failed during sync");
    }
    Ok(PathOutcome::Indexed)
}

/// Bring the index in step with one path under `vault_path`.
///
/// This is what a filesystem event should call: it ignores any state the event carried and
/// acts on the file as it is now. A missing file, a directory, or a path reached through a
/// symlink has its rows removed; a path the index does not cover is left alone.
pub fn reconcile_path(conn: &Connection, vault_path: &Path, rel: &str) -> Result<PathOutcome> {
    let Some(rel) = indexable_rel_path(rel) else {
        return Ok(PathOutcome::Ignored);
    };
    let abs = vault_path.join(&rel);
    let is_plain_file = !crosses_symlink(vault_path, &rel)
        && std::fs::symlink_metadata(&abs).is_ok_and(|meta| meta.file_type().is_file());
    if !is_plain_file {
        let removed = delete_content_rows(conn, &rel)?;
        return Ok(if removed > 0 {
            PathOutcome::Removed
        } else {
            PathOutcome::Unchanged
        });
    }
    let known = known_hash(conn, &rel)?;
    reconcile_file(conn, &abs, &rel, known.as_deref())
}

// ─── Reconciling the whole vault ─────────────────────────────────────────────

/// `path >= lower AND path < upper` selects exactly the paths that start with `prefix` (which
/// ends in `/`), and uses the primary key index: `0` is the character after `/`.
fn prefix_range(prefix: &str) -> (String, String) {
    let upper = format!("{}0", prefix.trim_end_matches('/'));
    (prefix.to_string(), upper)
}

fn load_known_hashes(conn: &Connection, prefix: &str) -> Result<HashMap<String, String>> {
    let (lower, upper) = prefix_range(prefix);
    let mut stmt =
        conn.prepare("SELECT path, content_hash FROM sync_state WHERE path >= ?1 AND path < ?2")?;
    let rows = stmt.query_map([&lower, &upper], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Every path under `prefix` that any derived table still holds. A note written through a tool
/// is in the FTS table before the scan has recorded it in `sync_state`, so `sync_state` alone
/// would miss it if the file were deleted in between.
fn load_indexed_paths(conn: &Connection, prefix: &str) -> Result<BTreeSet<String>> {
    let (lower, upper) = prefix_range(prefix);
    let mut paths = BTreeSet::new();
    for sql in [
        "SELECT path FROM sync_state WHERE path >= ?1 AND path < ?2",
        "SELECT path FROM notes_fts WHERE path >= ?1 AND path < ?2",
        "SELECT note_path FROM embeddings WHERE note_path >= ?1 AND note_path < ?2",
        "SELECT path FROM vault_cache WHERE path >= ?1 AND path < ?2",
    ] {
        let mut stmt = conn.prepare(sql)?;
        for path in stmt.query_map([&lower, &upper], |r| r.get::<_, String>(0))? {
            paths.insert(path?);
        }
    }
    Ok(paths)
}

/// Scan `notebooks_dir` and bring the index in step with it: index new and changed notes
/// (by content hash), and remove everything held for notes that are gone.
///
/// A directory that cannot be listed, or a file that cannot be read, never causes rows to be
/// removed: only a path the scan can positively see is missing is treated as deleted.
pub fn sync_index(conn: &Connection, notebooks_dir: &Path) -> Result<SyncReport> {
    sync_tree(conn, notebooks_dir, None)
}

/// Like `sync_index`, for the folder `rel_dir` (such as `Notebooks/Projects`) only.
///
/// What a folder-level filesystem event needs: moving or deleting a folder reports the folder,
/// not the notes inside it. A folder that no longer exists has every row under it removed.
pub fn sync_subtree(conn: &Connection, vault_path: &Path, rel_dir: &str) -> Result<SyncReport> {
    let Some(rel_dir) = indexable_dir_path(rel_dir) else {
        return Ok(SyncReport::default());
    };
    let notebooks_dir = vault_path.join(NOTEBOOKS_DIR);
    if rel_dir == NOTEBOOKS_DIR {
        return sync_tree(conn, &notebooks_dir, None);
    }
    sync_tree(conn, &notebooks_dir, Some(&rel_dir))
}

fn sync_tree(conn: &Connection, notebooks_dir: &Path, subtree: Option<&str>) -> Result<SyncReport> {
    let mut report = SyncReport::default();
    // An absent Notebooks/ is a missing mount as likely as an empty vault: remove nothing.
    if !notebooks_dir.exists() {
        return Ok(report);
    }
    let vault_path = notebooks_dir.parent().unwrap_or(notebooks_dir);
    let (walk, prefix) = match subtree {
        None => (
            walk_markdown(notebooks_dir, vault_path),
            format!("{NOTEBOOKS_DIR}/"),
        ),
        Some(rel_dir) => (walk_subtree(vault_path, rel_dir), format!("{rel_dir}/")),
    };
    let known = load_known_hashes(conn, &prefix)?;
    report.unlisted_dirs = walk.unlisted.len();

    let mut seen: HashSet<&str> = HashSet::with_capacity(walk.files.len());
    for (abs, rel) in &walk.files {
        seen.insert(rel);
        match reconcile_file(conn, abs, rel, known.get(rel).map(String::as_str)) {
            Ok(PathOutcome::Indexed) => report.indexed += 1,
            Ok(PathOutcome::Removed) => report.removed += 1,
            Ok(_) => report.unchanged += 1,
            Err(e) => {
                tracing::warn!(path = %rel, error = %e, "vault sync: could not index note");
                report.failed += 1;
            }
        }
    }

    for path in load_indexed_paths(conn, &prefix)? {
        if seen.contains(path.as_str()) || under_unlisted_dir(&path, &walk.unlisted) {
            continue;
        }
        match remove_path_everywhere(conn, &path) {
            Ok(()) => report.removed += 1,
            Err(e) => {
                tracing::warn!(path = %path, error = %e, "vault sync: could not remove note");
                report.failed += 1;
            }
        }
    }
    Ok(report)
}

/// How current the derived search state is, each index counted on its own. The keyword index can
/// be fully current while embeddings lag, and a health report must not call the vault current
/// because only one of them is.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct IndexFreshness {
    /// Notes in the keyword index.
    pub keyword_indexed: usize,
    /// Notes with an embedding.
    pub embedded: usize,
    /// Indexed notes that have no embedding yet.
    pub awaiting_embedding: usize,
    /// Embeddings whose note is no longer in the keyword index.
    pub orphan_embeddings: usize,
    /// Embeddings made from an older version of their note.
    pub stale_embeddings: usize,
    /// When the last note was recorded in `sync_state` (seconds since the Unix epoch), if ever.
    pub last_synced_at: Option<i64>,
}

pub fn index_freshness(conn: &Connection) -> Result<IndexFreshness> {
    let count = |sql: &str| -> Result<usize> { Ok(conn.query_row(sql, [], |r| r.get(0))?) };
    Ok(IndexFreshness {
        keyword_indexed: count("SELECT COUNT(*) FROM notes_fts")?,
        embedded: count("SELECT COUNT(*) FROM embeddings")?,
        awaiting_embedding: count(
            "SELECT COUNT(*) FROM notes_fts WHERE path NOT IN (SELECT note_path FROM embeddings)",
        )?,
        orphan_embeddings: count(
            "SELECT COUNT(*) FROM embeddings WHERE note_path NOT IN (SELECT path FROM notes_fts)",
        )?,
        stale_embeddings: super::semantic::stale_embedding_count(conn)?,
        last_synced_at: conn
            .query_row("SELECT MAX(synced_at) FROM sync_state", [], |r| r.get(0))?,
    })
}

/// `sync_index` as `(indexed, removed, unchanged)`.
pub fn sync_index_incremental(
    conn: &Connection,
    notebooks_dir: &Path,
) -> Result<(usize, usize, usize)> {
    let report = sync_index(conn, notebooks_dir)?;
    Ok((report.indexed, report.removed, report.unchanged))
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn hash_is_stable_and_tagged() {
        assert_eq!(
            content_hash("hello"),
            "b3:ea8f163db38682925e4491c5e58d4bb3506ef8c14eb78a86e908c5624a67200f"
        );
        assert_ne!(content_hash("a"), content_hash("b"));
    }

    #[test]
    fn rel_paths_must_be_plain_notes_under_notebooks() {
        let ok = |p: &str| indexable_rel_path(p);
        assert_eq!(ok("Notebooks/a.md").as_deref(), Some("Notebooks/a.md"));
        assert_eq!(
            ok("Notebooks/Projects/x/y.md").as_deref(),
            Some("Notebooks/Projects/x/y.md")
        );
        assert_eq!(ok("./Notebooks/a.md").as_deref(), Some("Notebooks/a.md"));
        for bad in [
            "a.md",
            "Notebooks",
            "Notebooks/a.txt",
            "Notebooks/a",
            "Notebooks/_meta.md",
            "Notebooks/.hidden.md",
            "Notebooks/.git/a.md",
            "Notebooks/_trash/a.md",
            "Notebooks/../_system/a.md",
            "/etc/Notebooks/a.md",
            "_system/Notebooks/a.md",
            "Other/a.md",
            "",
        ] {
            assert_eq!(ok(bad), None, "{bad}");
        }
    }

    #[test]
    fn folder_paths_must_be_under_notebooks_and_not_excluded() {
        let ok = |p: &str| indexable_dir_path(p);
        assert_eq!(ok("Notebooks").as_deref(), Some("Notebooks"));
        assert_eq!(ok("Notebooks/A/b").as_deref(), Some("Notebooks/A/b"));
        for bad in [
            "",
            "Other",
            "Notebooks/.git",
            "Notebooks/A/_trash",
            "Notebooks/../x",
            "/Notebooks",
        ] {
            assert_eq!(ok(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_prefix_selects_exactly_the_paths_that_start_with_it() {
        assert_eq!(
            prefix_range("Notebooks/A/"),
            ("Notebooks/A/".to_string(), "Notebooks/A0".to_string())
        );
        let (lower, upper) = prefix_range("Notebooks/A/");
        for inside in ["Notebooks/A/x.md", "Notebooks/A/z/y.md"] {
            assert!(
                inside >= lower.as_str() && inside < upper.as_str(),
                "{inside}"
            );
        }
        for outside in [
            "Notebooks/A.md",
            "Notebooks/A2/x.md",
            "Notebooks/B/x.md",
            "Notebooks/",
        ] {
            assert!(
                !(outside >= lower.as_str() && outside < upper.as_str()),
                "{outside}"
            );
        }
    }

    #[test]
    fn unlisted_directories_protect_only_their_own_subtree() {
        let unlisted = vec!["Notebooks/Projects".to_string()];
        assert!(under_unlisted_dir("Notebooks/Projects/a.md", &unlisted));
        assert!(under_unlisted_dir("Notebooks/Projects/x/a.md", &unlisted));
        assert!(!under_unlisted_dir("Notebooks/Projects2/a.md", &unlisted));
        assert!(!under_unlisted_dir("Notebooks/a.md", &unlisted));
        assert!(under_unlisted_dir(
            "Notebooks/a.md",
            &["Notebooks".to_string()]
        ));
    }
}
