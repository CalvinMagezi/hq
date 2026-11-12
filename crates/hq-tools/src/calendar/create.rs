//! `calendar_create_event`: validate here, hand the event to the phone, wait for its answer.
//! The phone writes through Android's Calendar Provider; HQ never learns what a calendar holds
//! beyond the one event it asked for.

use chrono_tz::Tz;
use serde_json::{Value, json};

use super::ACTION_NAME;
use super::check::{Checked, check};
use super::outcome::{self, PHONE_OUTCOMES, SAVED_TO};
use crate::device_actions::{DeviceLink, DeviceToolSpec, RiskClass, RunFuture, outcome_is_known};
use crate::skills::builtin::CALENDAR_CREATION_SKILL;

pub const TOOL_NAME: &str = "calendar_create_event";

const TOOL_DESCRIPTION: &str = "Create one event in the owner's phone calendar, directly, with no \
    review screen. Load the calendar-creation skill first. Dates are the phone's local time, \
    already resolved by you. Attendees, recurrence and reminders are not supported.";

pub(super) const SPEC: DeviceToolSpec = DeviceToolSpec {
    name: TOOL_NAME,
    wire_action: ACTION_NAME,
    risk: RiskClass::Create,
    skill: CALENDAR_CREATION_SKILL,
    phone_outcomes: PHONE_OUTCOMES,
    description: TOOL_DESCRIPTION,
    parameters,
    describe,
    finalize,
    unavailable: outcome::unavailable,
    lost: outcome::unverified,
    run,
};

fn run<'a>(link: &'a DeviceLink, tool_call_id: &'a str, args: &'a Value) -> RunFuture<'a> {
    Box::pin(run_create(link, tool_call_id, args))
}

fn describe(_args: &Value) -> String {
    "would add an event to the owner's phone calendar".into()
}

fn parameters() -> Value {
    json!({
        "type": "object",
        "properties": {
            "title": {"type": "string", "description": "1 to 200 characters."},
            "start_local": {
                "type": "string",
                "description": "Phone-local start: YYYY-MM-DDTHH:MM, or YYYY-MM-DD when all_day. No offset."
            },
            "end_local": {"type": "string", "description": "Phone-local end, same format. Give this or duration_minutes, not both."},
            "duration_minutes": {"type": "integer", "minimum": 1, "maximum": 1440},
            "all_day": {"type": "boolean", "description": "Only when the owner said all day."},
            "location": {"type": "string", "description": "Only if the owner gave one."},
            "description": {"type": "string", "description": "Only if the owner gave one."}
        },
        "required": ["title", "start_local"],
        "additionalProperties": false
    })
}

/// Runs one call. Always returns one of the contract's outcomes, never an error.
pub async fn run_create(link: &DeviceLink, tool_call_id: &str, args: &Value) -> Value {
    if let Some(refusal) = link.skill_gate(&SPEC) {
        return refusal;
    }
    let checked = match check(args, link.phone.zone, link.phone.now()) {
        Ok(c) => c,
        Err(refusal) => return refusal,
    };
    let request_id = uuid::Uuid::new_v4().to_string();
    let payload = wire_payload(&request_id, link.phone.zone, &checked);
    link.round_trip(&SPEC, tool_call_id, payload).await
}

fn wire_payload(request_id: &str, zone: Tz, c: &Checked) -> Value {
    let mut v = json!({
        "request_id": request_id,
        "title": c.title,
        "start_local": c.start_local,
        "end_local": c.end_local,
        "time_zone": zone.name(),
        "all_day": c.all_day,
        "duration_defaulted": c.duration_defaulted,
    });
    if let Some(l) = &c.location {
        v["location"] = json!(l);
    }
    if let Some(d) = &c.description {
        v["description"] = json!(d);
    }
    v
}

/// Whether `body` is a result worth storing for a `calendar.create`: a known version and a
/// known outcome.
pub fn result_is_acceptable(body: &Value) -> bool {
    outcome_is_known(body, PHONE_OUTCOMES)
}

const CREATED_FIELDS: &[&str] = &[
    "event_ref",
    "title",
    "start_local",
    "end_local",
    "time_zone",
    "calendar_label",
];

/// Turns the phone's body into the outcome the model sees. Keeps only the fields the contract
/// names, adds what HQ alone knows, and checks that a created event is the one asked for.
pub fn finalize(body: &Value, payload: &Value) -> Value {
    let code = body.get("outcome").and_then(Value::as_str).unwrap_or("");
    if code != outcome::CREATED {
        return pass_through(code, body);
    }
    let mut out = json!({"outcome": outcome::CREATED});
    for field in CREATED_FIELDS {
        match body.get(*field).and_then(Value::as_str) {
            Some(s) if !s.is_empty() => out[*field] = json!(clean(s, PHONE_TEXT_MAX_CHARS)),
            _ => return outcome::unverified("incomplete_result"),
        }
    }
    let same = |key: &str| body.get(key) == payload.get(key);
    // An all-day event is stored in UTC whatever the phone's zone, so only a timed one is compared.
    let all_day = payload.get("all_day").and_then(Value::as_bool) == Some(true);
    if !(same("title") && same("start_local") && (all_day || same("time_zone"))) {
        return outcome::unverified("result_differs_from_request");
    }
    out["duration_defaulted"] = payload
        .get("duration_defaulted")
        .cloned()
        .unwrap_or(json!(false));
    out["saved_to"] = json!(SAVED_TO);
    out
}

/// Longest phone-supplied string the model is shown. Calendar names and accounts can be set by
/// whoever shared a calendar, so they are bounded and stripped of line breaks before any model sees them.
pub(super) const PHONE_TEXT_MAX_CHARS: usize = 200;
/// Most calendars offered as choices in one question.
const MAX_CHOICES: usize = 8;

/// One line, no control characters, at most `max` characters.
pub(super) fn clean(raw: &str, max: usize) -> String {
    let one_line: String = raw
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    one_line
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(max)
        .collect()
}

pub(super) fn pass_through(code: &str, body: &Value) -> Value {
    let keep: &[&str] = match code {
        outcome::CLARIFICATION_REQUIRED => &["reason", "ask", "choices"],
        outcome::PERMISSION_DENIED => &["missing", "next_step"],
        outcome::INVALID_EVENT => &["reason", "field"],
        _ => &["reason"],
    };
    let mut out = json!({"outcome": code});
    for key in keep {
        match body.get(*key) {
            Some(Value::String(s)) => out[*key] = json!(clean(s, PHONE_TEXT_MAX_CHARS)),
            Some(Value::Array(items)) => {
                let strings: Vec<String> = items
                    .iter()
                    .filter_map(Value::as_str)
                    .take(MAX_CHOICES)
                    .map(|s| clean(s, PHONE_TEXT_MAX_CHARS))
                    .collect();
                out[*key] = json!(strings);
            }
            _ => {}
        }
    }
    out
}
