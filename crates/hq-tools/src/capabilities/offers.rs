//! The detectors behind "I can also do this for you" (phase 2 of `docs/plans/simple-hq.md`):
//! deterministic checks on what the person just wrote that say a capability would help now.
//! No model decides an offer, and nothing here leaves the device.
//!
//! [`replay`] runs them over past chats and returns counts only, so a detector can be tuned on
//! real history before the apps ever show an offer.

use std::collections::HashMap;

use anyhow::Result;
use hq_db::adoption::{DAY_SECS, OWNER_AGENT, UserMessage, user_messages_since};
use rusqlite::{Connection, params};
use serde::Serialize;

/// One kind of moment at which a capability would help.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Detector {
    /// A photo came with the message: its text can be read.
    PhotoArrived,
    /// A document came with the message: it can be kept as a note to search.
    DocumentArrived,
    /// Three or more list items: they can become tasks.
    PastedList,
    /// Something to do on a day: it can be a task with that due date.
    DatedTodo,
    /// Something to send or print: a note can become a PDF or Word file.
    SendOrPrint,
    /// The same question for the third time in two weeks: HQ can look it up on a schedule.
    RepeatedQuestion,
}

impl Detector {
    /// In the order a turn picks one: at most one offer per reply.
    pub const ALL: [Detector; 6] = [
        Detector::PhotoArrived,
        Detector::DocumentArrived,
        Detector::PastedList,
        Detector::DatedTodo,
        Detector::SendOrPrint,
        Detector::RepeatedQuestion,
    ];

    /// The catalog entry it offers (`assets/capabilities.yaml`).
    pub fn capability(self) -> &'static str {
        match self {
            Detector::PhotoArrived => "read-photo",
            Detector::DocumentArrived => "file-to-note",
            Detector::PastedList => "tasks-from-list",
            Detector::DatedTodo => "track-todo",
            Detector::SendOrPrint => "note-to-file",
            Detector::RepeatedQuestion => "look-up",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Detector::PhotoArrived => "photo_arrived",
            Detector::DocumentArrived => "document_arrived",
            Detector::PastedList => "pasted_list",
            Detector::DatedTodo => "dated_todo",
            Detector::SendOrPrint => "send_or_print",
            Detector::RepeatedQuestion => "repeated_question",
        }
    }
}

/// The web app saves attachments after the text inside this marker (`hq-web` `chat_uploads`).
const ATTACHMENTS_OPEN: &str = "<hq-attachments>";
const ATTACHMENTS_CLOSE: &str = "</hq-attachments>";

/// The mime types of the files that came with a message, and the text without the marker.
fn split_attachments(text: &str) -> (&str, Vec<String>) {
    let Some(start) = text.find(ATTACHMENTS_OPEN) else {
        return (text, Vec::new());
    };
    let body = &text[start + ATTACHMENTS_OPEN.len()..];
    let json = body.split(ATTACHMENTS_CLOSE).next().unwrap_or_default();
    let mimes = serde_json::from_str::<Vec<serde_json::Value>>(json)
        .unwrap_or_default()
        .iter()
        .filter_map(|a| {
            a.get("mime")
                .and_then(|m| m.as_str())
                .map(str::to_lowercase)
        })
        .collect();
    (&text[..start], mimes)
}

fn is_document(mime: &str) -> bool {
    mime == "application/pdf"
        || mime.starts_with("text/")
        || mime.contains("officedocument")
        || mime.contains("msword")
        || mime.contains("ms-excel")
        || mime.contains("opendocument")
}

const LIST_ITEMS_FOR_TASKS: usize = 3;

fn is_list_item(line: &str) -> bool {
    let t = line.trim_start();
    if let Some(rest) = t.strip_prefix(['-', '*', '\u{2022}']) {
        return rest.starts_with(' ') && rest.trim().len() > 1;
    }
    let digits = t.chars().take_while(char::is_ascii_digit).count();
    digits > 0
        && digits <= 2
        && t[digits..].starts_with(['.', ')'])
        && t[digits + 1..].trim().len() > 1
}

const DAY_WORDS: &[&str] = &[
    "tomorrow",
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
    "sunday",
    "next week",
    "end of the month",
    "end of month",
];

const TODO_CUES: &[&str] = &[
    "remind me",
    "reminder",
    "need to",
    "have to",
    "must ",
    "don't forget",
    "dont forget",
    "deadline",
    " due ",
    "pay ",
    "collect",
    "call ",
    "meet ",
    "submit",
];

const SEND_CUES: &[&str] = &[
    " pdf",
    "pdf ",
    "print",
    "word document",
    "word file",
    "docx",
    "to send to",
    "send it to",
    "share it with",
];

fn has_any(lower: &str, words: &[&str]) -> bool {
    words.iter().any(|w| lower.contains(w))
}

/// What a single message shows, apart from repeats (which need history).
pub fn detect_one(text: &str) -> Vec<Detector> {
    let (body, mimes) = split_attachments(text);
    let lower = format!(" {} ", body.to_lowercase().replace('\u{2019}', "'"));
    let mut found = Vec::new();
    if mimes.iter().any(|m| m.starts_with("image/")) {
        found.push(Detector::PhotoArrived);
    }
    if mimes.iter().any(|m| is_document(m)) {
        found.push(Detector::DocumentArrived);
    }
    if body.lines().filter(|l| is_list_item(l)).count() >= LIST_ITEMS_FOR_TASKS {
        found.push(Detector::PastedList);
    }
    if has_any(&lower, DAY_WORDS) && has_any(&lower, TODO_CUES) {
        found.push(Detector::DatedTodo);
    }
    if has_any(&lower, SEND_CUES) {
        found.push(Detector::SendOrPrint);
    }
    found
}

const REPEAT_WINDOW_SECS: i64 = 14 * DAY_SECS;
const REPEATS_FOR_OFFER: usize = 3;
/// Shorter questions ("why?", "what now?") repeat by nature and say nothing.
const MIN_QUESTION_WORDS: usize = 4;

/// A question reduced to its words, so the same question written twice compares equal.
fn question_key(text: &str) -> Option<String> {
    let (body, _) = split_attachments(text);
    let body = body.trim();
    if !body.ends_with('?') {
        return None;
    }
    let words: Vec<String> = body
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect();
    (words.len() >= MIN_QUESTION_WORDS).then(|| words.join(" "))
}

/// Counts the same question across messages, for [`Detector::RepeatedQuestion`].
#[derive(Default)]
pub struct Repeats {
    seen: HashMap<String, Vec<i64>>,
}

impl Repeats {
    /// Records a message and says whether it is the third asking in two weeks.
    pub fn observe(&mut self, text: &str, at: i64) -> bool {
        let Some(key) = question_key(text) else {
            return false;
        };
        let times = self.seen.entry(key).or_default();
        times.retain(|t| at - *t <= REPEAT_WINDOW_SECS);
        times.push(at);
        times.len() >= REPEATS_FOR_OFFER
    }
}

/// How long after a message a call to the capability's tools counts as "HQ already did it",
/// which makes the offer pointless.
const ALREADY_DONE_SECS: i64 = 600;
pub const DEFAULT_REPLAY_DAYS: i64 = 56;

/// One detector's numbers over the replay window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DetectorCount {
    pub detector: Detector,
    pub capability: &'static str,
    /// Messages it fired on.
    pub fired: usize,
    /// Of those, ones where HQ used the capability's tools within ten minutes anyway.
    pub already_done: usize,
    /// Of the rest, ones where it would be the offer under that reply (one per reply).
    pub would_offer: usize,
}

/// What a replay found: counts only, never a message or any part of one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReplayReport {
    pub days: i64,
    pub messages: usize,
    /// Replies that would carry an offer.
    pub replies_with_offer: usize,
    pub detectors: Vec<DetectorCount>,
}

impl ReplayReport {
    /// Offers per week across all detectors.
    pub fn offers_per_week(&self) -> f64 {
        let weeks = (self.days as f64 / 7.0).max(1.0);
        self.replies_with_offer as f64 / weeks
    }
}

/// Times the owner's sessions called any of `tools` in `[from, until)`, ascending.
fn tool_times(conn: &Connection, tools: &[String], from: i64, until: i64) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare(
        "SELECT timestamp FROM tool_usage
          WHERE tool_name = ?1 AND agent_name = ?2 AND success = 1
            AND timestamp >= ?3 AND timestamp < ?4",
    )?;
    let mut times = Vec::new();
    for tool in tools {
        let rows = stmt.query_map(params![tool, OWNER_AGENT, from, until], |r| r.get(0))?;
        for t in rows {
            times.push(t?);
        }
    }
    times.sort_unstable();
    Ok(times)
}

fn done_soon_after(times: &[i64], at: i64) -> bool {
    let i = times.partition_point(|t| *t < at);
    times.get(i).is_some_and(|t| *t - at <= ALREADY_DONE_SECS)
}

/// Runs every detector over the person's messages of the last `days` days, as of `now`.
pub fn replay(conn: &Connection, now: i64, days: i64) -> Result<ReplayReport> {
    let from = now - days.max(1) * DAY_SECS;
    // The two weeks before the window teach the repeat counter without counting.
    let messages = user_messages_since(conn, from - REPEAT_WINDOW_SECS)?;
    let mut times: HashMap<Detector, Vec<i64>> = HashMap::new();
    for d in Detector::ALL {
        let tools: Vec<String> = super::catalog()
            .iter()
            .find(|c| c.id == d.capability())
            .map(|c| c.tools.clone())
            .unwrap_or_default();
        times.insert(
            d,
            tool_times(conn, &tools, from, now + ALREADY_DONE_SECS + 1)?,
        );
    }
    let mut counts: HashMap<Detector, DetectorCount> = Detector::ALL
        .iter()
        .map(|d| {
            let c = DetectorCount {
                detector: *d,
                capability: d.capability(),
                fired: 0,
                already_done: 0,
                would_offer: 0,
            };
            (*d, c)
        })
        .collect();
    let mut repeats = Repeats::default();
    let mut scanned = 0;
    let mut replies_with_offer = 0;
    for UserMessage { at, text } in &messages {
        let repeated = repeats.observe(text, *at);
        if *at < from || *at > now {
            continue;
        }
        scanned += 1;
        let mut found = detect_one(text);
        if repeated {
            found.push(Detector::RepeatedQuestion);
        }
        let mut offered = false;
        for d in Detector::ALL.iter().filter(|d| found.contains(d)) {
            let c = counts.get_mut(d).expect("every detector has a row");
            c.fired += 1;
            if done_soon_after(&times[d], *at) {
                c.already_done += 1;
            } else if !offered {
                c.would_offer += 1;
                offered = true;
            }
        }
        replies_with_offer += usize::from(offered);
    }
    Ok(ReplayReport {
        days,
        messages: scanned,
        replies_with_offer,
        detectors: Detector::ALL.iter().map(|d| counts[d].clone()).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_db::Database;

    const NOW: i64 = 1_800_000_000;

    #[test]
    fn single_messages_fire_the_detectors_they_should_and_no_others() {
        assert_eq!(
            detect_one("Remind me to collect payment from the shop on Friday"),
            vec![Detector::DatedTodo]
        );
        assert!(
            detect_one("Friday was a good day").is_empty(),
            "a day without a to-do"
        );
        assert!(
            detect_one("I need to think about it").is_empty(),
            "a to-do without a day"
        );
        assert_eq!(
            detect_one("Things for the shop:\n- order stock\n- call the landlord\n1. fix the till"),
            vec![Detector::PastedList]
        );
        assert!(
            detect_one("- one\n- two").is_empty(),
            "two items are not a list to split"
        );
        assert_eq!(
            detect_one("Can you make it a PDF I can print?"),
            vec![Detector::SendOrPrint]
        );
        let photo = r#"what does this say<hq-attachments>[{"name":"r.jpg","path":"_media/web/r.jpg","mime":"image/jpeg"}]</hq-attachments>"#;
        assert_eq!(detect_one(photo), vec![Detector::PhotoArrived]);
        let pdf = r#"here<hq-attachments>[{"name":"a.pdf","path":"_media/web/a.pdf","mime":"application/pdf"}]</hq-attachments>"#;
        assert_eq!(detect_one(pdf), vec![Detector::DocumentArrived]);
    }

    #[test]
    fn a_question_repeats_on_its_third_asking_within_two_weeks() {
        let mut r = Repeats::default();
        let q = "What is the price of cement today?";
        assert!(!r.observe(q, 0));
        assert!(!r.observe("what is the price of cement today?", DAY_SECS));
        assert!(r.observe(q, 2 * DAY_SECS));
        let mut late = Repeats::default();
        late.observe(q, 0);
        late.observe(q, DAY_SECS);
        assert!(!late.observe(q, 20 * DAY_SECS), "the first two are too old");
        assert!(
            !r.observe("why?", 3 * DAY_SECS),
            "too short to mean anything"
        );
    }

    fn say(c: &Connection, at: i64, text: &str) {
        c.execute(
            "INSERT OR IGNORE INTO chat_threads (thread_id, title, created_at, updated_at)
             VALUES ('t', 't', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00')",
            [],
        )
        .unwrap();
        let when = chrono::DateTime::<chrono::Utc>::from_timestamp(at, 0)
            .unwrap()
            .to_rfc3339();
        c.execute(
            "INSERT INTO chat_messages (message_id, thread_id, role, content, created_at)
             VALUES (?1, 't', 'user', ?2, ?3)",
            params![uuid::Uuid::new_v4().to_string(), text, when],
        )
        .unwrap();
    }

    #[test]
    fn a_replay_counts_offers_and_skips_what_hq_already_did() {
        let db = Database::open_memory().unwrap();
        let r = db
            .with_conn(|c| {
                // Offered: nothing was done about it.
                say(c, NOW - 3 * DAY_SECS, "Remind me to pay rent on Friday");
                // Already done: a task was made two minutes later.
                say(c, NOW - 2 * DAY_SECS, "I need to call the bank tomorrow");
                c.execute(
                    "INSERT INTO tool_usage (tool_name, agent_name, session_id, timestamp, success)
                     VALUES ('task_create', ?1, 's', ?2, 1)",
                    params![OWNER_AGENT, NOW - 2 * DAY_SECS + 120],
                )?;
                // Two detectors on one message: only one offer for that reply.
                say(
                    c,
                    NOW - DAY_SECS,
                    "Make a PDF to print:\n- cement\n- sand\n- iron sheets",
                );
                // Outside the window.
                say(c, NOW - 90 * DAY_SECS, "Remind me to pay rent on Friday");
                say(c, NOW - 60, "hello");
                replay(c, NOW, 28)
            })
            .unwrap();
        assert_eq!(r.messages, 4);
        assert_eq!(r.replies_with_offer, 2);
        let by = |d: Detector| {
            r.detectors
                .iter()
                .find(|c| c.detector == d)
                .unwrap()
                .clone()
        };
        let dated = by(Detector::DatedTodo);
        assert_eq!(
            (dated.fired, dated.already_done, dated.would_offer),
            (2, 1, 1)
        );
        assert_eq!(by(Detector::PastedList).would_offer, 1);
        let print = by(Detector::SendOrPrint);
        assert_eq!(
            (print.fired, print.would_offer),
            (1, 0),
            "the list took that reply"
        );
        assert!((r.offers_per_week() - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn a_report_serializes_with_counts_only() {
        let db = Database::open_memory().unwrap();
        let r = db
            .with_conn(|c| {
                say(
                    c,
                    NOW - DAY_SECS,
                    "Remind me to pay the secret supplier on Friday",
                );
                replay(c, NOW, 7)
            })
            .unwrap();
        let json = serde_json::to_string(&r).unwrap();
        assert!(
            !json.contains("secret"),
            "no message text in a report: {json}"
        );
        assert!(json.contains("\"dated_todo\""));
    }
}
