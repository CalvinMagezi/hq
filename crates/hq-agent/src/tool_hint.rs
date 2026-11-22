//! A per-turn "tools you may want" line, worked out on the device.
//!
//! When a message looks like it needs a tool that is not loaded yet, one short line names it, so
//! the model asks `tool_search` for it instead of guessing or doing the job by hand. The ranking
//! is the shared offline scorer in `hq_tool_api::tool_rank`: no model, no network.
//!
//! Inputs are the person's message and the catalogue of tools not loaded yet, nothing else. It
//! reads no history, no usage and no adoption data, so what the model sees can never depend on
//! what the person has or has not used. The line is added to the per-turn context note (the one
//! that goes just before the message), never to the system prompt or the tool list, so the cached
//! prefix is not touched.

use hq_tool_api::tool_rank::{RankDoc, rank_scored};

/// Most tools named in one line.
pub(crate) const MAX_HINTED: usize = 3;

/// Weakest top score that earns a line at all. A word found in the name or hint of a tool few
/// others mention clears it; a common word that only some hint happens to contain does not.
const MIN_TOP_SCORE: f32 = 5.0;

/// A further tool is named only if it scores at least this share of the best one.
const RELATIVE_FLOOR: f32 = 0.6;

/// Only the start of a long message is read. A pasted document would otherwise match whatever
/// tool shares a few words with it.
const MESSAGE_CHARS: usize = 400;

/// Verbs that say little about which tool is wanted ("write me a poem" is not a note-writing
/// request). They are left out of the message before it is ranked for a hint. An explicit
/// `tool_search` still matches them, since someone typing "write note" there means it.
const GENERIC_WORDS: &[&str] = &[
    "write", "create", "show", "list", "give", "tell", "find", "add", "new", "open", "read",
    "check", "look", "please", "thanks", "thank", "say", "like", "just", "some", "about",
    "tonight", "today", "tomorrow", "week", "month", "year", "short", "long", "big", "small",
];

/// The message with its generic words removed.
fn meaningful_words(message: &str) -> String {
    message
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty() && !GENERIC_WORDS.contains(w))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The deferred tools worth naming for `message`, best first, at most [`MAX_HINTED`]. Empty when
/// nothing clearly fits, which is most turns.
pub(crate) fn likely_tools(message: &str, deferred: &[(String, String)]) -> Vec<String> {
    if deferred.is_empty() {
        return Vec::new();
    }
    let text = meaningful_words(&message.chars().take(MESSAGE_CHARS).collect::<String>());
    let docs: Vec<RankDoc<'_>> = deferred
        .iter()
        .map(|(name, hint)| RankDoc {
            name,
            hint,
            description: "",
            keywords: &[],
        })
        .collect();
    let scored = rank_scored(&docs, &text, MAX_HINTED);
    let Some(&(_, top)) = scored.first() else {
        return Vec::new();
    };
    if top < MIN_TOP_SCORE {
        return Vec::new();
    }
    scored
        .into_iter()
        .filter(|(_, score)| *score >= top * RELATIVE_FLOOR)
        .map(|(i, _)| deferred[i].0.clone())
        .collect()
}

/// The line for the turn context, or `None` when no tool clearly fits.
pub(crate) fn hint_line(message: &str, deferred: &[(String, String)]) -> Option<String> {
    let names = likely_tools(message, deferred);
    (!names.is_empty()).then(|| {
        format!(
            "Tools that may fit this message (not loaded yet; call tool_search to load): {}",
            names.join(", ")
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> Vec<(String, String)> {
        [
            ("office_create", "Make a spreadsheet, document or dashboard"),
            ("office_export", "Export an office file as PDF or Word"),
            ("task_create", "Add a task or reminder"),
            ("task_list", "List your tasks"),
            ("calendar_event_create", "Add an event to the calendar"),
            ("calendar_list", "Show what is on the calendar"),
            ("web_fetch", "Fetch a web page"),
            ("web_search", "Search the web"),
            ("vault_search", "Search notes in the vault"),
            ("vault_write_note", "Save a note to the vault"),
            ("image_generate", "Make an image from a description"),
            ("email_send", "Send an email"),
            ("email_search", "Search your email"),
            ("convert_document", "Convert a document between formats"),
            ("shrink_document", "Make a document smaller"),
            ("harness_session", "Run a coding agent session"),
            ("github_pr", "Open or read a pull request"),
            ("people_find", "Look up a person in your contacts"),
            ("project_status", "Show the status of a project"),
            ("share_file", "Share a file with someone by link"),
        ]
        .iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect()
    }

    fn names(message: &str) -> Vec<String> {
        likely_tools(message, &catalog())
    }

    #[test]
    fn a_message_that_needs_a_tool_names_it_first() {
        assert_eq!(
            names("make me a spreadsheet of this month's expenses")[0],
            "office_create"
        );
        assert_eq!(names("remind me to call mum tomorrow")[0], "task_create");
        assert_eq!(
            names("send an email to the supplier about the invoice")[0],
            "email_send"
        );
        assert_eq!(
            names("what is on my calendar this week")[0],
            "calendar_event_create"
        );
        assert_eq!(names("share the report with Anna")[0], "share_file");
        assert_eq!(names("give me a picture of a sunset")[0], "image_generate");
        assert_eq!(
            names("make this document smaller so I can email it")[0],
            "shrink_document"
        );
    }

    #[test]
    fn ordinary_talk_names_nothing() {
        for message in [
            "thanks!",
            "how are you today",
            "can you explain how photosynthesis works",
            "write me a short poem about the sea",
            "summarise what we discussed earlier",
            "tell me a joke",
            "what's 15% of 2400",
            "help me write a cover letter",
            "",
        ] {
            assert!(names(message).is_empty(), "{message}: {:?}", names(message));
        }
    }

    #[test]
    fn at_most_three_are_named_and_a_weak_second_is_left_out() {
        assert!(names("convert this pdf to word").len() <= MAX_HINTED);
        let got = names("what is the status of the website project");
        assert_eq!(got, ["project_status"], "web_fetch only shares a word");
    }

    #[test]
    fn a_long_pasted_text_is_read_only_at_its_start() {
        let filler = "lorem ipsum dolor sit amet ".repeat(40);
        let message = format!("{filler} please send an email now");
        assert!(names(&message).is_empty(), "the request is past the cut");
    }

    #[test]
    fn the_line_is_built_from_the_message_and_the_catalogue_alone() {
        let line = hint_line("remind me to call mum tomorrow", &catalog()).unwrap();
        assert!(line.contains("tool_search"), "{line}");
        assert!(line.ends_with("task_create"), "{line}");
        assert_eq!(hint_line("thanks!", &catalog()), None);
        assert_eq!(
            hint_line("send an email", &[]),
            None,
            "nothing deferred, nothing to hint"
        );
        // Same inputs, same line: nothing else is read.
        assert_eq!(
            hint_line("send an email to Anna", &catalog()),
            hint_line("send an email to Anna", &catalog())
        );
    }
}
