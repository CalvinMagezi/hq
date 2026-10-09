//! Screens captured from real agent sessions, each labelled with the state the
//! agent was really in. The label is the start of the file name:
//! `<state>--<name>.screen` (plus an optional `.title` with the terminal title).
//!
//! Where an agent could say what it was doing, the label comes from the agent:
//! Claude Code's own `SessionStart`, `UserPromptSubmit`, `Stop` and
//! `Notification` hooks. Codex has no such hook, so its labels come from what
//! was on screen (a dialog waiting for a choice, "esc to interrupt" mid-turn).
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
