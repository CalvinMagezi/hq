//! Memory metadata from a chat turn without a model: an extractive summary of what the owner
//! said, the names they used, tags borrowed from the nearest tagged notes, and a base importance
//! the salience pattern can raise. Chosen with `daemon.memory_extraction: local`.
//!
//! Only the owner's words are kept. What the assistant answered is its own claim, and a memory
//! that repeats it would be recalled later as something the owner said.

use hq_db::Database;

use crate::types::ExtractedMemory;

/// A routine turn; the salience boost in the ingester lifts decisions, failures and milestones.
const BASE_IMPORTANCE: f64 = 0.5;
const OWNER_CHARS: usize = 240;
const MAX_ENTITIES: usize = 10;
/// Capitalised words that start sentences or fill speech, never names on their own.
const NOT_NAMES: &[&str] = &[
    "I",
    "A",
    "An",
    "The",
    "This",
    "That",
    "These",
    "Those",
    "It",
    "Its",
    "If",
    "You",
    "Your",
    "We",
    "Our",
    "They",
    "He",
    "She",
    "Yes",
    "No",
    "OK",
    "Ok",
    "Also",
    "And",
    "But",
    "Or",
    "For",
    "Now",
    "Then",
    "So",
    "Here",
    "There",
    "What",
    "When",
    "Where",
    "Why",
    "How",
    "Which",
    "Who",
    "Note",
    "Done",
    "Sure",
    "Thanks",
    "Please",
    "Let",
    "Next",
    "First",
    "Second",
    "Finally",
    "Today",
    "Tomorrow",
    "Yesterday",
    "In",
    "On",
    "At",
    "To",
    "Of",
    "With",
    "By",
    "As",
    "Is",
    "Are",
    "Was",
    "Do",
    "Does",
    "Did",
    "Can",
    "Could",
    "Should",
    "Would",
    "Will",
    "My",
    "Me",
];

pub async fn extract_local(db: &Database, text: &str) -> ExtractedMemory {
    let owner = owner_text(text);
    let topics = crate::neighbour_tags(db, &owner, None)
        .await
        .unwrap_or_default();
    ExtractedMemory {
        summary: summarize(&owner),
        entities: entities(&owner),
        topics,
        importance: BASE_IMPORTANCE,
    }
}

/// What the owner wrote in a turn made of `role: content` blocks joined by `---` lines. Text with
/// no role blocks at all is taken to be the owner's own. A turn with only an assistant block has
/// nothing of theirs, so it gives nothing.
pub(crate) fn owner_text(text: &str) -> String {
    let blocks: Vec<&str> = text.split("\n---\n").collect();
    let is_turn = blocks
        .iter()
        .any(|b| b.starts_with("user:") || b.starts_with("assistant:"));
    if !is_turn {
        return text.to_string();
    }
    blocks
        .iter()
        .filter_map(|b| b.strip_prefix("user:"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The first sentence the owner wrote, marked "Asked:" when it is a question.
fn summarize(owner: &str) -> String {
    match first_sentence(owner, OWNER_CHARS) {
        Some(s) if s.trim_end_matches("...").ends_with('?') => format!("Asked: {s}"),
        Some(s) => s,
        None => String::new(),
    }
}

/// The first sentence of prose in `body`, skipping headings, list markers and code.
fn first_sentence(body: &str, max_chars: usize) -> Option<String> {
    let mut in_code = false;
    for line in body.lines() {
        let line = line.trim();
        if line.starts_with("```") {
            in_code = !in_code;
            continue;
        }
        let prose = line.trim_start_matches(['-', '*', '>', ' ', '|']).trim();
        if in_code || line.starts_with('#') || prose.len() < 3 {
            continue;
        }
        let end = prose
            .char_indices()
            .find(|&(i, c)| {
                matches!(c, '.' | '?' | '!') && prose[i + c.len_utf8()..].starts_with(' ')
            })
            .map_or(prose.len(), |(i, c)| i + c.len_utf8());
        let sentence = &prose[..end];
        return Some(cap_chars(sentence, max_chars));
    }
    None
}

fn cap_chars(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}...", s[..i].trim_end()),
        None => s.to_string(),
    }
}

/// Names the turn mentions: runs of capitalised words, CamelCase words and short acronyms that
/// are not sentence filler. A single plain capitalised word counts only if it recurs.
// ponytail: surface heuristic with no dictionary; good enough to seed concept pages, upgrade to the
// entity graph's known names when its recall matters.
pub(crate) fn entities(text: &str) -> Vec<String> {
    let mut found: Vec<(String, usize, bool)> = Vec::new();
    let mut run: Vec<&str> = Vec::new();
    let flush = |run: &mut Vec<&str>, found: &mut Vec<(String, usize, bool)>| {
        if run.is_empty() {
            return;
        }
        let name = run.join(" ");
        let strong = run.len() > 1 || run.iter().any(|w| is_camel_or_acronym(w));
        match found.iter_mut().find(|(n, _, _)| *n == name) {
            Some(entry) => entry.1 += 1,
            None => found.push((name, 1, strong)),
        }
        run.clear();
    };
    for raw in text.split_whitespace() {
        let word = raw.trim_matches(|c: char| !c.is_alphanumeric() && c != '-');
        let ends_clause = raw.ends_with([',', '.', ':', ';', '?', '!', ')']);
        if is_name_word(word) {
            run.push(word);
        } else {
            flush(&mut run, &mut found);
        }
        if ends_clause {
            flush(&mut run, &mut found);
        }
    }
    flush(&mut run, &mut found);
    found.retain(|(_, count, strong)| *strong || *count > 1);
    found.sort_by_key(|f| std::cmp::Reverse(f.1));
    found
        .into_iter()
        .take(MAX_ENTITIES)
        .map(|(name, _, _)| name)
        .collect()
}

fn is_name_word(word: &str) -> bool {
    let mut chars = word.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_uppercase() && word.chars().count() >= 2 && !NOT_NAMES.contains(&word)
}

fn is_camel_or_acronym(word: &str) -> bool {
    let upper = word.chars().filter(|c| c.is_uppercase()).count();
    let letters = word.chars().filter(|c| c.is_alphabetic()).count();
    upper >= 2 && (upper < letters || letters <= 6)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TURN: &str = "user: How do I pair the Galaxy A32 with the laptop?\n---\nassistant: ## Pairing\n\nOpen the desktop app and scan the QR code with the Galaxy A32. The bridge then reconnects by key over iroh. Agent HQ keeps the paired list.";

    #[test]
    fn the_summary_is_what_the_owner_asked_and_none_of_the_answer() {
        let owner = owner_text(TURN);
        assert_eq!(
            summarize(&owner),
            "Asked: How do I pair the Galaxy A32 with the laptop?"
        );
        assert!(
            !summarize(&owner).contains("desktop app"),
            "the answer is not the owner's"
        );
    }

    #[test]
    fn a_statement_is_kept_as_the_owner_said_it() {
        let turn = "user: Acme Feeds delivers on Thursdays now. Please keep that in mind.\n---\nassistant: Noted, I will remember Fridays.";
        assert_eq!(
            summarize(&owner_text(turn)),
            "Acme Feeds delivers on Thursdays now."
        );
    }

    #[test]
    fn names_come_from_the_owner_and_not_from_the_assistant() {
        let names = entities(&owner_text(TURN));
        assert!(names.contains(&"Galaxy A32".to_string()), "{names:?}");
        // These appear only in the assistant's answer.
        for answer_only in ["Agent HQ", "QR", "Pairing"] {
            assert!(
                !names.contains(&answer_only.to_string()),
                "{answer_only} in {names:?}"
            );
        }
        for filler in ["How", "Open", "The"] {
            assert!(
                !names.contains(&filler.to_string()),
                "{filler} in {names:?}"
            );
        }
    }

    #[test]
    fn a_turn_with_nothing_from_the_owner_gives_no_memory() {
        let turn = "assistant: The release installed and the data saver stays on.";
        assert_eq!(owner_text(turn), "");
        assert_eq!(summarize(&owner_text(turn)), "");
    }

    #[test]
    fn plain_text_with_no_roles_is_taken_as_the_owners() {
        let note = "Decided to stop buying packaging from Bright Boxes because of late deliveries.";
        assert_eq!(owner_text(note), note);
        assert_eq!(summarize(note), note);
    }

    #[test]
    fn code_and_list_markers_are_skipped_and_long_sentences_capped() {
        let body = "```\nlet x = 1;\n```\n- Rebuilt the APK and installed it on the phone without losing any data at all, which took a while";
        let s = first_sentence(body, 40).unwrap();
        assert!(s.starts_with("Rebuilt the APK"), "{s}");
        assert!(s.ends_with("..."), "{s}");
    }
}
