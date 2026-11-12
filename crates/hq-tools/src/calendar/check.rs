//! Everything HQ checks before a request reaches the phone. Pure: the caller supplies the
//! phone's zone and clock, so the same input always gives the same answer.

use chrono::{DateTime, LocalResult, Months, NaiveDate, NaiveDateTime, TimeDelta, TimeZone, Utc};
use chrono_tz::Tz;
use serde_json::{Map, Value};

use super::outcome::{clarification, invalid};

pub const TITLE_MAX_CHARS: usize = 200;
pub const LOCATION_MAX_CHARS: usize = 200;
pub const DESCRIPTION_MAX_CHARS: usize = 2000;
pub const DURATION_MIN_MINUTES: i64 = 1;
pub const DURATION_MAX_MINUTES: i64 = 1440;
pub const DEFAULT_DURATION_MINUTES: i64 = 60;
/// A timed start this far behind the phone's clock still counts as "now".
pub const PAST_TOLERANCE_MINUTES: i64 = 5;
/// Beyond this many months ahead the date is more likely a slip of the year than a plan.
pub const HORIZON_MONTHS: u32 = 60;

pub const TIMED_FORMAT: &str = "%Y-%m-%dT%H:%M";
pub const DATE_FORMAT: &str = "%Y-%m-%d";

/// Fields the model must not set meaningfully: HQ writes them itself.
const IGNORED_FIELDS: &[&str] = &["request_id", "time_zone"];
/// Named in the contract as later work; passing one is an error, not something to drop silently.
const UNSUPPORTED_FIELDS: &[&str] = &["attendees", "recurrence", "reminders"];
const KNOWN_FIELDS: &[&str] = &[
    "title",
    "start_local",
    "end_local",
    "duration_minutes",
    "all_day",
    "location",
    "description",
];
/// Runtime-owned argument prefix, never model input.
const RESERVED_PREFIX: &str = "_hq_";

/// A request that passed every check, in the form the phone receives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checked {
    pub title: String,
    pub start_local: String,
    /// Always resolved, so the phone does no calendar arithmetic. For an all-day event it is the
    /// last day, inclusive.
    pub end_local: String,
    pub all_day: bool,
    pub location: Option<String>,
    pub description: Option<String>,
    pub duration_defaulted: bool,
}

pub(super) enum Local {
    At(DateTime<Tz>),
    Gap,
    Overlap,
}

pub(super) fn resolve(zone: Tz, naive: NaiveDateTime) -> Local {
    match zone.from_local_datetime(&naive) {
        LocalResult::Single(t) => Local::At(t),
        LocalResult::Ambiguous(..) => Local::Overlap,
        LocalResult::None => Local::Gap,
    }
}

/// Validates `args` against the phone's `zone` and `now`. `Err` is the outcome to return.
pub fn check(args: &Value, zone: Tz, now: DateTime<Utc>) -> Result<Checked, Value> {
    let Some(map) = args.as_object() else {
        return Err(invalid("arguments_not_an_object", None));
    };
    reject_fields(map)?;
    let title = text(map, "title", TITLE_MAX_CHARS, true)?.unwrap_or_default();
    let location = text(map, "location", LOCATION_MAX_CHARS, false)?;
    let description = text(map, "description", DESCRIPTION_MAX_CHARS, false)?;
    let all_day = flag(map, "all_day")?;
    let start_raw = text(map, "start_local", usize::MAX, true)?.unwrap_or_default();
    let end_raw = text(map, "end_local", usize::MAX, false)?;
    let minutes = duration(map)?;
    if end_raw.is_some() && minutes.is_some() {
        return Err(invalid(
            "end_and_duration_both_given",
            Some("duration_minutes"),
        ));
    }
    let (start_local, end_local, duration_defaulted) = if all_day {
        all_day_span(&start_raw, end_raw.as_deref(), minutes, now, zone)?
    } else {
        timed_span(&start_raw, end_raw.as_deref(), minutes, now, zone)?
    };
    Ok(Checked {
        title,
        start_local,
        end_local,
        all_day,
        location,
        description,
        duration_defaulted,
    })
}

fn reject_fields(map: &Map<String, Value>) -> Result<(), Value> {
    for key in map.keys() {
        if UNSUPPORTED_FIELDS.contains(&key.as_str()) && !map[key].is_null() {
            return Err(invalid("unsupported_field", Some(key)));
        }
        let known = KNOWN_FIELDS.contains(&key.as_str())
            || IGNORED_FIELDS.contains(&key.as_str())
            || UNSUPPORTED_FIELDS.contains(&key.as_str())
            || key.starts_with(RESERVED_PREFIX);
        if !known {
            return Err(invalid("unknown_field", Some(key)));
        }
    }
    Ok(())
}

/// An optional or required trimmed string within `max` characters. Empty counts as absent.
pub(super) fn text(
    map: &Map<String, Value>,
    key: &str,
    max: usize,
    required: bool,
) -> Result<Option<String>, Value> {
    let value = match map.get(key) {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.trim().to_string()).filter(|s| !s.is_empty()),
        Some(_) => return Err(invalid("wrong_type", Some(key))),
    };
    match value {
        None if required => Err(invalid("missing", Some(key))),
        Some(s) if s.chars().count() > max => Err(invalid("too_long", Some(key))),
        other => Ok(other),
    }
}

fn flag(map: &Map<String, Value>, key: &str) -> Result<bool, Value> {
    match map.get(key) {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(b)) => Ok(*b),
        Some(_) => Err(invalid("wrong_type", Some(key))),
    }
}

fn duration(map: &Map<String, Value>) -> Result<Option<i64>, Value> {
    let field = "duration_minutes";
    match map.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => match v.as_i64() {
            Some(m) if (DURATION_MIN_MINUTES..=DURATION_MAX_MINUTES).contains(&m) => Ok(Some(m)),
            Some(_) => Err(invalid("out_of_range", Some(field))),
            None => Err(invalid("wrong_type", Some(field))),
        },
    }
}

fn timed_span(
    start_raw: &str,
    end_raw: Option<&str>,
    minutes: Option<i64>,
    now: DateTime<Utc>,
    zone: Tz,
) -> Result<(String, String, bool), Value> {
    let start_naive = NaiveDateTime::parse_from_str(start_raw, TIMED_FORMAT)
        .map_err(|_| invalid("bad_format", Some("start_local")))?;
    let end_naive = end_raw
        .map(|e| {
            NaiveDateTime::parse_from_str(e, TIMED_FORMAT)
                .map_err(|_| invalid("bad_format", Some("end_local")))
        })
        .transpose()?;
    // A time that never happened is a mistake to fix; one that happens twice needs the owner.
    let start = match resolve(zone, start_naive) {
        Local::Gap => return Err(invalid("nonexistent_local_time", Some("start_local"))),
        Local::Overlap => return Err(overlap_question(start_raw)),
        Local::At(t) => t,
    };
    let (end, defaulted) = match end_naive {
        Some(naive) => match resolve(zone, naive) {
            Local::Gap => return Err(invalid("nonexistent_local_time", Some("end_local"))),
            Local::Overlap => return Err(overlap_question(end_raw.unwrap_or_default())),
            Local::At(t) => (t, false),
        },
        None => {
            let m = minutes.unwrap_or(DEFAULT_DURATION_MINUTES);
            (start + TimeDelta::minutes(m), minutes.is_none())
        }
    };
    if end <= start {
        return Err(invalid("end_not_after_start", Some("end_local")));
    }
    check_timed_start(start.with_timezone(&Utc), now)?;
    Ok((
        start.format(TIMED_FORMAT).to_string(),
        end.format(TIMED_FORMAT).to_string(),
        defaulted,
    ))
}

fn all_day_span(
    start_raw: &str,
    end_raw: Option<&str>,
    minutes: Option<i64>,
    now: DateTime<Utc>,
    zone: Tz,
) -> Result<(String, String, bool), Value> {
    if minutes.is_some() {
        return Err(invalid("duration_with_all_day", Some("duration_minutes")));
    }
    let date = |raw: &str, field: &str| {
        NaiveDate::parse_from_str(raw, DATE_FORMAT).map_err(|_| invalid("bad_format", Some(field)))
    };
    let start = date(start_raw, "start_local")?;
    let end = end_raw
        .map(|e| date(e, "end_local"))
        .transpose()?
        .unwrap_or(start);
    if end < start {
        return Err(invalid("end_not_after_start", Some("end_local")));
    }
    check_all_day_start(start, now, zone)?;
    Ok((
        start.format(DATE_FORMAT).to_string(),
        end.format(DATE_FORMAT).to_string(),
        false,
    ))
}

/// A new timed start may not be in the past (beyond the tolerance) or years away.
pub(super) fn check_timed_start(start: DateTime<Utc>, now: DateTime<Utc>) -> Result<(), Value> {
    if start < now - TimeDelta::minutes(PAST_TOLERANCE_MINUTES) {
        return Err(clarification(
            "start_in_past",
            "That start time has already passed. Which date and time did you mean?",
            &[],
        ));
    }
    too_far(start, now)
}

/// An all-day start has no past check, only the horizon, judged at local midnight.
pub(super) fn check_all_day_start(
    start: NaiveDate,
    now: DateTime<Utc>,
    zone: Tz,
) -> Result<(), Value> {
    let midnight = start.and_hms_opt(0, 0, 0).unwrap_or_default();
    let start_utc = zone
        .from_local_datetime(&midnight)
        .earliest()
        .map_or_else(|| midnight.and_utc(), |t| t.with_timezone(&Utc));
    too_far(start_utc, now)
}

fn too_far(start: DateTime<Utc>, now: DateTime<Utc>) -> Result<(), Value> {
    let horizon = now
        .checked_add_months(Months::new(HORIZON_MONTHS))
        .unwrap_or(now);
    if start > horizon {
        return Err(clarification(
            "start_too_far_ahead",
            "That is more than five years away. Is the date right?",
            &[],
        ));
    }
    Ok(())
}

pub(super) fn overlap_question(local: &str) -> Value {
    clarification(
        "ambiguous_local_time",
        &format!(
            "{local} happens twice that night because the clocks go back. Which one did you mean?"
        ),
        &["the first one".to_string(), "the second one".to_string()],
    )
}
