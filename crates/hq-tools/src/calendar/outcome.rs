//! The only results `calendar_create_event` returns. Each is a JSON object with an `outcome`
//! code; none carries more than the owner's own event details, and none is ever logged whole.

use serde_json::{Value, json};

pub const CREATED: &str = "created";
pub const CLARIFICATION_REQUIRED: &str = "clarification_required";
pub const PERMISSION_DENIED: &str = "permission_denied";
pub const INVALID_EVENT: &str = "invalid_event";
pub const CALENDAR_UNAVAILABLE: &str = "calendar_unavailable";
pub const CREATION_UNVERIFIED: &str = "creation_unverified";

/// What the phone may report. `device_unreachable` is HQ's own verdict (`device_actions::unreachable`) and never comes from it.
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

pub const UPDATED: &str = "updated";
pub const DELETED: &str = "deleted";
pub const EVENT_NOT_FOUND: &str = "event_not_found";
pub const EVENT_CHANGED: &str = "event_changed";
pub const READ_ONLY_CALENDAR: &str = "read_only_calendar";
pub const RECURRING_UNSUPPORTED: &str = "recurring_unsupported";
pub const UPDATE_UNVERIFIED: &str = "update_unverified";
pub const DELETE_UNVERIFIED: &str = "delete_unverified";

/// What the phone may report for `calendar.update`.
pub const UPDATE_PHONE_OUTCOMES: &[&str] = &[
    UPDATED,
    EVENT_NOT_FOUND,
    EVENT_CHANGED,
    READ_ONLY_CALENDAR,
    RECURRING_UNSUPPORTED,
    INVALID_EVENT,
    PERMISSION_DENIED,
    CALENDAR_UNAVAILABLE,
    UPDATE_UNVERIFIED,
];

/// What the phone may report for `calendar.delete`.
pub const DELETE_PHONE_OUTCOMES: &[&str] = &[
    DELETED,
    EVENT_NOT_FOUND,
    EVENT_CHANGED,
    READ_ONLY_CALENDAR,
    RECURRING_UNSUPPORTED,
    INVALID_EVENT,
    PERMISSION_DENIED,
    CALENDAR_UNAVAILABLE,
    DELETE_UNVERIFIED,
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

pub fn unverified(reason: &str) -> Value {
    json!({"outcome": CREATION_UNVERIFIED, "reason": reason})
}

pub fn update_unverified(reason: &str) -> Value {
    json!({"outcome": UPDATE_UNVERIFIED, "reason": reason})
}

pub fn delete_unverified(reason: &str) -> Value {
    json!({"outcome": DELETE_UNVERIFIED, "reason": reason})
}
