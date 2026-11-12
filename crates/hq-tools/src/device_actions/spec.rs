//! What each device tool is, as data: its name on the wire, how risky it is, which skill teaches
//! it, and the outcomes the phone may report. Governance, the result route and the agent wrapper
//! read this table instead of naming tools, so a new domain is one more set of entries.

use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;

use serde_json::Value;

use super::link::DeviceLink;

/// How much a tool can hurt, which is all governance needs to know about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RiskClass {
    /// Returns what an outsider could have written (titles, sender names): taints the thread.
    Read,
    /// Adds something new. Asks the owner only once the thread is tainted.
    Create,
    /// Changes something that exists. Always asks.
    Modify,
    /// Removes something. Always asks.
    Destructive,
    /// Sends something to another person. Always asks. Where nobody can be asked it does not run:
    /// every device tool needs a live user turn, so an unattended session is never offered it.
    Outbound,
}

pub type RunFuture<'a> = Pin<Box<dyn Future<Output = Value> + Send + 'a>>;

/// Validates `args`, builds the payload and calls `DeviceLink::round_trip`. Returns an outcome,
/// never an error.
pub type DeviceRun = for<'a> fn(&'a DeviceLink, &'a str, &'a Value) -> RunFuture<'a>;

pub struct DeviceToolSpec {
    /// The tool's name in the model's catalog.
    pub name: &'static str,
    /// The `name` of the `device_action` frame, which the phone dispatches on.
    pub wire_action: &'static str,
    pub risk: RiskClass,
    /// The built-in skill that teaches the model this tool; the tool fails closed without it.
    pub skill: &'static str,
    /// The outcome codes the phone may report. Anything else is refused at the result route.
    pub phone_outcomes: &'static [&'static str],
    pub description: &'static str,
    pub parameters: fn() -> Value,
    /// What the call would do, as "would ..." for the owner's approval line. Arguments are
    /// untrusted, so a describer cleans whatever it quotes.
    pub describe: fn(&Value) -> String,
    /// Turns the phone's body into the outcome the model sees, given the payload HQ sent.
    pub finalize: fn(body: &Value, payload: &Value) -> Value,
    /// The domain's "cannot do this now" outcome for a stated reason.
    pub unavailable: fn(&str) -> Value,
    /// The outcome when the answer never came: a create may have happened, a read has nothing
    /// to report.
    pub lost: fn(&str) -> Value,
    pub run: DeviceRun,
}

/// Every domain's tools, in the order they are offered.
const DOMAINS: &[&[DeviceToolSpec]] = &[crate::calendar::SPECS];

/// Specs added at run time. Only tests use it, to prove a new domain needs no other edit.
static EXTRA: Mutex<Vec<&'static DeviceToolSpec>> = Mutex::new(Vec::new());

/// Adds a spec to the lookups below, once per name. It is not offered to any session.
pub fn register(spec: &'static DeviceToolSpec) {
    if spec_for(spec.name).is_none() {
        EXTRA.lock().unwrap_or_else(|e| e.into_inner()).push(spec);
    }
}

/// The tools a phone chat is offered.
pub fn offered() -> impl Iterator<Item = &'static DeviceToolSpec> {
    DOMAINS.iter().flat_map(|specs| specs.iter())
}

fn find(matches: impl Fn(&DeviceToolSpec) -> bool) -> Option<&'static DeviceToolSpec> {
    offered().find(|s| matches(s)).or_else(|| {
        let extra = EXTRA.lock().unwrap_or_else(|e| e.into_inner());
        extra.iter().find(|s| matches(s)).copied()
    })
}

pub fn spec_for(tool_name: &str) -> Option<&'static DeviceToolSpec> {
    find(|s| s.name == tool_name)
}

pub fn spec_for_action(wire_action: &str) -> Option<&'static DeviceToolSpec> {
    find(|s| s.wire_action == wire_action)
}

fn risk_of(tool_name: &str) -> Option<(&'static DeviceToolSpec, RiskClass)> {
    spec_for(tool_name).map(|s| (s, s.risk))
}

/// Whether the tool's results can carry an outsider's words.
pub fn is_untrusted_source(tool_name: &str) -> bool {
    matches!(risk_of(tool_name), Some((_, RiskClass::Read)))
}

/// Why this call needs the owner whether or not the thread is tainted, or `None`.
pub fn always_ask_reason(tool_name: &str, args: &Value) -> Option<String> {
    match risk_of(tool_name)? {
        (spec, RiskClass::Modify | RiskClass::Destructive | RiskClass::Outbound) => {
            Some((spec.describe)(args))
        }
        _ => None,
    }
}

/// Why this call needs the owner only because the thread is tainted, or `None`. The caller
/// checks the taint.
pub fn tainted_reason(tool_name: &str, args: &Value) -> Option<String> {
    match risk_of(tool_name)? {
        (spec, RiskClass::Create) => Some((spec.describe)(args)),
        _ => None,
    }
}

/// Whether `body` is a result worth storing for the action called `wire_action`: a known
/// version and an outcome the spec lists. An unknown action accepts nothing.
pub fn result_is_acceptable_for(wire_action: &str, body: &Value) -> bool {
    spec_for_action(wire_action).is_some_and(|s| outcome_is_known(body, s.phone_outcomes))
}

pub fn outcome_is_known(body: &Value, allowed: &[&str]) -> bool {
    body.get("v").and_then(Value::as_u64) == Some(1)
        && body
            .get("outcome")
            .and_then(Value::as_str)
            .is_some_and(|o| allowed.contains(&o))
}

/// The outcome the model sees for the phone's `body`. Only called after
/// `result_is_acceptable_for`, so an unknown action is not a case that happens.
pub fn finalize_for(wire_action: &str, body: &Value, payload: &Value) -> Value {
    spec_for_action(wire_action).map_or(Value::Null, |s| (s.finalize)(body, payload))
}
