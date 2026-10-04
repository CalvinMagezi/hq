//! Startup reconciler for durable background turns (Phase 3, strand recovery).
//!
//! Relay surfaces park turns that outlive their ack window in the
//! `background_turns` registry. If the daemon is killed mid-flight, those rows
//! are left in `running` forever. On boot this module sweeps the registry:
//! every stranded row is marked `interrupted`, and rows fresh enough to act on
//! (younger than `config.relay.background_turn_max_days`) get an FYI posted to
//! the `relay` mailbox so Telegram/Discord users hear about it and can
//! reply `resume <id>` (Task 9) to rerun the turn. Stale rows are marked
//! interrupted silently and only logged.
//!
//! Rows with `kind = 'watch'` are owned by the relay watch scheduler and are
//! normally left alone; only watches whose `watch_until` expiry is itself older
//! than the staleness ceiling (long-abandoned watches) are marked interrupted,
//! silently.
//!
//! Fully best-effort: callers are expected to log-and-continue on error, and
//! per-row failures never abort the sweep.

use anyhow::Result;
use hq_core::types::MailboxMessageType;
use hq_db::Database;
use hq_db::background_turns;
use std::path::Path;
use tracing::{info, warn};

/// Outcome of one reconciliation sweep, returned for logging and tests.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ReconcileStats {
    /// Rows found in `running` (all were marked interrupted).
    pub stranded: usize,
    /// FYI notices posted to the relay mailbox (fresh rows only).
    pub notified: usize,
    /// Rows too old to notify (`> max_age_days`), logged only.
    pub stale: usize,
}

/// Char-safe excerpt of the prompt for the FYI message.
fn prompt_excerpt(prompt: &str, max_chars: usize) -> String {
    let excerpt: String = prompt.chars().take(max_chars).collect();
    if prompt.chars().count() > max_chars {
        format!("{excerpt}…")
    } else {
        excerpt
    }
}

/// Periodic sweep for turns that stayed `running` past the age ceiling, which
/// happens when a relay listener dies while the daemon stays up. Unlike the
/// startup sweep it runs beside live relays, so it never touches a turn younger
/// than `max_age_days`: one of those may still be working. Expired rows are
/// interrupted silently, as the startup sweep does for stale rows.
pub fn reconcile_expired_turns(db: &Database, max_age_days: u64) -> Result<usize> {
    let now = chrono::Utc::now().timestamp();
    let max_age_secs = max_age_secs_saturating(max_age_days) as i64;
    let running = db.with_conn(background_turns::list_running)?;
    let mut expired = 0;
    for row in running {
        if row.kind == background_turns::KIND_WATCH || now.saturating_sub(row.created_at) <= max_age_secs {
            continue;
        }
        let id = row.id.clone();
        match db.with_conn(move |c| background_turns::mark_interrupted(c, &id, now)) {
            Ok(()) => expired += 1,
            Err(e) => warn!(turn = %row.id, error = %e, "turn-reconciler: mark_interrupted failed"),
        }
    }
    Ok(expired)
}

/// Reconcile stranded background turns once at daemon startup.
///
/// `vault_path` locates the mailbox directory; `db` is the open vault database;
/// `max_age_days` is the staleness ceiling (`config.relay.background_turn_max_days`).
/// Every running row is marked interrupted; rows within the age ceiling also get
/// an FYI posted to the `relay` mailbox. Per-row errors are logged and skipped.
pub fn reconcile_stranded_turns(
    vault_path: &Path,
    db: &Database,
    max_age_days: u64,
) -> Result<ReconcileStats> {
    let now = chrono::Utc::now().timestamp();
    let max_age_secs = (max_age_secs_saturating(max_age_days)) as i64;

    let running = db.with_conn(background_turns::list_running)?;
    let mut stats = ReconcileStats::default();

    for row in running {
        // Watch rows are owned by the relay watch scheduler, which re-dispatches
        // them after restart. Only a watch whose expiry (`watch_until`) is itself
        // older than the staleness ceiling is abandoned work: interrupt it
        // silently. All other watches are left untouched.
        if row.kind == background_turns::KIND_WATCH {
            let abandoned = row
                .watch_until
                .is_some_and(|until| now.saturating_sub(until) > max_age_secs);
            if abandoned {
                let id = row.id.clone();
                match db.with_conn(move |c| background_turns::mark_interrupted(c, &id, now)) {
                    Ok(()) => {
                        stats.stale += 1;
                        info!(
                            turn = %row.id,
                            platform = %row.platform,
                            "turn-reconciler: abandoned watch past expiry ceiling, interrupted silently"
                        );
                    }
                    Err(e) => {
                        warn!(turn = %row.id, error = %e, "turn-reconciler: mark_interrupted failed");
                    }
                }
            }
            continue;
        }

        stats.stranded += 1;
        let age_secs = now.saturating_sub(row.created_at);
        let fresh = age_secs <= max_age_secs;

        let id = row.id.clone();
        if let Err(e) = db.with_conn(move |c| background_turns::mark_interrupted(c, &id, now)) {
            warn!(turn = %row.id, error = %e, "turn-reconciler: mark_interrupted failed");
            continue;
        }

        if !fresh {
            stats.stale += 1;
            info!(
                turn = %row.id,
                platform = %row.platform,
                age_days = age_secs / 86_400,
                "turn-reconciler: stranded turn older than {max_age_days}d ceiling, interrupted without notice"
            );
            continue;
        }

        let text = format!(
            "Turn `{}` ({}) was interrupted by a restart: '{}'.{} Say 'resume {}' to rerun it.",
            row.id,
            row.platform,
            prompt_excerpt(&row.prompt, 120),
            blocked_suffix(row.blocked_on.as_deref(), row.blocked_detail.as_deref()),
            row.id
        );
        let msg = hq_core::mailbox::new_message(
            "background-turn",
            "relay",
            MailboxMessageType::Direct,
            Some("Background turn interrupted"),
            &text,
            None,
        );
        match hq_core::mailbox::send_message(vault_path, &msg) {
            Ok(()) => {
                stats.notified += 1;
                info!(turn = %row.id, platform = %row.platform, "turn-reconciler: interrupted turn, FYI posted");
            }
            Err(e) => {
                warn!(turn = %row.id, error = %e, "turn-reconciler: FYI post failed");
            }
        }
        record_interrupt_thread_entry(vault_path, &row, &text);
    }

    Ok(stats)
}

/// Record the interrupt FYI into the same per-interface `_threads/*.jsonl`
/// file a completed turn would land in, so a later chat turn's history
/// actually contains "this task died" instead of only the `background_turns`
/// DB row knowing. The mailbox delivery above reaches the chat surface but
/// bypasses thread history entirely, unlike `native_hq::record_thread_turn`
/// on the normal completion path.
fn record_interrupt_thread_entry(
    vault_path: &Path,
    row: &background_turns::BackgroundTurnRow,
    text: &str,
) {
    let identity = match row.platform.as_str() {
        "telegram" => row
            .chat_id
            .parse::<i64>()
            .ok()
            .map(hq_core::identity::RequestIdentity::from_telegram),
        "discord" => row
            .chat_id
            .parse::<u64>()
            .ok()
            .map(hq_core::identity::RequestIdentity::from_discord),
        _ => None,
    };
    let Some(identity) = identity else { return };
    if let Err(e) = hq_agent::threads::append_thread_entry(vault_path, &identity, "assistant", text)
    {
        warn!(turn = %row.id, error = %e, "turn-reconciler: thread append failed");
    }
}

/// The FYI's honest addendum for a turn that last reported itself blocked.
///
/// `report_progress(blocked_on=...)` persists here (`background_turns::set_blocked`)
/// specifically because the process that fired the transient `ProgressEvent`
/// is gone by the time this reconciler runs — without this column there is
/// nothing left to read, and the FYI would claim a generic restart when the
/// turn was actually waiting on an answer.
fn blocked_suffix(blocked_on: Option<&str>, blocked_detail: Option<&str>) -> String {
    let Some(on) = blocked_on else {
        return String::new();
    };
    match blocked_detail {
        Some(detail) => format!(" It was last waiting on {on}: {detail}."),
        None => format!(" It was last waiting on {on}."),
    }
}

/// Convert days to seconds without overflow risk on absurd config values.
fn max_age_secs_saturating(days: u64) -> u64 {
    days.saturating_mul(86_400)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const NOW_OFFSET: i64 = 1_700_000_000;

    fn seed(db: &Database, id: &str, created_at: i64) {
        db.with_conn(move |c| {
            background_turns::insert(
                c,
                id,
                "telegram",
                "chat-42",
                None,
                Some("alex"),
                "refactor the relay ack window so turns can detach safely",
                created_at,
                None,
            )
        })
        .unwrap();
    }

    fn seed_watch(db: &Database, id: &str, created_at: i64, watch_until: Option<i64>) {
        db.with_conn(move |c| {
            background_turns::insert_watch(
                c,
                id,
                "telegram",
                "chat-42",
                None,
                Some("alex"),
                "check whether the agy review is done",
                created_at,
                900,
                watch_until,
            )
        })
        .unwrap();
    }

    fn mailbox_dir(vault: &Path) -> PathBuf {
        vault.join(hq_core::mailbox::MAILBOX_DIR).join("relay")
    }

    fn mailbox_files(vault: &Path) -> Vec<String> {
        let dir = mailbox_dir(vault);
        std::fs::read_dir(&dir)
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().to_string())
                    .filter(|n| n.ends_with(".json"))
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn empty_registry_is_noop() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::open_memory().unwrap();
        let stats = reconcile_stranded_turns(tmp.path(), &db, 5).unwrap();
        assert_eq!(stats, ReconcileStats::default());
        assert!(mailbox_files(tmp.path()).is_empty());
    }

    #[test]
    fn fresh_running_turn_is_interrupted_and_notified() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::open_memory().unwrap();
        // created_at in the future relative to nothing: reconciler uses real now,
        // so seed with "now" via chrono to stay fresh regardless of wall clock.
        let now = chrono::Utc::now().timestamp();
        seed(&db, "bt-fresh", now - 3600);

        let stats = reconcile_stranded_turns(tmp.path(), &db, 5).unwrap();
        assert_eq!(stats.stranded, 1);
        assert_eq!(stats.notified, 1);
        assert_eq!(stats.stale, 0);

        let row = db
            .with_conn(|c| background_turns::get(c, "bt-fresh"))
            .unwrap()
            .unwrap();
        assert_eq!(row.status, background_turns::STATUS_INTERRUPTED);
        assert!(row.completed_at.is_some());

        let files = mailbox_files(tmp.path());
        assert_eq!(files.len(), 1);
        let body = std::fs::read_to_string(mailbox_dir(tmp.path()).join(&files[0])).unwrap();
        assert!(body.contains("bt-fresh"));
        assert!(body.contains("telegram"));
        assert!(body.contains("resume bt-fresh"));
    }

    #[test]
    fn periodic_sweep_leaves_live_turns_alone_and_expires_old_ones() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::open_memory().unwrap();
        let now = chrono::Utc::now().timestamp();
        seed(&db, "bt-live", now - 3600);
        seed(&db, "bt-old", now - 10 * 86_400);

        assert_eq!(reconcile_expired_turns(&db, 5).unwrap(), 1);

        let status = |id: &'static str| db.with_conn(move |c| background_turns::get(c, id)).unwrap().unwrap().status;
        assert_eq!(status("bt-live"), background_turns::STATUS_RUNNING);
        assert_eq!(status("bt-old"), background_turns::STATUS_INTERRUPTED);
        assert!(mailbox_files(tmp.path()).is_empty());
    }

    #[test]
    fn stale_turn_is_interrupted_without_notice() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::open_memory().unwrap();
        // 10 days old, ceiling 5 days.
        let now = chrono::Utc::now().timestamp();
        seed(&db, "bt-stale", now - 10 * 86_400);

        let stats = reconcile_stranded_turns(tmp.path(), &db, 5).unwrap();
        assert_eq!(stats.stranded, 1);
        assert_eq!(stats.notified, 0);
        assert_eq!(stats.stale, 1);

        let row = db
            .with_conn(|c| background_turns::get(c, "bt-stale"))
            .unwrap()
            .unwrap();
        assert_eq!(row.status, background_turns::STATUS_INTERRUPTED);
        assert!(mailbox_files(tmp.path()).is_empty());
    }

    #[test]
    fn completed_turns_are_untouched() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::open_memory().unwrap();
        seed(&db, "bt-done", NOW_OFFSET);
        db.with_conn(|c| background_turns::mark_completed(c, "bt-done", "ok", NOW_OFFSET + 1))
            .unwrap();

        let stats = reconcile_stranded_turns(tmp.path(), &db, 5).unwrap();
        assert_eq!(stats, ReconcileStats::default());
        let row = db
            .with_conn(|c| background_turns::get(c, "bt-done"))
            .unwrap()
            .unwrap();
        assert_eq!(row.status, background_turns::STATUS_COMPLETED);
    }

    #[test]
    fn running_watch_is_left_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::open_memory().unwrap();
        // Very old watch with no expiry: the scheduler still owns it.
        seed_watch(&db, "bt-watch-live", NOW_OFFSET, None);

        let stats = reconcile_stranded_turns(tmp.path(), &db, 5).unwrap();
        assert_eq!(stats, ReconcileStats::default());

        let row = db
            .with_conn(|c| background_turns::get(c, "bt-watch-live"))
            .unwrap()
            .unwrap();
        assert_eq!(row.status, "running");
        assert!(row.completed_at.is_none());
        assert!(mailbox_files(tmp.path()).is_empty());
    }

    #[test]
    fn watch_with_recent_expiry_is_left_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::open_memory().unwrap();
        let now = chrono::Utc::now().timestamp();
        // Expired 1 hour ago, well within the 5-day ceiling: still scheduler-owned.
        seed_watch(&db, "bt-watch-recent", now - 2 * 86_400, Some(now - 3600));

        let stats = reconcile_stranded_turns(tmp.path(), &db, 5).unwrap();
        assert_eq!(stats, ReconcileStats::default());

        let row = db
            .with_conn(|c| background_turns::get(c, "bt-watch-recent"))
            .unwrap()
            .unwrap();
        assert_eq!(row.status, "running");
        assert!(mailbox_files(tmp.path()).is_empty());
    }

    #[test]
    fn abandoned_watch_is_interrupted_without_notice() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::open_memory().unwrap();
        let now = chrono::Utc::now().timestamp();
        // watch_until 10 days ago, ceiling 5 days: abandoned.
        seed_watch(
            &db,
            "bt-watch-abandoned",
            now - 30 * 86_400,
            Some(now - 10 * 86_400),
        );

        let stats = reconcile_stranded_turns(tmp.path(), &db, 5).unwrap();
        assert_eq!(stats.stranded, 0);
        assert_eq!(stats.notified, 0);
        assert_eq!(stats.stale, 1);

        let row = db
            .with_conn(|c| background_turns::get(c, "bt-watch-abandoned"))
            .unwrap()
            .unwrap();
        assert_eq!(row.status, background_turns::STATUS_INTERRUPTED);
        assert!(row.completed_at.is_some());
        assert!(mailbox_files(tmp.path()).is_empty());
    }

    /// The bug this fix closes: a turn genuinely blocked on Alex's answer
    /// (not stalled or crashed) used to get the exact same generic "interrupted
    /// by a restart" text as a turn that just silently died.
    #[test]
    fn fresh_running_turn_that_was_blocked_says_so_in_the_fyi() {
        let tmp = tempfile::tempdir().unwrap();
        let db = Database::open_memory().unwrap();
        let now = chrono::Utc::now().timestamp();
        seed(&db, "bt-blocked", now - 3600);
        db.with_conn(|c| {
            background_turns::set_blocked(
                c,
                "bt-blocked",
                "Alex",
                Some("which OpenRouter key to use"),
            )
        })
        .unwrap();

        reconcile_stranded_turns(tmp.path(), &db, 5).unwrap();

        let files = mailbox_files(tmp.path());
        assert_eq!(files.len(), 1);
        let body = std::fs::read_to_string(mailbox_dir(tmp.path()).join(&files[0])).unwrap();
        assert!(body.contains("waiting on Alex"));
        assert!(body.contains("which OpenRouter key to use"));
    }

    #[test]
    fn blocked_suffix_covers_all_three_shapes() {
        assert_eq!(blocked_suffix(None, None), "");
        assert_eq!(
            blocked_suffix(Some("hermes"), None),
            " It was last waiting on hermes."
        );
        assert_eq!(
            blocked_suffix(Some("Alex"), Some("a go/no-go call")),
            " It was last waiting on Alex: a go/no-go call."
        );
    }

    #[test]
    fn prompt_excerpt_truncates_char_safely() {
        let long = "x".repeat(200);
        let excerpt = prompt_excerpt(&long, 120);
        assert_eq!(excerpt.chars().count(), 121); // 120 + ellipsis
        let short = "short prompt";
        assert_eq!(prompt_excerpt(short, 120), short);
        // Multibyte safety: must not panic on char boundaries.
        let emoji = "🚀".repeat(200);
        assert_eq!(prompt_excerpt(&emoji, 120).chars().count(), 121);
    }
}
