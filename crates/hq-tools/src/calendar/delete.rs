//! `calendar_delete_event`: validate the event link here, ask the phone to delete that one event
//! if it still looks the way the owner approved, and report what the phone read back.

use chrono_tz::Tz;
use serde_json::{Value, json};

use super::create::DeviceCalendar;
use super::outcome::{self, DELETE_PHONE_OUTCOMES, invalid};
use super::store::DELETE_ACTION_NAME;
use super::target::{Target, base_payload, finalize_modification, reject_unknown, target};
use crate::skills::builtin::CALENDAR_EDITING_SKILL;

pub const DELETE_TOOL_NAME: &str = "calendar_delete_event";

const KNOWN_FIELDS: &[&str] = &["open_link", "title", "start_local"];

const DELETE_TOOL_DESCRIPTION: &str = "Delete one event from the owner's phone calendar. Load \
    the calendar-editing skill first. Name the event with the open_link, title and start_local \
    that calendar_list_events returned; never build a link. The owner is always asked to approve. \
    Recurring events are not supported.";

/// Validates `args` against the phone's `zone`. `Err` is the outcome to return.
pub(super) fn check_delete(args: &Value, zone: Tz) -> Result<Target, Value> {
    let Some(map) = args.as_object() else {
        return Err(invalid("arguments_not_an_object", None));
    };
    reject_unknown(map, KNOWN_FIELDS)?;
    target(map, zone)
}

/// Turns the phone's body into what the model sees.
pub fn finalize_delete(body: &Value, _payload: &Value) -> Value {
    finalize_modification(
        body,
        outcome::DELETED,
        DELETE_PHONE_OUTCOMES,
        outcome::delete_unverified,
    )
}

impl DeviceCalendar {
    pub fn delete_name(&self) -> &'static str {
        DELETE_TOOL_NAME
    }

    pub fn delete_description(&self) -> &'static str {
        DELETE_TOOL_DESCRIPTION
    }

    pub fn delete_parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "open_link": {"type": "string", "description": "The open_link of the event, copied from calendar_list_events."},
                "title": {"type": "string", "description": "The event's title exactly as listed."},
                "start_local": {"type": "string", "description": "The event's start exactly as listed."}
            },
            "required": ["open_link", "title", "start_local"],
            "additionalProperties": false
        })
    }

    pub async fn delete(&self, tool_call_id: &str, args: &Value) -> Value {
        let target = match check_delete(args, self.phone.zone) {
            Ok(t) => t,
            Err(refusal) => return refusal,
        };
        let request_id = uuid::Uuid::new_v4().to_string();
        let payload = base_payload(&request_id, self.phone.zone, &target);
        self.run_modification(
            DELETE_ACTION_NAME,
            CALENDAR_EDITING_SKILL,
            tool_call_id,
            payload,
            outcome::delete_unverified,
        )
        .await
    }
}
