use std::sync::Arc;

use chrono::{DateTime, TimeZone, Utc};
use chrono_tz::Tz;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use super::delete::{check_delete, finalize_delete, run_delete};
use super::target::parse_link;
use super::update::{check_update, finalize_update, run_update};
use super::{DELETE_ACTION_NAME, UPDATE_ACTION_NAME};
use crate::device_actions::store::{DeviceActionStore, Opened, Timeouts};
use crate::device_actions::{DeviceLink, PhoneClock, always_ask_reason, result_is_acceptable_for};
use crate::skills::builtin::set_active_for_this_thread;
use crate::skills::builtin::{CALENDAR_CREATION_SKILL, CALENDAR_EDITING_SKILL};

const LONDON: Tz = chrono_tz::Europe::London;
const DAY_MS: i64 = 86_400_000;

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, 10, 12, 0, 0).unwrap()
}

fn ms(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
    Utc.with_ymd_and_hms(y, mo, d, h, mi, 0)
        .unwrap()
        .timestamp_millis()
}

fn link(id: &str, begin: i64, end: i64) -> String {
    format!("hq://calendar/event/{id}?begin={begin}&end={end}")
}

/// A one-hour event at 15:00 London on 10 February 2026.
fn timed_args(extra: Value) -> Value {
    let begin = ms(2026, 2, 10, 15, 0);
    let mut v = json!({
        "open_link": link("42", begin, begin + 3_600_000),
        "title": "Dentist",
        "start_local": "2026-02-10T15:00",
    });
    merge(&mut v, extra);
    v
}

/// A two-day all-day event, 10 and 11 February 2026.
fn all_day_args(extra: Value) -> Value {
    let begin = ms(2026, 2, 10, 0, 0);
    let mut v = json!({
        "open_link": link("7", begin, begin + 2 * DAY_MS),
        "title": "Offsite",
        "start_local": "2026-02-10",
    });
    merge(&mut v, extra);
    v
}

fn merge(base: &mut Value, extra: Value) {
    for (k, v) in extra.as_object().unwrap() {
        base[k] = v.clone();
    }
}

fn refusal(r: &Value) -> (&str, &str, &str) {
    (
        r["outcome"].as_str().unwrap_or(""),
        r["reason"].as_str().unwrap_or(""),
        r["field"].as_str().unwrap_or(""),
    )
}

fn update(args: Value) -> Result<(super::target::Target, super::update::Changes), Value> {
    check_update(&args, LONDON, now())
}

#[test]
fn only_a_strict_event_link_parses() {
    let good = "hq://calendar/event/42?begin=100&end=200";
    assert_eq!(parse_link(good).unwrap(), ("42".to_string(), 100, 200));
    let long = "1".repeat(19);
    let bad = [
        "".to_string(),
        "https://calendar/event/42?begin=1&end=2".into(),
        "hq://calendar/event/?begin=1&end=2".into(),
        "hq://calendar/event/4x2?begin=1&end=2".into(),
        "hq://calendar/event/-4?begin=1&end=2".into(),
        format!("hq://calendar/event/{long}?begin=1&end=2"),
        format!("hq://calendar/event/4?begin={long}&end=2"),
        "hq://calendar/event/42?end=2&begin=1".into(),
        "hq://calendar/event/42?begin=1&end=2&x=3".into(),
        "hq://calendar/event/42?begin=1&end=2#f".into(),
        "hq://calendar/event/42?begin=1&end=2 ".into(),
        "hq://calendar/event/42/../9?begin=1&end=2".into(),
        "hq://calendar/event/42?begin=2&end=1".into(),
        "hq://calendar/event/42?begin=&end=1".into(),
        "hq://calendar/event/42?begin=+1&end=2".into(),
        "hq://calendar/event/42?begin=1&end=2\n".into(),
        "hq://calendar/event/42?begin=1&end=2&end=3".into(),
    ];
    for raw in bad {
        let err = parse_link(&raw).unwrap_err();
        assert_eq!(
            refusal(&err),
            ("invalid_event", "bad_link", "open_link"),
            "{raw:?}"
        );
    }
}

#[test]
fn the_start_must_match_the_link_in_the_phone_zone() {
    let ok = update(timed_args(json!({"new_title": "Check-up"})))
        .unwrap()
        .0;
    assert!(!ok.all_day);
    assert_eq!(ok.expected_title, "Dentist");
    let wrong = update(timed_args(
        json!({"start_local": "2026-02-10T16:00", "new_title": "x"}),
    ));
    assert_eq!(
        refusal(&wrong.unwrap_err()),
        ("invalid_event", "start_mismatch", "start_local")
    );
    // A date-only start on a timed event is a start mismatch, not a silent all-day edit.
    let date_only = update(timed_args(
        json!({"start_local": "2026-02-10", "new_title": "x"}),
    ));
    assert_eq!(refusal(&date_only.unwrap_err()).1, "start_mismatch");
    let all_day = update(all_day_args(json!({"new_title": "x"}))).unwrap().0;
    assert!(all_day.all_day);
    let off_midnight = update(all_day_args(
        json!({"start_local": "2026-02-11", "new_title": "x"}),
    ));
    assert_eq!(refusal(&off_midnight.unwrap_err()).1, "start_mismatch");
    assert_eq!(
        refusal(&update(timed_args(json!({"start_local": "soon", "new_title": "x"}))).unwrap_err())
            .1,
        "bad_format"
    );
    // The same instant reads differently in another zone, so a London phone cannot be checked as Nairobi.
    let nairobi = check_update(
        &timed_args(json!({"new_title": "x"})),
        chrono_tz::Africa::Nairobi,
        now(),
    );
    assert_eq!(refusal(&nairobi.unwrap_err()).1, "start_mismatch");
}

#[test]
fn an_untitled_event_expects_an_empty_title_and_titles_are_cleaned() {
    let t = update(timed_args(json!({"title": "(no title)", "new_title": "x"})))
        .unwrap()
        .0;
    assert_eq!(t.expected_title, "");
    let t = update(timed_args(
        json!({"title": "Den\ntist \t Visit", "new_title": "x"}),
    ))
    .unwrap()
    .0;
    assert_eq!(t.expected_title, "Den tist Visit");
}

#[test]
fn at_least_one_change_is_required_and_unknown_fields_are_refused() {
    assert_eq!(
        refusal(&update(timed_args(json!({}))).unwrap_err()).1,
        "no_changes"
    );
    assert_eq!(
        refusal(&update(timed_args(json!({"new_title": "  "}))).unwrap_err()).1,
        "no_changes",
        "an empty title is no change"
    );
    for field in [
        "location",
        "attendees",
        "recurrence",
        "calendar",
        "all_day",
        "end_local",
    ] {
        let err = update(timed_args(json!({"new_title": "x", field: "y"}))).unwrap_err();
        assert_eq!(refusal(&err), ("invalid_event", "unknown_field", field));
    }
    let long = update(timed_args(json!({"new_title": "x".repeat(201)})));
    assert_eq!(refusal(&long.unwrap_err()).1, "too_long");
    let runtime = update(timed_args(
        json!({"new_title": "x", "_hq_caller_session": "s"}),
    ));
    assert!(runtime.is_ok(), "runtime-owned keys are not the model's");
    assert_eq!(
        refusal(&check_update(&json!("x"), LONDON, now()).unwrap_err()).1,
        "arguments_not_an_object"
    );
}

#[test]
fn a_title_only_change_sends_no_times() {
    let (_, c) = update(timed_args(json!({"new_title": "Check-up"}))).unwrap();
    assert_eq!(c.title.as_deref(), Some("Check-up"));
    assert_eq!((c.start_ms, c.end_ms), (None, None));
}

#[test]
fn a_new_start_keeps_the_length_and_an_end_alone_keeps_the_start() {
    let (_, c) = update(timed_args(json!({"new_start_local": "2026-02-11T09:30"}))).unwrap();
    assert_eq!(c.start_ms, Some(ms(2026, 2, 11, 9, 30)));
    assert_eq!(c.end_ms, Some(ms(2026, 2, 11, 10, 30)));
    let (_, c) = update(timed_args(json!({"new_end_local": "2026-02-10T17:00"}))).unwrap();
    assert_eq!(c.start_ms, None);
    assert_eq!(c.end_ms, Some(ms(2026, 2, 10, 17, 0)));
    let (_, c) = update(timed_args(
        json!({"new_start_local": "2026-02-11T09:00", "new_end_local": "2026-02-11T09:15"}),
    ))
    .unwrap();
    assert_eq!(
        (c.start_ms, c.end_ms),
        (Some(ms(2026, 2, 11, 9, 0)), Some(ms(2026, 2, 11, 9, 15)))
    );
}

#[test]
fn local_times_become_instants_across_the_clock_change() {
    // Clocks go forward at 01:00 on 29 March 2026: noon is GMT the day before, BST after.
    let at = |local: &str| {
        update(timed_args(json!({"new_start_local": local}))).map(|(_, c)| c.start_ms.unwrap())
    };
    assert_eq!(at("2026-03-28T12:00").unwrap(), ms(2026, 3, 28, 12, 0));
    assert_eq!(at("2026-03-29T12:00").unwrap(), ms(2026, 3, 29, 11, 0));
    assert_eq!(
        refusal(&at("2026-03-29T01:30").unwrap_err()).1,
        "nonexistent_local_time"
    );
    // Clocks go back at 02:00 on 25 October 2026: 01:30 happens twice and the owner decides.
    let twice = at("2026-10-25T01:30").unwrap_err();
    assert_eq!(twice["outcome"], "clarification_required");
    assert_eq!(twice["reason"], "ambiguous_local_time");
}

#[test]
fn a_new_start_follows_the_create_rules_and_an_end_must_follow_the_start() {
    let past = update(timed_args(json!({"new_start_local": "2026-01-10T11:00"}))).unwrap_err();
    assert_eq!(past["reason"], "start_in_past");
    let far = update(timed_args(json!({"new_start_local": "2035-01-10T11:00"}))).unwrap_err();
    assert_eq!(far["reason"], "start_too_far_ahead");
    let end_before = update(timed_args(json!({"new_end_local": "2026-02-10T14:00"}))).unwrap_err();
    assert_eq!(
        refusal(&end_before),
        ("invalid_event", "end_not_after_start", "new_end_local")
    );
    let both = update(timed_args(
        json!({"new_start_local": "2026-02-12T10:00", "new_end_local": "2026-02-12T10:00"}),
    ));
    assert_eq!(refusal(&both.unwrap_err()).1, "end_not_after_start");
    // Renaming an event that already started is fine: only a new start is judged against the clock.
    let begin = ms(2026, 1, 1, 9, 0);
    let args = json!({"open_link": link("1", begin, begin + 60_000), "title": "Old",
        "start_local": "2026-01-01T09:00", "new_title": "Older"});
    assert!(check_update(&args, LONDON, now()).is_ok());
}

#[test]
fn a_kind_change_is_refused_in_both_directions() {
    let to_all_day = update(timed_args(json!({"new_start_local": "2026-02-11"}))).unwrap_err();
    assert_eq!(
        refusal(&to_all_day),
        ("invalid_event", "kind_change", "new_start_local")
    );
    let end = update(timed_args(json!({"new_end_local": "2026-02-11"}))).unwrap_err();
    assert_eq!(
        refusal(&end),
        ("invalid_event", "kind_change", "new_end_local")
    );
    let to_timed =
        update(all_day_args(json!({"new_start_local": "2026-02-11T09:00"}))).unwrap_err();
    assert_eq!(
        refusal(&to_timed),
        ("invalid_event", "kind_change", "new_start_local")
    );
    let junk = update(all_day_args(json!({"new_end_local": "later"}))).unwrap_err();
    assert_eq!(refusal(&junk).1, "bad_format");
}

#[test]
fn an_all_day_end_is_the_last_day_inclusive_and_is_stored_as_the_next_midnight() {
    let (_, c) = update(all_day_args(json!({"new_end_local": "2026-02-12"}))).unwrap();
    assert_eq!(c.start_ms, None);
    assert_eq!(c.end_ms, Some(ms(2026, 2, 13, 0, 0)));
    let same_day = update(all_day_args(json!({"new_end_local": "2026-02-10"})))
        .unwrap()
        .1;
    assert_eq!(same_day.end_ms, Some(ms(2026, 2, 11, 0, 0)));
    let before = update(all_day_args(json!({"new_end_local": "2026-02-09"}))).unwrap_err();
    assert_eq!(refusal(&before).1, "end_not_after_start");
    // The span is kept when only the start moves: two days from 20 February is the 20th and 21st.
    let (_, c) = update(all_day_args(json!({"new_start_local": "2026-02-20"}))).unwrap();
    assert_eq!(c.start_ms, Some(ms(2026, 2, 20, 0, 0)));
    assert_eq!(c.end_ms, Some(ms(2026, 2, 22, 0, 0)));
    let (_, c) = update(all_day_args(
        json!({"new_start_local": "2026-02-20", "new_end_local": "2026-02-20"}),
    ))
    .unwrap();
    assert_eq!(
        (c.start_ms, c.end_ms),
        (Some(ms(2026, 2, 20, 0, 0)), Some(ms(2026, 2, 21, 0, 0)))
    );
    // An all-day start in the past is allowed, as it is for creation.
    assert!(update(all_day_args(json!({"new_start_local": "2026-01-02"}))).is_ok());
    let far = update(all_day_args(json!({"new_start_local": "2040-01-02"}))).unwrap_err();
    assert_eq!(far["reason"], "start_too_far_ahead");
}

#[test]
fn a_delete_names_one_event_and_nothing_else() {
    let args = timed_args(json!({}));
    let t = check_delete(&args, LONDON).unwrap();
    assert_eq!(
        (t.event_id.as_str(), t.begin_ms),
        ("42", ms(2026, 2, 10, 15, 0))
    );
    for extra in ["new_title", "location"] {
        let err = check_delete(&timed_args(json!({extra: "x"})), LONDON).unwrap_err();
        assert_eq!(refusal(&err).1, "unknown_field");
    }
    let mut missing = args.clone();
    missing.as_object_mut().unwrap().remove("title");
    assert_eq!(
        refusal(&check_delete(&missing, LONDON).unwrap_err()).1,
        "missing"
    );
    let bad = timed_args(json!({"open_link": "hq://calendar/event/9"}));
    assert_eq!(
        refusal(&check_delete(&bad, LONDON).unwrap_err()).1,
        "bad_link"
    );
}

#[test]
fn each_phone_outcome_maps_and_an_unknown_one_is_unverified() {
    let payload = json!({});
    let up = |body: Value| finalize_update(&body, &payload);
    assert_eq!(
        up(json!({"v": 1, "outcome": "updated", "title": "secret"})),
        json!({"outcome": "updated"})
    );
    for code in [
        "event_not_found",
        "event_changed",
        "read_only_calendar",
        "recurring_unsupported",
        "calendar_unavailable",
        "update_unverified",
    ] {
        let out = up(json!({"v": 1, "outcome": code, "reason": "r\nx", "extra": "no"}));
        assert_eq!(out["outcome"], code);
        assert_eq!(out["reason"], "r x", "cleaned to one line");
        assert!(out.get("extra").is_none());
    }
    let denied = up(
        json!({"v": 1, "outcome": "permission_denied", "missing": "calendar_write", "next_step": "open settings"}),
    );
    assert_eq!(denied["next_step"], "open settings");
    let invalid = up(
        json!({"v": 1, "outcome": "invalid_event", "reason": "kind_change", "field": "new_start_local"}),
    );
    assert_eq!(
        refusal(&invalid),
        ("invalid_event", "kind_change", "new_start_local")
    );
    for unknown in ["created", "deleted", "success", "", "listed"] {
        let out = up(json!({"v": 1, "outcome": unknown}));
        assert_eq!(
            (
                out["outcome"].as_str().unwrap(),
                out["reason"].as_str().unwrap()
            ),
            ("update_unverified", "unexpected_outcome")
        );
    }
    let del = |body: Value| finalize_delete(&body, &payload);
    assert_eq!(
        del(json!({"v": 1, "outcome": "deleted"})),
        json!({"outcome": "deleted"})
    );
    assert_eq!(
        del(json!({"v": 1, "outcome": "updated"}))["outcome"],
        "delete_unverified"
    );
    assert_eq!(
        del(json!({"v": 1, "outcome": "event_changed"}))["outcome"],
        "event_changed"
    );
}

#[test]
fn the_route_accepts_only_each_actions_own_outcomes() {
    let body = |o: &str| json!({"v": 1, "outcome": o});
    assert!(result_is_acceptable_for(
        UPDATE_ACTION_NAME,
        &body("updated")
    ));
    assert!(result_is_acceptable_for(
        UPDATE_ACTION_NAME,
        &body("update_unverified")
    ));
    assert!(!result_is_acceptable_for(
        UPDATE_ACTION_NAME,
        &body("deleted")
    ));
    assert!(!result_is_acceptable_for(
        UPDATE_ACTION_NAME,
        &body("created")
    ));
    assert!(result_is_acceptable_for(
        DELETE_ACTION_NAME,
        &body("deleted")
    ));
    assert!(result_is_acceptable_for(
        DELETE_ACTION_NAME,
        &body("recurring_unsupported")
    ));
    assert!(!result_is_acceptable_for(
        DELETE_ACTION_NAME,
        &body("updated")
    ));
    assert!(!result_is_acceptable_for(
        DELETE_ACTION_NAME,
        &json!({"v": 2, "outcome": "deleted"})
    ));
    assert!(!result_is_acceptable_for(
        "calendar.other",
        &body("deleted")
    ));
}

type Frames = mpsc::UnboundedReceiver<Value>;

fn device(store: Arc<DeviceActionStore>, timeouts: Timeouts) -> (DeviceLink, Frames) {
    let (tx, rx) = mpsc::unbounded_channel();
    let emit = Arc::new(move |frame: Value| {
        let _ = tx.send(frame);
    });
    let phone = PhoneClock::parse("Europe/London", &Utc::now().to_rfc3339()).unwrap();
    (
        DeviceLink::new("thread-1", store, emit, phone).with_timeouts(timeouts),
        rx,
    )
}

const ROUND_TRIP: Timeouts = Timeouts {
    ack_secs: 2,
    result_secs: 2,
};
const FAST: Timeouts = Timeouts {
    ack_secs: 1,
    result_secs: 1,
};

/// An event two days out, so a new start is not in the past.
fn future_args(extra: Value) -> Value {
    let begin = (Utc::now() + chrono::TimeDelta::days(2)).timestamp_millis() / 60_000 * 60_000;
    let local = DateTime::from_timestamp_millis(begin)
        .unwrap()
        .with_timezone(&LONDON)
        .format("%Y-%m-%dT%H:%M")
        .to_string();
    let mut v = json!({"open_link": link("42", begin, begin + 3_600_000),
        "title": "Dentist", "start_local": local});
    merge(&mut v, extra);
    v
}

#[tokio::test]
async fn both_tools_refuse_without_their_skill_and_send_nothing() {
    let _off = set_active_for_this_thread(Some(&[CALENDAR_CREATION_SKILL]));
    let (t, mut frames) = device(Arc::default(), FAST);
    let up = run_update(&t, "c1", &future_args(json!({"new_title": "x"}))).await;
    let del = run_delete(&t, "c2", &future_args(json!({}))).await;
    for out in [up, del] {
        assert_eq!(
            (
                out["outcome"].as_str().unwrap(),
                out["reason"].as_str().unwrap()
            ),
            ("calendar_unavailable", "skill_not_active")
        );
    }
    assert!(frames.try_recv().is_err());
}

#[tokio::test]
async fn a_bad_request_never_reaches_the_phone() {
    let _on = set_active_for_this_thread(Some(&[CALENDAR_EDITING_SKILL]));
    let (t, mut frames) = device(Arc::default(), FAST);
    let out = run_update(&t, "c1", &future_args(json!({}))).await;
    assert_eq!(refusal(&out).1, "no_changes");
    let out = run_delete(
        &t,
        "c2",
        &json!({"open_link": "x", "title": "t", "start_local": "2030-01-01"}),
    )
    .await;
    assert_eq!(refusal(&out).1, "bad_link");
    assert!(frames.try_recv().is_err());
}

#[tokio::test]
async fn an_update_round_trips_with_the_wire_payload_the_phone_expects() {
    let _on = set_active_for_this_thread(Some(&[CALENDAR_EDITING_SKILL]));
    let store: Arc<DeviceActionStore> = Arc::default();
    let (t, mut frames) = device(store.clone(), ROUND_TRIP);
    let phone_side = {
        let store = store.clone();
        async move {
            let frame = frames.recv().await.unwrap();
            assert_eq!(frame["name"], "calendar.update");
            assert_eq!(frame["v"], 1);
            assert_eq!(frame["tool_call_id"], "call-9");
            let p = &frame["payload"];
            assert_eq!(p["event_id"], "42");
            assert_eq!(p["expected_title"], "Dentist");
            assert_eq!(p["zone"], "Europe/London");
            assert_eq!(p["all_day"], false);
            assert_eq!(p["changes"], json!({"title": "Check-up"}));
            assert!(p["request_id"].as_str().unwrap().len() >= 32);
            assert!(
                p.get("title").is_none() && p.get("start_local").is_none(),
                "no local times reach the phone"
            );
            let id = frame["action_id"].as_str().unwrap().to_string();
            store.ack(&id).unwrap();
            let name = store.action_name(&id).unwrap();
            let body = json!({"v": 1, "outcome": "updated"});
            store
                .settle(&id, &body, |b, p| {
                    crate::device_actions::finalize_for(name, b, p)
                })
                .unwrap();
        }
    };
    let args = future_args(json!({"new_title": "Check-up"}));
    let (out, ()) = tokio::join!(run_update(&t, "call-9", &args), phone_side);
    assert_eq!(out, json!({"outcome": "updated"}));
    assert!(store.pending(None).is_empty());
}

#[tokio::test]
async fn a_delete_round_trips_and_a_silent_phone_is_unverified_or_unreachable() {
    let _on = set_active_for_this_thread(Some(&[CALENDAR_EDITING_SKILL]));
    let store: Arc<DeviceActionStore> = Arc::default();
    let (t, mut frames) = device(store.clone(), ROUND_TRIP);
    let phone_side = {
        let store = store.clone();
        async move {
            let frame = frames.recv().await.unwrap();
            assert_eq!(frame["name"], "calendar.delete");
            assert!(frame["payload"].get("changes").is_none());
            let id = frame["action_id"].as_str().unwrap().to_string();
            store.ack(&id).unwrap();
            let name = store.action_name(&id).unwrap();
            let body = json!({"v": 1, "outcome": "deleted"});
            store
                .settle(&id, &body, |b, p| {
                    crate::device_actions::finalize_for(name, b, p)
                })
                .unwrap();
        }
    };
    let args = future_args(json!({}));
    let (out, ()) = tokio::join!(run_delete(&t, "c1", &args), phone_side);
    assert_eq!(out, json!({"outcome": "deleted"}));

    // Nobody acks: the request is withdrawn and nothing happened.
    let (t, _frames) = device(Arc::default(), FAST);
    assert_eq!(
        run_delete(&t, "c2", &args).await["outcome"],
        "device_unreachable"
    );

    // Acked but never answered: it may have happened.
    let store: Arc<DeviceActionStore> = Arc::default();
    let (t, mut frames) = device(store.clone(), FAST);
    let acker = async {
        let frame = frames.recv().await.unwrap();
        store.ack(frame["action_id"].as_str().unwrap()).unwrap();
    };
    let (out, ()) = tokio::join!(run_delete(&t, "c3", &args), acker);
    assert_eq!(
        (
            out["outcome"].as_str().unwrap(),
            out["reason"].as_str().unwrap()
        ),
        ("delete_unverified", "no_result_in_time")
    );
    let up_args = future_args(json!({"new_title": "x"}));
    let (t, mut frames) = device(store.clone(), FAST);
    let acker = async {
        let frame = frames.recv().await.unwrap();
        store.ack(frame["action_id"].as_str().unwrap()).unwrap();
    };
    let (out, ()) = tokio::join!(run_update(&t, "c4", &up_args), acker);
    assert_eq!(out["outcome"], "update_unverified");
}

#[test]
fn the_same_request_id_gets_the_first_answer_for_both_actions() {
    for name in [UPDATE_ACTION_NAME, DELETE_ACTION_NAME] {
        let store = DeviceActionStore::default();
        let open = || {
            store
                .open_named(
                    name,
                    "t1",
                    "call",
                    "req-1",
                    json!({"request_id": "req-1"}),
                    Timeouts::default(),
                )
                .ok()
                .unwrap()
        };
        let Opened::New { action_id, .. } = open() else {
            panic!("first call opens");
        };
        assert!(matches!(open(), Opened::Known { outcome: None }));
        store.ack(&action_id).unwrap();
        let body = json!({"v": 1, "outcome": "event_changed"});
        store
            .settle(&action_id, &body, |b, p| {
                crate::device_actions::finalize_for(name, b, p)
            })
            .unwrap();
        let Opened::Known {
            outcome: Some(first),
        } = open()
        else {
            panic!("a repeat returns the settled answer");
        };
        assert_eq!(first["outcome"], "event_changed");
        assert_eq!(store.pending(None).len(), 0, "no second frame is possible");
    }
}

#[test]
fn the_owner_is_told_the_action_and_a_short_cleaned_title() {
    let delete = always_ask_reason(
        "calendar_delete_event",
        &json!({"title": format!("Plan\n{}", "x".repeat(300)), "open_link": "l", "start_local": "s"}),
    )
    .unwrap();
    assert!(delete.starts_with("would delete the event \"Plan xxx"));
    assert!(delete.ends_with("from the owner's phone calendar"));
    assert!(
        !delete.contains('\n') && delete.chars().count() < 140,
        "{delete}"
    );
    let update = always_ask_reason(
        "calendar_update_event",
        &json!({"title": "Dentist", "new_title": "Check-up", "new_start_local": "2026-02-11T09:30"}),
    )
    .unwrap();
    assert_eq!(
        update,
        "would change the event \"Dentist\" on the owner's phone calendar: title to \"Check-up\", start to 2026-02-11T09:30"
    );
    assert_eq!(always_ask_reason("calendar_create_event", &json!({})), None);
    assert!(
        always_ask_reason("calendar_delete_event", &json!({}))
            .unwrap()
            .contains("an event")
    );
}
