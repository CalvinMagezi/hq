//! Dialogs read off the blocked screens captured from real sessions
//! (`tests/fixtures/<agent>/blocked--*.screen`): what each one asks, what it
//! is about, its options, and which of them would widen the agent's
//! permissions. Every blocked fixture of an agent whose rules can read
//! dialogs is covered, so a new capture without an expectation fails.
#![cfg(unix)]

use hq_host::{AgentState, DetectInput, Detector, Dialog, DialogKind};
use std::path::Path;

fn read(agent: &str, name: &str) -> Dialog {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(agent);
    let screen = std::fs::read_to_string(dir.join(format!("{name}.screen"))).unwrap();
    let title = std::fs::read_to_string(dir.join(format!("{name}.title"))).unwrap_or_default();
    let detector = Detector::builtin();
    let screen = screen.trim_end_matches('\n');
    let got = detector
        .detect(
            agent,
            DetectInput {
                screen,
                osc_title: title.trim_end_matches('\n'),
            },
        )
        .unwrap();
    assert_eq!(got.state, AgentState::Blocked, "{agent}/{name}");
    let rule = got.rule.expect("a blocked screen names its rule");
    let lines: Vec<String> = screen.lines().map(str::to_string).collect();
    detector
        .dialog(agent, &rule, &lines)
        .unwrap_or_else(|| panic!("{agent}/{name}: rule {rule} read no dialog"))
}

/// `(id, label, selected, remember, typed)` for each option.
fn options(d: &Dialog) -> Vec<(u32, &str, bool, bool, bool)> {
    d.options
        .iter()
        .map(|o| (o.id, o.label.as_str(), o.selected, o.remember, o.typed))
        .collect()
}

#[test]
fn copilot_run_command() {
    let d = read("copilot", "blocked--run-command");
    assert_eq!(d.kind, DialogKind::Command);
    assert_eq!(
        d.command.as_deref(),
        Some("mkdir -p sub && touch sub/a.txt && ls /tmp > /dev/null")
    );
    assert_eq!(
        d.question.as_deref(),
        Some("Do you want to run this command?")
    );
    let opts = options(&d);
    assert_eq!(opts[0], (1, "Yes", true, false, false));
    assert_eq!((opts[1].0, opts[1].3), (2, true), "{opts:?}");
    assert_eq!((opts[2].0, opts[2].4), (3, true), "{opts:?}");
    assert_eq!(opts.len(), 3);
    assert_eq!(d.decline, ["esc"]);
    assert!(!d.truncated);
    assert!(
        d.text.starts_with("Create sub/a.txt and list /tmp"),
        "{}",
        d.text
    );
}

#[test]
fn copilot_run_command_with_the_cursor_moved() {
    let d = read("copilot", "blocked--run-command-cursor-on-no");
    assert_eq!(d.selected().map(|o| o.id), Some(3));
    let moved = read("copilot", "blocked--run-command");
    assert_eq!(
        d.identity(),
        moved.identity(),
        "the cursor is not part of the dialog"
    );
}

#[test]
fn copilot_create_file() {
    let d = read("copilot", "blocked--create-file");
    assert_eq!(d.kind, DialogKind::Edit);
    assert_eq!(
        d.command.as_deref(),
        Some("/Users/you/Documents/HQ/dialog-check/check.txt")
    );
    let opts = options(&d);
    assert_eq!(opts[0], (1, "Yes", true, false, false));
    assert!(opts[1].3, "don't ask again widens permissions: {opts:?}");
    assert!(opts[2].4, "{opts:?}");
}

#[test]
fn copilot_path_access_highlights_the_remember_option() {
    let d = read("copilot", "blocked--path-access");
    assert_eq!(d.kind, DialogKind::Path);
    assert_eq!(d.command.as_deref(), Some("/tmp -> /private/tmp"));
    assert_eq!(d.question.as_deref(), Some("Do you want to allow this?"));
    assert_eq!(
        options(&d),
        [
            (1, "Yes", false, false, false),
            (
                2,
                "Yes, and remember this path for this session",
                true,
                true,
                false
            ),
            (3, "No (Esc)", false, false, false),
        ]
    );
}

#[test]
fn copilot_ask_user() {
    let d = read("copilot", "blocked--ask-user");
    assert_eq!(d.kind, DialogKind::Question);
    assert_eq!(
        d.question.as_deref(),
        Some("Which colour should check.txt mention?")
    );
    assert_eq!(
        options(&d),
        [
            (1, "red", true, false, false),
            (2, "blue", false, false, false),
            (3, "Other (type your answer)", false, false, true),
        ]
    );
    assert_eq!(d.decline, ["ctrl+d"]);
}

#[test]
fn claude_create_file() {
    let d = read("claude", "blocked--permission");
    assert_eq!(d.kind, DialogKind::Edit);
    assert_eq!(d.command.as_deref(), Some("hello.txt"));
    assert_eq!(
        d.question.as_deref(),
        Some("Do you want to create hello.txt?")
    );
    let opts = options(&d);
    assert_eq!(opts.len(), 3, "{opts:?}");
    assert_eq!(opts[0], (1, "Yes", true, false, false));
    assert!(opts[1].3, "accept-edits mode widens permissions: {opts:?}");
    assert_eq!(opts[2], (3, "No", false, false, false));
}

#[test]
fn claude_trust_dialog() {
    let d = read("claude", "blocked--trust-dialog");
    assert_eq!(d.kind, DialogKind::Trust);
    assert_eq!(
        options(&d),
        [
            (1, "No, exit", true, false, false),
            (2, "Yes, I trust this folder", false, false, false),
        ]
    );
}

#[test]
fn claude_ask_user_question() {
    // AskUserQuestion draws a second rule above "Chat about this": the dialog's top is the first.
    let d = read("claude", "blocked--ask-user-question");
    assert_eq!(d.kind, DialogKind::Other);
    assert_eq!(
        d.question.as_deref(),
        Some("Which colour should the test use?")
    );
    assert_eq!(
        options(&d),
        [
            (1, "Red", true, false, false),
            (2, "Blue", false, false, false),
            (3, "Type something.", false, false, true),
            (4, "Chat about this", false, false, true),
        ]
    );
}

#[test]
fn codex_trust_directory() {
    let d = read("codex", "blocked--trust-directory");
    assert_eq!(d.kind, DialogKind::Trust);
    assert_eq!(d.command.as_deref(), Some("/tmp/work"));
    assert_eq!(
        options(&d),
        [
            (1, "Yes, continue", true, false, false),
            (2, "No, quit", false, false, false),
        ]
    );
}

#[test]
fn codex_update_and_hooks_dialogs() {
    let update = read("codex", "blocked--update-dialog");
    assert_eq!(update.options.len(), 3);
    assert_eq!(update.selected().map(|o| o.id), Some(1));
    let hooks = read("codex", "blocked--hooks-review");
    assert_eq!(hooks.kind, DialogKind::Trust);
    assert_eq!(
        options(&hooks)
            .iter()
            .map(|o| (o.0, o.3))
            .collect::<Vec<_>>(),
        [(1, false), (2, true), (3, false)]
    );
}

#[test]
fn every_blocked_fixture_reads_as_a_dialog() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut seen = 0;
    for agent in ["claude", "codex", "copilot"] {
        for entry in std::fs::read_dir(root.join(agent)).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_stem().unwrap().to_string_lossy().to_string();
            if path.extension().is_some_and(|e| e == "screen") && name.starts_with("blocked--") {
                let d = read(agent, &name);
                assert!(!d.options.is_empty(), "{agent}/{name} has no options");
                seen += 1;
            }
        }
    }
    assert!(seen >= 10, "{seen}");
}
