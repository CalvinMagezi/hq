//! The only results `calendar_create_event` returns. Each is a JSON object with an `outcome`
//! code; none carries more than the owner's own event details, and none is ever logged whole.

use serde_json::{Value, json};

pub const CREATED: &str = "created";
pub const CLARIFICATION_REQUIRED: &str = "clarification_required";
pub const PERMISSION_DENIED: &str = "permission_denied";
pub const INVALID_EVENT: &str = "invalid_event";
pub const CALENDAR_UNAVAILABLE: &str = "calendar_unavailable";
pub const DEVICE_UNREACHABLE: &str = "device_unreachable";
pub const CREATION_UNVERIFIED: &str = "creation_unverified";

/// What the phone may report. `device_unreachable` is HQ's own verdict and never comes from it.
pub const PHONE_OUTCOMES: &[&str] = &[
    CREATED,
    CLARIFICATION_REQUIRED,
    PERMISSION_DENIED,
    INVALID_EVENT,
    CALENDAR_UNAVAILABLE,
    CREATION_UNVERIFIED,
];

pub const LISTED: &str = "listed";

/// What the phone may report for `calendar.list`.
pub const LIST_PHONE_OUTCOMES: &[&str] = &[
    LISTED,
    PERMISSION_DENIED,
    CALENDAR_UNAVAILABLE,
    INVALID_EVENT,
];

/// Where a created event lives. Says nothing about cloud sync, which HQ cannot know.
pub const SAVED_TO: &str = "device_calendar";

pub fn invalid(reason: &str, field: Option<&str>) -> Value {
    let mut v = json!({"outcome": INVALID_EVENT, "reason": reason});
    if let Some(field) = field {
        v["field"] = json!(field);
    }
    v
}

pub fn clarification(reason: &str, ask: &str, choices: &[String]) -> Value {
    let mut v = json!({"outcome": CLARIFICATION_REQUIRED, "reason": reason, "ask": ask});
    if !choices.is_empty() {
        v["choices"] = json!(choices);
    }
    v
}

pub fn unavailable(reason: &str) -> Value {
    json!({"outcome": CALENDAR_UNAVAILABLE, "reason": reason})
}

pub fn unreachable() -> Value {
    json!({"outcome": DEVICE_UNREACHABLE})
}

pub fn unverified(reason: &str) -> Value {
    json!({"outcome": CREATION_UNVERIFIED, "reason": reason})
}

/// The outcome code, for logs.
pub fn code(outcome: &Value) -> &str {
    outcome
        .get("outcome")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
}
