//! The detectors behind "I can also do this for you" (phase 2 of `docs/plans/simple-hq.md`):
//! deterministic checks on what the person just wrote that say a capability would help now.
//! No model decides an offer, and nothing here leaves the device.
//!
//! [`replay`] runs them over past chats and returns counts only, so a detector can be tuned on
//! real history before the apps ever show an offer.

use std::collections::{HashMap, HashSet};

use anyhow::Result;
use chrono::{Datelike, Duration, NaiveDate, Weekday};
use hq_db::adoption::{DAY_SECS, OWNER_AGENT, UserMessage, persons_own, user_messages_since};
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

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|d| d.as_str() == s)
    }

    /// What Yes asks for, in the person's words, and whether it can be sent as it is. Only the
    /// file request needs the person to say which note, so it goes to the composer.
    fn reply(self, due: Option<NaiveDate>) -> (String, bool) {
        let words = match self {
            Detector::PhotoArrived => "Read the text in that photo.".to_string(),
            Detector::DocumentArrived => "Keep that file as a note I can search later.".to_string(),
            Detector::PastedList => "Turn that list into separate tasks.".to_string(),
            Detector::DatedTodo => match due {
                Some(d) => format!("Make that a task due on {}.", d.format("%A %-d %B %Y")),
                None => "Make that a task with its due date.".to_string(),
            },
            Detector::SendOrPrint => "Make that into a PDF I can send.".to_string(),
            Detector::RepeatedQuestion => "Look that up on the internet for me.".to_string(),
        };
        (words, self != Detector::SendOrPrint)
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
/// Past this a pasted list is a log or a spec, not things to do.
const MAX_LIST_ITEMS: usize = 15;
const MAX_LIST_ITEM_CHARS: usize = 80;
/// A to-do said in passing ("I need to ... on Monday") inside a long message is planning, not a
/// reminder to keep.
const SHORT_TODO_WORDS: usize = 25;

fn is_list_item(line: &str) -> bool {
    list_item_text(line).is_some()
}

/// The text of a `-`, `*`, bullet or `1.` / `1)` list line.
fn list_item_text(line: &str) -> Option<&str> {
    let t = line.trim_start();
    if let Some(rest) = t.strip_prefix(['-', '*', '\u{2022}']) {
        return (rest.starts_with(' ') && rest.trim().len() > 1).then(|| rest.trim());
    }
    let digits = t.chars().take_while(char::is_ascii_digit).count();
    let ok = digits > 0
        && digits <= 2
        && t[digits..].starts_with(['.', ')'])
        && t[digits + 1..].trim().len() > 1;
    ok.then(|| t[digits + 1..].trim())
}

/// Phase 1 wording, matched as plain substrings. Kept only so `hq offers replay` can show the
/// rules before and after tuning side by side on the same history.
mod phase1 {
    pub const DAY_WORDS: &[&str] = &[
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
    pub const TODO_CUES: &[&str] = &[
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
    pub const SEND_CUES: &[&str] = &[
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
}

/// Tuned wording, matched as whole words (`print` no longer matches `println` or `blueprint`).
const DAY_WORDS: &[&str] = phase1::DAY_WORDS;
/// Words that ask for a reminder outright, whatever the length of the message.
const REMIND_CUES: &[&str] = &["remind me", "reminder", "don't forget", "dont forget"];
const TODO_CUES: &[&str] = &[
    "need to", "have to", "must", "deadline", "due", "pay", "collect", "call", "meet", "submit",
];
const FORMAT_WORDS: &[&str] = &["pdf", "print", "word document", "word file", "docx"];
/// A format word alone is talk about a file; with one of these it is a request for one.
const MAKE_CUES: &[&str] = &[
    "make",
    "turn",
    "create",
    "give me",
    "send me",
    "as a",
    "into a",
    "to pdf",
    "to a pdf",
    "save it as",
    "export it",
    "export this",
    "convert it",
    "convert this",
];

fn has_any(lower: &str, words: &[&str]) -> bool {
    words.iter().any(|w| lower.contains(w))
}

/// The message as lowercase words between single spaces, padded, so `" pdf "` is a whole word.
fn word_text(body: &str) -> String {
    let lower = body.to_lowercase().replace('\u{2019}', "'");
    let words: Vec<&str> = lower
        .split(|c: char| !(c.is_alphanumeric() || c == '\''))
        .filter(|w| !w.is_empty())
        .collect();
    format!(" {} ", words.join(" "))
}

fn has_word(words: &str, cues: &[&str]) -> bool {
    cues.iter().any(|c| words.contains(&format!(" {c} ")))
}

/// Which detector rules to run: the tuned ones the apps use, or phase 1's for comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rules {
    Phase1,
    Tuned,
}

fn attachment_detectors(mimes: &[String], found: &mut Vec<Detector>) {
    if mimes.iter().any(|m| m.starts_with("image/")) {
        found.push(Detector::PhotoArrived);
    }
    if mimes.iter().any(|m| is_document(m)) {
        found.push(Detector::DocumentArrived);
    }
}

/// What a single message shows, apart from repeats (which need history), under the tuned rules.
pub fn detect_one(text: &str) -> Vec<Detector> {
    detect(text, Rules::Tuned)
}

fn detect(text: &str, rules: Rules) -> Vec<Detector> {
    let (body, mimes) = split_attachments(text);
    let mut found = Vec::new();
    attachment_detectors(&mimes, &mut found);
    match rules {
        Rules::Phase1 => detect_phase1(body, &mut found),
        Rules::Tuned => detect_tuned(body, &mut found),
    }
    found
}

fn detect_phase1(body: &str, found: &mut Vec<Detector>) {
    let lower = format!(" {} ", body.to_lowercase().replace('\u{2019}', "'"));
    if body.lines().filter(|l| is_list_item(l)).count() >= LIST_ITEMS_FOR_TASKS {
        found.push(Detector::PastedList);
    }
    if has_any(&lower, phase1::DAY_WORDS) && has_any(&lower, phase1::TODO_CUES) {
        found.push(Detector::DatedTodo);
    }
    if has_any(&lower, phase1::SEND_CUES) {
        found.push(Detector::SendOrPrint);
    }
}

/// A short list of things, most of the message: not code, a log or a pasted document.
fn is_todo_list(body: &str) -> bool {
    if body.contains("```") || body.contains("://") {
        return false;
    }
    let lines: Vec<&str> = body.lines().filter(|l| !l.trim().is_empty()).collect();
    let items: Vec<&str> = lines.iter().filter_map(|l| list_item_text(l)).collect();
    (LIST_ITEMS_FOR_TASKS..=MAX_LIST_ITEMS).contains(&items.len())
        && items.len() * 2 >= lines.len()
        && items
            .iter()
            .all(|i| i.chars().count() <= MAX_LIST_ITEM_CHARS)
}

fn detect_tuned(body: &str, found: &mut Vec<Detector>) {
    let words = word_text(body);
    if is_todo_list(body) {
        found.push(Detector::PastedList);
    }
    let short = words.split_whitespace().count() <= SHORT_TODO_WORDS;
    if has_word(&words, DAY_WORDS)
        && (has_word(&words, REMIND_CUES) || (short && has_word(&words, TODO_CUES)))
    {
        found.push(Detector::DatedTodo);
    }
    if has_word(&words, FORMAT_WORDS) && has_word(&words, MAKE_CUES) {
        found.push(Detector::SendOrPrint);
    }
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

/// The same offer at most once every three days, and any offer at most once a day, so offers stay
/// a few a week however much the person writes.
pub const SAME_OFFER_GAP_SECS: i64 = 3 * DAY_SECS;
pub const ANY_OFFER_GAP_SECS: i64 = DAY_SECS;

/// When offers were last shown, which spaces them out.
#[derive(Debug, Clone, Default)]
pub struct Pacing {
    last_any: Option<i64>,
    last: HashMap<Detector, i64>,
    /// Phase 1 had no spacing; its replay runs without it.
    off: bool,
}

impl Pacing {
    fn unpaced() -> Self {
        Self {
            off: true,
            ..Self::default()
        }
    }

    pub fn allows(&self, d: Detector, at: i64) -> bool {
        let clear = |last: Option<i64>, gap: i64| last.is_none_or(|t| at - t >= gap);
        self.off
            || (clear(self.last_any, ANY_OFFER_GAP_SECS)
                && clear(self.last.get(&d).copied(), SAME_OFFER_GAP_SECS))
    }

    /// Notes that `d` was shown at `at`.
    pub fn shown(&mut self, d: Detector, at: i64) {
        self.last_any = Some(self.last_any.map_or(at, |t| t.max(at)));
        let last = self.last.entry(d).or_insert(at);
        *last = (*last).max(at);
    }
}

/// What became of one detector that fired on a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// HQ used the capability anyway: no offer.
    AlreadyDone,
    /// Not ready on this install, or offered too recently.
    HeldBack,
    /// The one offer under this reply.
    Offered,
    /// Another detector took this reply.
    Shadowed,
}

/// Judges the detectors that fired on one message, in [`Detector::ALL`] order, so at most one is
/// [`Outcome::Offered`]. The apps and the replay both decide through this.
pub fn judge(
    found: &[Detector],
    done: &dyn Fn(Detector) -> bool,
    ready: &HashSet<Detector>,
    pacing: &Pacing,
    at: i64,
) -> Vec<(Detector, Outcome)> {
    let mut offered = false;
    Detector::ALL
        .iter()
        .filter(|d| found.contains(d))
        .map(|d| {
            let outcome = if done(*d) {
                Outcome::AlreadyDone
            } else if !ready.contains(d) || !pacing.allows(*d, at) {
                Outcome::HeldBack
            } else if offered {
                Outcome::Shadowed
            } else {
                offered = true;
                Outcome::Offered
            };
            (*d, outcome)
        })
        .collect()
}

impl Pacing {
    /// From when each kind of offer was last shown (`hq_db::offers::last_shown`).
    pub fn from_last_shown(last: &HashMap<String, i64>) -> Self {
        let mut p = Self::default();
        for (kind, at) in last {
            if let Some(d) = Detector::parse(kind) {
                p.shown(d, *at);
            }
        }
        p
    }
}

/// The day a dated to-do names, as of `today`: tomorrow, the next such weekday (today when it is
/// that day), or the end of the month. `None` for "next week", which names no day.
pub fn due_day(text: &str, today: NaiveDate) -> Option<NaiveDate> {
    let words = word_text(split_attachments(text).0);
    if words.contains(" tomorrow ") {
        return today.succ_opt();
    }
    const WEEKDAYS: [(&str, Weekday); 7] = [
        ("monday", Weekday::Mon),
        ("tuesday", Weekday::Tue),
        ("wednesday", Weekday::Wed),
        ("thursday", Weekday::Thu),
        ("friday", Weekday::Fri),
        ("saturday", Weekday::Sat),
        ("sunday", Weekday::Sun),
    ];
    if let Some((_, day)) = WEEKDAYS
        .iter()
        .find(|(w, _)| words.contains(&format!(" {w} ")))
    {
        let ahead = (day.num_days_from_monday() + 7 - today.weekday().num_days_from_monday()) % 7;
        return Some(today + Duration::days(i64::from(ahead)));
    }
    if words.contains(" end of the month ") || words.contains(" end of month ") {
        let first_next = if today.month() == 12 {
            NaiveDate::from_ymd_opt(today.year() + 1, 1, 1)
        } else {
            NaiveDate::from_ymd_opt(today.year(), today.month() + 1, 1)
        };
        return first_next.and_then(|d| d.pred_opt());
    }
    None
}

/// The offer under one reply, as the apps draw it. Only what was shown: the person's answer is
/// kept in `offer_log`, never here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TurnOffer {
    pub kind: &'static str,
    /// The line under the reply, from the catalog's `says`.
    pub line: String,
    /// What Yes asks HQ for.
    pub reply: String,
    /// Yes sends `reply` at once; otherwise it goes to the composer to be finished.
    pub send: bool,
}

/// Everything a finished owner turn hands the chooser.
pub struct TurnInput<'a> {
    /// The person's message as saved, with its attachment marker.
    pub text: &'a str,
    /// Just before that message was saved: earlier messages are its history.
    pub asked_at: i64,
    pub now: i64,
    /// Tools this turn called: an offer for something HQ just did is pointless.
    pub turn_tools: &'a [String],
    pub ready: HashSet<Detector>,
    /// The person's local date, for the day a dated to-do names.
    pub today: NaiveDate,
}

fn line_for(says: &str) -> String {
    let mut chars = says.chars();
    let lowered: String = chars
        .next()
        .map(|c| c.to_lowercase().chain(chars).collect())
        .unwrap_or_default();
    format!("I can also {lowered}.")
}

/// Chooses the offer for a reply, deterministically: the detectors on the person's message and
/// their two weeks of questions, less what is muted, not ready, too soon or already done this
/// turn. The model takes no part, and no part of this reaches it.
pub fn for_turn(conn: &Connection, input: &TurnInput) -> Result<Option<TurnOffer>> {
    let mut found = detect_one(input.text);
    let mut repeats = Repeats::default();
    let history = persons_own(user_messages_since(conn, input.now - REPEAT_WINDOW_SECS)?);
    for m in history.iter().filter(|m| m.at < input.asked_at) {
        repeats.observe(&m.text, m.at);
    }
    if repeats.observe(input.text, input.now) {
        found.push(Detector::RepeatedQuestion);
    }
    if found.is_empty() {
        return Ok(None);
    }
    let muted = hq_db::offers::muted(conn, input.now)?;
    let ready: HashSet<Detector> = input
        .ready
        .iter()
        .copied()
        .filter(|d| !muted.contains(d.as_str()))
        .collect();
    let pacing = Pacing::from_last_shown(&hq_db::offers::last_shown(conn)?);
    let done = |d: Detector| {
        super::catalog()
            .iter()
            .find(|c| c.id == d.capability())
            .is_some_and(|c| c.tools.iter().any(|t| input.turn_tools.contains(t)))
    };
    let chosen = judge(&found, &done, &ready, &pacing, input.now)
        .into_iter()
        .find(|(_, o)| *o == Outcome::Offered)
        .map(|(d, _)| d);
    let Some(d) = chosen else {
        return Ok(None);
    };
    let Some(cap) = super::catalog().iter().find(|c| c.id == d.capability()) else {
        return Ok(None);
    };
    let (reply, send) = d.reply(due_day(input.text, input.today));
    Ok(Some(TurnOffer {
        kind: d.as_str(),
        line: line_for(&cap.says),
        reply,
        send,
    }))
}

/// How a report names a kind of offer: its capability in the catalog's words.
pub fn offer_says(kind: &str) -> String {
    Detector::parse(kind)
        .and_then(|d| super::catalog().iter().find(|c| c.id == d.capability()))
        .map(|c| c.says.clone())
        .unwrap_or_else(|| kind.to_string())
}

/// The detectors whose capability is ready on this install.
pub fn ready_detectors(offers: &[super::Offer]) -> HashSet<Detector> {
    Detector::ALL
        .into_iter()
        .filter(|d| {
            offers
                .iter()
                .any(|o| o.id == d.capability() && o.is_ready())
        })
        .collect()
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
    /// Of the rest, ones held back: not ready on this install, or offered too recently.
    pub held_back: usize,
    /// Of the rest, ones where it would be the offer under that reply (one per reply).
    pub would_offer: usize,
}

/// What one set of rules found: counts only, never a message or any part of one.
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

/// Phase 1's rules (every user-role message, no readiness, no spacing) beside the tuned rules the
/// apps use (only what the person typed, only ready capabilities, spaced out).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Comparison {
    pub phase1: ReplayReport,
    pub tuned: ReplayReport,
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

/// Everything one replay pass reads, so both rule sets see the same history.
struct History<'a> {
    messages: &'a [UserMessage],
    times: &'a HashMap<Detector, Vec<i64>>,
    from: i64,
    now: i64,
    days: i64,
}

fn empty_counts() -> HashMap<Detector, DetectorCount> {
    Detector::ALL
        .iter()
        .map(|d| {
            let c = DetectorCount {
                detector: *d,
                capability: d.capability(),
                fired: 0,
                already_done: 0,
                held_back: 0,
                would_offer: 0,
            };
            (*d, c)
        })
        .collect()
}

fn replay_pass(h: &History, rules: Rules, ready: &HashSet<Detector>) -> ReplayReport {
    let (all, mut pacing) = match rules {
        Rules::Phase1 => (Detector::ALL.into_iter().collect(), Pacing::unpaced()),
        Rules::Tuned => (ready.clone(), Pacing::default()),
    };
    let mut counts = empty_counts();
    let mut repeats = Repeats::default();
    let (mut scanned, mut replies_with_offer) = (0, 0);
    let own = h
        .messages
        .iter()
        .filter(|m| rules == Rules::Phase1 || !m.from_client);
    for UserMessage { at, text, .. } in own {
        let repeated = repeats.observe(text, *at);
        if *at < h.from || *at > h.now {
            continue;
        }
        scanned += 1;
        let mut found = detect(text, rules);
        if repeated {
            found.push(Detector::RepeatedQuestion);
        }
        let done = |d: Detector| done_soon_after(&h.times[&d], *at);
        for (d, outcome) in judge(&found, &done, &all, &pacing, *at) {
            let c = counts.get_mut(&d).expect("every detector has a row");
            c.fired += 1;
            match outcome {
                Outcome::AlreadyDone => c.already_done += 1,
                Outcome::HeldBack => c.held_back += 1,
                Outcome::Shadowed => {}
                Outcome::Offered => {
                    c.would_offer += 1;
                    replies_with_offer += 1;
                    pacing.shown(d, *at);
                }
            }
        }
    }
    ReplayReport {
        days: h.days,
        messages: scanned,
        replies_with_offer,
        detectors: Detector::ALL.iter().map(|d| counts[d].clone()).collect(),
    }
}

/// Runs both rule sets over the chats of the last `days` days, as of `now`. `ready` is what the
/// tuned rules may offer on this install ([`ready_detectors`]).
pub fn replay(
    conn: &Connection,
    now: i64,
    days: i64,
    ready: &HashSet<Detector>,
) -> Result<Comparison> {
    let days = days.max(1);
    let from = now - days * DAY_SECS;
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
    let h = History {
        messages: &messages,
        times: &times,
        from,
        now,
        days,
    };
    Ok(Comparison {
        phase1: replay_pass(&h, Rules::Phase1, ready),
        tuned: replay_pass(&h, Rules::Tuned, ready),
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
    fn tuned_rules_skip_what_phase_one_caught_in_ordinary_chat() {
        let cases = [
            ("Why does println panic here?", Detector::SendOrPrint),
            ("The PDF export test fails on CI", Detector::SendOrPrint),
            (
                "I need to understand why the build broke on Monday before we ship the next \
                 release, can you walk me through the failing steps and what changed since then",
                Detector::DatedTodo,
            ),
        ];
        for (text, d) in cases {
            assert!(detect(text, Rules::Phase1).contains(&d), "phase 1: {text}");
            assert!(!detect_one(text).contains(&d), "tuned: {text}");
        }
        let log = "Errors:\n- error[E0308]: mismatched types in the session builder module here\n\
                   - see https://example.com/a\n- see https://example.com/b";
        assert!(detect(log, Rules::Phase1).contains(&Detector::PastedList));
        assert!(!detect_one(log).contains(&Detector::PastedList));
        let fenced = "```\n- a1\n- b2\n- c3\n```";
        assert!(!detect_one(fenced).contains(&Detector::PastedList));
        assert_eq!(
            detect_one("Don't forget I have to pay school fees on Monday"),
            vec![Detector::DatedTodo]
        );
        assert_eq!(
            detect_one("Call the plumber tomorrow"),
            vec![Detector::DatedTodo]
        );
    }

    #[test]
    fn pacing_spaces_offers_and_judge_offers_at_most_one() {
        let ready: HashSet<Detector> = Detector::ALL.into_iter().collect();
        let not_done = |_: Detector| false;
        let mut p = Pacing::default();
        let both = [Detector::PastedList, Detector::SendOrPrint];
        let first = judge(&both, &not_done, &ready, &p, 0);
        assert_eq!(
            first,
            vec![
                (Detector::PastedList, Outcome::Offered),
                (Detector::SendOrPrint, Outcome::Shadowed)
            ]
        );
        p.shown(Detector::PastedList, 0);
        assert!(!p.allows(Detector::SendOrPrint, ANY_OFFER_GAP_SECS - 1));
        assert!(p.allows(Detector::SendOrPrint, ANY_OFFER_GAP_SECS));
        assert!(!p.allows(Detector::PastedList, SAME_OFFER_GAP_SECS - 1));
        assert!(p.allows(Detector::PastedList, SAME_OFFER_GAP_SECS));
        let only_print: HashSet<Detector> = [Detector::SendOrPrint].into();
        assert_eq!(
            judge(&both, &not_done, &only_print, &Pacing::default(), 0),
            vec![
                (Detector::PastedList, Outcome::HeldBack),
                (Detector::SendOrPrint, Outcome::Offered)
            ],
            "a capability that is not ready never takes the reply"
        );
        let done = |d: Detector| d == Detector::PastedList;
        assert_eq!(
            judge(&both, &done, &ready, &Pacing::default(), 0)[0].1,
            Outcome::AlreadyDone
        );
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

    fn every() -> HashSet<Detector> {
        Detector::ALL.into_iter().collect()
    }

    fn ask(c: &Connection, at: i64, text: &str) {
        say(c, at, text);
        c.execute(
            "UPDATE chat_messages SET meta = '{\"source\":{\"kind\":\"mcp\"}}'
              WHERE content = ?1",
            params![text],
        )
        .unwrap();
    }

    #[test]
    fn the_tuned_replay_reads_only_the_persons_words_and_spaces_offers() {
        let db = Database::open_memory().unwrap();
        let r = db
            .with_conn(|c| {
                ask(
                    c,
                    NOW - 5 * DAY_SECS,
                    "Remind me to check the build on Friday",
                );
                // Three dated to-dos on three days: the same offer at most every three days.
                for day in [4, 3, 2] {
                    say(
                        c,
                        NOW - day * DAY_SECS,
                        &format!("Remind me to pay rent on Friday {day}"),
                    );
                }
                replay(c, NOW, 28, &every())
            })
            .unwrap();
        assert_eq!(r.phase1.messages, 4);
        assert_eq!(r.phase1.replies_with_offer, 4);
        assert_eq!(r.tuned.messages, 3, "a client's question is not the person");
        let dated = &r.tuned.detectors[3];
        assert_eq!(dated.detector, Detector::DatedTodo);
        assert_eq!((dated.fired, dated.held_back, dated.would_offer), (3, 2, 1));
        let none = db
            .with_conn(|c| replay(c, NOW, 28, &HashSet::new()))
            .unwrap();
        assert_eq!(
            none.tuned.replies_with_offer, 0,
            "nothing ready, nothing offered"
        );
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
                replay(c, NOW, 28, &every())
            })
            .unwrap();
        assert_eq!(
            r.phase1, r.tuned,
            "these messages read the same under both rules"
        );
        let r = r.tuned;
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

    fn day(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn a_dated_to_do_names_its_day() {
        // 2026-11-05 is a Thursday.
        let today = day("2026-11-05");
        assert_eq!(
            due_day("Remind me tomorrow", today),
            Some(day("2026-11-06"))
        );
        assert_eq!(
            due_day("pay rent on Friday", today),
            Some(day("2026-11-06"))
        );
        assert_eq!(
            due_day("call them on Monday", today),
            Some(day("2026-11-09"))
        );
        assert_eq!(
            due_day("submit it Thursday", today),
            Some(day("2026-11-05"))
        );
        assert_eq!(
            due_day("pay by the end of the month", today),
            Some(day("2026-11-30"))
        );
        assert_eq!(
            due_day("pay by end of month", day("2026-12-10")),
            Some(day("2026-12-31"))
        );
        assert_eq!(due_day("sometime next week", today), None);
    }

    fn input<'a>(text: &'a str, tools: &'a [String]) -> TurnInput<'a> {
        TurnInput {
            text,
            asked_at: NOW,
            now: NOW,
            turn_tools: tools,
            ready: every(),
            today: day("2026-11-05"),
        }
    }

    #[test]
    fn a_turn_gets_one_offer_in_the_catalogs_words_unless_muted_soon_or_done() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            let text = "Remind me to pay rent on Friday";
            let o = for_turn(c, &input(text, &[]))?.expect("an offer");
            assert_eq!(o.kind, "dated_todo");
            assert_eq!(
                o.line,
                "I can also keep a list of things to do, with the day each one is due."
            );
            assert_eq!(o.reply, "Make that a task due on Friday 6 November 2026.");
            assert!(o.send);
            let made = ["task_create".to_string()];
            assert_eq!(
                for_turn(c, &input(text, &made))?,
                None,
                "HQ already made the task"
            );
            assert_eq!(for_turn(c, &input("hello there", &[]))?, None);
            hq_db::offers::record_shown(c, "m1", "dated_todo", NOW - DAY_SECS)?;
            assert_eq!(
                for_turn(c, &input(text, &[]))?,
                None,
                "the same offer too soon"
            );
            let print = for_turn(c, &input("Can you make it a PDF to print?", &[]))?.unwrap();
            assert!(!print.send, "which note to export is the person's to say");
            hq_db::offers::record_shown(c, "m2", "send_or_print", NOW - 10 * DAY_SECS)?;
            hq_db::offers::record_shown(c, "m3", "send_or_print", NOW - 9 * DAY_SECS)?;
            hq_db::offers::answer(c, "m2", hq_db::offers::Answer::NotNow, NOW - 10 * DAY_SECS)?;
            hq_db::offers::answer(c, "m3", hq_db::offers::Answer::NotNow, NOW - 9 * DAY_SECS)?;
            assert_eq!(
                for_turn(c, &input("Can you make it a PDF to print?", &[]))?,
                None,
                "Not now twice mutes it"
            );
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn the_third_asking_of_a_question_is_offered_a_look_up() {
        let db = Database::open_memory().unwrap();
        db.with_conn(|c| {
            let q = "What is the price of cement today?";
            say(c, NOW - 2 * DAY_SECS, q);
            say(c, NOW - DAY_SECS, q);
            say(c, NOW, q);
            let o = for_turn(c, &input(q, &[]))?.expect("a look-up offer");
            assert_eq!(o.kind, "repeated_question");
            Ok(())
        })
        .unwrap();
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
                replay(c, NOW, 7, &every())
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
