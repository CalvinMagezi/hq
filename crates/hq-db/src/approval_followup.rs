//! Approvals that wait on the owner are repeated once and, if nobody answers, closed with a
//! message in the chat that asked, so a call that never ran is never mistaken for one that did.

use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use rusqlite::params;

use crate::Database;

/// The source of owner approval items.
const SOURCE: &str = "owner-approval";
/// Minutes before an unanswered approval is pushed a second time.
pub const REMIND_AFTER_MINUTES: i64 = 15;
/// Approval scopes of web chats read `web:<thread>`.
const WEB_SCOPE_PREFIX: &str = "web:";
/// Days an expired approval may still get its note, so an old backlog is not announced.
const NOTE_WINDOW_DAYS: i64 = 3;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub reminded: usize,
    pub expiry_noted: usize,
}

/// The chat thread a key `owner-approval:<scope>:<tool>:<digest>` was asked from, if a web chat.
fn thread_of(key: &str) -> Option<String> {
    let rest = key.strip_prefix(SOURCE)?.strip_prefix(':')?;
    let scope = rest.rsplitn(3, ':').nth(2)?;
    scope.strip_prefix(WEB_SCOPE_PREFIX).map(str::to_string)
}

pub fn run(db: &Database, now: DateTime<Utc>) -> Result<Report> {
    Ok(Report {
        reminded: remind(db, now)?,
        expiry_noted: note_expiries(db, now)?,
    })
}

/// Clears `pushed_at` once per waiting approval older than [`REMIND_AFTER_MINUTES`], so the push
/// dispatcher sends it again.
fn remind(db: &Database, now: DateTime<Utc>) -> Result<usize> {
    let before = (now - Duration::minutes(REMIND_AFTER_MINUTES)).to_rfc3339();
    db.with_conn(|c| {
        let ids: Vec<String> = c
            .prepare(
                "SELECT id FROM value_items WHERE source_task = ?1
                   AND state IN ('pending','routed','delivered') AND created_at <= ?2
                   AND (expires_at IS NULL OR expires_at > ?3)
                   AND id NOT IN (SELECT item_id FROM approval_followups WHERE reminded_at IS NOT NULL)",
            )?
            .query_map(params![SOURCE, before, now.to_rfc3339()], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        for id in &ids {
            c.execute(
                "INSERT INTO approval_followups (item_id, reminded_at) VALUES (?1, ?2)
                 ON CONFLICT(item_id) DO UPDATE SET reminded_at = excluded.reminded_at",
                params![id, now.to_rfc3339()],
            )?;
            c.execute("UPDATE value_items SET pushed_at = NULL WHERE id = ?1", params![id])?;
        }
        Ok(ids.len())
    })
}

/// Posts "nothing ran" in the chat of each approval that expired unanswered, once.
fn note_expiries(db: &Database, now: DateTime<Utc>) -> Result<usize> {
    let floor = (now - Duration::days(NOTE_WINDOW_DAYS)).to_rfc3339();
    db.with_conn(|c| {
        let rows: Vec<(String, String, Option<String>)> = c
            .prepare(
                "SELECT id, title, dedup_key FROM value_items WHERE source_task = ?1
                   AND (state = 'expired' OR (state IN ('pending','routed') AND expires_at < ?2))
                   AND created_at >= ?3
                   AND id NOT IN (SELECT item_id FROM approval_followups WHERE expiry_noted_at IS NOT NULL)",
            )?
            .query_map(params![SOURCE, now.to_rfc3339(), floor], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })?
            .collect::<rusqlite::Result<_>>()?;
        let mut noted = 0;
        for (id, title, key) in rows {
            if let Some(thread) = key.as_deref().and_then(thread_of) {
                let text = format!(
                    "{title}: nobody answered within 24 hours, so it expired and nothing ran. Ask again if you still want it."
                );
                crate::chat::add_message(c, &thread, "assistant", &text)?;
                noted += 1;
            }
            c.execute(
                "INSERT INTO approval_followups (item_id, expiry_noted_at) VALUES (?1, ?2)
                 ON CONFLICT(item_id) DO UPDATE SET expiry_noted_at = excluded.expiry_noted_at",
                params![id, now.to_rfc3339()],
            )?;
        }
        Ok(noted)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_core::types::{ValueItem, ValueKind};

    fn approval(db: &Database, thread: &str, age_minutes: i64, ttl_hours: i64) -> String {
        let key = format!("owner-approval:web:{thread}:bash:abc123");
        let mut item = ValueItem::new(SOURCE, ValueKind::ActionNeeded, "HQ wants to run bash", "b")
            .with_dedup_key(key)
            .owner_only(true);
        item.created_at = Utc::now() - Duration::minutes(age_minutes);
        item.expires_at = Some(item.created_at + Duration::hours(ttl_hours));
        crate::value_items::insert(db, &item).unwrap();
        crate::value_items::claim_push(db, &item.id).unwrap();
        item.id
    }

    fn pushed(db: &Database, id: &str) -> bool {
        db.with_conn(|c| {
            Ok(c.query_row(
                "SELECT pushed_at IS NOT NULL FROM value_items WHERE id = ?1",
                [id],
                |r| r.get(0),
            )?)
        })
        .unwrap()
    }

    #[test]
    fn a_waiting_approval_is_repeated_once_after_fifteen_minutes() {
        let db = Database::open_memory().unwrap();
        let fresh = approval(&db, "t1", 5, 24);
        let old = approval(&db, "t1", 20, 24);
        assert_eq!(run(&db, Utc::now()).unwrap().reminded, 1);
        assert!(pushed(&db, &fresh));
        assert!(
            !pushed(&db, &old),
            "pushed_at cleared so the dispatcher sends it again"
        );
        crate::value_items::claim_push(&db, &old).unwrap();
        assert_eq!(run(&db, Utc::now()).unwrap().reminded, 0, "once only");
    }

    #[test]
    fn an_expired_approval_says_in_its_chat_that_nothing_ran_once() {
        let db = Database::open_memory().unwrap();
        let thread = db
            .with_conn(|c| crate::chat::create_thread(c, "t", "user", "user"))
            .unwrap()
            .thread_id;
        approval(&db, &thread, 25 * 60, 24);
        assert_eq!(run(&db, Utc::now()).unwrap().expiry_noted, 1);
        let msgs = db
            .with_conn(|c| crate::chat::get_messages(c, &thread, 10))
            .unwrap();
        assert!(msgs.iter().any(|m| m.content.contains("nothing ran")));
        assert_eq!(run(&db, Utc::now()).unwrap().expiry_noted, 0, "once only");
    }

    #[test]
    fn keys_from_other_surfaces_name_no_web_thread() {
        assert_eq!(
            thread_of("owner-approval:web:t9:bash:ab").as_deref(),
            Some("t9")
        );
        assert_eq!(thread_of("owner-approval:telegram:7:bash:ab"), None);
        assert_eq!(thread_of("other:web:t9:bash:ab"), None);
    }
}
