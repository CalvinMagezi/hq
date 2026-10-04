//! Deterministic dismissal of known harmless, non-task prompts in a driven session.
//!
//! Claude Code interrupts a session with an optional feedback survey that waits for a key. A
//! driver that reads it as "unrelated to the task" stalls the loop until a human presses `0`.
//! This module recognises that one prompt by exact text and answers it with a fixed key.
//!
//! Screen text is untrusted. The key is a constant, never derived from the screen, the match is
//! anchored on two exact lines near the end of the screen, and anything that looks like a trust
//! dialog or a permission request is refused outright. Only Drive-on sessions are touched: the
//! supervisor never types into an observe-only session.

use hq_db::Database;
use hq_db::harness_sessions_registry::{
    self as registry, DISMISSAL_CAP, DismissClaim, HarnessSessionRow,
};

/// Non-empty lines at the end of the screen where the prompt must sit. Covers the survey, the
/// update banner and the input box below it, but not scrollback the agent printed earlier.
const TAIL_LINES: usize = 12;

const SURVEY_TITLE: &str = "How is Claude doing this session? (optional)";
const SURVEY_OPTIONS: &str = "1: Bad 2: Fine 3: Good 0: Dismiss";

/// Text that marks a dialog that needs a human decision. Seeing any of it in the tail vetoes
/// every dismissal, even next to a survey.
const DECISION_MARKERS: &[&str] = &[
    "do you trust",
    "trust the files",
    "do you want to",
    "would you like",
    "no, exit",
    "yes, proceed",
    "allow this",
    "permission to",
    "requires permission",
    "enter to select",
    "esc to cancel",
    "(y/n)",
];

/// A prompt HQ may dismiss, with the fixed keys that dismiss it.
#[derive(Debug, PartialEq, Eq)]
pub struct KnownPrompt {
    pub name: &'static str,
    pub keys: &'static [&'static str],
}

const CLAUDE_SURVEY: KnownPrompt = KnownPrompt {
    name: "claude-code feedback survey",
    keys: &["0"],
};

fn squash(line: &str) -> String {
    line.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The last non-empty lines of `screen`, whitespace squashed. Blank padding between the survey
/// lines does not count against the window.
fn tail_lines(screen: &str) -> Vec<String> {
    let mut tail: Vec<String> = screen
        .lines()
        .map(squash)
        .filter(|l| !l.is_empty())
        .rev()
        .take(TAIL_LINES)
        .collect();
    tail.reverse();
    tail
}

/// FNV-1a of the tail, hex. Stable across processes, unlike `DefaultHasher`.
pub fn tail_hash(screen: &str) -> String {
    let hash = tail_lines(screen)
        .join("\n")
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
        });
    format!("{hash:016x}")
}

/// Claude Code's input line starts with one of these (the box variant sits inside `│ … │`).
const PROMPT_GLYPHS: [char; 2] = ['❯', '>'];

/// True when an input line carries any text. A ghost suggestion looks the same as a draft and
/// cannot be told apart from the screen text alone, so any text counts as a draft.
fn has_draft(line: &str) -> bool {
    let inner =
        line.trim_matches(|c: char| matches!(c, '│' | '┃' | '╭' | '╰' | '─') || c.is_whitespace());
    inner
        .strip_prefix(PROMPT_GLYPHS)
        .is_some_and(|after| !after.trim().is_empty())
}

/// The known prompt waiting at the end of `screen` (already ANSI-cleaned), if any.
pub fn recognise(harness: &str, screen: &str) -> Option<&'static KnownPrompt> {
    if harness != "claude-code" {
        return None;
    }
    let tail = tail_lines(screen);
    let lowered = tail.join("\n").to_lowercase();
    if DECISION_MARKERS.iter().any(|m| lowered.contains(m)) {
        return None;
    }
    let survey_end = tail.iter().enumerate().find_map(|(i, line)| {
        let line = line.trim_start_matches(['●', '•', '*']).trim_start();
        let rest = line.strip_prefix(SURVEY_TITLE)?.trim();
        if rest.is_empty() {
            (tail.get(i + 1).map(String::as_str) == Some(SURVEY_OPTIONS)).then_some(i + 1)
        } else {
            (rest == SURVEY_OPTIONS).then_some(i)
        }
    })?;
    // A draft in the input box would get the key typed into it.
    let draft_in_box = tail[survey_end + 1..].iter().any(|line| has_draft(line));
    (!draft_in_box).then_some(&CLAUDE_SURVEY)
}

/// Why a key press failed, which decides whether the dismissal claim is handed back.
#[derive(Debug, PartialEq, Eq)]
pub enum PressError {
    /// The key certainly never reached the pane (host refused or was never called).
    NotSent,
    /// A timeout or lost connection: the key may have landed, so the claim stays spent.
    MaybeSent,
}

/// What one supervisor pass did about a recognised prompt.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Nothing recognised, or the session is not one the supervisor may type into.
    Untouched,
    /// The key was sent. `first` is true for the session's first dismissal ever.
    Dismissed { first: bool },
    /// The cap was just reached: nothing sent, the operator should be told (returned once).
    CapReached,
    /// The previous key left the screen tail unchanged, so it did nothing: stopped and the
    /// operator should be told (returned once).
    Stuck,
    /// The key could not be sent (the claim was handed back).
    Failed,
}

/// Dismiss a known prompt on `row` if its screen shows one. `reread` fetches the screen again
/// right before the key goes out, so a prompt that closed in the meantime does not get a stray
/// `0` typed into the input box. `press` sends the fixed keys. Neither the nudge budget nor the
/// key allowance is touched: dismissals have their own counter and cap.
pub fn dismiss_known_prompt(
    db: &Database,
    row: &HarnessSessionRow,
    screen: &str,
    working: bool,
    reread: impl FnOnce() -> Option<String>,
    press: impl FnOnce(&[String]) -> Result<(), PressError>,
) -> Outcome {
    // Text on screen while the agent is working is the agent's own output, not a prompt.
    if working || !row.drive || row.owner_thread.is_none() {
        return Outcome::Untouched;
    }
    let Some(prompt) = recognise(&row.harness, screen) else {
        return Outcome::Untouched;
    };
    let hash = tail_hash(screen);
    if row.last_dismiss_tail.as_deref() == Some(hash.as_str()) {
        let id = row.id.clone();
        let first = db
            .with_conn(move |c| registry::mark_dismiss_stuck(c, &id, DISMISSAL_CAP))
            .unwrap_or(false);
        if first {
            let id = row.id.clone();
            let _ = db.with_conn(move |c| {
                registry::record_event(
                    c,
                    &id,
                    registry::EVENT_DISMISS_CAP,
                    registry::ACTOR_GUARD,
                    Some("prompt did not close"),
                )
            });
            return Outcome::Stuck;
        }
        return Outcome::Untouched;
    }
    let id = row.id.clone();
    match db.with_conn(move |c| registry::claim_dismissal(c, &id, DISMISSAL_CAP)) {
        Ok(DismissClaim::Granted(_)) => {}
        Ok(DismissClaim::CapReached) => {
            let id = row.id.clone();
            let _ = db.with_conn(move |c| {
                registry::record_event(
                    c,
                    &id,
                    registry::EVENT_DISMISS_CAP,
                    registry::ACTOR_GUARD,
                    Some(prompt.name),
                )
            });
            return Outcome::CapReached;
        }
        _ => return Outcome::Untouched,
    }
    let still_there = reread().is_some_and(|fresh| recognise(&row.harness, &fresh) == Some(prompt));
    let keys: Vec<String> = prompt.keys.iter().map(|k| (*k).to_string()).collect();
    let pressed = still_there.then(|| press(&keys));
    let id = row.id.clone();
    match pressed {
        None | Some(Err(PressError::NotSent)) => {
            let _ = db.with_conn(move |c| registry::refund_dismissal(c, &id));
            return if still_there {
                Outcome::Failed
            } else {
                Outcome::Untouched
            };
        }
        Some(Err(PressError::MaybeSent)) => {
            let _ = db.with_conn(move |c| {
                registry::set_dismiss_tail(c, &id, &hash)?;
                registry::record_event(
                    c,
                    &id,
                    registry::EVENT_PROMPT_DISMISSED,
                    registry::ACTOR_GUARD,
                    Some("send unconfirmed"),
                )
            });
            return Outcome::Failed;
        }
        Some(Ok(())) => {}
    }
    let first = db
        .with_conn(move |c| {
            registry::set_dismiss_tail(c, &id, &hash)?;
            let first = registry::count_events(c, &id, registry::EVENT_PROMPT_DISMISSED)? == 0;
            registry::record_event(
                c,
                &id,
                registry::EVENT_PROMPT_DISMISSED,
                registry::ACTOR_GUARD,
                Some(prompt.name),
            )?;
            Ok(first)
        })
        .unwrap_or(false);
    Outcome::Dismissed { first }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    const SURVEY: &str = "● How is Claude doing this session? (optional)\n  1: Bad    2: Fine   3: Good   0: Dismiss\n\n╭────╮\n│ >  │\n╰────╯\n  Update installed · Restart to update";

    fn seed(db: &Database, drive: bool) -> HarnessSessionRow {
        db.with_conn(|c| {
            registry::insert(
                c,
                &registry::NewSession {
                    id: "hs-d",
                    harness: "claude-code",
                    label: "l",
                    cwd: "/tmp",
                    mission_id: None,
                    placement: registry::Placement {
                        host: "local",
                        agent_name: "hs-d",
                        workspace_id: "w1",
                        pane_id: "w1:p1",
                    },
                },
            )?;
            registry::set_owner(c, "hs-d", Some("th-1"))?;
            registry::set_drive(c, "hs-d", drive)?;
            Ok(registry::get(c, "hs-d")?.unwrap())
        })
        .unwrap()
    }

    fn run(db: &Database, screen: &str, sent: &RefCell<Vec<Vec<String>>>) -> Outcome {
        let row = db.with_conn(|c| registry::get(c, "hs-d")).unwrap().unwrap();
        dismiss_known_prompt(
            db,
            &row,
            screen,
            false,
            || Some(screen.to_string()),
            |k| {
                sent.borrow_mut().push(k.to_vec());
                Ok(())
            },
        )
    }

    #[test]
    fn survey_is_recognised_and_dismissed_with_zero() {
        let db = Database::open_memory().unwrap();
        seed(&db, true);
        let sent = RefCell::new(vec![]);
        assert_eq!(run(&db, SURVEY, &sent), Outcome::Dismissed { first: true });
        assert_eq!(sent.borrow().as_slice(), &[vec!["0".to_string()]]);
        let row = db.with_conn(|c| registry::get(c, "hs-d")).unwrap().unwrap();
        assert_eq!((row.dismissals, row.nudges_sent, row.keys_sent), (1, 0, 0));
        let events = db
            .with_conn(|c| registry::list_events(c, "hs-d", 10))
            .unwrap();
        assert!(events.iter().any(
            |e| e.kind == registry::EVENT_PROMPT_DISMISSED && e.actor == registry::ACTOR_GUARD
        ));
        let changed = format!("{SURVEY}\ntick 1");
        assert_eq!(
            run(&db, &changed, &sent),
            Outcome::Dismissed { first: false }
        );
    }

    #[test]
    fn survey_on_one_line_is_recognised() {
        let one = "done\n● How is Claude doing this session? (optional)  1: Bad  2: Fine  3: Good  0: Dismiss";
        assert!(recognise("claude-code", one).is_some());
    }

    #[test]
    fn survey_text_in_old_scrollback_is_ignored() {
        let filler = "output line\n".repeat(TAIL_LINES + 3);
        let screen = format!("{SURVEY}\n{filler}$ ");
        assert!(recognise("claude-code", &screen).is_none());
    }

    #[test]
    fn partial_or_altered_survey_is_ignored() {
        assert!(
            recognise(
                "claude-code",
                "How is Claude doing this session? (optional)"
            )
            .is_none()
        );
        assert!(recognise("claude-code", "1: Bad  2: Fine  3: Good  0: Dismiss").is_none());
        let altered = "How is Claude doing this session? (optional)\n1: Bad  2: Fine  3: Good  0: Quit everything";
        assert!(recognise("claude-code", altered).is_none());
        assert!(recognise("codex", SURVEY).is_none());
    }

    #[test]
    fn dialogs_needing_a_human_are_never_dismissed() {
        for dialog in [
            "Do you trust the files in this folder?\n 1. Yes, proceed\n 2. No, exit",
            "Do you want to proceed?\n 1. Yes\n 2. No",
            "Bash command: rm -rf x\nDo you want to run this?",
        ] {
            assert!(recognise("claude-code", dialog).is_none(), "{dialog}");
            let with_survey = format!("{dialog}\n{SURVEY}");
            assert!(
                recognise("claude-code", &with_survey).is_none(),
                "{with_survey}"
            );
        }
    }

    #[test]
    fn cap_stops_dismissing_and_reports_once() {
        let db = Database::open_memory().unwrap();
        seed(&db, true);
        let sent = RefCell::new(vec![]);
        let screen = |n: i64| format!("{SURVEY}\ntick {n}");
        for n in 0..DISMISSAL_CAP {
            assert!(matches!(
                run(&db, &screen(n), &sent),
                Outcome::Dismissed { .. }
            ));
        }
        assert_eq!(run(&db, &screen(90), &sent), Outcome::CapReached);
        assert_eq!(run(&db, &screen(91), &sent), Outcome::Untouched);
        assert_eq!(sent.borrow().len() as i64, DISMISSAL_CAP);
        let events = db
            .with_conn(|c| registry::list_events(c, "hs-d", 50))
            .unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|e| e.kind == registry::EVENT_DISMISS_CAP)
                .count(),
            1
        );
    }

    #[test]
    fn observe_only_session_is_untouched() {
        let db = Database::open_memory().unwrap();
        seed(&db, false);
        let sent = RefCell::new(vec![]);
        assert_eq!(run(&db, SURVEY, &sent), Outcome::Untouched);
        assert!(sent.borrow().is_empty());
        let row = db.with_conn(|c| registry::get(c, "hs-d")).unwrap().unwrap();
        assert_eq!(row.dismissals, 0);
    }

    #[test]
    fn prompt_that_closed_before_the_key_is_not_typed_into() {
        let db = Database::open_memory().unwrap();
        let row = seed(&db, true);
        let sent = RefCell::new(vec![]);
        let out = dismiss_known_prompt(
            &db,
            &row,
            SURVEY,
            false,
            || Some("> ".into()),
            |k| {
                sent.borrow_mut().push(k.to_vec());
                Ok(())
            },
        );
        assert_eq!(out, Outcome::Untouched);
        assert!(sent.borrow().is_empty());
        let row = db.with_conn(|c| registry::get(c, "hs-d")).unwrap().unwrap();
        assert_eq!(row.dismissals, 0, "claim handed back");
    }

    #[test]
    fn refused_send_hands_the_claim_back() {
        let db = Database::open_memory().unwrap();
        let row = seed(&db, true);
        let out = dismiss_known_prompt(
            &db,
            &row,
            SURVEY,
            false,
            || Some(SURVEY.into()),
            |_| Err(PressError::NotSent),
        );
        assert_eq!(out, Outcome::Failed);
        let row = db.with_conn(|c| registry::get(c, "hs-d")).unwrap().unwrap();
        assert_eq!(row.dismissals, 0);
    }

    #[test]
    fn unconfirmed_send_keeps_the_claim_to_avoid_a_double_key() {
        let db = Database::open_memory().unwrap();
        let row = seed(&db, true);
        let out = dismiss_known_prompt(
            &db,
            &row,
            SURVEY,
            false,
            || Some(SURVEY.into()),
            |_| Err(PressError::MaybeSent),
        );
        assert_eq!(out, Outcome::Failed);
        let row = db.with_conn(|c| registry::get(c, "hs-d")).unwrap().unwrap();
        assert_eq!(row.dismissals, 1);
    }

    const BYPASS_FOOTER: &str = "  ⏵⏵ bypass permissions on (shift+tab to cycle)";

    #[test]
    fn bypass_mode_footer_does_not_block_dismissal() {
        let screen = format!(
            "{SURVEY}
{BYPASS_FOOTER}"
        );
        assert!(recognise("claude-code", &screen).is_some());
    }

    #[test]
    fn real_dialogs_with_the_bypass_footer_are_still_vetoed() {
        for dialog in [
            "Claude requires permission to edit src/main.rs\n 1. Yes\n 2. No",
            "Do you want to make this edit?\n 1. Yes",
            "Would you like to proceed with this plan?\n 1. Yes",
            "Which option?\n Enter to select · ↑/↓ to navigate · Esc to cancel",
            "Proceed? (y/n)",
        ] {
            let screen = format!("{dialog}\n{SURVEY}\n{BYPASS_FOOTER}");
            assert!(recognise("claude-code", &screen).is_none(), "{dialog}");
        }
    }

    #[test]
    fn survey_text_printed_while_the_agent_works_gets_no_key() {
        let db = Database::open_memory().unwrap();
        let row = seed(&db, true);
        let sent = RefCell::new(vec![]);
        let out = dismiss_known_prompt(
            &db,
            &row,
            SURVEY,
            true,
            || Some(SURVEY.into()),
            |k| {
                sent.borrow_mut().push(k.to_vec());
                Ok(())
            },
        );
        assert_eq!(out, Outcome::Untouched);
        assert!(sent.borrow().is_empty());
        let row = db.with_conn(|c| registry::get(c, "hs-d")).unwrap().unwrap();
        assert_eq!(row.dismissals, 0);
    }

    #[test]
    fn a_draft_in_the_input_box_gets_no_key() {
        let drafted = SURVEY.replace("│ >  │", "│ > fix the login bug │");
        assert!(recognise("claude-code", &drafted).is_none());
        assert!(recognise("claude-code", SURVEY).is_some());
    }

    const OBSERVED_DRAFT: &str = "● How is Claude doing this session? (optional)\n\n\n  1: Bad  2: Fine  3: Good  0: Dismiss\n\n\n❯ 00000\n  ⏵⏵ bypass permissions on (shift+tab to cycle)";

    #[test]
    fn the_real_prompt_glyph_marks_a_draft() {
        assert!(recognise("claude-code", OBSERVED_DRAFT).is_none());
        assert!(recognise("claude-code", &OBSERVED_DRAFT.replace("❯ 00000", "> 00000")).is_none());
        assert!(
            recognise(
                "claude-code",
                &OBSERVED_DRAFT.replace("❯ 00000", "│ ❯ 00000 │")
            )
            .is_none()
        );
        // NBSP after the glyph is whitespace too.
        assert!(
            recognise(
                "claude-code",
                &OBSERVED_DRAFT.replace("❯ 00000", "❯\u{a0}0")
            )
            .is_none()
        );
    }

    #[test]
    fn an_empty_prompt_line_does_not_block_dismissal() {
        for empty in ["❯", "❯ ", "│ ❯ │", "> "] {
            let screen = OBSERVED_DRAFT.replace("❯ 00000", empty);
            assert!(recognise("claude-code", &screen).is_some(), "{empty:?}");
        }
    }

    #[test]
    fn an_unchanged_tail_after_a_dismissal_stops_once_and_types_nothing_more() {
        let db = Database::open_memory().unwrap();
        seed(&db, true);
        let sent = RefCell::new(vec![]);
        assert!(matches!(run(&db, SURVEY, &sent), Outcome::Dismissed { .. }));
        assert_eq!(run(&db, SURVEY, &sent), Outcome::Stuck);
        assert_eq!(run(&db, SURVEY, &sent), Outcome::Untouched);
        assert_eq!(sent.borrow().len(), 1);
        let row = db.with_conn(|c| registry::get(c, "hs-d")).unwrap().unwrap();
        assert!(
            row.dismissals > DISMISSAL_CAP,
            "claims refused from here on"
        );
        let events = db
            .with_conn(|c| registry::list_events(c, "hs-d", 50))
            .unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|e| e.detail.as_deref() == Some("prompt did not close"))
                .count(),
            1
        );
    }
}
