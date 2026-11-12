//! A made-up domain with one tool per risk class that matters, registered through the same
//! table the calendar uses. Tests in this and other crates use it to show that taint, approval
//! and result filtering follow from the risk class alone: nothing in governance or the agent
//! wrapper names these tools.

use serde_json::{Value, json};

use super::spec::{DeviceToolSpec, RiskClass, RunFuture, register};
use super::{DeviceLink, code};
use crate::skills::builtin::CALENDAR_READING_SKILL;

pub const FAKE_READ: &str = "fake_read";
pub const FAKE_ADD: &str = "fake_add";
pub const FAKE_DELETE: &str = "fake_delete";
pub const FAKE_SEND: &str = "fake_send";

pub const FAKE_READ_ACTION: &str = "fake.read";
pub const FAKE_ADD_ACTION: &str = "fake.add";
pub const FAKE_DELETE_ACTION: &str = "fake.delete";
pub const FAKE_SEND_ACTION: &str = "fake.send";

/// The phone's outcomes for the fake send, including one the fake does not know.
pub const SENT: &str = "sent";
pub const FAKE_OUTCOMES: &[&str] = &["listed", "added", "deleted", SENT, "refused"];

/// Reuses a shipped skill: the gate only asks whether the named skill is active.
const SKILL: &str = CALENDAR_READING_SKILL;

pub const FAKE_SPECS: &[DeviceToolSpec] = &[
    spec(FAKE_READ, FAKE_READ_ACTION, RiskClass::Read, run_read),
    spec(FAKE_ADD, FAKE_ADD_ACTION, RiskClass::Create, run_add),
    spec(
        FAKE_DELETE,
        FAKE_DELETE_ACTION,
        RiskClass::Destructive,
        run_delete,
    ),
    spec(FAKE_SEND, FAKE_SEND_ACTION, RiskClass::Outbound, run_send),
];

const fn spec(
    name: &'static str,
    wire_action: &'static str,
    risk: RiskClass,
    run: super::DeviceRun,
) -> DeviceToolSpec {
    DeviceToolSpec {
        name,
        wire_action,
        risk,
        skill: SKILL,
        phone_outcomes: FAKE_OUTCOMES,
        description: "A fake device tool for tests.",
        parameters,
        describe,
        finalize,
        unavailable,
        lost,
        run,
    }
}

/// Makes the fake tools known to the lookups governance and the result route use.
pub fn register_fake() {
    for spec in FAKE_SPECS {
        register(spec);
    }
}

fn parameters() -> Value {
    json!({"type": "object", "properties": {}, "additionalProperties": false})
}

fn describe(args: &Value) -> String {
    format!(
        "would act on {}",
        args.get("target")
            .and_then(Value::as_str)
            .unwrap_or("a thing")
    )
}

/// Keeps only the outcome and a reason, whatever else the phone sent.
fn finalize(body: &Value, _payload: &Value) -> Value {
    let mut out = json!({"outcome": body["outcome"]});
    if let Some(reason) = body.get("reason") {
        out["reason"] = reason.clone();
    }
    out
}

fn unavailable(reason: &str) -> Value {
    json!({"outcome": "fake_unavailable", "reason": reason})
}

fn lost(reason: &str) -> Value {
    json!({"outcome": "fake_unverified", "reason": reason})
}

async fn call(link: &DeviceLink, spec: &DeviceToolSpec, id: &str) -> Value {
    let payload = json!({"request_id": uuid::Uuid::new_v4().to_string()});
    let out = link.round_trip(spec, id, payload).await;
    tracing::debug!(outcome = code(&out), "fake device action");
    out
}

fn run_read<'a>(link: &'a DeviceLink, id: &'a str, _args: &'a Value) -> RunFuture<'a> {
    Box::pin(call(link, &FAKE_SPECS[0], id))
}

fn run_add<'a>(link: &'a DeviceLink, id: &'a str, _args: &'a Value) -> RunFuture<'a> {
    Box::pin(call(link, &FAKE_SPECS[1], id))
}

fn run_delete<'a>(link: &'a DeviceLink, id: &'a str, _args: &'a Value) -> RunFuture<'a> {
    Box::pin(call(link, &FAKE_SPECS[2], id))
}

fn run_send<'a>(link: &'a DeviceLink, id: &'a str, _args: &'a Value) -> RunFuture<'a> {
    Box::pin(call(link, &FAKE_SPECS[3], id))
}
