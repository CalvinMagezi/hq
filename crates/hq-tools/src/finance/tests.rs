use serde_json::{Value, json};

use super::import::{
    IMPORT_TOOL_NAME, LEDGER_LINK, MAX_MESSAGES, check_request, finalize_import, wire_payload,
};
use super::outcome;
use crate::device_actions::{RiskClass, always_ask_reason, is_untrusted_source, spec_for};

fn counts() -> Value {
    json!({"v": 1, "outcome": "imported", "seen": 40, "ignored": 3, "transactions": 30,
           "added": 12, "waiting_for_review": 12})
}

#[test]
fn the_tool_always_asks_and_its_result_is_not_an_untrusted_source() {
    let spec = spec_for(IMPORT_TOOL_NAME).expect("registered");
    assert_eq!(spec.risk, RiskClass::Modify);
    assert!(always_ask_reason(IMPORT_TOOL_NAME, &json!({})).is_some());
    // Counts carry no outsider's words, so the thread is not tainted by an import.
    assert!(!is_untrusted_source(IMPORT_TOOL_NAME));
}

#[test]
fn arguments_are_checked_before_anything_is_sent() {
    assert_eq!(check_request(&json!({})).unwrap().since_days, None);
    assert_eq!(
        check_request(&json!({"since_days": 30}))
            .unwrap()
            .since_days,
        Some(30)
    );
    for (args, why) in [
        (json!({"since_days": 0}), "out_of_range"),
        (json!({"since_days": 366}), "out_of_range"),
        (json!({"since_days": "30"}), "wrong_type"),
        (json!({"senders": ["x"]}), "unknown_field"),
        (json!([]), "arguments_not_an_object"),
    ] {
        let refusal = check_request(&args).unwrap_err();
        assert_eq!(refusal["outcome"], outcome::INVALID_REQUEST, "{args}");
        assert_eq!(refusal["reason"], why, "{args}");
    }
}

#[test]
fn the_phone_is_only_told_to_read_the_transaction_senders() {
    let payload = wire_payload("req-1", &check_request(&json!({"since_days": 7})).unwrap());
    assert_eq!(
        payload["senders"],
        json!(hq_finance::parse::TRANSACTION_SENDERS)
    );
    assert_eq!(payload["max_messages"], MAX_MESSAGES);
    assert_eq!(payload["since_days"], 7);
}

#[test]
fn an_import_result_is_counts_and_a_link_and_nothing_a_phone_adds() {
    let mut body = counts();
    body["transcript"] = json!("You have sent UGX 32,000 to a person");
    body["sample"] = json!(["a message"]);
    let out = finalize_import(&body, &Value::Null);
    assert_eq!(out["added"], 12);
    assert_eq!(out["open_link"], LEDGER_LINK);
    let mut keys: Vec<&str> = out
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "added",
            "ignored",
            "open_link",
            "outcome",
            "seen",
            "transactions",
            "waiting_for_review"
        ]
    );
}

#[test]
fn a_missing_or_absurd_count_is_not_an_import() {
    let mut missing = counts();
    missing.as_object_mut().unwrap().remove("added");
    assert_eq!(
        finalize_import(&missing, &Value::Null)["outcome"],
        outcome::FINANCE_UNAVAILABLE
    );
    let mut huge = counts();
    huge["seen"] = json!(u64::MAX);
    assert_eq!(
        finalize_import(&huge, &Value::Null)["outcome"],
        outcome::FINANCE_UNAVAILABLE
    );
}

#[test]
fn a_refusal_from_the_phone_keeps_its_reason_only() {
    let body = json!({"v": 1, "outcome": "permission_missing", "reason": "denied", "extra": "x"});
    let out = finalize_import(&body, &Value::Null);
    assert_eq!(out["outcome"], outcome::PERMISSION_MISSING);
    assert!(out.get("extra").is_none());
    assert_eq!(
        finalize_import(&json!({"outcome": "made_up"}), &Value::Null)["outcome"],
        outcome::FINANCE_UNAVAILABLE
    );
}
