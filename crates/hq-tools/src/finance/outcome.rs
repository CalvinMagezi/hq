//! The only results `finance_import_messages` returns. Each is a JSON object with an `outcome`
//! code and, for an import, five counts. No message text, name, amount or id is ever in one.

use serde_json::{Value, json};

pub const IMPORTED: &str = "imported";
/// The owner has not allowed the app to read messages yet, or has not turned Finance on.
pub const NOT_SET_UP: &str = "not_set_up";
pub const PERMISSION_MISSING: &str = "permission_missing";
pub const READ_FAILED: &str = "read_failed";
pub const INVALID_REQUEST: &str = "invalid_request";
pub const FINANCE_UNAVAILABLE: &str = "finance_unavailable";

/// What the phone may report. `device_unreachable` is HQ's own verdict and never comes from it.
pub const IMPORT_PHONE_OUTCOMES: &[&str] = &[
    IMPORTED,
    NOT_SET_UP,
    PERMISSION_MISSING,
    READ_FAILED,
    INVALID_REQUEST,
    FINANCE_UNAVAILABLE,
];

pub fn invalid(reason: &str, field: Option<&str>) -> Value {
    let mut v = json!({"outcome": INVALID_REQUEST, "reason": reason});
    if let Some(field) = field {
        v["field"] = json!(field);
    }
    v
}

pub fn unavailable(reason: &str) -> Value {
    json!({"outcome": FINANCE_UNAVAILABLE, "reason": reason})
}
