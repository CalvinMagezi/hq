//! Recorded sessions (tests/fixtures, captured 2026-11-12 from real agents with paths and menus
//! trimmed) replayed through the tracker. A change that breaks how a real agent's messages are
//! read fails here without starting any agent.

use hq_acp::{
    AgentState, Choice, FailureKind, Incoming, OptionKind, StopReason, Tracker, TurnEnd, parse_line,
};
use serde_json::Value;

struct Replay {
    tracker: Tracker,
    dialogs: usize,
    remember_options: usize,
}

fn replay(name: &str) -> Replay {
    let path = format!("{}/tests/fixtures/{name}.jsonl", env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let mut r = Replay {
        tracker: Tracker::default(),
        dialogs: 0,
        remember_options: 0,
    };
    r.tracker.set_cwd("/work");
    for line in text.lines() {
        let rec: Value = serde_json::from_str(line).expect("fixture line is JSON");
        let msg = rec["msg"].to_string();
        if rec["dir"] == "out" {
            if rec["msg"]["method"] == "session/prompt" {
                r.tracker
                    .start_prompt(rec["msg"]["id"].as_u64().expect("prompt id"));
            }
            continue;
        }
        let incoming: Incoming = parse_line(&msg).expect("fixture message parses");
        r.tracker.feed(&incoming);
        answer_like_the_recorder(&mut r);
    }
    r
}

/// The recording client picked the first allow-once option; do the same so the turn continues.
fn answer_like_the_recorder(r: &mut Replay) {
    let Some(p) = r.tracker.pending() else { return };
    assert_eq!(r.tracker.state(), AgentState::Blocked);
    r.dialogs += 1;
    r.remember_options += p
        .dialog
        .options
        .iter()
        .filter(|o| o.kind.remembers())
        .count();
    let allow = p
        .dialog
        .options
        .iter()
        .find(|o| o.kind == OptionKind::AllowOnce);
    let (seq, id) = (p.seq, allow.expect("an allow-once option").id.clone());
    r.tracker.answer(seq, &Choice::Option(id)).expect("answer");
}

fn assert_ended(r: &Replay, reason: StopReason) {
    assert!(r.tracker.done(), "turn did not finish");
    assert_eq!(r.tracker.state(), AgentState::Idle);
    assert!(
        matches!(r.tracker.end(), Some(TurnEnd::Stopped { reason: got, .. }) if *got == reason)
    );
}

fn assert_edited_hello(r: &Replay) {
    assert!(
        r.tracker.touched().contains("/work/hello.txt"),
        "touched: {:?}",
        r.tracker.touched()
    );
}

#[test]
fn hermes_turn() {
    let r = replay("hermes-turn");
    assert_ended(&r, StopReason::EndTurn);
    assert!(r.tracker.text().contains("DONE_SPIKE"));
    assert_edited_hello(&r);
    assert_eq!(r.dialogs, 1);
    assert_eq!(r.remember_options, 0);
    assert_eq!(r.tracker.context_use().1, Some(272_000));
}

#[test]
fn agy_turn() {
    let r = replay("agy-turn");
    assert_ended(&r, StopReason::EndTurn);
    assert!(r.tracker.text().contains("DONE_SPIKE"));
    assert_edited_hello(&r);
    assert_eq!(r.dialogs, 1);
}

#[test]
fn copilot_turn_offers_a_remember_option() {
    let r = replay("copilot-turn");
    assert_ended(&r, StopReason::EndTurn);
    assert_edited_hello(&r);
    assert_eq!(r.remember_options, 1);
}

#[test]
fn cursor_edits_without_asking() {
    let r = replay("cursor-turn");
    assert_ended(&r, StopReason::EndTurn);
    assert_eq!(r.dialogs, 0);
    assert_edited_hello(&r);
}

#[test]
fn opencode_turn() {
    let r = replay("opencode-turn");
    assert_ended(&r, StopReason::EndTurn);
}

#[test]
fn opencode_provider_error_is_a_billing_failure() {
    let r = replay("opencode-error");
    assert!(r.tracker.done());
    assert!(matches!(
        r.tracker.end(),
        Some(TurnEnd::Failed {
            kind: FailureKind::Billing,
            ..
        })
    ));
}

#[test]
fn cancelled_turns_end_cancelled() {
    for name in ["agy-cancel", "copilot-cancel", "hermes-cancel"] {
        assert_ended(&replay(name), StopReason::Cancelled);
    }
}
