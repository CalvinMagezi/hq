//! `calendar_update_event`: change an existing event's title, start or end. HQ turns the new
//! local times into epoch milliseconds in the phone's zone, so the phone never does calendar
//! arithmetic. Timed stays timed and all-day stays all-day.

use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use chrono_tz::Tz;
use serde_json::{Value, json};

use super::check::{
    DATE_FORMAT, Local, TIMED_FORMAT, TITLE_MAX_CHARS, check_all_day_start, check_timed_start,
    overlap_question, resolve, text,
};
use super::create::DeviceCalendar;
use super::list::MS_PER_DAY;
use super::outcome::{self, UPDATE_PHONE_OUTCOMES, invalid};
use super::store::UPDATE_ACTION_NAME;
use super::target::{Target, base_payload, finalize_modification, reject_unknown, target};
use crate::skills::builtin::CALENDAR_EDITING_SKILL;

pub const UPDATE_TOOL_NAME: &str = "calendar_update_event";

const KNOWN_FIELDS: &[&str] = &[
    "open_link",
    "title",
    "start_local",
    "new_title",
    "new_start_local",
    "new_end_local",
];

const UPDATE_TOOL_DESCRIPTION: &str = "Change the title, start or end of one event on the owner's \
    phone calendar. Load the calendar-editing skill first. Name the event with the open_link, \
    title and start_local that calendar_list_events returned; never build a link. Give at least \
    one of new_title, new_start_local, new_end_local. Timed stays timed and all-day stays all-day. \
    With only a new start the length is kept. The owner is always asked to approve. Recurring \
    events, places, notes, guests and reminders are not supported.";

/// The changes HQ sends, already in epoch milliseconds. A new start always comes with its end.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct Changes {
    pub title: Option<String>,
    pub start_ms: Option<i64>,
    pub end_ms: Option<i64>,
}

fn kind_change(field: &str) -> Value {
    invalid("kind_change", Some(field))
}

/// A timed local time in `zone`. A date-only value on a timed event is a kind change.
fn timed(raw: &str, field: &str, zone: Tz) -> Result<DateTime<Tz>, Value> {
    let Ok(naive) = NaiveDateTime::parse_from_str(raw, TIMED_FORMAT) else {
        return Err(if NaiveDate::parse_from_str(raw, DATE_FORMAT).is_ok() {
            kind_change(field)
        } else {
            invalid("bad_format", Some(field))
        });
    };
    match resolve(zone, naive) {
        Local::At(t) => Ok(t),
        Local::Gap => Err(invalid("nonexistent_local_time", Some(field))),
        Local::Overlap => Err(overlap_question(raw)),
    }
}

fn day(raw: &str, field: &str) -> Result<NaiveDate, Value> {
    NaiveDate::parse_from_str(raw, DATE_FORMAT).map_err(|_| {
        if NaiveDateTime::parse_from_str(raw, TIMED_FORMAT).is_ok() {
            kind_change(field)
        } else {
            invalid("bad_format", Some(field))
        }
    })
}

fn timed_changes(
    t: &Target,
    new_start: Option<&str>,
    new_end: Option<&str>,
    zone: Tz,
    now: DateTime<Utc>,
) -> Result<(Option<i64>, Option<i64>), Value> {
    let start = new_start
        .map(|s| timed(s, "new_start_local", zone))
        .transpose()?;
    let end = new_end
        .map(|e| timed(e, "new_end_local", zone))
        .transpose()?;
    let start_ms = start.map(|s| s.timestamp_millis());
    let end_ms = match (start_ms, end) {
        (_, Some(e)) => Some(e.timestamp_millis()),
        (Some(s), None) => Some(s + (t.end_ms - t.begin_ms)),
        (None, None) => None,
    };
    if let (Some(e), Some(_)) = (end_ms, end)
        && e <= start_ms.unwrap_or(t.begin_ms)
    {
        return Err(invalid("end_not_after_start", Some("new_end_local")));
    }
    if let Some(s) = start {
        check_timed_start(s.with_timezone(&Utc), now)?;
    }
    Ok((start_ms, end_ms))
}

fn utc_midnight_ms(date: NaiveDate) -> i64 {
    date.and_hms_opt(0, 0, 0)
        .unwrap_or_default()
        .and_utc()
        .timestamp_millis()
}

fn all_day_changes(
    t: &Target,
    new_start: Option<&str>,
    new_end: Option<&str>,
    zone: Tz,
    now: DateTime<Utc>,
) -> Result<(Option<i64>, Option<i64>), Value> {
    let date_of = |ms: i64| DateTime::from_timestamp_millis(ms).map(|d| d.date_naive());
    // The stored end is the next UTC midnight; the owner-facing end is the last day, inclusive.
    let last_ms = if t.end_ms > t.begin_ms {
        t.end_ms - MS_PER_DAY.min(t.end_ms - t.begin_ms)
    } else {
        t.end_ms
    };
    let (Some(old_start), Some(old_last)) = (date_of(t.begin_ms), date_of(last_ms)) else {
        return Err(invalid("bad_link", Some("open_link")));
    };
    let start = new_start.map(|s| day(s, "new_start_local")).transpose()?;
    let end = new_end.map(|e| day(e, "new_end_local")).transpose()?;
    let first = start.unwrap_or(old_start);
    let last = match (start, end) {
        (_, Some(e)) => e,
        (Some(s), None) => s + (old_last - old_start),
        (None, None) => old_last,
    };
    if end.is_some() && last < first {
        return Err(invalid("end_not_after_start", Some("new_end_local")));
    }
    if start.is_some() {
        check_all_day_start(first, now, zone)?;
    }
    let after_last = last
        .succ_opt()
        .ok_or_else(|| invalid("bad_format", Some("new_end_local")))?;
    Ok((
        start.map(|_| utc_midnight_ms(first)),
        Some(utc_midnight_ms(after_last)),
    ))
}

/// Validates `args` against the phone's `zone` and `now`. `Err` is the outcome to return.
pub(super) fn check_update(
    args: &Value,
    zone: Tz,
    now: DateTime<Utc>,
) -> Result<(Target, Changes), Value> {
    let Some(map) = args.as_object() else {
        return Err(invalid("arguments_not_an_object", None));
    };
    reject_unknown(map, KNOWN_FIELDS)?;
    let t = target(map, zone)?;
    let title = text(map, "new_title", TITLE_MAX_CHARS, false)?;
    let new_start = text(map, "new_start_local", usize::MAX, false)?;
    let new_end = text(map, "new_end_local", usize::MAX, false)?;
    if title.is_none() && new_start.is_none() && new_end.is_none() {
        return Err(invalid("no_changes", None));
    }
    let (start_ms, end_ms) = if t.all_day {
        all_day_changes(&t, new_start.as_deref(), new_end.as_deref(), zone, now)?
    } else {
        timed_changes(&t, new_start.as_deref(), new_end.as_deref(), zone, now)?
    };
    // An all-day end moves only when a day was named; a bare title change sends no times.
    let moved = new_start.is_some() || new_end.is_some();
    let changes = Changes {
        title,
        start_ms: start_ms.filter(|_| moved),
        end_ms: end_ms.filter(|_| moved),
    };
    Ok((t, changes))
}

fn wire_payload(request_id: &str, zone: Tz, t: &Target, c: &Changes) -> Value {
    let mut changes = serde_json::Map::new();
    if let Some(title) = &c.title {
        changes.insert("title".into(), json!(title));
    }
    if let Some(ms) = c.start_ms {
        changes.insert("start_ms".into(), json!(ms));
    }
    if let Some(ms) = c.end_ms {
        changes.insert("end_ms".into(), json!(ms));
    }
    let mut v = base_payload(request_id, zone, t);
    v["all_day"] = json!(t.all_day);
    v["changes"] = Value::Object(changes);
    v
}

/// Turns the phone's body into what the model sees.
pub fn finalize_update(body: &Value, _payload: &Value) -> Value {
    finalize_modification(
        body,
        outcome::UPDATED,
        UPDATE_PHONE_OUTCOMES,
        outcome::update_unverified,
    )
}

impl DeviceCalendar {
    pub fn update_name(&self) -> &'static str {
        UPDATE_TOOL_NAME
    }

    pub fn update_description(&self) -> &'static str {
        UPDATE_TOOL_DESCRIPTION
    }

    pub fn update_parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "open_link": {"type": "string", "description": "The open_link of the event, copied from calendar_list_events."},
                "title": {"type": "string", "description": "The event's current title exactly as listed."},
                "start_local": {"type": "string", "description": "The event's current start exactly as listed."},
                "new_title": {"type": "string", "description": "1 to 200 characters."},
                "new_start_local": {"type": "string", "description": "Same format as start_local (a date for an all-day event). Keeps the length unless new_end_local is given."},
                "new_end_local": {"type": "string", "description": "Same format as start_local. For an all-day event, the last day, inclusive."}
            },
            "required": ["open_link", "title", "start_local"],
            "additionalProperties": false
        })
    }

    pub async fn update(&self, tool_call_id: &str, args: &Value) -> Value {
        let (target, changes) = match check_update(args, self.phone.zone, self.phone.now()) {
            Ok(checked) => checked,
            Err(refusal) => return refusal,
        };
        let request_id = uuid::Uuid::new_v4().to_string();
        let payload = wire_payload(&request_id, self.phone.zone, &target, &changes);
        self.run_modification(
            UPDATE_ACTION_NAME,
            CALENDAR_EDITING_SKILL,
            tool_call_id,
            payload,
            outcome::update_unverified,
        )
        .await
    }
}
