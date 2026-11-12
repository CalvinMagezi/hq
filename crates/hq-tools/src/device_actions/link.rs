//! One chat turn's link to the phone that sent it: the phone's clock, the way to push a frame to
//! it, and the open-wait-settle round trip every device tool shares. What a tool asks for and how
//! it reads the answer belongs to its domain; this is only the plumbing.

use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use serde_json::{Value, json};
use tokio::sync::watch;

use super::spec::DeviceToolSpec;
use super::store::{DeviceActionStore, Opened, Phase, Timeouts};
use crate::skills::builtin;

pub const DEVICE_UNREACHABLE: &str = "device_unreachable";

/// HQ's own verdict that the phone never took the action; the phone never reports it.
pub fn unreachable() -> Value {
    json!({"outcome": DEVICE_UNREACHABLE})
}

/// The outcome code, for logs.
pub fn code(outcome: &Value) -> &str {
    outcome
        .get("outcome")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
}

/// The phone's zone and clock as the app last reported them, advanced by the time since.
/// HQ never reads its own clock for a device action: the phone is where the thing lives.
#[derive(Debug, Clone)]
pub struct PhoneClock {
    pub zone: Tz,
    reported: DateTime<Utc>,
    received: Instant,
}

impl PhoneClock {
    /// `None` when the zone is not an IANA name or the time is not RFC 3339: the tools are then
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

pub type Emit = Arc<dyn Fn(Value) + Send + Sync>;

pub struct DeviceLink {
    pub(crate) thread_id: String,
    pub(crate) store: Arc<DeviceActionStore>,
    pub(crate) emit: Emit,
    pub(crate) phone: PhoneClock,
    pub(crate) timeouts: Timeouts,
}

impl DeviceLink {
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

    /// The zone and date the model needs to resolve words like "tomorrow".
    pub fn context_line(&self) -> String {
        let local = self.phone.now().with_timezone(&self.phone.zone);
        format!(
            "The owner's phone is in {} and it is {} there.",
            self.phone.zone.name(),
            local.format("%A %Y-%m-%d %H:%M")
        )
    }

    /// The refusal to return when the skill that teaches `spec`'s tool is not shipped and
    /// active, or `None`. Fails closed: a tool nobody told the model how to use does not run.
    pub fn skill_gate(&self, spec: &DeviceToolSpec) -> Option<Value> {
        (builtin::shipped_hash(spec.skill).is_none() || !builtin::is_active(spec.skill))
            .then(|| (spec.unavailable)("skill_not_active"))
    }

    /// Sends one validated payload to the phone and waits for its answer. The payload names its
    /// own `request_id`, which makes a repeated tool call id idempotent. Always returns one of
    /// the domain's outcomes, never an error. Logs hold ids and codes, never a payload field.
    pub async fn round_trip(
        &self,
        spec: &DeviceToolSpec,
        tool_call_id: &str,
        payload: Value,
    ) -> Value {
        if let Some(refusal) = self.skill_gate(spec) {
            return refusal;
        }
        let request_id = payload["request_id"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let opened = self.store.open_named(
            spec.wire_action,
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
                return outcome.unwrap_or_else(|| (spec.lost)("still_waiting"));
            }
            Err(_) => return (spec.unavailable)("too_many_pending"),
        };
        let action = spec.wire_action;
        tracing::info!(%request_id, %action_id, action, phase = "sent", "device action");
        (self.emit)(frame);
        let result = self.wait(spec, &action_id, phase).await;
        tracing::info!(
            %request_id, %action_id, action, phase = "done", outcome = code(&result),
            "device action"
        );
        result
    }

    async fn wait(
        &self,
        spec: &DeviceToolSpec,
        action_id: &str,
        mut phase: watch::Receiver<Phase>,
    ) -> Value {
        let lost = spec.lost;
        let ack = Duration::from_secs(self.timeouts.ack_secs);
        let result = Duration::from_secs(self.timeouts.result_secs);
        let first = tokio::time::timeout(ack, phase.wait_for(|p| *p != Phase::Open))
            .await
            .map(|r| r.map(|p| *p));
        match first {
            Ok(Ok(Phase::Abandoned)) => return unreachable(),
            Ok(Ok(_)) => {}
            // The sender lives in the store; losing it means the action was pruned.
            Ok(Err(_)) => return lost("action_lost"),
            Err(_) if self.store.abandon_if_unacked(action_id) => return unreachable(),
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
