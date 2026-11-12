//! The offers HQ showed under a reply and how the person answered (`offer_log`, migration 109).
//! What the person told HQ about a TYPE of offer lives in `offer_type_memory` (migration 128):
//! "Not now" hides the type for 7 days, the third in a row pauses it for 30, "Fewer like this"
//! doubles the window, "Never show" keeps it out until a Clear. "Later" parks the offer in a quiet
//! inbox item (`offer_later`) that brings it back to its chat once.
//!
//! Adoption data: it never leaves the device and never reaches a model
//! (`docs/security/ADOPTION_DATA.md`). Only the offer's kind is stored, never the message.

use std::collections::{BTreeMap, HashMap, HashSet};

use anyhow::Result;
use hq_core::types::ValueItem;
use rusqlite::{Connection, OptionalExtension, params};

use crate::adoption::DAY_SECS;

/// How long a type of offer stays out of the chooser after one "Not now".
pub const NOT_NOW_HIDE_SECS: i64 = 7 * DAY_SECS;
/// "Not now" answers in a row, on one type, that pause it.
pub const NOT_NOW_RUN_TO_PAUSE: i64 = 3;
/// How long the pause lasts.
pub const PAUSE_SECS: i64 = 30 * DAY_SECS;
/// How long "Fewer like this" lasts, and how many times longer a "Not now" hides in that time.
pub const FEWER_SECS: i64 = 30 * DAY_SECS;
const FEWER_FACTOR: i64 = 2;
/// The window the Settings counts cover.
pub const SUMMARY_DAYS: i64 = 30;

/// Each kind of offer and the capability (the pill's id in `capabilities.yaml`) it offers. Not now,
/// Never show and Fewer like this on either side apply to both. `hq-tools` checks this table
/// against its detectors and next steps, so it cannot drift.
pub const KIND_CAPABILITY: &[(&str, &str)] = &[
    ("photo_arrived", "read-photo"),
    ("document_arrived", "file-to-note"),
    ("pasted_list", "tasks-from-list"),
    ("dated_todo", "track-todo"),
    ("send_or_print", "note-to-file"),
    ("repeated_question", "look-up"),
    ("next_track_todo", "track-todo"),
    ("next_tasks_from_list", "tasks-from-list"),
    ("next_read_photo", "read-photo"),
    ("next_file_to_note", "file-to-note"),
    ("next_note_to_file", "note-to-file"),
    ("next_remember_people", "remember-people"),
    ("next_look_up", "look-up"),
    ("next_email", "email"),
    ("next_projects", "projects"),
    ("next_helpers", "helpers"),
];

/// The type an answer is remembered under: the capability id when the offer has one.
pub fn canonical(offer_type: &str) -> &str {
    KIND_CAPABILITY
        .iter()
        .find(|(kind, _)| *kind == offer_type)
        .map_or(offer_type, |(_, capability)| capability)
}

/// Whether `offer_type` is a kind in [`KIND_CAPABILITY`] or a capability it names.
pub fn is_mapped(offer_type: &str) -> bool {
    KIND_CAPABILITY
        .iter()
        .any(|(kind, capability)| *kind == offer_type || *capability == offer_type)
}
/// The furthest ahead a next step can be scheduled.
pub const MAX_SCHEDULE_SECS: i64 = 90 * DAY_SECS;
/// Only next steps stand alone; a rule's offer says "that list" and means this conversation.
const SCHEDULABLE_PREFIX: &str = "next_";
/// What a scheduled step says when it comes back. Its request stays in the offer.
const BROUGHT_BACK: &str = "You asked me to bring this back.";

/// The dedup key of the inbox item a Later offer leaves starts with this. The apps show an `open`
/// action ("Bring it back") on such items only. A client without that button never brings the
/// offer back: it waits until the item expires and is then dropped.
pub const LATER_ITEM_PREFIX: &str = "offer-later-";

/// Whether the value item `item_id` is the inbox item of a Later offer.
pub fn is_later_item(conn: &Connection, item_id: &str) -> Result<bool> {
    let key: Option<Option<String>> = conn
        .query_row(
            "SELECT dedup_key FROM value_items WHERE id = ?1 AND source_task = ?2",
            params![item_id, hq_core::types::SCHEDULED_OFFER_SOURCE],
            |r| r.get(0),
        )
        .optional()?;
    Ok(key
        .flatten()
        .is_some_and(|k| k.starts_with(LATER_ITEM_PREFIX)))
}

/// The person's answer to one offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Yes,
    NotNow,
    Later,
}

impl Answer {
    pub fn as_str(self) -> &'static str {
        match self {
            Answer::Yes => "yes",
            Answer::NotNow => "not_now",
            Answer::Later => "later",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "yes" => Some(Answer::Yes),
            "not_now" => Some(Answer::NotNow),
            "later" => Some(Answer::Later),
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

/// Like [`record_shown`] for an offer that came back (Later, scheduled). It must not count toward
/// the hourly cap or the days between offers, which gate fresh offers.
fn record_resurfaced(conn: &Connection, message_id: &str, offer: &str, at: i64) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO offer_log (message_id, offer, shown_at, resurfaced)
         VALUES (?1, ?2, ?3, 1)",
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
    if let Some(kind) = &offer {
        // The reply keeps only what was shown; how the person answered lives here alone.
        conn.execute(
            "UPDATE chat_messages SET meta = json_remove(meta, '$.offer')
              WHERE message_id = ?1 AND json_valid(meta)",
            params![message_id],
        )?;
        match answer {
            Answer::NotNow => note_not_now(conn, kind, at)?,
            Answer::Yes => end_run(conn, kind)?,
            Answer::Later => {}
        }
    }
    Ok(offer)
}

/// The offer a reply still carries in its saved copy.
struct SavedOffer {
    thread_id: String,
    kind: String,
    line: String,
    reply: String,
    send: bool,
    /// It already came back once from Later, so Later is not offered again.
    later_used: bool,
}

fn saved_offer(conn: &Connection, message_id: &str) -> Result<Option<SavedOffer>> {
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
    Ok(Some(SavedOffer {
        thread_id,
        kind: kind.to_string(),
        line: line.to_string(),
        reply: reply.to_string(),
        send: offer["send"].as_bool().unwrap_or(false),
        later_used: offer["later_used"].as_bool().unwrap_or(false),
    }))
}

/// Schedules the open offer under `message_id` to come back at `due_at`, from the reply's own saved
/// offer. Returns its kind, or `None` when that reply has no open offer that can be scheduled.
pub fn schedule(
    conn: &Connection,
    message_id: &str,
    due_at: i64,
    now: i64,
) -> Result<Option<String>> {
    let Some(offer) = saved_offer(conn, message_id)? else {
        return Ok(None);
    };
    if !offer.kind.starts_with(SCHEDULABLE_PREFIX) {
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
                offer.thread_id,
                offer.kind,
                offer.line,
                offer.reply,
                offer.send,
                due_at
            ],
        )?;
    }
    tx.commit()?;
    Ok(answered)
}

/// What [`later`] did.
#[derive(Debug, PartialEq, Eq)]
pub enum LaterOutcome {
    /// Parked; carries the offer's kind.
    Parked(String),
    /// That reply has no open offer.
    NoOffer,
    /// The offer already came back from Later once; it cannot be parked again.
    AlreadyUsed,
}

/// Parks the open offer under `message_id` until the person answers the inbox item
/// `item_for(kind)` builds, which brings the offer back to its chat once. The item is written in
/// the same transaction as the parking row, so a parked offer always has its item, and a failure
/// leaves the offer open to try again. Any kind of offer can wait, once.
pub fn later(
    conn: &Connection,
    message_id: &str,
    item_for: impl FnOnce(&str) -> ValueItem,
    now: i64,
) -> Result<LaterOutcome> {
    let Some(offer) = saved_offer(conn, message_id)? else {
        return Ok(LaterOutcome::NoOffer);
    };
    if offer.later_used {
        return Ok(LaterOutcome::AlreadyUsed);
    }
    let tx = conn.unchecked_transaction()?;
    if answer(&tx, message_id, Answer::Later, now)?.is_none() {
        return Ok(LaterOutcome::NoOffer);
    }
    let item = item_for(&offer.kind);
    crate::value_items::insert_on(&tx, &item)?;
    tx.execute(
        "INSERT INTO offer_later (item_id, thread_id, message_id, offer, line, reply, send, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            item.id,
            offer.thread_id,
            message_id,
            offer.kind,
            offer.line,
            offer.reply,
            offer.send,
            now
        ],
    )?;
    tx.commit()?;
    Ok(LaterOutcome::Parked(offer.kind))
}

/// Puts an offer at the end of its chat as a new reply, if the chat still exists.
fn put_back(
    conn: &Connection,
    thread_id: &str,
    (kind, line, reply, send): (&str, &str, &str, bool),
    now: i64,
) -> Result<bool> {
    // What the person has since hidden or turned off does not come back.
    if muted(conn, now)?.contains(kind) {
        return Ok(false);
    }
    let chat_exists = conn
        .query_row(
            "SELECT 1 FROM chat_threads WHERE thread_id = ?1",
            params![thread_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if !chat_exists {
        return Ok(false);
    }
    let meta = serde_json::json!({
        "offer": {
            "kind": kind, "line": line, "reply": reply, "send": send, "later_used": true
        }
    });
    let message = crate::chat::add_message_with_meta(
        conn,
        thread_id,
        "assistant",
        BROUGHT_BACK,
        Some(&meta),
    )?;
    record_resurfaced(conn, &message.message_id, kind, now)?;
    Ok(true)
}

type LaterRow = (String, String, String, String, String, bool);

/// Brings a Later offer back to its chat and removes its row. The row's own item is marked
/// answered so the inbox stops showing it.
fn deliver_later_row(conn: &Connection, row: &LaterRow, now: i64) -> Result<bool> {
    let (item_id, thread_id, offer, line, reply, send) = row;
    let tx = conn.unchecked_transaction()?;
    // Whoever deletes the row owns the return, so two callers never bring it back twice.
    let owned = tx.execute(
        "DELETE FROM offer_later WHERE item_id = ?1",
        params![item_id],
    )? == 1;
    let back = owned && put_back(&tx, thread_id, (offer, line, reply, *send), now)?;
    if owned {
        tx.execute(
            "UPDATE value_items SET state = 'engaged', engagement = 'opened', engaged_at = ?2
              WHERE id = ?1 AND state IN ('pending','routed','delivered')",
            params![
                item_id,
                chrono::DateTime::from_timestamp(now, 0).map(|d| d.to_rfc3339())
            ],
        )?;
    }
    tx.commit()?;
    Ok(back)
}

/// The parked Later offers, with the state of their inbox item (`None` when it is gone), for one
/// reply or, with `None`, all of them.
fn later_rows(
    conn: &Connection,
    message_id: Option<&str>,
) -> Result<Vec<(LaterRow, Option<String>)>> {
    let mut stmt = conn.prepare(
        "SELECT l.item_id, l.thread_id, l.offer, l.line, l.reply, l.send, v.state
           FROM offer_later l LEFT JOIN value_items v ON v.id = l.item_id
          WHERE ?1 IS NULL OR l.message_id = ?1",
    )?;
    let rows = stmt.query_map(params![message_id], |r| {
        Ok((
            (
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ),
            r.get(6)?,
        ))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// The chat side of a Later offer: the person answers it from the chat, so it returns now.
/// `None` when no Later offer is parked for that reply.
pub fn bring_back(conn: &Connection, message_id: &str, now: i64) -> Result<Option<String>> {
    let Some((row, _)) = later_rows(conn, Some(message_id))?.into_iter().next() else {
        return Ok(None);
    };
    deliver_later_row(conn, &row, now)?;
    Ok(Some(row.2))
}

/// Settles the parked Later offers once their inbox item has an answer: opened means the offer
/// comes back to its chat; dismissed, expired or gone means it is dropped. Each comes back at most
/// once, because its row goes either way. Returns how many came back.
pub fn deliver_later(conn: &Connection, now: i64) -> Result<usize> {
    let mut delivered = 0;
    for (row, state) in later_rows(conn, None)? {
        match state.as_deref() {
            Some("pending" | "routed" | "delivered") => {}
            Some("engaged") => match deliver_later_row(conn, &row, now) {
                Ok(true) => delivered += 1,
                Ok(false) => {}
                // One bad row must not starve the rest of the pass; it is tried again next time.
                Err(e) => tracing::warn!("offers: could not bring a Later offer back: {e}"),
            },
            _ => {
                if let Err(e) =
                    conn.execute("DELETE FROM offer_later WHERE item_id = ?1", params![row.0])
                {
                    tracing::warn!("offers: could not drop a Later offer: {e}");
                }
            }
        }
    }
    Ok(delivered)
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
        if put_back(&tx, &thread_id, (&kind, &line, &reply, send), now)? {
            delivered += 1;
        }
        tx.execute("DELETE FROM offer_schedule WHERE id = ?1", params![id])?;
        tx.commit()?;
    }
    Ok(delivered)
}

/// When each kind of offer was last shown, for spacing offers out.
pub fn last_shown(conn: &Connection) -> Result<HashMap<String, i64>> {
    let mut stmt = conn.prepare(
        "SELECT offer, MAX(shown_at) FROM offer_log WHERE resurfaced = 0 GROUP BY offer",
    )?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Notes a "Not now" on `offer` at `at`: the type hides for [`NOT_NOW_HIDE_SECS`] (doubled while
/// "Fewer like this" lasts); the [`NOT_NOW_RUN_TO_PAUSE`]th in a row pauses it for [`PAUSE_SECS`].
fn note_not_now(conn: &Connection, offer: &str, at: i64) -> Result<()> {
    let offer = canonical(offer);
    let (run, fewer_until, hidden): (i64, i64, i64) = conn
        .query_row(
            "SELECT not_now_run, fewer_until, hidden_until FROM offer_type_memory WHERE offer = ?1",
            params![offer],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?
        .unwrap_or((0, 0, 0));
    let run = run + 1;
    // The pause is not doubled by Fewer. An older offer's Not now never shortens a longer hide.
    let (run, hidden_until) = if run >= NOT_NOW_RUN_TO_PAUSE {
        (0, at + PAUSE_SECS)
    } else {
        let factor = if fewer_until > at { FEWER_FACTOR } else { 1 };
        (run, at + NOT_NOW_HIDE_SECS * factor)
    };
    let hidden_until = hidden_until.max(hidden);
    conn.execute(
        "INSERT INTO offer_type_memory (offer, not_now_run, hidden_until) VALUES (?1, ?2, ?3)
         ON CONFLICT(offer) DO UPDATE SET not_now_run = ?2, hidden_until = ?3",
        params![offer, run, hidden_until],
    )?;
    Ok(())
}

/// A Yes ends a run of "Not now".
fn end_run(conn: &Connection, offer: &str) -> Result<()> {
    let offer = canonical(offer);
    conn.execute(
        "UPDATE offer_type_memory SET not_now_run = 0 WHERE offer = ?1",
        params![offer],
    )?;
    Ok(())
}

/// "Never show": the type stays out of the chooser until [`clear`].
pub fn set_never(conn: &Connection, offer: &str, at: i64) -> Result<()> {
    let offer = canonical(offer);
    conn.execute(
        "INSERT INTO offer_type_memory (offer, never, never_at) VALUES (?1, 1, ?2)
         ON CONFLICT(offer) DO UPDATE SET never = 1, never_at = ?2",
        params![offer, at],
    )?;
    Ok(())
}

/// "Fewer like this": for [`FEWER_SECS`] a "Not now" hides the type twice as long.
pub fn set_fewer(conn: &Connection, offer: &str, at: i64) -> Result<()> {
    let offer = canonical(offer);
    conn.execute(
        "INSERT INTO offer_type_memory (offer, fewer_until) VALUES (?1, ?2)
         ON CONFLICT(offer) DO UPDATE SET fewer_until = ?2",
        params![offer, at + FEWER_SECS],
    )?;
    Ok(())
}

/// Forgets every choice, parked Later offers and scheduled steps, and stops the answered offers
/// counting in Settings. The log rows stay, stamped, so the pacing (the hourly cap, the days
/// between offers) survives a Clear.
pub fn clear(conn: &Connection, now: i64) -> Result<()> {
    conn.execute("DELETE FROM offer_type_memory", [])?;
    conn.execute("DELETE FROM offer_later", [])?;
    conn.execute("DELETE FROM offer_schedule", [])?;
    conn.execute(
        "UPDATE offer_log SET cleared_at = ?1 WHERE cleared_at IS NULL",
        params![now],
    )?;
    Ok(())
}

/// The types out of the chooser at `now`: hidden by a Not now or a pause, or never. Both sides
/// of [`KIND_CAPABILITY`] are listed, so a caller may look up a kind or a capability id.
pub fn muted(conn: &Connection, now: i64) -> Result<HashSet<String>> {
    let mut stmt =
        conn.prepare("SELECT offer FROM offer_type_memory WHERE never = 1 OR hidden_until > ?1")?;
    let rows = stmt.query_map(params![now], |r| r.get::<_, String>(0))?;
    let mut out = HashSet::new();
    for offer in rows {
        let offer = offer?;
        out.extend(
            KIND_CAPABILITY
                .iter()
                .filter(|(_, capability)| *capability == offer)
                .map(|(kind, _)| kind.to_string()),
        );
        out.insert(offer);
    }
    Ok(out)
}

/// Offers shown in one type over [`SUMMARY_DAYS`], for Settings. Counts only.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TypeSummary {
    pub offer: String,
    pub shown: usize,
    /// Answered Yes.
    pub tapped: usize,
    pub not_now: usize,
    pub later: usize,
    /// 1 when "Never show" is on for the type.
    pub never: usize,
}

/// Per type (one row for an offer kind and its capability together): how many offers were shown in
/// the last [`SUMMARY_DAYS`] and how they were answered, plus types set to never. A scheduled step
/// counts as a Yes: the person wanted it, just later. Later is its own count. Offers that came back
/// count again as shown. Rows a Clear stamped are left out. In type order.
pub fn summary(conn: &Connection, now: i64) -> Result<Vec<TypeSummary>> {
    let from = now - SUMMARY_DAYS * DAY_SECS;
    let mut stmt = conn.prepare(
        "SELECT offer, COUNT(*), SUM(answer = 'yes'), SUM(answer = 'not_now'), SUM(answer = 'later')
           FROM offer_log WHERE shown_at >= ?1 AND cleared_at IS NULL GROUP BY offer",
    )?;
    let rows = stmt.query_map(params![from], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, Option<i64>>(2)?.unwrap_or(0),
            r.get::<_, Option<i64>>(3)?.unwrap_or(0),
            r.get::<_, Option<i64>>(4)?.unwrap_or(0),
        ))
    })?;
    let count = |n: i64| usize::try_from(n).unwrap_or(0);
    let mut by: BTreeMap<String, TypeSummary> = BTreeMap::new();
    for row in rows {
        let (offer, shown, yes, no, later) = row?;
        let offer = canonical(&offer).to_string();
        let t = by.entry(offer.clone()).or_insert_with(|| TypeSummary {
            offer,
            ..Default::default()
        });
        t.shown += count(shown);
        t.tapped += count(yes);
        t.not_now += count(no);
        t.later += count(later);
    }
    let mut stmt = conn.prepare("SELECT offer FROM offer_type_memory WHERE never = 1")?;
    for offer in stmt.query_map([], |r| r.get::<_, String>(0))? {
        let offer = offer?;
        by.entry(offer.clone())
            .or_insert_with(|| TypeSummary {
                offer,
                ..Default::default()
            })
            .never = 1;
    }
    Ok(by.into_values().collect())
}

/// How many offers were shown at or after `since`, for the hourly limit.
pub fn shown_since(conn: &Connection, since: i64) -> Result<usize> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM offer_log WHERE shown_at >= ?1 AND resurfaced = 0",
        params![since],
        |r| r.get(0),
    )?;
    Ok(usize::try_from(n).unwrap_or(0))
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
    /// Hidden at the end of the window: a Not now within the week, a 30-day pause, or Never.
    pub muted: bool,
}

/// Offers shown in `[from, until)`, per kind, with how they were answered, in kind order.
pub fn stats(conn: &Connection, from: i64, until: i64) -> Result<Vec<OfferStats>> {
    let mut stmt = conn.prepare(
        "SELECT offer, COUNT(*),
                SUM(answer = 'yes'), SUM(answer = 'not_now')
           FROM offer_log WHERE shown_at >= ?1 AND shown_at < ?2 AND cleared_at IS NULL
          GROUP BY offer",
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
                muted(c, NOW + 3)?.contains("next_email"),
                "the Not now hid the type; the schedule is a Yes and ends the run"
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
    fn not_now_hides_a_type_for_a_week_and_the_third_in_a_row_pauses_it_for_a_month() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            for (i, id) in ["m1", "m2", "m3", "m4"].iter().enumerate() {
                record_shown(c, id, "pasted_list", NOW + i as i64)?;
            }
            record_shown(c, "y1", "dated_todo", NOW)?;
            assert_eq!(
                answer(c, "m1", Answer::NotNow, NOW + 1)?.as_deref(),
                Some("pasted_list")
            );
            assert_eq!(answer(c, "m1", Answer::Yes, NOW + 2)?, None, "once only");
            assert_eq!(answer(c, "nope", Answer::Yes, NOW + 2)?, None);
            assert!(
                muted(c, NOW + 5)?.contains("pasted_list"),
                "one Not now hides the type"
            );
            assert!(muted(c, NOW + 1 + NOT_NOW_HIDE_SECS)?.is_empty(), "a week");
            assert!(
                !muted(c, NOW + 1 + NOT_NOW_HIDE_SECS - 1)?.is_empty(),
                "not a second sooner"
            );

            // A Yes on another type changes nothing here; a second Not now is still a week.
            answer(c, "y1", Answer::Yes, NOW + 3)?;
            answer(c, "m2", Answer::NotNow, NOW + 11)?;
            assert!(muted(c, NOW + 11 + NOT_NOW_HIDE_SECS)?.is_empty());

            // The third in a row pauses for a month.
            answer(c, "m3", Answer::NotNow, NOW + 21)?;
            assert!(!muted(c, NOW + 21 + NOT_NOW_HIDE_SECS)?.is_empty());
            assert!(
                muted(c, NOW + 21 + PAUSE_SECS)?.is_empty(),
                "the pause ends"
            );
            // After a pause the count starts again, but a later Not now (an older open offer)
            // cannot shorten the pause that is running.
            answer(c, "m4", Answer::NotNow, NOW + 31)?;
            assert!(!muted(c, NOW + 31 + NOT_NOW_HIDE_SECS)?.is_empty());
            assert!(muted(c, NOW + 21 + PAUSE_SECS)?.is_empty());
            assert_eq!(last_shown(c)?["pasted_list"], NOW + 3);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_yes_breaks_a_run_of_not_nows() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            for id in ["a", "b", "c", "d"] {
                record_shown(c, id, "dated_todo", NOW)?;
            }
            answer(c, "a", Answer::NotNow, NOW)?;
            answer(c, "b", Answer::NotNow, NOW + 1)?;
            answer(c, "c", Answer::Yes, NOW + 2)?;
            answer(c, "d", Answer::NotNow, NOW + 3)?;
            assert!(
                muted(c, NOW + 3 + NOT_NOW_HIDE_SECS)?.is_empty(),
                "the third Not now was not in a row"
            );
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn fewer_doubles_the_window_never_stays_and_clear_forgets() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            record_shown(c, "a", "dated_todo", NOW)?;
            record_shown(c, "b", "dated_todo", NOW)?;
            set_fewer(c, "dated_todo", NOW)?;
            answer(c, "a", Answer::NotNow, NOW + 1)?;
            let doubled = NOW + 1 + NOT_NOW_HIDE_SECS * FEWER_FACTOR;
            assert!(!muted(c, doubled - 1)?.is_empty());
            assert!(muted(c, doubled)?.is_empty());

            // Fewer ends after its own window: a later Not now is a plain week.
            answer(c, "b", Answer::NotNow, NOW + FEWER_SECS + 5)?;
            let plain = NOW + FEWER_SECS + 5 + NOT_NOW_HIDE_SECS;
            assert!(muted(c, plain)?.is_empty());

            set_never(c, "find-notes", NOW)?;
            assert!(muted(c, NOW + 400 * DAY_SECS)?.contains("find-notes"));
            let s = summary(c, NOW + 10)?;
            assert_eq!(s.iter().find(|t| t.offer == "find-notes").unwrap().never, 1);

            record_shown(c, "open", "pasted_list", NOW)?;
            clear(c, NOW + 10)?;
            assert!(muted(c, NOW)?.is_empty());
            assert!(
                summary(c, NOW + 10)?.is_empty(),
                "nothing counts after a Clear"
            );
            assert_eq!(
                last_shown(c)?["pasted_list"],
                NOW,
                "the pacing survives a Clear"
            );
            assert_eq!(shown_since(c, NOW - 10)?, 3, "so does the hourly count");
            assert_eq!(
                answer(c, "open", Answer::Yes, NOW + 20)?.as_deref(),
                Some("pasted_list"),
                "an open offer still answers"
            );
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn the_summary_counts_the_last_thirty_days_per_type() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            let old = NOW - (SUMMARY_DAYS + 1) * DAY_SECS;
            record_shown(c, "o", "dated_todo", old)?;
            record_shown(c, "y", "dated_todo", NOW - DAY_SECS)?;
            record_shown(c, "n", "dated_todo", NOW - DAY_SECS)?;
            record_shown(c, "l", "pasted_list", NOW - DAY_SECS)?;
            record_shown(c, "u", "pasted_list", NOW - DAY_SECS)?;
            answer(c, "o", Answer::Yes, old)?;
            answer(c, "y", Answer::Yes, NOW)?;
            answer(c, "n", Answer::NotNow, NOW)?;
            answer(c, "l", Answer::Later, NOW)?;
            assert_eq!(
                summary(c, NOW)?,
                vec![
                    TypeSummary {
                        offer: "tasks-from-list".into(),
                        shown: 2,
                        tapped: 0,
                        not_now: 0,
                        later: 1,
                        never: 0
                    },
                    TypeSummary {
                        offer: "track-todo".into(),
                        shown: 2,
                        tapped: 1,
                        not_now: 1,
                        later: 0,
                        never: 0
                    },
                ]
            );
            assert_eq!(shown_since(c, NOW - 3600)?, 0);
            assert_eq!(shown_since(c, NOW - 2 * DAY_SECS)?, 4);
            Ok(())
        })
        .unwrap();
    }

    fn later_item(id: &str) -> ValueItem {
        let mut item = ValueItem::new(
            hq_core::types::SCHEDULED_OFFER_SOURCE,
            hq_core::types::ValueKind::Fyi,
            "Still want to?",
            "b",
        );
        item.id = id.to_string();
        item
    }

    fn park(c: &Connection, message: &str, id: &str) -> LaterOutcome {
        later(c, message, |_| later_item(id), NOW).unwrap()
    }

    fn set_state(c: &Connection, id: &str, state: &str) {
        c.execute(
            "UPDATE value_items SET state = ?2 WHERE id = ?1",
            params![id, state],
        )
        .unwrap();
    }

    fn chat_offers(c: &Connection, thread: &str) -> i64 {
        c.query_row(
            "SELECT COUNT(*) FROM chat_messages WHERE thread_id = ?1 AND content = ?2",
            params![thread, BROUGHT_BACK],
            |r| r.get(0),
        )
        .unwrap()
    }

    #[test]
    fn later_parks_any_offer_and_it_comes_back_once_when_the_item_is_answered() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            let t = crate::chat::create_thread(c, "t", "user", "user")?;
            let m = offered_reply(c, &t.thread_id, "pasted_list");
            assert_eq!(
                park(c, &m, "item-1"),
                LaterOutcome::Parked("pasted_list".into())
            );
            assert_eq!(
                park(c, &m, "item-2"),
                LaterOutcome::NoOffer,
                "answered already"
            );
            assert_eq!(summary(c, NOW)?[0].later, 1);
            assert!(muted(c, NOW)?.is_empty(), "Later hides nothing");

            // While the item waits, nothing comes back.
            assert_eq!(deliver_later(c, NOW + 10)?, 0);
            assert_eq!(chat_offers(c, &t.thread_id), 0);

            // Answered from the inbox: the offer returns to the chat, once.
            set_state(c, "item-1", "engaged");
            assert_eq!(deliver_later(c, NOW + 20)?, 1);
            assert_eq!(deliver_later(c, NOW + 30)?, 0, "no second resurfacing");
            assert_eq!(chat_offers(c, &t.thread_id), 1);
            let meta: String = c.query_row(
                "SELECT meta FROM chat_messages WHERE thread_id = ?1 ORDER BY rowid DESC LIMIT 1",
                params![t.thread_id],
                |r| r.get(0),
            )?;
            let meta: serde_json::Value = serde_json::from_str(&meta)?;
            assert_eq!(meta["offer"]["kind"], "pasted_list");
            assert_eq!(meta["offer"]["reply"], "Look it up.");
            assert_eq!(meta["offer"]["later_used"], true);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn an_offer_that_came_back_from_later_cannot_be_parked_again() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            let t = crate::chat::create_thread(c, "t", "user", "user")?;
            let m = offered_reply(c, &t.thread_id, "pasted_list");
            park(c, &m, "i1");
            set_state(c, "i1", "engaged");
            deliver_later(c, NOW + 5)?;
            let back: String = c.query_row(
                "SELECT message_id FROM chat_messages WHERE thread_id = ?1 ORDER BY rowid DESC LIMIT 1",
                params![t.thread_id],
                |r| r.get(0),
            )?;
            assert_eq!(park(c, &back, "i2"), LaterOutcome::AlreadyUsed);
            // It can still be answered Yes or Not now.
            assert!(answer(c, &back, Answer::Yes, NOW + 6)?.is_some());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_failed_item_leaves_the_offer_open_and_nothing_parked() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            let t = crate::chat::create_thread(c, "t", "user", "user")?;
            let m = offered_reply(c, &t.thread_id, "pasted_list");
            park(c, &m, "dup");
            let n = offered_reply(c, &t.thread_id, "dated_todo");
            // The same item id again fails the insert: the whole step rolls back.
            assert!(later(c, &n, |_| later_item("dup"), NOW).is_err());
            let parked: i64 = c.query_row("SELECT COUNT(*) FROM offer_later", [], |r| r.get(0))?;
            assert_eq!(parked, 1);
            let open: Option<String> = c.query_row(
                "SELECT answer FROM offer_log WHERE message_id = ?1",
                params![n],
                |r| r.get(0),
            )?;
            assert_eq!(open, None, "still open, so a retry works");
            assert_eq!(
                park(c, &n, "fresh"),
                LaterOutcome::Parked("dated_todo".into())
            );
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_dismissed_or_gone_later_item_is_dropped_and_the_chat_side_brings_it_back_now() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            let t = crate::chat::create_thread(c, "t", "user", "user")?;
            let dismissed = offered_reply(c, &t.thread_id, "pasted_list");
            let gone = offered_reply(c, &t.thread_id, "dated_todo");
            let from_chat = offered_reply(c, &t.thread_id, "send_or_print");
            park(c, &dismissed, "i-dismissed");
            park(c, &gone, "i-gone");
            park(c, &from_chat, "i-chat");
            set_state(c, "i-dismissed", "dismissed");
            c.execute("DELETE FROM value_items WHERE id = 'i-gone'", [])?;
            set_state(c, "i-chat", "delivered");
            assert_eq!(deliver_later(c, NOW + 10)?, 0);
            assert_eq!(chat_offers(c, &t.thread_id), 0);
            let parked: i64 = c.query_row("SELECT COUNT(*) FROM offer_later", [], |r| r.get(0))?;
            assert_eq!(parked, 1, "only the one still waiting is parked");

            assert_eq!(
                bring_back(c, &from_chat, NOW + 20)?.as_deref(),
                Some("send_or_print")
            );
            assert_eq!(bring_back(c, &from_chat, NOW + 21)?, None, "once");
            assert_eq!(chat_offers(c, &t.thread_id), 1);
            let state: String = c.query_row(
                "SELECT state FROM value_items WHERE id = 'i-chat'",
                [],
                |r| r.get(0),
            )?;
            assert_eq!(state, "engaged", "the inbox item is settled too");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn two_callers_bring_a_later_offer_back_exactly_once() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(&dir.path().join("v.db")).unwrap();
        let (thread, message) = db
            .with_conn(|c| {
                let t = crate::chat::create_thread(c, "t", "user", "user")?;
                let m = offered_reply(c, &t.thread_id, "pasted_list");
                park(c, &m, "item");
                Ok((t.thread_id, m))
            })
            .unwrap();
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let (db, message) = (db.clone(), message.clone());
                std::thread::spawn(move || db.with_conn(|c| bring_back(c, &message, NOW + 5)))
            })
            .collect();
        for h in handles {
            h.join().unwrap().ok();
        }
        let n = db.with_conn(|c| Ok(chat_offers(c, &thread))).unwrap();
        assert_eq!(n, 1, "one chat message, not two");
    }

    #[test]
    fn what_was_hidden_since_does_not_come_back() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            let t = crate::chat::create_thread(c, "t", "user", "user")?;
            let m = offered_reply(c, &t.thread_id, "dated_todo");
            park(c, &m, "i");
            set_state(c, "i", "engaged");
            set_never(c, "track-todo", NOW + 1)?;
            assert_eq!(deliver_later(c, NOW + 5)?, 0, "never wins over Later");
            assert_eq!(chat_offers(c, &t.thread_id), 0);
            let parked: i64 = c.query_row("SELECT COUNT(*) FROM offer_later", [], |r| r.get(0))?;
            assert_eq!(parked, 0, "and the item is settled, not retried");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_returned_offer_does_not_count_toward_the_cap_for_fresh_ones() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            let t = crate::chat::create_thread(c, "t", "user", "user")?;
            let m = offered_reply(c, &t.thread_id, "dated_todo");
            park(c, &m, "i");
            set_state(c, "i", "engaged");
            let before = shown_since(c, NOW - 3600)?;
            let last = last_shown(c)?["dated_todo"];
            deliver_later(c, NOW + 5)?;
            assert_eq!(shown_since(c, NOW - 3600)?, before);
            assert_eq!(last_shown(c)?["dated_todo"], last);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn an_older_not_now_never_shortens_a_longer_hide_and_fewer_does_not_double_a_pause() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            for id in ["a", "b", "c", "d"] {
                record_shown(c, id, "dated_todo", NOW)?;
            }
            set_fewer(c, "dated_todo", NOW)?;
            answer(c, "a", Answer::NotNow, NOW + 1)?;
            answer(c, "b", Answer::NotNow, NOW + 2)?;
            answer(c, "c", Answer::NotNow, NOW + 3)?; // the third: a pause
            let pause_end = NOW + 3 + PAUSE_SECS;
            assert!(!muted(c, pause_end - 1)?.is_empty());
            assert!(
                muted(c, pause_end)?.is_empty(),
                "Fewer does not double a pause"
            );
            // An older open offer answered later cannot shorten it.
            answer(c, "d", Answer::NotNow, NOW + 4)?;
            assert!(!muted(c, pause_end - 1)?.is_empty());
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_kind_and_its_capability_share_one_memory_and_one_summary_row() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            assert_eq!(canonical("dated_todo"), "track-todo");
            assert_eq!(canonical("track-todo"), "track-todo");
            assert!(is_mapped("dated_todo") && is_mapped("track-todo"));
            assert!(!is_mapped("nope"));
            for id in ["a", "b", "c"] {
                record_shown(c, id, "dated_todo", NOW)?;
                answer(c, id, Answer::NotNow, NOW + 1)?;
            }
            let hidden = muted(c, NOW + 2)?;
            assert!(hidden.contains("track-todo"), "the pill is hidden");
            assert!(
                hidden.contains("next_track_todo"),
                "and every kind it feeds"
            );

            set_never(c, "find-notes", NOW)?; // a pill with no kind
            set_never(c, "pasted_list", NOW)?; // a kind: stored under its capability
            let hidden = muted(c, NOW + 2)?;
            assert!(hidden.contains("tasks-from-list") && hidden.contains("pasted_list"));

            record_shown(c, "x", "next_track_todo", NOW)?;
            let rows: Vec<_> = summary(c, NOW + 2)?
                .into_iter()
                .filter(|t| t.offer == "track-todo")
                .collect();
            assert_eq!(rows.len(), 1, "one row per type");
            assert_eq!((rows[0].shown, rows[0].not_now), (4, 3));
            Ok(())
        })
        .unwrap();
    }
}
