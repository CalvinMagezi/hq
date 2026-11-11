//! `calendar_create_event`: validate here, hand the event to the phone, wait for its answer.
//! The phone writes through Android's Calendar Provider; HQ never learns what a calendar holds
//! beyond the one event it asked for.

use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use serde_json::{Value, json};
use tokio::sync::watch;

use super::check::{Checked, check};
use super::outcome::{self, PHONE_OUTCOMES, SAVED_TO};
use super::store::{self, DeviceActionStore, Opened, Phase, Timeouts};
use crate::skills::builtin::{self, CALENDAR_CREATION_SKILL};

pub const TOOL_NAME: &str = "calendar_create_event";

const TOOL_DESCRIPTION: &str = "Create one event in the owner's phone calendar, directly, with no \
    review screen. Load the calendar-creation skill first. Dates are the phone's local time, \
    already resolved by you. Attendees, recurrence and reminders are not supported.";

/// The phone's zone and clock as the app last reported them, advanced by the time since.
/// HQ never reads its own clock for a calendar: the phone is where the event lives.
#[derive(Debug, Clone)]
pub struct PhoneClock {
    pub zone: Tz,
    reported: DateTime<Utc>,
    received: Instant,
}

impl PhoneClock {
    /// `None` when the zone is not an IANA name or the time is not RFC 3339: the tool is then
    /// not offered, since a guessed zone would put an event at the wrong hour.
    pub fn parse(zone: &str, now_utc: &str) -> Option<Self> {
        let zone: Tz = zone.trim().parse().ok()?;
        let reported = DateTime::parse_from_rfc3339(now_utc.trim())
            .ok()?
            .with_timezone(&Utc);
        Some(Self {
            zone,
            reported,
            received: Instant::now(),
        })
    }

    pub fn now(&self) -> DateTime<Utc> {
        self.reported + chrono::TimeDelta::from_std(self.received.elapsed()).unwrap_or_default()
    }
}

pub(super) type Emit = Arc<dyn Fn(Value) + Send + Sync>;

/// One chat turn's link to the phone that sent it.
pub struct DeviceCalendar {
    pub(super) thread_id: String,
    pub(super) store: Arc<DeviceActionStore>,
    pub(super) emit: Emit,
    pub(super) phone: PhoneClock,
    pub(super) timeouts: Timeouts,
}

impl DeviceCalendar {
    pub fn new(
        thread_id: impl Into<String>,
        store: Arc<DeviceActionStore>,
        emit: Emit,
        phone: PhoneClock,
    ) -> Self {
        Self {
            thread_id: thread_id.into(),
            store,
            emit,
            phone,
            timeouts: Timeouts::default(),
        }
    }

    pub fn with_timeouts(mut self, timeouts: Timeouts) -> Self {
        self.timeouts = timeouts;
        self
    }

    pub fn name(&self) -> &'static str {
        TOOL_NAME
    }

    pub fn description(&self) -> &'static str {
        TOOL_DESCRIPTION
    }

    pub fn parameters(&self) -> Value {
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

    /// The zone and date the model needs to resolve words like "tomorrow".
    pub fn context_line(&self) -> String {
        let local = self.phone.now().with_timezone(&self.phone.zone);
        format!(
            "The owner's phone is in {} and it is {} there.",
            self.phone.zone.name(),
            local.format("%A %Y-%m-%d %H:%M")
        )
    }

    /// Runs one call. Always returns one of the contract's outcomes, never an error.
    pub async fn create(&self, tool_call_id: &str, args: &Value) -> Value {
        if builtin::shipped_hash(CALENDAR_CREATION_SKILL).is_none()
            || !builtin::is_active(CALENDAR_CREATION_SKILL)
        {
            return outcome::unavailable("skill_not_active");
        }
        let checked = match check(args, self.phone.zone, self.phone.now()) {
            Ok(c) => c,
            Err(refusal) => return refusal,
        };
        let request_id = uuid::Uuid::new_v4().to_string();
        let payload = wire_payload(&request_id, self.phone.zone, &checked);
        let opened = self.store.open(
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
                return outcome.unwrap_or_else(|| outcome::unverified("still_waiting"));
            }
            Err(_) => return outcome::unavailable("too_many_pending"),
        };
        tracing::info!(%request_id, %action_id, phase = "sent", "device action");
        (self.emit)(frame);
        let result = self.wait(&action_id, phase, outcome::unverified).await;
        tracing::info!(%request_id, %action_id, phase = "done", outcome = outcome::code(&result), "device action");
        result
    }

    /// `lost` builds the outcome for "the answer never came", which differs by action: a create
    /// may have happened, a read has nothing to report.
    pub(super) async fn wait(
        &self,
        action_id: &str,
        mut phase: watch::Receiver<Phase>,
        lost: fn(&str) -> Value,
    ) -> Value {
        let ack = Duration::from_secs(self.timeouts.ack_secs);
        let result = Duration::from_secs(self.timeouts.result_secs);
        let first = tokio::time::timeout(ack, phase.wait_for(|p| *p != Phase::Open))
            .await
            .map(|r| r.map(|p| *p));
        match first {
            Ok(Ok(Phase::Abandoned)) => return outcome::unreachable(),
            Ok(Ok(_)) => {}
            // The sender lives in the store; losing it means the action was pruned.
            Ok(Err(_)) => return lost("action_lost"),
            Err(_) if self.store.abandon_if_unacked(action_id) => return outcome::unreachable(),
            // The ack landed in the instant the timer fired.
            Err(_) => {}
        }
        let second = tokio::time::timeout(result, phase.wait_for(|p| *p == Phase::Settled)).await;
        if second.is_err() {
            self.store.give_up(action_id);
            return lost("no_result_in_time");
        }
        self.store
            .outcome(action_id)
            .unwrap_or_else(|| lost("action_lost"))
    }
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

pub(super) fn outcome_is_known(body: &Value, allowed: &[&str]) -> bool {
    body.get("v").and_then(Value::as_u64) == Some(1)
        && body
            .get("outcome")
            .and_then(Value::as_str)
            .is_some_and(|o| allowed.contains(&o))
}

/// `result_is_acceptable` for the action called `name`. An unknown name accepts nothing.
pub fn result_is_acceptable_for(name: &str, body: &Value) -> bool {
    match name {
        store::ACTION_NAME => result_is_acceptable(body),
        store::LIST_ACTION_NAME => outcome_is_known(body, outcome::LIST_PHONE_OUTCOMES),
        _ => false,
    }
}

/// `finalize` for the action called `name`.
pub fn finalize_for(name: &str, body: &Value, payload: &Value) -> Value {
    match name {
        store::LIST_ACTION_NAME => super::list::finalize_list(body, payload),
        _ => finalize(body, payload),
    }
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
