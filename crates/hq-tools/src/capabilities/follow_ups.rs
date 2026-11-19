//! Next steps under a finished reply, chosen from what this one turn did (the tools it called and
//! the files it made). Pure and on the device: no model, no history, no stored usage, so nothing
//! here can leak what the person uses HQ for (`docs/security/ADOPTION_DATA.md`).

use serde::Serialize;

/// Fewer than this reads as an accident, so the row is left out.
const MIN_STEPS: usize = 2;
const MAX_STEPS: usize = 4;

const RESEARCH_TOOLS: [&str; 2] = ["web_search", "web_fetch"];
const HELPER_TOOLS: [&str; 2] = ["spawn_subagents", "agent_delegate"];
const NOTE_TOOLS: [&str; 2] = ["vault_write_note", "office_create"];

/// One chip: the plain words on it and the request a tap sends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FollowUp {
    pub label: &'static str,
    pub prompt: &'static str,
}

const fn step(label: &'static str, prompt: &'static str) -> FollowUp {
    FollowUp { label, prompt }
}

/// What a turn did, as far as a next step cares.
struct Done {
    researched: bool,
    helpers: bool,
    sheet: bool,
    page: bool,
    note: bool,
}

fn what_was_done(tools: &[String], file_kinds: &[&str]) -> Done {
    let called = |set: &[&str]| tools.iter().any(|t| set.contains(&t.as_str()));
    Done {
        researched: called(&RESEARCH_TOOLS),
        helpers: called(&HELPER_TOOLS),
        sheet: file_kinds.contains(&"sheet"),
        page: file_kinds
            .iter()
            .any(|k| ["html", "doc", "pdf", "slides"].contains(k)),
        note: called(&NOTE_TOOLS),
    }
}

/// Two to four next steps that fit this turn, most useful first; empty when the turn gives no
/// reason for any (a plain chat answer).
pub fn follow_ups(tools: &[String], file_kinds: &[&str]) -> Vec<FollowUp> {
    let d = what_was_done(tools, file_kinds);
    let mut out = Vec::new();
    if d.sheet {
        out.push(step(
            "Add more places or dates",
            "Add more places and earlier dates to the spreadsheet so I can compare them over time.",
        ));
        out.push(step(
            "Email the sheet",
            "Draft an email with the spreadsheet attached, and show me before anything is sent.",
        ));
    }
    if d.page || d.sheet {
        out.push(step(
            "Make a PDF to share",
            "Make a PDF of this that I can send to someone.",
        ));
    }
    if d.researched {
        out.push(step(
            "Check one more source",
            "Check one more source and tell me whether the numbers agree.",
        ));
        out.push(step(
            "Tell me when it changes",
            "Keep an eye on this and tell me when it changes.",
        ));
        if !d.note {
            out.push(step(
                "Keep this as a note",
                "Save what you found as a note I can search later.",
            ));
        }
    }
    if d.helpers {
        out.push(step(
            "Double-check the numbers",
            "Ask a helper to double-check the numbers and say what is uncertain.",
        ));
    }
    out.truncate(MAX_STEPS);
    if out.len() < MIN_STEPS {
        out.clear();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tools(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn research_that_built_a_sheet_gets_specific_steps() {
        let got = follow_ups(
            &tools(&["web_search", "spawn_subagents"]),
            &["sheet", "html"],
        );
        assert!((MIN_STEPS..=MAX_STEPS).contains(&got.len()), "{got:?}");
        assert_eq!(got[0].label, "Add more places or dates");
    }

    #[test]
    fn a_plain_answer_gets_none() {
        assert!(follow_ups(&[], &[]).is_empty());
        assert!(follow_ups(&tools(&["vault_search"]), &[]).is_empty());
    }

    #[test]
    fn a_single_reason_is_not_enough_to_show_a_row() {
        assert!(follow_ups(&tools(&["spawn_subagents"]), &[]).is_empty());
    }

    #[test]
    fn a_saved_note_is_not_offered_again() {
        let got = follow_ups(&tools(&["web_search", "vault_write_note"]), &[]);
        assert!(
            got.iter().all(|s| s.label != "Keep this as a note"),
            "{got:?}"
        );
    }
}
