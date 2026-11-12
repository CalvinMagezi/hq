//! What `calendar_update_event` and `calendar_delete_event` share: reading the event link from a
//! listing, checking that the owner-visible title and start still match it, and the round trip to
//! the phone. The link is the only way to name an event, so it is parsed strictly and the id in it
//! is digits only.

use chrono::{DateTime, NaiveDate, NaiveDateTime};
use chrono_tz::Tz;
use serde_json::{Map, Value, json};

use super::check::{DATE_FORMAT, TIMED_FORMAT, TITLE_MAX_CHARS, text};
use super::create::{PHONE_TEXT_MAX_CHARS, clean, pass_through};
use super::list::{ID_MAX_DIGITS, LINK_PREFIX, MS_PER_DAY, UNTITLED};
use super::outcome::invalid;

const RESERVED_PREFIX: &str = "_hq_";
const BEGIN_PARAM: &str = "?begin=";
const END_PARAM: &str = "&end=";
/// Longest title the owner sees in an approval line; the full arguments are shown beside it.
const APPROVAL_TITLE_MAX_CHARS: usize = 60;

/// An event as the owner approved it: the ids from its link, the title as listed, and which kind
/// of event the model says it is (a date-only `start_local` means all-day).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Target {
    pub event_id: String,
    pub begin_ms: i64,
    pub end_ms: i64,
    /// Cleaned like the phone cleans a stored title; empty for an untitled event.
    pub expected_title: String,
    pub all_day: bool,
}

fn digits(raw: &str, max: usize) -> Option<&str> {
    (!raw.is_empty() && raw.len() <= max && raw.bytes().all(|b| b.is_ascii_digit())).then_some(raw)
}

fn bad_link() -> Value {
    invalid("bad_link", Some("open_link"))
}

/// `hq://calendar/event/{id}?begin={ms}&end={ms}` and nothing else: no whitespace, no further
/// parameters, no fragment, digits only.
pub(super) fn parse_link(raw: &str) -> Result<(String, i64, i64), Value> {
    let rest = raw.strip_prefix(LINK_PREFIX).ok_or_else(bad_link)?;
    let (id, rest) = rest.split_once(BEGIN_PARAM).ok_or_else(bad_link)?;
    let (begin, end) = rest.split_once(END_PARAM).ok_or_else(bad_link)?;
    let number = |s: &str| {
        digits(s, ID_MAX_DIGITS)
            .and_then(|d| d.parse::<i64>().ok())
            .ok_or_else(bad_link)
    };
    digits(id, ID_MAX_DIGITS).ok_or_else(bad_link)?;
    let (begin_ms, end_ms) = (number(begin)?, number(end)?);
    if end_ms < begin_ms {
        return Err(bad_link());
    }
    Ok((id.to_string(), begin_ms, end_ms))
}

pub(super) fn reject_unknown(map: &Map<String, Value>, known: &[&str]) -> Result<(), Value> {
    match map
        .keys()
        .find(|k| !known.contains(&k.as_str()) && !k.starts_with(RESERVED_PREFIX))
    {
        Some(key) => Err(invalid("unknown_field", Some(key))),
        None => Ok(()),
    }
}

fn utc_date(ms: i64) -> Option<NaiveDate> {
    DateTime::from_timestamp_millis(ms).map(|t| t.date_naive())
}

/// Reads `open_link`, `title` and `start_local` and checks the start against the link.
pub(super) fn target(map: &Map<String, Value>, zone: Tz) -> Result<Target, Value> {
    let link = text(map, "open_link", usize::MAX, true)?.unwrap_or_default();
    let (event_id, begin_ms, end_ms) = parse_link(&link)?;
    let title = text(map, "title", TITLE_MAX_CHARS, true)?.unwrap_or_default();
    let start = text(map, "start_local", usize::MAX, true)?.unwrap_or_default();
    let all_day = match (
        NaiveDateTime::parse_from_str(&start, TIMED_FORMAT).is_ok(),
        NaiveDate::parse_from_str(&start, DATE_FORMAT).is_ok(),
    ) {
        (true, _) => false,
        (false, true) => true,
        (false, false) => return Err(invalid("bad_format", Some("start_local"))),
    };
    let matches = if all_day {
        begin_ms % MS_PER_DAY == 0
            && end_ms % MS_PER_DAY == 0
            && utc_date(begin_ms).is_some_and(|d| d.format(DATE_FORMAT).to_string() == start)
    } else {
        DateTime::from_timestamp_millis(begin_ms)
            .is_some_and(|t| t.with_timezone(&zone).format(TIMED_FORMAT).to_string() == start)
    };
    if !matches {
        return Err(invalid("start_mismatch", Some("start_local")));
    }
    let cleaned = clean(&title, PHONE_TEXT_MAX_CHARS);
    Ok(Target {
        event_id,
        begin_ms,
        end_ms,
        expected_title: if cleaned == UNTITLED {
            String::new()
        } else {
            cleaned
        },
        all_day,
    })
}

pub(super) fn base_payload(request_id: &str, zone: Tz, t: &Target) -> Value {
    json!({
        "request_id": request_id,
        "event_id": t.event_id,
        "begin_ms": t.begin_ms,
        "expected_title": t.expected_title,
        "zone": zone.name(),
    })
}

/// What the phone may say for one modification, as HQ keeps it: the success outcome as a bare
/// code, any other known outcome through the same field filter as create, anything else unverified.
pub(super) fn finalize_modification(
    body: &Value,
    success: &str,
    allowed: &[&str],
    unverified: fn(&str) -> Value,
) -> Value {
    let code = body.get("outcome").and_then(Value::as_str).unwrap_or("");
    if code == success {
        json!({"outcome": success})
    } else if allowed.contains(&code) {
        pass_through(code, body)
    } else {
        unverified("unexpected_outcome")
    }
}

/// `key` of the tool call's arguments, cleaned and cut for an approval line.
pub(super) fn quoted(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(|t| clean(t, APPROVAL_TITLE_MAX_CHARS))
        .filter(|t| !t.is_empty())
}

/// The event a modification names, as the owner reads it.
pub(super) fn describe_target(args: &Value) -> String {
    quoted(args, "title").map_or_else(|| "an event".to_string(), |t| format!("the event \"{t}\""))
}
