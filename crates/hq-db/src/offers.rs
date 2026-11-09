//! The offers HQ showed under a reply and how the person answered (`offer_log`, migration 109).
//! "Not now" twice in a row mutes that kind of offer for 30 days.
//!
//! Adoption data: it never leaves the device and never reaches a model
//! (`docs/security/ADOPTION_DATA.md`). Only the offer's kind is stored, never the message.

use std::collections::{BTreeMap, HashMap, HashSet};

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};

use crate::adoption::DAY_SECS;

/// How long an offer stays quiet after the person said "Not now" to it twice in a row.
pub const MUTE_SECS: i64 = 30 * DAY_SECS;
/// "Not now" answers in a row that mute an offer.
const NOT_NOW_TO_MUTE: usize = 2;
/// The furthest ahead a next step can be scheduled.
pub const MAX_SCHEDULE_SECS: i64 = 90 * DAY_SECS;
/// Only next steps stand alone; a rule's offer says "that list" and means this conversation.
const SCHEDULABLE_PREFIX: &str = "next_";
/// What a scheduled step says when it comes back. Its request stays in the offer.
const BROUGHT_BACK: &str = "You asked me to bring this back.";

/// The person's answer to one offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Yes,
    NotNow,
}

impl Answer {
    pub fn as_str(self) -> &'static str {
        match self {
            Answer::Yes => "yes",
            Answer::NotNow => "not_now",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "yes" => Some(Answer::Yes),
            "not_now" => Some(Answer::NotNow),
            _ => None,
        }
    }
}

/// Notes that `offer` was shown under the reply `message_id`.
pub fn record_shown(conn: &Connection, message_id: &str, offer: &str, at: i64) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO offer_log (message_id, offer, shown_at) VALUES (?1, ?2, ?3)",
        params![message_id, offer, at],
    )?;
    Ok(())
}

/// Records the answer to the offer under `message_id`. Returns its kind, or `None` when that
/// reply has no offer still open (never shown, or answered already).
pub fn answer(
    conn: &Connection,
    message_id: &str,
    answer: Answer,
    at: i64,
) -> Result<Option<String>> {
    let offer: Option<String> = conn
        .query_row(
            "UPDATE offer_log SET answer = ?2, answered_at = ?3
              WHERE message_id = ?1 AND answer IS NULL
              RETURNING offer",
            params![message_id, answer.as_str(), at],
            |r| r.get(0),
        )
        .optional()?;
    if offer.is_some() {
        // The reply keeps only what was shown; how the person answered lives here alone.
        conn.execute(
            "UPDATE chat_messages SET meta = json_remove(meta, '$.offer')
              WHERE message_id = ?1 AND json_valid(meta)",
            params![message_id],
        )?;
    }
    Ok(offer)
}

/// Schedules the open offer under `message_id` to come back at `due_at`, from the reply's own saved
/// offer. Returns its kind, or `None` when that reply has no open offer that can be scheduled.
pub fn schedule(
    conn: &Connection,
    message_id: &str,
    due_at: i64,
    now: i64,
) -> Result<Option<String>> {
    let saved: Option<(String, String)> = conn
        .query_row(
            "SELECT thread_id, meta FROM chat_messages WHERE message_id = ?1 AND json_valid(meta)",
            params![message_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((thread_id, meta)) = saved else {
        return Ok(None);
    };
    let meta: serde_json::Value = serde_json::from_str(&meta)?;
    let offer = &meta["offer"];
    let (Some(kind), Some(line), Some(reply)) = (
        offer["kind"].as_str(),
        offer["line"].as_str(),
        offer["reply"].as_str(),
    ) else {
        return Ok(None);
    };
    if !kind.starts_with(SCHEDULABLE_PREFIX) {
        return Ok(None);
    }
    let tx = conn.unchecked_transaction()?;
    // In `offer_log` a scheduled step is a Yes: the person wanted it, just later.
    let answered = answer(&tx, message_id, Answer::Yes, now)?;
    if answered.is_some() {
        tx.execute(
            "INSERT INTO offer_schedule (thread_id, offer, line, reply, send, due_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                thread_id,
                kind,
                line,
                reply,
                offer["send"].as_bool().unwrap_or(false),
                due_at
            ],
        )?;
    }
    tx.commit()?;
    Ok(answered)
}

/// Brings back up to `limit` scheduled steps that are due, each as a new offer at the end of its
/// chat and exactly once. A step whose chat has been deleted is dropped. Returns how many came back.
pub fn deliver_due(conn: &Connection, now: i64, limit: usize) -> Result<usize> {
    let due = {
        let mut stmt = conn.prepare(
            "SELECT id, thread_id, offer, line, reply, send FROM offer_schedule
              WHERE due_at <= ?1 ORDER BY due_at LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![now, limit as i64], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, bool>(5)?,
            ))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()?
    };
    let mut delivered = 0;
    for (id, thread_id, kind, line, reply, send) in due {
        let tx = conn.unchecked_transaction()?;
        let chat_exists = tx
            .query_row(
                "SELECT 1 FROM chat_threads WHERE thread_id = ?1",
                params![thread_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if chat_exists {
            let meta = serde_json::json!({
                "offer": { "kind": kind, "line": line, "reply": reply, "send": send }
            });
            let message = crate::chat::add_message_with_meta(
                &tx,
                &thread_id,
                "assistant",
                BROUGHT_BACK,
                Some(&meta),
            )?;
            record_shown(&tx, &message.message_id, &kind, now)?;
            delivered += 1;
        }
        tx.execute("DELETE FROM offer_schedule WHERE id = ?1", params![id])?;
        tx.commit()?;
    }
    Ok(delivered)
}

/// When each kind of offer was last shown, for spacing offers out.
pub fn last_shown(conn: &Connection) -> Result<HashMap<String, i64>> {
    let mut stmt = conn.prepare("SELECT offer, MAX(shown_at) FROM offer_log GROUP BY offer")?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// The kinds of offer muted at `now`: the last two answers were "Not now", the latest within
/// [`MUTE_SECS`].
pub fn muted(conn: &Connection, now: i64) -> Result<HashSet<String>> {
    let mut stmt = conn.prepare(
        "SELECT offer, answer, answered_at FROM offer_log
          WHERE answer IS NOT NULL ORDER BY answered_at DESC, rowid DESC",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
        ))
    })?;
    let mut recent: HashMap<String, Vec<(String, i64)>> = HashMap::new();
    for row in rows {
        let (offer, answer, at) = row?;
        let answers = recent.entry(offer).or_default();
        if answers.len() < NOT_NOW_TO_MUTE {
            answers.push((answer, at));
        }
    }
    Ok(recent
        .into_iter()
        .filter(|(_, a)| {
            a.len() == NOT_NOW_TO_MUTE
                && a.iter()
                    .all(|(answer, _)| answer == Answer::NotNow.as_str())
                && now - a[0].1 < MUTE_SECS
        })
        .map(|(offer, _)| offer)
        .collect())
}

/// A task a turn worked on, as the next-step suggestion weighs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TouchedTask {
    pub title: String,
    pub description: String,
    pub status: String,
}

/// The open task changed most recently at or after `since`.
// ponytail: takes the newest change by anyone, so another agent's write during the turn can win.
// A wrong pick costs one dismissible line; record the turn's task ids if that shows up.
pub fn task_touched_since(conn: &Connection, since: i64) -> Result<Option<TouchedTask>> {
    Ok(conn
        .query_row(
            "SELECT title, description, status FROM tasks
              WHERE archived_at IS NULL AND status != 'complete' AND unixepoch(updated_at) >= ?1
              ORDER BY unixepoch(updated_at) DESC, rowid DESC LIMIT 1",
            params![since],
            |r| {
                Ok(TouchedTask {
                    title: r.get(0)?,
                    description: r.get(1)?,
                    status: r.get(2)?,
                })
            },
        )
        .optional()?)
}

/// One kind of offer over a window.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OfferStats {
    pub offer: String,
    pub shown: usize,
    pub accepted: usize,
    pub declined: usize,
    /// Muted at the end of the window.
    pub muted: bool,
}

/// Offers shown in `[from, until)`, per kind, with how they were answered, in kind order.
pub fn stats(conn: &Connection, from: i64, until: i64) -> Result<Vec<OfferStats>> {
    let mut stmt = conn.prepare(
        "SELECT offer, COUNT(*),
                SUM(answer = 'yes'), SUM(answer = 'not_now')
           FROM offer_log WHERE shown_at >= ?1 AND shown_at < ?2 GROUP BY offer",
    )?;
    let rows = stmt.query_map(params![from, until], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, Option<i64>>(2)?.unwrap_or(0),
            r.get::<_, Option<i64>>(3)?.unwrap_or(0),
        ))
    })?;
    let muted = muted(conn, until)?;
    let mut by: BTreeMap<String, OfferStats> = BTreeMap::new();
    for row in rows {
        let (offer, shown, yes, no) = row?;
        let count = |n: i64| usize::try_from(n).unwrap_or(0);
        by.insert(
            offer.clone(),
            OfferStats {
                muted: muted.contains(&offer),
                offer,
                shown: count(shown),
                accepted: count(yes),
                declined: count(no),
            },
        );
    }
    for offer in muted {
        by.entry(offer.clone()).or_insert(OfferStats {
            offer,
            muted: true,
            ..Default::default()
        });
    }
    Ok(by.into_values().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;

    const NOW: i64 = 1_800_000_000;

    #[test]
    fn an_answer_takes_the_offer_off_the_saved_reply_and_keeps_the_rest() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            let t = crate::chat::create_thread(c, "t", "user", "user")?;
            let meta = serde_json::json!({"offer": {"kind": "pasted_list"}, "tool_steps": []});
            let m = crate::chat::add_message_with_meta(
                c,
                &t.thread_id,
                "assistant",
                "ok",
                Some(&meta),
            )?;
            record_shown(c, &m.message_id, "pasted_list", NOW)?;
            answer(c, &m.message_id, Answer::Yes, NOW + 1)?;
            let kept: String = c.query_row(
                "SELECT meta FROM chat_messages WHERE message_id = ?1",
                params![m.message_id],
                |r| r.get(0),
            )?;
            let kept: serde_json::Value = serde_json::from_str(&kept)?;
            assert!(kept.get("offer").is_none());
            assert!(kept["tool_steps"].is_array());
            Ok(())
        })
        .unwrap();
    }

    fn offered_reply(c: &Connection, thread: &str, kind: &str) -> String {
        let meta = serde_json::json!({
            "offer": { "kind": kind, "line": "I can also look it up.", "reply": "Look it up.", "send": false }
        });
        let m =
            crate::chat::add_message_with_meta(c, thread, "assistant", "ok", Some(&meta)).unwrap();
        record_shown(c, &m.message_id, kind, NOW).unwrap();
        m.message_id
    }

    fn scheduled_count(c: &Connection) -> i64 {
        c.query_row("SELECT COUNT(*) FROM offer_schedule", [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn a_scheduled_step_comes_back_once_as_a_new_offer_in_its_chat() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            let t = crate::chat::create_thread(c, "t", "user", "user")?;
            let m = offered_reply(c, &t.thread_id, "next_look_up");
            assert_eq!(
                schedule(c, &m, NOW + 100, NOW)?.as_deref(),
                Some("next_look_up")
            );
            assert_eq!(schedule(c, &m, NOW + 100, NOW)?, None, "already answered");

            assert_eq!(deliver_due(c, NOW + 99, 10)?, 0, "not due yet");
            assert_eq!(deliver_due(c, NOW + 100, 10)?, 1);
            assert_eq!(deliver_due(c, NOW + 200, 10)?, 0, "only once");

            let (content, meta): (String, String) = c.query_row(
                "SELECT content, meta FROM chat_messages
                  WHERE thread_id = ?1 ORDER BY rowid DESC LIMIT 1",
                params![t.thread_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            let meta: serde_json::Value = serde_json::from_str(&meta)?;
            assert_eq!(content, BROUGHT_BACK);
            assert_eq!(meta["offer"]["reply"], "Look it up.");
            assert_eq!(meta["offer"]["send"], false);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn only_next_steps_can_be_scheduled_and_scheduling_is_no_refusal() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            let t = crate::chat::create_thread(c, "t", "user", "user")?;
            let rule = offered_reply(c, &t.thread_id, "pasted_list");
            assert_eq!(schedule(c, &rule, NOW + 100, NOW)?, None);
            assert_eq!(scheduled_count(c), 0);

            let first = offered_reply(c, &t.thread_id, "next_email");
            let second = offered_reply(c, &t.thread_id, "next_email");
            answer(c, &first, Answer::NotNow, NOW + 1)?;
            schedule(c, &second, NOW + 100, NOW + 2)?;
            assert!(
                muted(c, NOW + 3)?.is_empty(),
                "scheduling breaks a run of Not now"
            );
            let s = stats(c, NOW, NOW + 10)?;
            let email = s.iter().find(|o| o.offer == "next_email").unwrap();
            assert_eq!((email.accepted, email.declined), (1, 1));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_step_for_a_deleted_chat_is_dropped() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            c.execute(
                "INSERT INTO offer_schedule (thread_id, offer, line, reply, send, due_at)
                 VALUES ('gone', 'next_email', 'l', 'r', 0, ?1)",
                params![NOW],
            )?;
            assert_eq!(deliver_due(c, NOW, 10)?, 0);
            assert_eq!(scheduled_count(c), 0);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn not_now_twice_in_a_row_mutes_for_thirty_days_and_yes_breaks_the_run() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            record_shown(c, "m1", "pasted_list", NOW)?;
            record_shown(c, "m2", "pasted_list", NOW + 10)?;
            record_shown(c, "m3", "dated_todo", NOW + 20)?;
            assert_eq!(
                answer(c, "m1", Answer::NotNow, NOW + 1)?.as_deref(),
                Some("pasted_list")
            );
            assert_eq!(
                answer(c, "m1", Answer::Yes, NOW + 2)?,
                None,
                "answered once only"
            );
            assert_eq!(answer(c, "nope", Answer::Yes, NOW + 2)?, None);
            assert!(muted(c, NOW + 5)?.is_empty(), "one Not now is not a mute");
            answer(c, "m2", Answer::NotNow, NOW + 11)?;
            assert_eq!(
                muted(c, NOW + 12)?,
                HashSet::from(["pasted_list".to_string()])
            );
            assert!(muted(c, NOW + 11 + MUTE_SECS)?.is_empty(), "the mute ends");
            answer(c, "m3", Answer::Yes, NOW + 21)?;
            record_shown(c, "m4", "dated_todo", NOW + 30)?;
            answer(c, "m4", Answer::NotNow, NOW + 31)?;
            assert!(
                !muted(c, NOW + 32)?.contains("dated_todo"),
                "a Yes came before"
            );
            assert_eq!(last_shown(c)?["pasted_list"], NOW + 10);
            let s = stats(c, NOW, NOW + 100)?;
            assert_eq!(
                s,
                vec![
                    OfferStats {
                        offer: "dated_todo".into(),
                        shown: 2,
                        accepted: 1,
                        declined: 1,
                        muted: false
                    },
                    OfferStats {
                        offer: "pasted_list".into(),
                        shown: 2,
                        accepted: 0,
                        declined: 2,
                        muted: true
                    },
                ]
            );
            Ok(())
        })
        .unwrap();
    }
}
