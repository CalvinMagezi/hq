//! `finance_import_messages`: check the request here, ask the phone to read the owner's mobile-money
//! messages into the private ledger, and show the model only the counts the server computed.

use serde_json::{Map, Value, json};

use super::IMPORT_ACTION_NAME;
use super::outcome::{self, IMPORT_PHONE_OUTCOMES, invalid};
use crate::calendar::create::pass_through;
use crate::device_actions::{DeviceLink, DeviceToolSpec, RiskClass, RunFuture};
use crate::skills::builtin::FINANCE_IMPORT_SKILL;

pub const IMPORT_TOOL_NAME: &str = "finance_import_messages";
pub const SINCE_DAYS_MIN: u64 = 1;
pub const SINCE_DAYS_MAX: u64 = 365;
/// Most messages the phone may hand over in one import.
pub const MAX_MESSAGES: u64 = 2_000;
/// The link the app opens to show the ledger.
pub const LEDGER_LINK: &str = "hq://finance/ledger";
const KNOWN_FIELDS: &[&str] = &["since_days"];
const RESERVED_PREFIX: &str = "_hq_";
const COUNT_KEYS: &[&str] = &[
    "seen",
    "ignored",
    "transactions",
    "added",
    "waiting_for_review",
];
/// A count above this is not a count of one phone's messages.
const COUNT_CEILING: u64 = 1_000_000;

const IMPORT_TOOL_DESCRIPTION: &str = "Read the owner's MTN and Airtel Money transaction messages \
    on their phone and add them to their private finance ledger as drafts to review. Load the \
    finance-import skill first. The owner is asked to approve every import. The messages are \
    never shown to you: the result is only counts (messages seen, ignored, transactions found, \
    rows added, rows waiting for review) and a link to the ledger. Optional since_days limits \
    how far back the phone reads, 1 to 365; without it the phone reads since its last import.";

/// A checked request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub since_days: Option<u64>,
}

/// Validates `args`. `Err` is the outcome to return.
pub fn check_request(args: &Value) -> Result<Request, Value> {
    let Some(map) = args.as_object() else {
        return Err(invalid("arguments_not_an_object", None));
    };
    for key in map.keys() {
        if !KNOWN_FIELDS.contains(&key.as_str()) && !key.starts_with(RESERVED_PREFIX) {
            return Err(invalid("unknown_field", Some(key)));
        }
    }
    let since_days = match map.get("since_days") {
        None | Some(Value::Null) => None,
        Some(v) => {
            let days = v
                .as_u64()
                .ok_or_else(|| invalid("wrong_type", Some("since_days")))?;
            if !(SINCE_DAYS_MIN..=SINCE_DAYS_MAX).contains(&days) {
                return Err(invalid("out_of_range", Some("since_days")));
            }
            Some(days)
        }
    };
    Ok(Request { since_days })
}

/// The frame payload. The senders are fixed in HQ, so a phone never reads a sender the owner was
/// not told about, and the route checks them again.
pub fn wire_payload(request_id: &str, request: &Request) -> Value {
    let mut payload = Map::new();
    payload.insert("request_id".into(), json!(request_id));
    payload.insert(
        "senders".into(),
        json!(hq_finance::parse::TRANSACTION_SENDERS),
    );
    payload.insert("max_messages".into(), json!(MAX_MESSAGES));
    if let Some(days) = request.since_days {
        payload.insert("since_days".into(), json!(days));
    }
    Value::Object(payload)
}

/// Turns the settled body into what the model sees: the outcome and counts, nothing else.
pub fn finalize_import(body: &Value, _payload: &Value) -> Value {
    let code = body.get("outcome").and_then(Value::as_str).unwrap_or("");
    if code != outcome::IMPORTED {
        if IMPORT_PHONE_OUTCOMES.contains(&code) {
            return pass_through(code, body);
        }
        return outcome::unavailable("unexpected_outcome");
    }
    let mut out = json!({"outcome": outcome::IMPORTED, "open_link": LEDGER_LINK});
    for key in COUNT_KEYS {
        let Some(n) = body
            .get(*key)
            .and_then(Value::as_u64)
            .filter(|n| *n <= COUNT_CEILING)
        else {
            return outcome::unavailable("incomplete_result");
        };
        out[*key] = json!(n);
    }
    out
}

pub(super) const SPEC: DeviceToolSpec = DeviceToolSpec {
    name: IMPORT_TOOL_NAME,
    wire_action: IMPORT_ACTION_NAME,
    risk: RiskClass::Modify,
    skill: FINANCE_IMPORT_SKILL,
    phone_outcomes: IMPORT_PHONE_OUTCOMES,
    description: IMPORT_TOOL_DESCRIPTION,
    parameters,
    describe,
    finalize: finalize_import,
    unavailable: outcome::unavailable,
    lost: outcome::unavailable,
    run,
};

fn run<'a>(link: &'a DeviceLink, tool_call_id: &'a str, args: &'a Value) -> RunFuture<'a> {
    Box::pin(run_import(link, tool_call_id, args))
}

fn describe(args: &Value) -> String {
    match check_request(args).ok().and_then(|r| r.since_days) {
        Some(days) => format!(
            "would read your MTN and Airtel Money transaction messages from the last {days} days on your phone and add them to your private ledger as drafts"
        ),
        None => "would read your new MTN and Airtel Money transaction messages on your phone and add them to your private ledger as drafts".into(),
    }
}

fn parameters() -> Value {
    json!({
        "type": "object",
        "properties": {
            "since_days": {
                "type": "integer",
                "minimum": SINCE_DAYS_MIN,
                "maximum": SINCE_DAYS_MAX,
                "description": "How many days back to read. Leave out to read since the last import."
            }
        },
        "additionalProperties": false
    })
}

/// Runs one import. Always returns one of the contract's outcomes, never an error.
pub async fn run_import(link: &DeviceLink, tool_call_id: &str, args: &Value) -> Value {
    if let Some(refusal) = link.skill_gate(&SPEC) {
        return refusal;
    }
    let request = match check_request(args) {
        Ok(r) => r,
        Err(refusal) => return refusal,
    };
    let request_id = uuid::Uuid::new_v4().to_string();
    link.round_trip(&SPEC, tool_call_id, wire_payload(&request_id, &request))
        .await
}
