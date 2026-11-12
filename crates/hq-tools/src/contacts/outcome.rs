//! The only results `contacts_search` returns. Each is a JSON object with an `outcome` code.

use serde_json::{Value, json};

pub const LISTED: &str = "listed";
pub const PERMISSION_DENIED: &str = "permission_denied";
pub const INVALID_QUERY: &str = "invalid_query";
pub const CONTACTS_UNAVAILABLE: &str = "contacts_unavailable";

/// What the phone may report. `device_unreachable` is HQ's own verdict and never comes from it.
pub const SEARCH_PHONE_OUTCOMES: &[&str] = &[
    LISTED,
    PERMISSION_DENIED,
    INVALID_QUERY,
    CONTACTS_UNAVAILABLE,
];

pub fn invalid(reason: &str, field: Option<&str>) -> Value {
    let mut v = json!({"outcome": INVALID_QUERY, "reason": reason});
    if let Some(field) = field {
        v["field"] = json!(field);
    }
    v
}

pub fn unavailable(reason: &str) -> Value {
    json!({"outcome": CONTACTS_UNAVAILABLE, "reason": reason})
}
