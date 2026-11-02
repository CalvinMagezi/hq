//! Memory metadata from a chat turn without a model: an extractive summary, the names the turn
//! keeps returning to, tags borrowed from the nearest tagged notes, and a base importance the
//! salience pattern can raise. Chosen with `daemon.memory_extraction: local`.

use hq_db::Database;

use crate::types::ExtractedMemory;

/// A routine turn; the salience boost in the ingester lifts decisions, failures and milestones.
const BASE_IMPORTANCE: f64 = 0.5;
const QUESTION_CHARS: usize = 140;
const ANSWER_CHARS: usize = 240;
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
    let topics = crate::neighbour_tags(db, text, None)
        .await
        .unwrap_or_default();
    ExtractedMemory {
        summary: summarize(text),
        entities: entities(text),
        topics,
        importance: BASE_IMPORTANCE,
    }
}

/// "Asked: <first sentence of the first user message> Answered: <first sentence of the last
/// reply>", from the `role: content` blocks the post-turn hook joins with `---` lines.
fn summarize(text: &str) -> String {
    let mut question = None;
    let mut answer = None;
    for block in text.split("\n---\n") {
        if let Some(body) = block.strip_prefix("user:") {
            question = question.or_else(|| first_sentence(body, QUESTION_CHARS));
        } else if let Some(body) = block.strip_prefix("assistant:") {
            answer = first_sentence(body, ANSWER_CHARS).or(answer);
        }
    }
    match (question, answer) {
        (Some(q), Some(a)) => format!("Asked: {q} Answered: {a}"),
        (None, Some(a)) => a,
        (Some(q), None) => format!("Asked: {q}"),
        (None, None) => first_sentence(text, ANSWER_CHARS).unwrap_or_default(),
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
fn entities(text: &str) -> Vec<String> {
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
    fn the_summary_is_the_question_and_the_first_sentence_of_the_answer() {
        assert_eq!(
            summarize(TURN),
            "Asked: How do I pair the Galaxy A32 with the laptop? Answered: Open the desktop app and scan the QR code with the Galaxy A32."
        );
    }

    #[test]
    fn names_are_kept_and_sentence_filler_is_not() {
        let names = entities(TURN);
        assert!(names.contains(&"Galaxy A32".to_string()), "{names:?}");
        assert!(names.contains(&"Agent HQ".to_string()), "{names:?}");
        assert!(names.contains(&"QR".to_string()), "{names:?}");
        for filler in ["How", "Open", "The", "Pairing"] {
            assert!(
                !names.contains(&filler.to_string()),
                "{filler} in {names:?}"
            );
        }
    }

    #[test]
    fn code_and_list_markers_are_skipped_and_long_sentences_capped() {
        let body = "```\nlet x = 1;\n```\n- Rebuilt the APK and installed it on the phone without losing any data at all, which took a while";
        let s = first_sentence(body, 40).unwrap();
        assert!(s.starts_with("Rebuilt the APK"), "{s}");
        assert!(s.ends_with("..."), "{s}");
    }
}
