//! Cross-interface conversation continuity.
//!
//! Every interface (Telegram, Discord, web, CLI) appends its turns to a
//! per-interface JSONL file under `.vault/_threads/`. At frame-build time the
//! tails of all files are merged by timestamp so HQ carries one conversation
//! across surfaces. Messages from other interfaces are prefixed with
//! `[via <interface>, HH:MM]` so the model knows where they happened.

use chrono::{DateTime, Utc};
use hq_core::identity::RequestIdentity;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

pub const MAX_MERGED_MESSAGES: usize = 20;
const ENTRY_CONTENT_CAP: usize = 2000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadEntry {
    pub ts: DateTime<Utc>,
    pub source: String,
    pub role: String,
    pub content: String,
    pub session_key: String,
}

/// The proxy API serves the web UI.
fn source_file_stem(label: &str) -> &str {
    match label {
        "proxy" => "web",
        other => other,
    }
}

pub fn thread_file(vault_path: &Path, source_label: &str) -> PathBuf {
    vault_path
        .join("_threads")
        .join(format!("{}.jsonl", source_file_stem(source_label)))
}

/// Strip a leading `<hq-context>…</hq-context>` block prepended by the web chat overlay.
pub fn strip_hq_context(s: &str) -> String {
    let trimmed = s.trim_start();
    if let Some(rest) = trimmed.strip_prefix("<hq-context>")
        && let Some(end) = rest.find("</hq-context>")
    {
        return rest[end + "</hq-context>".len()..].trim_start().to_string();
    }
    s.to_string()
}

pub fn append_thread_entry(
    vault_path: &Path,
    identity: &RequestIdentity,
    role: &str,
    content: &str,
) -> std::io::Result<()> {
    let content = strip_hq_context(content);
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    let entry = ThreadEntry {
        ts: Utc::now(),
        source: source_file_stem(identity.source.label()).to_string(),
        role: role.to_string(),
        content: hq_core::text::truncate_chars(trimmed, ENTRY_CONTENT_CAP).to_string(),
        session_key: identity.session_key.clone(),
    };
    let path = thread_file(vault_path, identity.source.label());
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let line = serde_json::to_string(&entry)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    writeln!(file, "{line}")
}

/// Merge the tails of every interface thread file into one timeline.
///
/// Returns at most `max` messages, oldest first. Messages from interfaces
/// other than `current_source` carry a `[via <interface>, HH:MM]` prefix
/// (local time). Pass `include_current: false` when the caller injects its
/// own interface's history separately (the relay does) so those turns are
/// not duplicated.
pub fn load_merged_thread(
    vault_path: &Path,
    current_source: &str,
    max: usize,
    include_current: bool,
) -> Vec<crate::context::layers::ConversationMessage> {
    let threads_dir = vault_path.join("_threads");
    let Ok(dir) = std::fs::read_dir(&threads_dir) else {
        return vec![];
    };
    let current = source_file_stem(current_source);
    let mut entries: Vec<ThreadEntry> = Vec::new();
    for dirent in dir.filter_map(|e| e.ok()) {
        let path = dirent.path();
        if path.extension().and_then(|x| x.to_str()) != Some("jsonl") {
            continue;
        }
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        if !include_current && stem == current {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        // Files are rotated to a bounded length by the daemon; the tail cap
        // here guards against an unrotated file growing unbounded.
        let tail: Vec<&str> = content.lines().rev().take(max * 4).collect();
        for line in tail.into_iter().rev() {
            let Ok(entry) = serde_json::from_str::<ThreadEntry>(line) else {
                continue;
            };
            if entry.role == "user" || entry.role == "assistant" {
                entries.push(entry);
            }
        }
    }
    entries.sort_by_key(|e| e.ts);
    let keep_from = entries.len().saturating_sub(max);
    entries
        .split_off(keep_from)
        .into_iter()
        .map(|e| {
            let content = if e.source == current {
                e.content
            } else {
                let local = e.ts.with_timezone(&chrono::Local);
                format!(
                    "[via {}, {}] {}",
                    e.source,
                    local.format("%H:%M"),
                    e.content
                )
            };
            crate::context::layers::ConversationMessage {
                role: e.role,
                content,
            }
        })
        .collect()
}

/// Trim a thread file to its last `keep` entries, returning the trimmed-off
/// lines so callers can feed them to memory consolidation.
pub fn rotate_thread_file(path: &Path, keep: usize) -> std::io::Result<Vec<String>> {
    let content = std::fs::read_to_string(path)?;
    let lines: Vec<&str> = content.lines().collect();
    if lines.len() <= keep {
        return Ok(vec![]);
    }
    let cut = lines.len() - keep;
    let trimmed: Vec<String> = lines[..cut].iter().map(|s| s.to_string()).collect();
    let kept = lines[cut..].join("\n");
    std::fs::write(path, format!("{kept}\n"))?;
    Ok(trimmed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn strip_hq_context_removes_leading_block_only() {
        assert_eq!(
            strip_hq_context("<hq-context>{\"a\":1}</hq-context>\n\nhi"),
            "hi"
        );
        assert_eq!(
            strip_hq_context("plain <hq-context>x</hq-context>"),
            "plain <hq-context>x</hq-context>"
        );
    }

    fn entry(ts_secs: i64, source: &str, role: &str, content: &str) -> String {
        let e = ThreadEntry {
            ts: Utc.timestamp_opt(ts_secs, 0).unwrap(),
            source: source.into(),
            role: role.into(),
            content: content.into(),
            session_key: format!("hq-{source}"),
        };
        serde_json::to_string(&e).unwrap()
    }

    fn write_thread(dir: &Path, name: &str, lines: &[String]) {
        let threads = dir.join("_threads");
        std::fs::create_dir_all(&threads).unwrap();
        std::fs::write(threads.join(name), lines.join("\n") + "\n").unwrap();
    }

    #[test]
    fn merges_interfaces_by_timestamp_with_prefixes() {
        let tmp = tempfile::tempdir().unwrap();
        write_thread(
            tmp.path(),
            "telegram.jsonl",
            &[
                entry(100, "telegram", "user", "check the deploy"),
                entry(200, "telegram", "assistant", "deploy is green"),
            ],
        );
        write_thread(
            tmp.path(),
            "cli.jsonl",
            &[entry(150, "cli", "user", "run the tests")],
        );

        let merged = load_merged_thread(tmp.path(), "cli", 10, true);
        assert_eq!(merged.len(), 3);
        assert!(merged[0].content.starts_with("[via telegram,"));
        assert!(merged[0].content.ends_with("check the deploy"));
        assert_eq!(merged[1].content, "run the tests");
        assert!(merged[2].content.starts_with("[via telegram,"));
    }

    #[test]
    fn caps_at_max_keeping_newest() {
        let tmp = tempfile::tempdir().unwrap();
        let lines: Vec<String> = (0..30)
            .map(|i| entry(i, "cli", "user", &format!("msg {i}")))
            .collect();
        write_thread(tmp.path(), "cli.jsonl", &lines);

        let merged = load_merged_thread(tmp.path(), "cli", 5, true);
        assert_eq!(merged.len(), 5);
        assert_eq!(merged[0].content, "msg 25");
        assert_eq!(merged[4].content, "msg 29");
    }

    #[test]
    fn excludes_current_source_when_requested() {
        let tmp = tempfile::tempdir().unwrap();
        write_thread(
            tmp.path(),
            "telegram.jsonl",
            &[entry(100, "telegram", "user", "from telegram")],
        );
        write_thread(
            tmp.path(),
            "cli.jsonl",
            &[entry(150, "cli", "user", "from cli")],
        );

        let merged = load_merged_thread(tmp.path(), "cli", 10, false);
        assert_eq!(merged.len(), 1);
        assert!(merged[0].content.ends_with("from telegram"));
    }

    #[test]
    fn skips_corrupt_lines_and_system_roles() {
        let tmp = tempfile::tempdir().unwrap();
        write_thread(
            tmp.path(),
            "cli.jsonl",
            &[
                "not json at all".to_string(),
                entry(1, "cli", "system", "ignored"),
                entry(2, "cli", "user", "kept"),
            ],
        );
        let merged = load_merged_thread(tmp.path(), "cli", 10, true);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].content, "kept");
    }

    #[test]
    fn append_maps_proxy_to_web_file() {
        let tmp = tempfile::tempdir().unwrap();
        let id = RequestIdentity::from_proxy_user("alex", vec!["*".into()], None);
        append_thread_entry(tmp.path(), &id, "user", "hello from web").unwrap();
        assert!(tmp.path().join("_threads/web.jsonl").is_file());
        let merged = load_merged_thread(tmp.path(), "proxy", 10, true);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].content, "hello from web");
    }

    #[test]
    fn rotate_returns_trimmed_lines() {
        let tmp = tempfile::tempdir().unwrap();
        let lines: Vec<String> = (0..10)
            .map(|i| entry(i, "cli", "user", &format!("m{i}")))
            .collect();
        write_thread(tmp.path(), "cli.jsonl", &lines);
        let path = tmp.path().join("_threads/cli.jsonl");

        let trimmed = rotate_thread_file(&path, 4).unwrap();
        assert_eq!(trimmed.len(), 6);
        let merged = load_merged_thread(tmp.path(), "cli", 100, true);
        assert_eq!(merged.len(), 4);
        assert_eq!(merged[0].content, "m6");
    }

    #[test]
    fn empty_content_is_not_appended() {
        let tmp = tempfile::tempdir().unwrap();
        let id = RequestIdentity::local();
        append_thread_entry(tmp.path(), &id, "user", "   ").unwrap();
        assert!(!tmp.path().join("_threads/cli.jsonl").exists());
    }
}
