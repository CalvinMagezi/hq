//! Where the ledger lives on disk and how a batch of messages gets into it. The one writer, so the
//! CLI, the phone route and any later caller share one set of rules: the same file, a private
//! mode, an atomic replace and a single lock against two imports at once.

use crate::export::drafts_from_messages;
use crate::ledger::{self, STATUS_DRAFT};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Under `_data/`, which agent read tools refuse and the vault listing skips.
pub const LEDGER_REL: &str = "_data/finance/ledger.md";

/// One message as the phone hands it over.
#[derive(Debug, Clone, Deserialize)]
pub struct Message {
    pub sender: String,
    /// When the phone received it, in milliseconds.
    pub date: i64,
    pub body: String,
}

/// What an import did, in numbers only. Safe to show a model, a log or an inbox.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Receipt {
    /// Messages handed over.
    pub seen: usize,
    /// Messages from a sender outside the allowlist, dropped before parsing.
    pub ignored: usize,
    /// Transactions found in the messages that were read.
    pub transactions: usize,
    /// Rows new to the ledger.
    pub added: usize,
    /// Rows now waiting for the owner to confirm.
    pub waiting_for_review: usize,
}

static WRITE_LOCK: Mutex<()> = Mutex::new(());

pub fn ledger_path(vault: &Path) -> PathBuf {
    vault.join(LEDGER_REL)
}

/// Parses `messages` from `senders` only and adds the new transactions to the vault's ledger.
/// A sender not in the list is dropped before its text is looked at.
pub fn import_messages(
    vault: &Path,
    messages: &[Message],
    senders: &[&str],
) -> std::io::Result<Receipt> {
    let kept: Vec<&Message> = messages
        .iter()
        .filter(|m| senders.iter().any(|s| s.eq_ignore_ascii_case(&m.sender)))
        .collect();
    let drafts = drafts_from_messages(
        kept.iter()
            .map(|m| (m.sender.as_str(), m.date, m.body.as_str())),
    );
    let _guard = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = ledger_path(vault);
    let existing = match std::fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e),
    };
    let (note, added) = ledger::import(existing.as_deref(), &drafts);
    if added > 0 || existing.is_none() {
        write_private_atomic(&path, &note)?;
    }
    Ok(Receipt {
        seen: messages.len(),
        ignored: messages.len() - kept.len(),
        transactions: drafts.len(),
        added,
        waiting_for_review: count_waiting(&note),
    })
}

fn count_waiting(note: &str) -> usize {
    ledger::parse_rows(note)
        .iter()
        .filter(|r| r.cells[6] == STATUS_DRAFT)
        .count()
}

/// Writes next to the target and renames over it, so a reader never sees half a file and a crash
/// leaves the old ledger. The file is readable by its owner only.
fn write_private_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| std::io::Error::other("ledger path has no folder"))?;
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(".ledger.md.tmp");
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    let mut file = opts.open(&tmp)?;
    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SENT: &str = "You have sent UGX 32,000 to <NAME>, 0700000000 on 2026-03-04 10:11:12, fee: 218. Reason: . New balance: 80,455. ID :47152250553.";

    fn msg(sender: &str, body: &str) -> Message {
        Message {
            sender: sender.into(),
            date: 1_772_600_000_000,
            body: body.into(),
        }
    }

    #[test]
    fn a_sender_outside_the_allowlist_is_dropped_before_parsing() {
        let dir = tempfile::tempdir().unwrap();
        let r = import_messages(
            dir.path(),
            &[msg("MTNMobMoney", SENT), msg("SOMEBANK", SENT)],
            &["MTNMobMoney"],
        )
        .unwrap();
        assert_eq!((r.seen, r.ignored, r.transactions, r.added), (2, 1, 1, 1));
    }

    #[test]
    fn importing_the_same_batch_twice_adds_nothing_and_counts_the_drafts() {
        let dir = tempfile::tempdir().unwrap();
        let batch = [msg("MTNMobMoney", SENT)];
        import_messages(dir.path(), &batch, &["MTNMobMoney"]).unwrap();
        let again = import_messages(dir.path(), &batch, &["MTNMobMoney"]).unwrap();
        assert_eq!((again.added, again.waiting_for_review), (0, 1));
    }

    #[cfg(unix)]
    #[test]
    fn the_ledger_is_private_and_no_temp_file_is_left() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        import_messages(dir.path(), &[msg("MTNMobMoney", SENT)], &["MTNMobMoney"]).unwrap();
        let mode = std::fs::metadata(ledger_path(dir.path()))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0, "group and other have no access");
        assert!(!dir.path().join("_data/finance/.ledger.md.tmp").exists());
    }

    #[test]
    fn the_receipt_holds_numbers_only() {
        let json = serde_json::to_string(&Receipt::default()).unwrap();
        assert!(
            !json.contains('"') || json.matches('"').count() == 10,
            "only five numeric fields: {json}"
        );
    }
}
