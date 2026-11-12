use std::sync::Arc;

use serde_json::{Value, json};
use tokio::sync::mpsc;

use super::fake::*;
use super::store::{DeviceActionStore, Timeouts};
use super::*;
use crate::skills::builtin::{CALENDAR_READING_SKILL, set_active_for_this_thread};

const FAST: Timeouts = Timeouts {
    ack_secs: 2,
    result_secs: 2,
};

fn link(store: Arc<DeviceActionStore>) -> (DeviceLink, mpsc::UnboundedReceiver<Value>) {
    let (tx, rx) = mpsc::unbounded_channel();
    let emit = Arc::new(move |frame: Value| {
        let _ = tx.send(frame);
    });
    let phone = PhoneClock::parse("Europe/London", "2026-01-10T12:00:00Z").unwrap();
    (
        DeviceLink::new("thread-1", store, emit, phone).with_timeouts(FAST),
        rx,
    )
}

fn phone_answers(store: &Arc<DeviceActionStore>, frame: &Value, body: Value) {
    let id = frame["action_id"].as_str().unwrap();
    store.ack(id).unwrap();
    let name = frame["name"].as_str().unwrap().to_string();
    store
        .settle(id, &body, |b, p| finalize_for(&name, b, p))
        .unwrap();
}

#[test]
fn the_domains_fill_the_table_one_entry_per_tool_in_offer_order() {
    let table: Vec<(&str, &str, RiskClass)> =
        offered().map(|s| (s.name, s.wire_action, s.risk)).collect();
    assert_eq!(
        table,
        [
            (
                "calendar_create_event",
                "calendar.create",
                RiskClass::Create
            ),
            (
                "calendar_update_event",
                "calendar.update",
                RiskClass::Modify
            ),
            (
                "calendar_delete_event",
                "calendar.delete",
                RiskClass::Destructive
            ),
            ("calendar_list_events", "calendar.list", RiskClass::Read),
            ("contacts_search", "contacts.search", RiskClass::Read),
        ]
    );
    for spec in offered() {
        assert!(!spec.phone_outcomes.is_empty(), "{}", spec.name);
        assert!(
            crate::skills::builtin::BUILTIN_SKILLS
                .iter()
                .any(|shipped| shipped.name == spec.skill),
            "{} names a skill HQ ships",
            spec.name
        );
        assert_eq!(
            spec_for(spec.name).map(|s| s.wire_action),
            Some(spec.wire_action)
        );
        assert_eq!(
            spec_for_action(spec.wire_action).map(|s| s.name),
            Some(spec.name)
        );
    }
}

#[test]
fn the_risk_class_alone_decides_taint_and_approval() {
    register_fake();
    let args = json!({"target": "Alex"});
    assert!(is_untrusted_source(FAKE_READ));
    for tool in [FAKE_ADD, FAKE_DELETE, FAKE_SEND, "web_fetch"] {
        assert!(!is_untrusted_source(tool), "{tool}");
    }
    assert_eq!(always_ask_reason(FAKE_READ, &args), None);
    assert_eq!(always_ask_reason(FAKE_ADD, &args), None);
    for tool in [FAKE_DELETE, FAKE_SEND] {
        assert_eq!(
            always_ask_reason(tool, &args).as_deref(),
            Some("would act on Alex"),
            "{tool}"
        );
        assert_eq!(tainted_reason(tool, &args), None, "{tool}");
    }
    assert!(tainted_reason(FAKE_ADD, &args).is_some());
    assert_eq!(tainted_reason(FAKE_READ, &args), None);
}

#[test]
fn a_result_is_kept_only_for_an_outcome_the_spec_lists_and_only_its_fields() {
    register_fake();
    let body = |outcome: &str| json!({"v": 1, "outcome": outcome, "reason": "r", "extra": "x"});
    assert!(result_is_acceptable_for(FAKE_SEND_ACTION, &body(SENT)));
    assert!(!result_is_acceptable_for(
        FAKE_SEND_ACTION,
        &body("invented")
    ));
    assert!(!result_is_acceptable_for(
        FAKE_SEND_ACTION,
        &json!({"v": 2, "outcome": SENT})
    ));
    assert!(!result_is_acceptable_for("fake.unknown", &body(SENT)));
    let kept = finalize_for(FAKE_SEND_ACTION, &body(SENT), &Value::Null);
    assert_eq!(kept, json!({"outcome": SENT, "reason": "r"}));
}

#[tokio::test]
async fn a_new_domain_round_trips_through_the_shared_link() {
    register_fake();
    let _on = set_active_for_this_thread(Some(&[CALENDAR_READING_SKILL]));
    let store = Arc::new(DeviceActionStore::default());
    let (link, mut frames) = link(store.clone());
    let phone = async {
        let frame = frames.recv().await.unwrap();
        assert_eq!(frame["name"], FAKE_READ_ACTION);
        phone_answers(
            &store,
            &frame,
            json!({"v": 1, "outcome": "listed", "extra": "dropped"}),
        );
    };
    let args = json!({});
    let (out, ()) = tokio::join!(
        (spec_for(FAKE_READ).unwrap().run)(&link, "call-1", &args),
        phone
    );
    assert_eq!(out, json!({"outcome": "listed"}));
}

#[tokio::test]
async fn a_domain_whose_skill_is_off_fails_closed_and_sends_nothing() {
    register_fake();
    let _off = set_active_for_this_thread(Some(&[]));
    let (link, mut frames) = link(Arc::default());
    let args = json!({});
    let out = (spec_for(FAKE_SEND).unwrap().run)(&link, "call-1", &args).await;
    assert_eq!(out["outcome"], "fake_unavailable");
    assert_eq!(out["reason"], "skill_not_active");
    assert!(frames.try_recv().is_err());
}

#[tokio::test]
async fn a_silent_phone_gets_the_domains_own_outcomes() {
    register_fake();
    let _on = set_active_for_this_thread(Some(&[CALENDAR_READING_SKILL]));
    let (link, mut frames) = link(Arc::default());
    let link = link.with_timeouts(Timeouts {
        ack_secs: 1,
        result_secs: 1,
    });
    let args = json!({});
    let out = (spec_for(FAKE_ADD).unwrap().run)(&link, "call-1", &args).await;
    assert_eq!(out, unreachable());
    assert!(frames.try_recv().is_ok(), "the frame was sent once");
}
