use std::sync::Arc;

use serde_json::{Value, json};
use tokio::sync::mpsc;

use super::search::{check_request, finalize_search, run_search};
use super::{SEARCH_ACTION_NAME, SEARCH_TOOL_NAME, SPECS};
use crate::device_actions::store::{DeviceActionStore, Timeouts};
use crate::device_actions::{
    DeviceLink, PhoneClock, RiskClass, finalize_for, is_untrusted_source, result_is_acceptable_for,
    spec_for,
};
use crate::skills::builtin::{CONTACTS_LOOKUP_SKILL, set_active_for_this_thread};

fn reason(r: &Value) -> (&str, &str) {
    (
        r["outcome"].as_str().unwrap_or(""),
        r["reason"].as_str().unwrap_or(""),
    )
}

fn row(id: &str, name: &str) -> Value {
    json!({"id": id, "display_name": name,
        "phones": [{"label": "Mobile", "number": "+15550100"}],
        "emails": [{"label": "Home", "address": "alex@example.com"}]})
}

fn listed(rows: Vec<Value>, truncated: bool) -> Value {
    json!({"v": 1, "outcome": "listed", "contacts": rows, "truncated": truncated})
}

fn payload(query: &str, limit: u64) -> Value {
    json!({"request_id": "r", "query": query, "limit": limit})
}

#[test]
fn the_validation_table() {
    let ok = check_request(&json!({"query": "  Alex  "})).unwrap();
    assert_eq!((ok.query.as_str(), ok.limit), ("Alex", 5));
    assert_eq!(
        check_request(&json!({"query": "Al", "limit": 10, "_hq_caller": "x"}))
            .unwrap()
            .limit,
        10
    );
    let table: Vec<(Value, (&str, &str))> = vec![
        (json!({}), ("invalid_query", "missing")),
        (json!({"query": null}), ("invalid_query", "missing")),
        (json!({"query": 5}), ("invalid_query", "wrong_type")),
        (json!({"query": " a "}), ("invalid_query", "too_short")),
        (json!({"query": "   "}), ("invalid_query", "too_short")),
        (
            json!({"query": "x".repeat(65)}),
            ("invalid_query", "too_long"),
        ),
        (
            json!({"query": "al\nex"}),
            ("invalid_query", "bad_characters"),
        ),
        (
            json!({"query": "Alex", "limit": 0}),
            ("invalid_query", "out_of_range"),
        ),
        (
            json!({"query": "Alex", "limit": 11}),
            ("invalid_query", "out_of_range"),
        ),
        (
            json!({"query": "Alex", "limit": -1}),
            ("invalid_query", "wrong_type"),
        ),
        (
            json!({"query": "Alex", "limit": "3"}),
            ("invalid_query", "wrong_type"),
        ),
        (
            json!({"query": "Alex", "limit": 2.5}),
            ("invalid_query", "wrong_type"),
        ),
        (
            json!({"query": "Alex", "notes": true}),
            ("invalid_query", "unknown_field"),
        ),
        (json!("Alex"), ("invalid_query", "arguments_not_an_object")),
    ];
    for (args, want) in table {
        assert_eq!(reason(&check_request(&args).unwrap_err()), want, "{args}");
    }
    assert!(check_request(&json!({"query": "x".repeat(64)})).is_ok());
    assert!(
        check_request(&json!({"query": "日本"})).is_ok(),
        "length counts characters"
    );
}

#[test]
fn a_digits_only_query_is_allowed() {
    assert!(check_request(&json!({"query": "5550100"})).is_ok());
}

#[test]
fn the_result_keeps_only_named_fields_caps_lengths_and_builds_the_link_from_digits() {
    let mut full = row("42", "Alex  Example\u{7}");
    full["notes"] = json!("secret");
    full["birthday"] = json!("2000-01-01");
    full["phones"] = json!([
        {"label": "M".repeat(100), "number": "+1555010\n0", "extra": "x"},
        {"label": "W", "number": "+15550101"},
        {"label": "H", "number": "+15550102"},
        {"label": "O", "number": "+15550103"},
        {"label": "Blank", "number": "  "},
    ]);
    full["emails"] = json!([
        {"label": "a", "address": "b".repeat(300)},
        {"label": "c", "address": "c@example.com"},
        {"label": "d", "address": "d@example.com"},
        {"label": "e", "address": "e@example.com"},
    ]);
    let out = finalize_search(&listed(vec![full], false), &payload("alex", 5));
    assert_eq!(out["v"], 1);
    assert_eq!(out["outcome"], "listed");
    assert_eq!(out["truncated"], false);
    let c = &out["contacts"][0];
    assert_eq!(c["display_name"], "Alex Example");
    assert_eq!(c["open_link"], "hq://contacts/contact/42");
    assert_eq!(
        c.as_object().unwrap().keys().collect::<Vec<_>>(),
        ["display_name", "emails", "id", "open_link", "phones"]
    );
    let phones = c["phones"].as_array().unwrap();
    assert_eq!(phones.len(), 3);
    assert_eq!(phones[0]["number"], "+1555010 0");
    assert_eq!(phones[0]["label"].as_str().unwrap().chars().count(), 40);
    assert!(phones[0].get("extra").is_none());
    let emails = c["emails"].as_array().unwrap();
    assert_eq!(emails.len(), 3);
    assert_eq!(emails[0]["address"].as_str().unwrap().chars().count(), 120);
}

#[test]
fn the_link_is_built_from_ascii_digits_only() {
    for bad in ["4/../2", "4?x=1", "", "-1", "１２", "12345678901234567890"] {
        let out = finalize_search(&listed(vec![row(bad, "Alex")], false), &payload("alex", 5));
        assert_eq!(out["contacts"], json!([]), "{bad:?}");
    }
    let numeric = json!({"id": 7, "display_name": "Alex"});
    let out = finalize_search(&listed(vec![numeric], false), &payload("alex", 5));
    assert_eq!(out["contacts"][0]["open_link"], "hq://contacts/contact/7");
}

#[test]
fn rows_are_sorted_deduped_filtered_by_the_query_and_capped_at_the_limit() {
    let rows = vec![
        row("3", "alex Zed"),
        row("1", "Alex Able"),
        row("1", "Alex Able"),
        row("9", "Unrelated"),
        json!({"id": "5", "display_name": 5}),
        json!({"display_name": "Alex"}),
        json!("junk"),
        row("2", "ALEX Mid"),
    ];
    let out = finalize_search(&listed(rows, false), &payload("Alex", 2));
    let names: Vec<&str> = out["contacts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["display_name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Alex Able", "ALEX Mid"]);
    assert_eq!(out["truncated"], true, "a third match was cut");
    let phone_cut = finalize_search(&listed(vec![row("1", "Alex")], true), &payload("alex", 5));
    assert_eq!(phone_cut["truncated"], true);
}

#[test]
fn a_hostile_reply_is_read_only_up_to_a_bounded_number_of_rows() {
    let rows: Vec<Value> = (0..1000).map(|i| row(&i.to_string(), "Alex")).collect();
    let out = finalize_search(&listed(rows, false), &payload("alex", 1));
    assert_eq!(out["contacts"].as_array().unwrap().len(), 1);
    let no_name = finalize_search(
        &listed(vec![json!({"id": "1", "display_name": " \n "})], false),
        &payload("(no", 5),
    );
    assert_eq!(no_name["contacts"][0]["display_name"], "(no name)");
}

#[test]
fn other_outcomes_pass_through_only_their_fields_and_the_rest_is_unavailable() {
    let denied = finalize_search(
        &json!({"v": 1, "outcome": "permission_denied", "missing": "contacts_read",
            "next_step": "open settings", "extra": "x"}),
        &json!({}),
    );
    assert_eq!(denied["outcome"], "permission_denied");
    assert_eq!(denied["missing"], "contacts_read");
    assert!(denied.get("extra").is_none());
    let unavailable = finalize_search(
        &json!({"v": 1, "outcome": "contacts_unavailable", "reason": "provider_error"}),
        &json!({}),
    );
    assert_eq!(
        reason(&unavailable),
        ("contacts_unavailable", "provider_error")
    );
    assert_eq!(
        reason(&finalize_search(&json!({"outcome": "created"}), &json!({}))).1,
        "unexpected_outcome"
    );
    assert_eq!(
        reason(&finalize_search(&json!({"outcome": "listed"}), &json!({}))).1,
        "incomplete_result"
    );
}

#[test]
fn the_spec_is_a_read_registered_in_the_table_and_the_phone_may_report_four_outcomes() {
    assert_eq!(SPECS.len(), 1);
    let spec = spec_for(SEARCH_TOOL_NAME).unwrap();
    assert_eq!(spec.wire_action, SEARCH_ACTION_NAME);
    assert_eq!(spec.risk, RiskClass::Read);
    assert_eq!(spec.skill, CONTACTS_LOOKUP_SKILL);
    assert!(is_untrusted_source(SEARCH_TOOL_NAME), "a read taints");
    for ok in [
        "listed",
        "permission_denied",
        "invalid_query",
        "contacts_unavailable",
    ] {
        assert!(result_is_acceptable_for(
            "contacts.search",
            &json!({"v": 1, "outcome": ok})
        ));
    }
    for bad in ["created", "device_unreachable", "whatever"] {
        assert!(!result_is_acceptable_for(
            "contacts.search",
            &json!({"v": 1, "outcome": bad})
        ));
    }
    assert!(!result_is_acceptable_for(
        "contacts.search",
        &json!({"v": 2, "outcome": "listed"})
    ));
    assert!(!result_is_acceptable_for(
        "calendar.list",
        &json!({"v": 1, "outcome": "invalid_query"})
    ));
}

type Frames = mpsc::UnboundedReceiver<Value>;

fn tool(store: Arc<DeviceActionStore>, timeouts: Timeouts) -> (DeviceLink, Frames) {
    let (tx, rx) = mpsc::unbounded_channel();
    let emit = Arc::new(move |frame: Value| {
        let _ = tx.send(frame);
    });
    let phone = PhoneClock::parse("Europe/London", "2026-01-10T12:00:00Z").unwrap();
    (
        DeviceLink::new("thread-1", store, emit, phone).with_timeouts(timeouts),
        rx,
    )
}

const FAST: Timeouts = Timeouts {
    ack_secs: 1,
    result_secs: 1,
};

async fn answer(store: &Arc<DeviceActionStore>, frames: &mut Frames, body: Value) -> Value {
    let frame = frames.recv().await.unwrap();
    assert_eq!(frame["name"], "contacts.search");
    let id = frame["action_id"].as_str().unwrap().to_string();
    store.ack(&id).unwrap();
    store
        .settle(&id, &body, |b, p| finalize_for(SEARCH_ACTION_NAME, b, p))
        .unwrap();
    frame
}

#[tokio::test]
async fn it_refuses_without_the_skill_and_a_bad_query_never_reaches_the_phone() {
    let (t, mut frames) = tool(Arc::default(), FAST);
    {
        let _off = set_active_for_this_thread(Some(&[]));
        let out = run_search(&t, "c1", &json!({"query": "Alex"})).await;
        assert_eq!(reason(&out), ("contacts_unavailable", "skill_not_active"));
    }
    let _on = set_active_for_this_thread(Some(&[CONTACTS_LOOKUP_SKILL]));
    let out = run_search(&t, "c1", &json!({"query": "a"})).await;
    assert_eq!(reason(&out), ("invalid_query", "too_short"));
    assert!(frames.try_recv().is_err(), "nothing was sent");
}

#[tokio::test]
async fn a_fake_phone_round_trip_lists_and_a_denial_passes_through() {
    let _on = set_active_for_this_thread(Some(&[CONTACTS_LOOKUP_SKILL]));
    let store: Arc<DeviceActionStore> = Arc::default();
    let slow = Timeouts {
        ack_secs: 2,
        result_secs: 2,
    };
    let (t, mut frames) = tool(store.clone(), slow);
    let request = json!({"query": " alex ", "limit": 3});
    let phone_side = async {
        let frame = answer(
            &store,
            &mut frames,
            listed(vec![row("5", "Alex Example")], false),
        )
        .await;
        assert_eq!(frame["payload"]["query"], "alex");
        assert_eq!(frame["payload"]["limit"], 3);
        assert!(frame["payload"]["request_id"].as_str().unwrap().len() >= 32);
    };
    let (out, ()) = tokio::join!(run_search(&t, "c1", &request), phone_side);
    assert_eq!(out["outcome"], "listed");
    assert_eq!(out["contacts"][0]["open_link"], "hq://contacts/contact/5");

    let denied = json!({"v": 1, "outcome": "permission_denied", "missing": "contacts_read"});
    let phone_side = async {
        answer(&store, &mut frames, denied).await;
    };
    let (out, ()) = tokio::join!(run_search(&t, "c2", &request), phone_side);
    assert_eq!(out["outcome"], "permission_denied");
    assert_eq!(out["missing"], "contacts_read");
}

#[tokio::test]
async fn timeouts_give_device_unreachable_then_contacts_unavailable() {
    let _on = set_active_for_this_thread(Some(&[CONTACTS_LOOKUP_SKILL]));
    let request = json!({"query": "Alex"});
    let (t, mut frames) = tool(Arc::default(), FAST);
    let out = run_search(&t, "c1", &request).await;
    assert_eq!(out["outcome"], "device_unreachable");
    assert!(frames.try_recv().is_ok(), "the frame was sent once");

    let store: Arc<DeviceActionStore> = Arc::default();
    let (t, mut frames) = tool(store.clone(), FAST);
    let phone_side = async {
        let frame = frames.recv().await.unwrap();
        store.ack(frame["action_id"].as_str().unwrap()).unwrap();
    };
    let (out, ()) = tokio::join!(run_search(&t, "c2", &request), phone_side);
    assert_eq!(reason(&out), ("contacts_unavailable", "no_result_in_time"));
}
