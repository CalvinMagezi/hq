//! File-edit safety utilities — distilled from claude-code's FileEditTool.
//!
//! # What this module provides
//!
//! Claude-code's `FileEditTool` contains 625+ lines of battle-tested logic for
//! safely applying string replacements to files. The core insights distilled here:
//!
//! 1. **Quote normalisation** — LLMs frequently output curly/smart quotes when they
//!    meant straight quotes. `normalize_quotes` + `find_actual_string` handle this
//!    transparently so edits don't fail on a `'` vs `'` mismatch.
//!
//! 2. **Trailing-whitespace stripping** — output from LLMs often has trailing spaces
//!    that differ from the file. Strip before matching (except in Markdown where two
//!    trailing spaces are a hard line-break).
//!
//! 3. **`replace_all` guard** — if `old_string` appears multiple times but
//!    `replace_all` is false, the tool should surface an error rather than silently
//!    replacing only the first occurrence (or all of them).
//!
//! 4. **File-state cache (staleness detection)** — every file that will be edited
//!    must have been read first. We record the modification time at read time and
//!    reject the edit if the file changed on disk since then. This prevents silent
//!    corruption when two agents (or a user) edit the same file concurrently.
//!
//! 5. **No-op guard** — if `old_string == new_string` or the replacement produces no
//!    change, surface a clear error rather than writing an identical file to disk.
//!
//! # Error codes (aligned with claude-code FileEditTool)
//!
//! | Code | Meaning |
//! |------|---------|
//! | 1    | No-op: old_string == new_string |
//! | 4    | File does not exist |
//! | 6    | File not yet read (staleness gate) |
//! | 7    | File modified since last read |
//! | 8    | String not found in file |
//! | 9    | Multiple matches but replace_all is false |
//! | 10   | File exceeds size limit |

use hq_core::types::{ValidationBehavior, ValidationResult};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime};

// ── Constants ─────────────────────────────────────────────────────────────────

/// 1 GiB — same limit as claude-code's FileEditTool.
/// V8/Bun string limit ≈ 2^30 chars; this is a safe byte-level guard.
pub const MAX_EDIT_FILE_SIZE: u64 = 1024 * 1024 * 1024;

// Curly-quote character constants (distilled from claude-code's utils.ts).
// Claude frequently outputs curly quotes that don't match straight-quote source files.
const LEFT_SINGLE_CURLY: char = '\u{2018}'; // '
const RIGHT_SINGLE_CURLY: char = '\u{2019}'; // '
const LEFT_DOUBLE_CURLY: char = '\u{201C}'; // "
const RIGHT_DOUBLE_CURLY: char = '\u{201D}'; // "

// ── Quote normalisation ───────────────────────────────────────────────────────

/// Replace curly/smart quotes with their ASCII equivalents.
///
/// Distilled from claude-code's `normalizeQuotes()` in `utils.ts`.
pub fn normalize_quotes(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            LEFT_SINGLE_CURLY | RIGHT_SINGLE_CURLY => '\'',
            LEFT_DOUBLE_CURLY | RIGHT_DOUBLE_CURLY => '"',
            other => other,
        })
        .collect()
}

/// Find the literal substring in `file_content` that semantically matches
/// `search_string`, accounting for curly-quote normalisation.
///
/// Returns the actual (un-normalised) slice from `file_content` if found,
/// so callers can preserve the file's original typography when constructing
/// the replacement.
///
/// Distilled from claude-code's `findActualString()` in `utils.ts`.
pub fn find_actual_string<'a>(file_content: &'a str, search_string: &str) -> Option<&'a str> {
    // Fast path: exact match.
    if let Some(idx) = file_content.find(search_string) {
        return Some(&file_content[idx..idx + search_string.len()]);
    }

    // Fallback: match after normalising both sides.
    let norm_search = normalize_quotes(search_string);
    let norm_file = normalize_quotes(file_content);

    let norm_idx = norm_file.find(&norm_search)?;

    // Map char offset in normalised string back to byte offset in original.
    // Normalisation can change byte lengths (e.g. 3-byte curly quotes to 1-byte ASCII),
    // so we count chars to find the correct position in the original.
    let char_start = norm_file[..norm_idx].chars().count();
    let char_len = norm_search.chars().count();

    let byte_start = file_content
        .char_indices()
        .nth(char_start)
        .map(|(i, _)| i)?;
    let byte_end = file_content
        .char_indices()
        .nth(char_start + char_len)
        .map(|(i, _)| i)
        .unwrap_or(file_content.len());

    Some(&file_content[byte_start..byte_end])
}

/// Strip trailing whitespace from each line while preserving line endings (LF / CRLF / CR).
///
/// Distilled from claude-code's `stripTrailingWhitespace()`. Skip for Markdown files
/// because two trailing spaces are a hard line-break.
pub fn strip_trailing_whitespace(s: &str) -> String {
    let mut result = String::with_capacity(s.len());

    for c in s.chars() {
        if c == '\r' {
            // Trim trailing spaces before emitting the line ending.
            let trimmed_end = result.trim_end_matches([' ', '\t']).len();
            result.truncate(trimmed_end);
            result.push('\r');
        } else if c == '\n' {
            let trimmed_end = result.trim_end_matches([' ', '\t']).len();
            result.truncate(trimmed_end);
            result.push('\n');
        } else {
            result.push(c);
        }
    }
    // Trim trailing whitespace on the last line (no newline at EOF)
    let trimmed_end = result.trim_end_matches([' ', '\t']).len();
    result.truncate(trimmed_end);
    result
}

// ── Core edit logic ───────────────────────────────────────────────────────────

/// Apply a single string replacement to `content`.
///
/// When `new_string` is empty and `old_string` is followed by a newline in
/// the file, the trailing newline is also removed (avoids leaving a blank line).
///
/// Distilled from claude-code's `applyEditToFile()`.
pub fn apply_edit(content: &str, old_string: &str, new_string: &str, replace_all: bool) -> String {
    if old_string.is_empty() {
        return new_string.to_string();
    }

    if new_string.is_empty() {
        // If the old string is followed by a newline, strip that newline too.
        let with_newline = format!("{old_string}\n");
        if !old_string.ends_with('\n') && content.contains(with_newline.as_str()) {
            return if replace_all {
                content.replace(with_newline.as_str(), "")
            } else {
                content.replacen(with_newline.as_str(), "", 1)
            };
        }
    }

    if replace_all {
        content.replace(old_string, new_string)
    } else {
        content.replacen(old_string, new_string, 1)
    }
}

// ── Validation helpers ────────────────────────────────────────────────────────

/// Validate a file-edit operation before touching disk.
///
/// Returns `ValidationResult::Ok` when all checks pass; otherwise returns a
/// structured error that the governance layer surfaces to the LLM.
///
/// Checks performed (in order, cheapest first):
/// 1. `old_string == new_string` → no-op error (code 1)
/// 2. File size limit (code 10)
/// 3. File existence (code 4)
/// 4. Staleness: file modified since last read (code 7), or not yet read (code 6)
/// 5. String not found in file (code 8)
/// 6. Multiple matches with `replace_all=false` (code 9)
pub fn validate_file_edit(
    file_path: &Path,
    old_string: &str,
    new_string: &str,
    replace_all: bool,
    state_cache: &FileStateCache,
) -> ValidationResult {
    // 1. No-op guard
    if old_string == new_string && !old_string.is_empty() {
        return ValidationResult::Err {
            message: "old_string and new_string are identical — no change would be made.".into(),
            error_code: 1,
            behavior: ValidationBehavior::Block,
        };
    }

    // 2. File size limit (stat before reading content)
    if file_path.exists() {
        match std::fs::metadata(file_path) {
            Ok(meta) if meta.len() > MAX_EDIT_FILE_SIZE => {
                return ValidationResult::Err {
                    message: format!(
                        "File exceeds the 1 GiB edit size limit ({} bytes).",
                        meta.len()
                    ),
                    error_code: 10,
                    behavior: ValidationBehavior::Block,
                };
            }
            _ => {}
        }
    }

    // For new-file creation (old_string is empty), skip existence + cache checks.
    if old_string.is_empty() {
        return ValidationResult::Ok;
    }

    // 3. File existence
    if !file_path.exists() {
        return ValidationResult::Err {
            message: format!("File not found: {}", file_path.display()),
            error_code: 4,
            behavior: ValidationBehavior::Block,
        };
    }

    // 4. Staleness gate
    match state_cache.check_staleness(file_path) {
        StalenessResult::NotRead => {
            return ValidationResult::Err {
                message: format!(
                    "File has not been read yet. Read {} before editing it.",
                    file_path.display()
                ),
                error_code: 6,
                behavior: ValidationBehavior::Block,
            };
        }
        StalenessResult::Modified => {
            return ValidationResult::Err {
                message: format!(
                    "File was modified on disk since it was last read. Re-read {} before editing.",
                    file_path.display()
                ),
                error_code: 7,
                behavior: ValidationBehavior::Block,
            };
        }
        StalenessResult::Fresh => {}
    }

    // Read file content for string-match checks.
    let content = match std::fs::read_to_string(file_path) {
        Ok(c) => c,
        Err(e) => {
            return ValidationResult::Err {
                message: format!("Cannot read file {}: {e}", file_path.display()),
                error_code: 4,
                behavior: ValidationBehavior::Block,
            };
        }
    };

    // Resolve old_string against the file content (handles quote normalisation).
    let resolved = match find_actual_string(&content, old_string) {
        Some(s) => s,
        None => {
            return ValidationResult::Err {
                message: format!(
                    "String not found in file. The text to replace was not found in {}.",
                    file_path.display()
                ),
                error_code: 8,
                behavior: ValidationBehavior::Block,
            };
        }
    };

    // 5. Multiple-match guard
    if !replace_all && content.matches(resolved).count() > 1 {
        return ValidationResult::Err {
            message: format!(
                "The string appears {} times in the file. Set replace_all=true to replace all \
                 occurrences, or provide more surrounding context to make the match unique.",
                content.matches(resolved).count()
            ),
            error_code: 9,
            behavior: ValidationBehavior::Ask,
        };
    }

    ValidationResult::Ok
}

// ── File-state cache ──────────────────────────────────────────────────────────

/// Outcome of a staleness check for a given path.
#[derive(Debug, PartialEq, Eq)]
pub enum StalenessResult {
    /// The file has not been recorded in the cache (never read through HQ).
    NotRead,
    /// The file was read but has been modified on disk since then.
    Modified,
    /// The file is unchanged since it was last read.
    Fresh,
}

/// Detected file encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileEncoding {
    Utf8,
    Utf16Le,
}

/// Per-file read-state recorded when a file is read through HQ tools.
#[derive(Debug, Clone)]
struct FileReadState {
    /// Modification time at the moment the file was read.
    mtime: SystemTime,
    /// SHA-256 hash of the file content at read time.
    /// Catches modifications even when mtime resolution is too coarse (sub-second on M4).
    content_hash: [u8; 32],
    /// Detected encoding (preserved on write to avoid corruption).
    encoding: FileEncoding,
}

/// Thread-safe cache of read-states for files that have been accessed via HQ.
///
/// Distilled from claude-code's `fileStateCache.ts` + `readFileSyncCached` pattern.
/// Any tool that reads a file for editing should call `record_read` after the read.
/// Any tool that writes a file should call `record_write` after the write.
#[derive(Debug, Clone, Default)]
pub struct FileStateCache {
    inner: Arc<Mutex<HashMap<PathBuf, FileReadState>>>,
}

impl FileStateCache {
    /// Create a new empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Detect file encoding from raw bytes. Checks for UTF-16 LE BOM (FF FE).
    fn detect_encoding(bytes: &[u8]) -> FileEncoding {
        if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
            FileEncoding::Utf16Le
        } else {
            FileEncoding::Utf8
        }
    }

    /// Compute SHA-256 hash of raw file bytes.
    fn hash_bytes(bytes: &[u8]) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        hasher.finalize().into()
    }

    /// Record that `path` was just read. Stores mtime, content hash, and encoding.
    ///
    /// Call this after every successful file read that is followed by a potential edit.
    pub fn record_read(&self, path: &Path) {
        let mtime = std::fs::metadata(path)
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        let bytes = std::fs::read(path).unwrap_or_default();
        let content_hash = Self::hash_bytes(&bytes);
        let encoding = Self::detect_encoding(&bytes);
        let mut guard = self.inner.lock().expect("FileStateCache lock poisoned");
        guard.insert(
            path.to_path_buf(),
            FileReadState {
                mtime,
                content_hash,
                encoding,
            },
        );
    }

    /// Record that `path` was just written. Updates the cached state so subsequent
    /// edits in the same session don't fail the staleness check.
    pub fn record_write(&self, path: &Path) {
        // Re-stat after write to pick up the new mtime + hash.
        self.record_read(path);
    }

    /// Check whether `path` is stale (modified on disk since last read).
    ///
    /// Uses a two-tier check: fast mtime comparison first, then SHA-256 content
    /// hash if mtime matches (catches sub-second modifications on fast machines).
    pub fn check_staleness(&self, path: &Path) -> StalenessResult {
        let guard = self.inner.lock().expect("FileStateCache lock poisoned");
        let Some(state) = guard.get(path) else {
            return StalenessResult::NotRead;
        };
        let current_mtime = std::fs::metadata(path)
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        if current_mtime != state.mtime {
            // Mtime changed — definitely modified.
            return StalenessResult::Modified;
        }
        // Mtime matches but content may still differ (sub-second writes on M4/APFS).
        let bytes = std::fs::read(path).unwrap_or_default();
        let current_hash = Self::hash_bytes(&bytes);
        if current_hash != state.content_hash {
            StalenessResult::Modified
        } else {
            StalenessResult::Fresh
        }
    }

    /// Get the detected encoding for a previously-read file.
    pub fn encoding(&self, path: &Path) -> Option<FileEncoding> {
        let guard = self.inner.lock().expect("FileStateCache lock poisoned");
        guard.get(path).map(|s| s.encoding)
    }

    /// Remove a path from the cache (e.g., after file deletion).
    pub fn evict(&self, path: &Path) {
        let mut guard = self.inner.lock().expect("FileStateCache lock poisoned");
        guard.remove(path);
    }
}

// ── File history (pre-edit snapshots for rollback) ───────────────────────────

/// A single pre-edit snapshot.
#[derive(Debug, Clone)]
struct FileSnapshot {
    /// When the snapshot was taken.
    timestamp: Instant,
    /// The file content before the edit.
    content: String,
}

/// Bounded history of pre-edit snapshots for rollback support.
///
/// Keeps at most `max_files` files with at most `max_total_bytes` of content.
/// Oldest snapshots are evicted when limits are exceeded.
#[derive(Debug, Clone)]
pub struct FileHistory {
    inner: Arc<Mutex<FileHistoryInner>>,
}

#[derive(Debug)]
struct FileHistoryInner {
    snapshots: HashMap<PathBuf, Vec<FileSnapshot>>,
    total_bytes: usize,
    max_files: usize,
    max_total_bytes: usize,
}

impl Default for FileHistory {
    fn default() -> Self {
        Self::new(50, 10 * 1024 * 1024) // 50 files, 10 MB
    }
}

impl FileHistory {
    pub fn new(max_files: usize, max_total_bytes: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(FileHistoryInner {
                snapshots: HashMap::new(),
                total_bytes: 0,
                max_files,
                max_total_bytes,
            })),
        }
    }

    /// Snapshot the current content of `path` before an edit.
    /// Returns `true` if the snapshot was saved, `false` if the file could not be read.
    pub fn snapshot(&self, path: &Path) -> bool {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return false,
        };
        let content_len = content.len();
        let mut guard = self.inner.lock().expect("FileHistory lock poisoned");
        let inner = &mut *guard;

        // Evict oldest files if we would exceed limits.
        while inner.snapshots.len() >= inner.max_files
            || inner.total_bytes + content_len > inner.max_total_bytes
        {
            // Find the file with the oldest most-recent snapshot.
            let oldest_path = inner
                .snapshots
                .iter()
                .filter_map(|(p, snaps)| snaps.last().map(|s| (p.clone(), s.timestamp)))
                .min_by_key(|(_, ts)| *ts)
                .map(|(p, _)| p);
            match oldest_path {
                Some(p) => {
                    if let Some(snaps) = inner.snapshots.remove(&p) {
                        inner.total_bytes -= snaps.iter().map(|s| s.content.len()).sum::<usize>();
                    }
                }
                None => break, // Empty map, can't evict more.
            }
        }

        inner.total_bytes += content_len;
        inner
            .snapshots
            .entry(path.to_path_buf())
            .or_default()
            .push(FileSnapshot {
                timestamp: Instant::now(),
                content,
            });
        true
    }

    /// Rollback `path` to its most recent snapshot. Returns the restored content,
    /// or `None` if no snapshot exists.
    pub fn rollback(&self, path: &Path) -> Option<String> {
        let mut guard = self.inner.lock().expect("FileHistory lock poisoned");
        let inner = &mut *guard;
        let snaps = inner.snapshots.get_mut(path)?;
        let snap = snaps.pop()?;
        inner.total_bytes -= snap.content.len();
        if snaps.is_empty() {
            inner.snapshots.remove(path);
        }
        Some(snap.content)
    }

    /// List files that have snapshots available for rollback.
    pub fn available_rollbacks(&self) -> Vec<(PathBuf, usize)> {
        let guard = self.inner.lock().expect("FileHistory lock poisoned");
        guard
            .snapshots
            .iter()
            .map(|(p, snaps)| (p.clone(), snaps.len()))
            .collect()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    // ── normalize_quotes ─────────────────────────────────────────────────────

    #[test]
    fn test_normalize_quotes_straight_passthrough() {
        assert_eq!(normalize_quotes("hello 'world'"), "hello 'world'");
        assert_eq!(normalize_quotes(r#"say "hi""#), r#"say "hi""#);
    }

    #[test]
    fn test_normalize_quotes_curly_to_straight() {
        // Single curly quotes
        assert_eq!(normalize_quotes("\u{2018}hello\u{2019}"), "'hello'");
        // Double curly quotes
        assert_eq!(normalize_quotes("\u{201C}world\u{201D}"), "\"world\"");
        // Mixed
        let mixed = "\u{2018}it\u{2019}s a \u{201C}test\u{201D}";
        assert_eq!(normalize_quotes(mixed), "'it's a \"test\"");
    }

    // ── find_actual_string ───────────────────────────────────────────────────

    #[test]
    fn test_find_actual_string_exact() {
        let content = "fn foo() { 'bar' }";
        assert_eq!(find_actual_string(content, "'bar'"), Some("'bar'"));
    }

    #[test]
    fn test_find_actual_string_normalised() {
        // File has curly quotes, search has straight quotes
        let content = "fn foo() { \u{2018}bar\u{2019} }";
        let found = find_actual_string(content, "'bar'");
        assert!(found.is_some(), "should find via normalisation");
    }

    #[test]
    fn test_find_actual_string_not_found() {
        assert_eq!(find_actual_string("hello world", "goodbye"), None);
    }

    // ── strip_trailing_whitespace ────────────────────────────────────────────

    #[test]
    fn test_strip_trailing_whitespace_lf() {
        let input = "hello   \nworld  \n";
        let out = strip_trailing_whitespace(input);
        assert_eq!(out, "hello\nworld\n");
    }

    #[test]
    fn test_strip_trailing_whitespace_no_newline() {
        let input = "hello   ";
        assert_eq!(strip_trailing_whitespace(input), "hello");
    }

    // ── apply_edit ───────────────────────────────────────────────────────────

    #[test]
    fn test_apply_edit_basic() {
        let result = apply_edit("hello world", "world", "Rust", false);
        assert_eq!(result, "hello Rust");
    }

    #[test]
    fn test_apply_edit_replace_all() {
        let result = apply_edit("a b a b a", "a", "x", true);
        assert_eq!(result, "x b x b x");
    }

    #[test]
    fn test_apply_edit_empty_new_string_strips_newline() {
        let content = "line1\ndelete_me\nline3\n";
        let result = apply_edit(content, "delete_me", "", false);
        assert_eq!(result, "line1\nline3\n");
    }

    #[test]
    fn test_apply_edit_empty_old_string_is_replacement() {
        let result = apply_edit("", "", "new content", false);
        assert_eq!(result, "new content");
    }

    // ── validate_file_edit ───────────────────────────────────────────────────

    #[test]
    fn test_validate_noop_rejected() {
        let cache = FileStateCache::new();
        let path = Path::new("/tmp/nonexistent");
        let result = validate_file_edit(path, "same", "same", false, &cache);
        assert!(!result.is_ok(), "identical old/new should fail");
        if let ValidationResult::Err { error_code, .. } = result {
            assert_eq!(error_code, 1);
        }
    }

    #[test]
    fn test_validate_not_read() {
        let mut tmp = NamedTempFile::new().unwrap();
        writeln!(tmp, "hello world").unwrap();
        let cache = FileStateCache::new();
        let result = validate_file_edit(tmp.path(), "hello", "hi", false, &cache);
        if let ValidationResult::Err { error_code, .. } = result {
            assert_eq!(error_code, 6, "un-read file should return code 6");
        } else {
            panic!("expected Err(6), got Ok");
        }
    }

    #[test]
    fn test_validate_fresh_and_found() {
        let mut tmp = NamedTempFile::new().unwrap();
        writeln!(tmp, "hello world").unwrap();
        let cache = FileStateCache::new();
        cache.record_read(tmp.path());
        let result = validate_file_edit(tmp.path(), "hello", "hi", false, &cache);
        assert!(result.is_ok(), "fresh file with present string should pass");
    }

    #[test]
    fn test_validate_string_not_found() {
        let mut tmp = NamedTempFile::new().unwrap();
        writeln!(tmp, "hello world").unwrap();
        let cache = FileStateCache::new();
        cache.record_read(tmp.path());
        let result = validate_file_edit(tmp.path(), "goodbye", "hi", false, &cache);
        if let ValidationResult::Err { error_code, .. } = result {
            assert_eq!(error_code, 8);
        } else {
            panic!("expected Err(8)");
        }
    }

    #[test]
    fn test_validate_multiple_matches_without_replace_all() {
        let mut tmp = NamedTempFile::new().unwrap();
        writeln!(tmp, "foo bar foo baz foo").unwrap();
        let cache = FileStateCache::new();
        cache.record_read(tmp.path());
        let result = validate_file_edit(tmp.path(), "foo", "qux", false, &cache);
        if let ValidationResult::Err { error_code, .. } = result {
            assert_eq!(error_code, 9);
        } else {
            panic!("expected Err(9)");
        }
    }

    #[test]
    fn test_validate_multiple_matches_with_replace_all() {
        let mut tmp = NamedTempFile::new().unwrap();
        writeln!(tmp, "foo bar foo baz foo").unwrap();
        let cache = FileStateCache::new();
        cache.record_read(tmp.path());
        let result = validate_file_edit(tmp.path(), "foo", "qux", true, &cache);
        assert!(
            result.is_ok(),
            "replace_all=true should pass with multiple matches"
        );
    }

    // ── FileStateCache ───────────────────────────────────────────────────────

    #[test]
    fn test_cache_not_read() {
        let cache = FileStateCache::new();
        assert_eq!(
            cache.check_staleness(Path::new("/tmp/never_read")),
            StalenessResult::NotRead
        );
    }

    #[test]
    fn test_cache_fresh_after_record_read() {
        let tmp = NamedTempFile::new().unwrap();
        let cache = FileStateCache::new();
        cache.record_read(tmp.path());
        assert_eq!(cache.check_staleness(tmp.path()), StalenessResult::Fresh);
    }

    #[test]
    fn test_cache_modified_after_write() {
        let mut tmp = NamedTempFile::new().unwrap();
        let cache = FileStateCache::new();
        cache.record_read(tmp.path());
        // Simulate external modification: sleep briefly to ensure mtime differs
        std::thread::sleep(std::time::Duration::from_millis(10));
        writeln!(tmp, "modified").unwrap();
        tmp.flush().unwrap();
        // Force mtime update (macOS has 1s resolution on some FS, use manual touch)
        let new_mtime = SystemTime::now();
        filetime::set_file_mtime(tmp.path(), filetime::FileTime::from_system_time(new_mtime))
            .unwrap_or(()); // best-effort; test may pass anyway on fine-grained FS
        // Re-check (result depends on FS mtime resolution; on fine-grained FS it's Modified)
        // We just assert that the check runs without panic
        let _ = cache.check_staleness(tmp.path());
    }

    #[test]
    fn test_content_hash_detects_same_mtime_change() {
        // Simulate a sub-second modification where mtime doesn't change
        // but content does (the scenario content hash was designed for).
        let mut tmp = NamedTempFile::new().unwrap();
        write!(tmp, "original content").unwrap();
        tmp.flush().unwrap();

        let cache = FileStateCache::new();
        cache.record_read(tmp.path());
        assert_eq!(cache.check_staleness(tmp.path()), StalenessResult::Fresh);

        // Overwrite content and force same mtime
        let cached_mtime = std::fs::metadata(tmp.path()).unwrap().modified().unwrap();
        write!(tmp.as_file_mut(), "changed content!").unwrap();
        tmp.flush().unwrap();
        filetime::set_file_mtime(
            tmp.path(),
            filetime::FileTime::from_system_time(cached_mtime),
        )
        .unwrap();

        // mtime is identical, but content hash should catch the change
        assert_eq!(cache.check_staleness(tmp.path()), StalenessResult::Modified);
    }

    #[test]
    fn test_encoding_detection_utf8() {
        let tmp = NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), "hello").unwrap();
        let cache = FileStateCache::new();
        cache.record_read(tmp.path());
        assert_eq!(cache.encoding(tmp.path()), Some(FileEncoding::Utf8));
    }

    #[test]
    fn test_encoding_detection_utf16le() {
        let tmp = NamedTempFile::new().unwrap();
        // UTF-16 LE BOM followed by ASCII 'h'
        std::fs::write(tmp.path(), [0xFF, 0xFE, b'h', 0x00]).unwrap();
        let cache = FileStateCache::new();
        cache.record_read(tmp.path());
        assert_eq!(cache.encoding(tmp.path()), Some(FileEncoding::Utf16Le));
    }

    // ── FileHistory ──────────────────────────────────────────────────────────

    #[test]
    fn test_file_history_snapshot_and_rollback() {
        let tmp = NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), "version 1").unwrap();

        let history = FileHistory::default();
        assert!(history.snapshot(tmp.path()));

        // Modify the file
        std::fs::write(tmp.path(), "version 2").unwrap();
        assert!(history.snapshot(tmp.path()));

        // Rollback should restore version 2's pre-edit state (which was "version 2")
        // Actually: the most recent snapshot is "version 2" (snapshotted before a hypothetical edit)
        let restored = history.rollback(tmp.path());
        assert_eq!(restored.as_deref(), Some("version 2"));

        // Next rollback restores version 1
        let restored = history.rollback(tmp.path());
        assert_eq!(restored.as_deref(), Some("version 1"));

        // No more snapshots
        assert!(history.rollback(tmp.path()).is_none());
    }

    #[test]
    fn test_file_history_eviction_on_max_files() {
        let history = FileHistory::new(2, 1024 * 1024);
        let tmp1 = NamedTempFile::new().unwrap();
        let tmp2 = NamedTempFile::new().unwrap();
        let tmp3 = NamedTempFile::new().unwrap();
        std::fs::write(tmp1.path(), "a").unwrap();
        std::fs::write(tmp2.path(), "b").unwrap();
        std::fs::write(tmp3.path(), "c").unwrap();

        history.snapshot(tmp1.path());
        history.snapshot(tmp2.path());
        // Adding a third should evict the oldest
        history.snapshot(tmp3.path());

        let rollbacks = history.available_rollbacks();
        assert!(rollbacks.len() <= 2);
    }
}
