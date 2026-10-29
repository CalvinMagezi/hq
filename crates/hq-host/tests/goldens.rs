//! Screens captured from real agent sessions, each labelled with the state the
//! agent was really in. The label is the start of the file name:
//! `<state>--<name>.screen` (plus an optional `.title` with the terminal title).
//!
//! Where an agent could say what it was doing, the label comes from the agent:
//! Claude Code's own `SessionStart`, `UserPromptSubmit`, `Stop` and
//! `Notification` hooks. Codex and Copilot have no such hook, so their labels
//! come from what was on screen (a dialog waiting for a choice, "esc to
//! interrupt" mid-turn). The Copilot screens come from a throwaway session in a
//! scratch folder, with the folder's path replaced.
#![cfg(unix)]

use hq_host::{AgentState, DetectInput, Detector};
use std::path::Path;

fn check_agent(agent: &str) -> usize {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(agent);
    let detector = Detector::builtin();
    let mut checked = 0;
    let mut failures = Vec::new();
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "screen") {
            continue;
        }
        let stem = path.file_stem().unwrap().to_string_lossy().to_string();
        let want = AgentState::parse(stem.split("--").next().unwrap())
            .expect("fixture name starts with a state");
        let screen = std::fs::read_to_string(&path).unwrap();
        let title = std::fs::read_to_string(path.with_extension("title")).unwrap_or_default();
        let got = detector
            .detect(
                agent,
                DetectInput {
                    screen: screen.trim_end_matches('\n'),
                    osc_title: title.trim_end_matches('\n'),
                },
            )
            .expect("rule file exists");
        if got.state != want {
            failures.push(format!(
                "{agent}/{stem}: wanted {want:?}, got {:?} (rule {:?})",
                got.state, got.rule
            ));
        }
        checked += 1;
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    checked
}

#[test]
fn claude_screens_from_real_sessions_are_detected_correctly() {
    assert!(check_agent("claude") >= 7);
}

#[test]
fn codex_screens_from_real_sessions_are_detected_correctly() {
    assert!(check_agent("codex") >= 8);
}

#[test]
fn copilot_screens_from_real_sessions_are_detected_correctly() {
    assert!(check_agent("copilot") >= 10);
}

/// A screen the rules do not recognise is unknown for Copilot, never idle: idle
/// is what lets text be typed into the pane.
#[test]
fn an_unrecognised_copilot_screen_is_unknown() {
    let got = Detector::builtin()
        .detect(
            "github-copilot",
            DetectInput {
                screen: " [Current]  Sessions   Issues   Pull requests   Gists\n\n some other view",
                osc_title: "",
            },
        )
        .unwrap();
    assert_eq!(got.state, AgentState::Unknown);
}

/// A dialog quoted higher up in the conversation, with the idle footer below
/// it, is not a dialog that is open now.
#[test]
fn a_copilot_dialog_far_above_the_footer_does_not_block() {
    let mut screen = String::from(
        "│ Do you want to run this command?\n│ ❯ 1. Yes\n│   3. No\n│ ↑/↓ to navigate · enter to select · esc to cancel\n",
    );
    for _ in 0..14 {
        screen.push_str(" ● some later output\n");
    }
    screen.push_str(&"─".repeat(40));
    screen.push_str("\n❯\n");
    screen.push_str(&"─".repeat(40));
    screen.push_str("\n ← open sidebar · Interactive · Manual Approval · / commands");
    let got = Detector::builtin()
        .detect(
            "copilot",
            DetectInput {
                screen: &screen,
                osc_title: "",
            },
        )
        .unwrap();
    assert_eq!(got.state, AgentState::Idle, "rule {:?}", got.rule);
}
