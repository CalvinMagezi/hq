//! What `calendar_update_event` and `calendar_delete_event` share: reading the event link from a
//! listing, checking that the owner-visible title and start still match it, and the round trip to
//! the phone. The link is the only way to name an event, so it is parsed strictly and the id in it
//! is digits only.

use chrono::{DateTime, NaiveDate, NaiveDateTime};
use chrono_tz::Tz;
use serde_json::{Map, Value, json};

use super::check::{DATE_FORMAT, TIMED_FORMAT, TITLE_MAX_CHARS, text};
use super::create::{DeviceCalendar, PHONE_TEXT_MAX_CHARS, clean, pass_through};
use super::list::{ID_MAX_DIGITS, LINK_PREFIX, MS_PER_DAY, UNTITLED};
use super::outcome::{self, invalid};
use super::store::Opened;
use crate::skills::builtin;

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

impl DeviceCalendar {
    /// Sends one update or delete and waits for the phone. Always returns one of the contract's
    /// outcomes, never an error. Logs hold ids and codes, never a title.
    pub(super) async fn run_modification(
        &self,
        action: &'static str,
        skill: &str,
        tool_call_id: &str,
        payload: Value,
        unverified: fn(&str) -> Value,
    ) -> Value {
        if builtin::shipped_hash(skill).is_none() || !builtin::is_active(skill) {
            return outcome::unavailable("skill_not_active");
        }
        let request_id = payload["request_id"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let opened = self.store.open_named(
            action,
            &self.thread_id,
            tool_call_id,
            &request_id,
            payload,
            self.timeouts,
        );
        let (action_id, frame, phase) = match opened {
            Ok(Opened::New {
                action_id,
                frame,
                phase,
            }) => (action_id, frame, phase),
            Ok(Opened::Known { outcome }) => {
                return outcome.unwrap_or_else(|| unverified("still_waiting"));
            }
            Err(_) => return outcome::unavailable("too_many_pending"),
        };
        tracing::info!(%request_id, %action_id, action, phase = "sent", "device action");
        (self.emit)(frame);
        let result = self.wait(&action_id, phase, unverified).await;
        tracing::info!(
            %request_id, %action_id, action, phase = "done", outcome = outcome::code(&result),
            "device action"
        );
        result
    }
}

/// One line for the owner's approval: what a modification call would do to which event. `None`
/// for any other tool. Titles are untrusted, so they are cleaned and cut.
pub fn describe_modification(tool: &str, args: &Value) -> Option<String> {
    let short = |key: &str| {
        args.get(key)
            .and_then(Value::as_str)
            .map(|t| clean(t, APPROVAL_TITLE_MAX_CHARS))
            .filter(|t| !t.is_empty())
    };
    let event =
        short("title").map_or_else(|| "an event".to_string(), |t| format!("the event \"{t}\""));
    match tool {
        super::delete::DELETE_TOOL_NAME => Some(format!(
            "would delete {event} from the owner's phone calendar"
        )),
        super::update::UPDATE_TOOL_NAME => {
            let mut changes = Vec::new();
            if let Some(t) = short("new_title") {
                changes.push(format!("title to \"{t}\""));
            }
            for (key, label) in [("new_start_local", "start"), ("new_end_local", "end")] {
                if let Some(v) = short(key) {
                    changes.push(format!("{label} to {v}"));
                }
            }
            Some(format!(
                "would change {event} on the owner's phone calendar: {}",
                changes.join(", ")
            ))
        }
        _ => None,
    }
}
