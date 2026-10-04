//! Agent Mailbox — point-to-point messaging between harnesses.
//!
//! Each harness gets a mailbox directory at `.vault/_mailboxes/{harness-id}/`.
//! Messages are atomic JSON files (tmp + rename) consumed in FIFO order.
//!
//! Lives in `hq-core` because `hq-daemon`, `hq-relay`, `hq-web`, `hq-tools`,
//! `hq-agent` and `hq-cli` all read or write mailboxes, and `hq-daemon`
//! depends on `hq-web`, so the shared file I/O cannot live in either of them.

use crate::types::{MailboxMessage, MailboxMessageType};
use anyhow::Result;
use chrono::Utc;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

/// Base directory within the vault for mailboxes.
pub const MAILBOX_DIR: &str = "_mailboxes";

/// Validate that a harness ID is safe to use as a path component.
fn validate_harness_id(id: &str) -> Result<()> {
    if id.is_empty()
        || id.contains('/')
        || id.contains('\\')
        || id.contains("..")
        || id.contains('\0')
    {
        anyhow::bail!("invalid harness_id: must not contain path separators or '..'");
    }
    Ok(())
}

/// Ensure mailbox directory exists for a harness.
fn ensure_mailbox(vault_path: &Path, harness_id: &str) -> Result<PathBuf> {
    validate_harness_id(harness_id)?;
    let dir = vault_path.join(MAILBOX_DIR).join(harness_id);
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Send a message to a harness's mailbox.
///
/// Uses atomic write (tmp file + rename) to prevent partial reads.
pub fn send_message(vault_path: &Path, message: &MailboxMessage) -> Result<()> {
    let mailbox_dir = ensure_mailbox(vault_path, &message.to)?;

    let filename = format!(
        "msg-{}-{}.json",
        message.timestamp.timestamp_millis(),
        &message.id[..8]
    );
    let target_path = mailbox_dir.join(&filename);
    let tmp_path = mailbox_dir.join(format!(".tmp-{}", filename));

    // Atomic write: write to tmp, then rename
    let json = serde_json::to_string_pretty(message)?;
    std::fs::write(&tmp_path, &json)?;
    std::fs::rename(&tmp_path, &target_path)?;

    info!(
        from = %message.from,
        to = %message.to,
        msg_type = ?message.msg_type,
        "mailbox: message delivered"
    );

    Ok(())
}

/// Read and consume all messages in a harness's mailbox (FIFO order).
///
/// Messages are deleted after reading. Returns empty vec if no messages.
pub fn receive_messages(vault_path: &Path, harness_id: &str) -> Result<Vec<MailboxMessage>> {
    validate_harness_id(harness_id)?;
    let mailbox_dir = vault_path.join(MAILBOX_DIR).join(harness_id);
    if !mailbox_dir.exists() {
        return Ok(Vec::new());
    }

    let mut entries: Vec<_> = std::fs::read_dir(&mailbox_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            name.starts_with("msg-") && name.ends_with(".json")
        })
        .collect();

    // Sort by filename (timestamp-based, ensures FIFO)
    entries.sort_by_key(|e| e.file_name());

    let mut messages = Vec::new();
    for entry in entries {
        let path = entry.path();
        match std::fs::read_to_string(&path) {
            Ok(content) => match serde_json::from_str::<MailboxMessage>(&content) {
                Ok(msg) => {
                    // Consume (delete) the message
                    if let Err(e) = std::fs::remove_file(&path) {
                        warn!(path = %path.display(), error = %e, "failed to consume mailbox message");
                    }
                    messages.push(msg);
                }
                Err(e) => {
                    warn!(path = %path.display(), error = %e, "malformed mailbox message");
                }
            },
            Err(e) => {
                warn!(path = %path.display(), error = %e, "failed to read mailbox message");
            }
        }
    }

    Ok(messages)
}

/// Peek at messages without consuming them.
pub fn peek_messages(vault_path: &Path, harness_id: &str) -> Result<Vec<MailboxMessage>> {
    validate_harness_id(harness_id)?;
    let mailbox_dir = vault_path.join(MAILBOX_DIR).join(harness_id);
    if !mailbox_dir.exists() {
        return Ok(Vec::new());
    }

    let mut entries: Vec<_> = std::fs::read_dir(&mailbox_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            name.starts_with("msg-") && name.ends_with(".json")
        })
        .collect();

    entries.sort_by_key(|e| e.file_name());

    let mut messages = Vec::new();
    for entry in entries {
        if let Ok(content) = std::fs::read_to_string(entry.path())
            && let Ok(msg) = serde_json::from_str::<MailboxMessage>(&content)
        {
            messages.push(msg);
        }
    }

    Ok(messages)
}

/// `MailboxMessage.meta` key a `Nudge` producer sets to force `Urgent` past
/// `hq_daemon::notif_gate`'s noise-learning gate — reserved for messages
/// that need same-tick delivery (an approval ask, an actionable failure),
/// not machine-generated status content that happens to use the `Nudge`
/// type purely for its faster poller cadence. Defined here (rather than in
/// `hq-daemon`) so producer crates that can't depend on `hq-daemon` — like
/// `hq-agent`, which `hq-daemon` itself depends on — can still tag their
/// messages. See FEATURE-REQUESTS.md FR-001b.
pub const META_INTERRUPT: &str = "interrupt";

/// Broadcast a message to all harnesses with mailboxes.
pub fn broadcast(
    vault_path: &Path,
    from: &str,
    msg_type: MailboxMessageType,
    content: &str,
) -> Result<usize> {
    broadcast_with_meta(vault_path, from, msg_type, content, &[])
}

/// Same as [`broadcast`], with caller-supplied metadata attached to every
/// copy — e.g. `("interrupt", "true")` so `notif_gate::gate_decision` treats
/// a `Nudge` as urgent instead of falling through to the learned gate. See
/// FEATURE-REQUESTS.md FR-001b.
pub fn broadcast_with_meta(
    vault_path: &Path,
    from: &str,
    msg_type: MailboxMessageType,
    content: &str,
    meta: &[(&str, &str)],
) -> Result<usize> {
    let mailbox_base = vault_path.join(MAILBOX_DIR);
    if !mailbox_base.exists() {
        return Ok(0);
    }

    let harnesses: Vec<String> = std::fs::read_dir(&mailbox_base)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|name| name != from)
        .collect();

    let mut sent = 0;
    for harness in &harnesses {
        let mut msg = new_message(from, harness, msg_type.clone(), None, content, None);
        for (k, v) in meta {
            msg.meta.insert((*k).to_string(), (*v).to_string());
        }
        if send_message(vault_path, &msg).is_ok() {
            sent += 1;
        }
    }

    Ok(sent)
}

/// `MailboxMessage.meta` key carrying a task's internal id, set by
/// [`notify_tagged_agents`] so a recipient can look the task up directly
/// instead of parsing it back out of `subject`/`content`.
pub const META_TASK_ID: &str = "task_id";

/// Push a `Direct` message to every tag in `tags` that already has a mailbox
/// (`_mailboxes/<tag>/` exists) — the same discovery rule [`broadcast`] uses,
/// so `hermes`/`hq`-style routing tags work with zero extra config. Called
/// synchronously right after a task create/update from both the MCP tool
/// layer (`hq-tools::tasks`) and the web REST layer (`hq-web::tasks_api`),
/// so routing is identical regardless of who filed the task. Nothing here
/// polls anything — see docs/plans/native-tasks for why a poll isn't needed
/// once tasks live in `hq-db` instead of an external API.
pub fn notify_tagged_agents(
    vault_path: &Path,
    task_id: &str,
    display_id: &str,
    title: &str,
    tags: &[String],
) -> Result<usize> {
    notify_tagged_agents_with(
        vault_path,
        task_id,
        display_id,
        &format!("New/updated task: {title}"),
        tags,
    )
}

/// Same routing as [`notify_tagged_agents`], with the message body chosen by
/// the caller (e.g. "Unblocked: ..." when a dependency completes).
pub fn notify_tagged_agents_with(
    vault_path: &Path,
    task_id: &str,
    display_id: &str,
    content: &str,
    tags: &[String],
) -> Result<usize> {
    let mut sent = 0;
    for tag in tags {
        let mailbox_dir = vault_path.join(MAILBOX_DIR).join(tag);
        if !mailbox_dir.exists() {
            continue;
        }
        let mut msg = new_message(
            "tasks",
            tag,
            MailboxMessageType::Direct,
            Some(display_id),
            content,
            None,
        );
        msg.meta.insert(META_TASK_ID.to_string(), task_id.to_string());
        if send_message(vault_path, &msg).is_ok() {
            sent += 1;
        }
    }
    Ok(sent)
}

/// Create a new mailbox message with generated ID and timestamp.
pub fn new_message(
    from: &str,
    to: &str,
    msg_type: MailboxMessageType,
    subject: Option<&str>,
    content: &str,
    job_id: Option<&str>,
) -> MailboxMessage {
    MailboxMessage {
        id: uuid::Uuid::new_v4().to_string(),
        timestamp: Utc::now(),
        from: from.to_string(),
        to: to.to_string(),
        msg_type,
        subject: subject.map(|s| s.to_string()),
        content: content.to_string(),
        job_id: job_id.map(|s| s.to_string()),
        meta: HashMap::new(),
    }
}

/// Subdirectory holding messages retired by [`archive_older_than`].
///
/// Readers glob `msg-*.json` at the mailbox root, so a subdirectory is
/// invisible to them without any reader changes.
pub const ARCHIVE_DIR: &str = "archive";

/// Mailboxes with live pollers. Moving their mail would drop work in flight.
pub const RESERVED_MAILBOXES: &[&str] = &["relay", "agent-worker"];

/// Move messages older than `older_than_days` into each mailbox's archive.
///
/// Returns the per-mailbox counts that were moved (or, when `dry_run`, would
/// be). Reserved mailboxes are skipped entirely.
pub fn archive_older_than(
    vault_path: &Path,
    older_than_days: i64,
    dry_run: bool,
) -> Result<Vec<(String, usize)>> {
    let mailbox_base = vault_path.join(MAILBOX_DIR);
    if !mailbox_base.exists() {
        return Ok(Vec::new());
    }

    let cutoff = Utc::now() - chrono::Duration::days(older_than_days);
    let mut moved = Vec::new();

    for entry in std::fs::read_dir(&mailbox_base)?.filter_map(|e| e.ok()) {
        if !entry.path().is_dir() {
            continue;
        }
        let harness = entry.file_name().to_string_lossy().to_string();
        if RESERVED_MAILBOXES.contains(&harness.as_str()) {
            continue;
        }

        let count = archive_one_mailbox(&entry.path(), cutoff, dry_run)?;
        if count > 0 {
            moved.push((harness, count));
        }
    }

    moved.sort();
    Ok(moved)
}

/// Archive a single mailbox directory. Messages that cannot be parsed are left
/// alone: an unreadable file is a bug to look at, not something to hide.
fn archive_one_mailbox(
    mailbox_dir: &Path,
    cutoff: chrono::DateTime<Utc>,
    dry_run: bool,
) -> Result<usize> {
    let stale: Vec<PathBuf> = std::fs::read_dir(mailbox_dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().unwrap_or_default().to_string_lossy();
            name.starts_with("msg-") && name.ends_with(".json")
        })
        .filter(|p| match std::fs::read_to_string(p) {
            Ok(content) => serde_json::from_str::<MailboxMessage>(&content)
                .map(|m| m.timestamp < cutoff)
                .unwrap_or(false),
            Err(_) => false,
        })
        .collect();

    if stale.is_empty() || dry_run {
        return Ok(stale.len());
    }

    let archive = mailbox_dir.join(ARCHIVE_DIR);
    std::fs::create_dir_all(&archive)?;

    let mut count = 0;
    for path in stale {
        let Some(name) = path.file_name() else {
            continue;
        };
        match std::fs::rename(&path, archive.join(name)) {
            Ok(()) => count += 1,
            Err(e) => warn!(path = %path.display(), error = %e, "failed to archive message"),
        }
    }

    Ok(count)
}

/// Count pending messages for a harness.
pub fn message_count(vault_path: &Path, harness_id: &str) -> Result<usize> {
    validate_harness_id(harness_id)?;
    let mailbox_dir = vault_path.join(MAILBOX_DIR).join(harness_id);
    if !mailbox_dir.exists() {
        return Ok(0);
    }

    let count = std::fs::read_dir(&mailbox_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            name.starts_with("msg-") && name.ends_with(".json")
        })
        .count();

    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_send_and_receive() {
        let tmp = TempDir::new().unwrap();
        let vault = tmp.path();

        let msg = new_message(
            "hq-worker-a",
            "hq-worker-b",
            MailboxMessageType::TaskResult,
            Some("job done"),
            "Completed analysis of module X.",
            Some("job-123"),
        );

        send_message(vault, &msg).unwrap();

        // Peek should show the message
        let peeked = peek_messages(vault, "hq-worker-b").unwrap();
        assert_eq!(peeked.len(), 1);
        assert_eq!(peeked[0].from, "hq-worker-a");

        // Receive should consume it
        let received = receive_messages(vault, "hq-worker-b").unwrap();
        assert_eq!(received.len(), 1);
        assert_eq!(received[0].content, "Completed analysis of module X.");

        // Should be empty now
        let empty = receive_messages(vault, "hq-worker-b").unwrap();
        assert!(empty.is_empty());
    }

    #[test]
    fn test_fifo_ordering() {
        let tmp = TempDir::new().unwrap();
        let vault = tmp.path();

        for i in 0..3 {
            let msg = new_message(
                "agent-a",
                "agent-b",
                MailboxMessageType::Direct,
                None,
                &format!("message {}", i),
                None,
            );
            send_message(vault, &msg).unwrap();
            // Small delay for distinct timestamps
            std::thread::sleep(std::time::Duration::from_millis(5));
        }

        let received = receive_messages(vault, "agent-b").unwrap();
        assert_eq!(received.len(), 3);
        assert!(received[0].content.contains("message 0"));
        assert!(received[1].content.contains("message 1"));
        assert!(received[2].content.contains("message 2"));
    }

    #[test]
    fn test_broadcast() {
        let tmp = TempDir::new().unwrap();
        let vault = tmp.path();

        // Create mailbox dirs for 3 agents
        ensure_mailbox(vault, "agent-a").unwrap();
        ensure_mailbox(vault, "agent-b").unwrap();
        ensure_mailbox(vault, "agent-c").unwrap();

        let sent = broadcast(
            vault,
            "agent-a",
            MailboxMessageType::Idle,
            "I'm idle, available for work.",
        )
        .unwrap();

        assert_eq!(sent, 2); // b and c, not a (sender excluded)

        assert_eq!(message_count(vault, "agent-b").unwrap(), 1);
        assert_eq!(message_count(vault, "agent-c").unwrap(), 1);
        assert_eq!(message_count(vault, "agent-a").unwrap(), 0);
    }

    /// Write a message backdated past the archival cutoff.
    fn send_aged(vault: &Path, to: &str, days_old: i64) {
        let mut msg = new_message("daemon", to, MailboxMessageType::Direct, None, "old", None);
        msg.timestamp = Utc::now() - chrono::Duration::days(days_old);
        send_message(vault, &msg).unwrap();
    }

    #[test]
    fn archive_moves_only_stale_messages_and_leaves_readers_intact() {
        let tmp = TempDir::new().unwrap();
        let vault = tmp.path();

        send_aged(vault, "pi", 30);
        send_aged(vault, "pi", 30);
        send_message(
            vault,
            &new_message("hq", "pi", MailboxMessageType::Direct, None, "fresh", None),
        )
        .unwrap();

        let moved = archive_older_than(vault, 7, false).unwrap();
        assert_eq!(moved, vec![("pi".to_string(), 2)]);

        // The recent message is still the only thing a reader sees.
        assert_eq!(message_count(vault, "pi").unwrap(), 1);
        let remaining = peek_messages(vault, "pi").unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].content, "fresh");

        let archived = std::fs::read_dir(vault.join(MAILBOX_DIR).join("pi").join(ARCHIVE_DIR))
            .unwrap()
            .count();
        assert_eq!(archived, 2);
    }

    #[test]
    fn archive_skips_mailboxes_with_live_consumers() {
        let tmp = TempDir::new().unwrap();
        let vault = tmp.path();

        send_aged(vault, "relay", 30);
        send_aged(vault, "agent-worker", 30);

        let moved = archive_older_than(vault, 7, false).unwrap();
        assert!(moved.is_empty(), "reserved mailboxes must not be touched");
        assert_eq!(message_count(vault, "relay").unwrap(), 1);
        assert_eq!(message_count(vault, "agent-worker").unwrap(), 1);
    }

    #[test]
    fn dry_run_reports_without_moving() {
        let tmp = TempDir::new().unwrap();
        let vault = tmp.path();
        send_aged(vault, "hermes", 30);

        let moved = archive_older_than(vault, 7, true).unwrap();
        assert_eq!(moved, vec![("hermes".to_string(), 1)]);
        assert_eq!(
            message_count(vault, "hermes").unwrap(),
            1,
            "dry run must leave the mailbox untouched"
        );
    }
}
