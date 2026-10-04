//! The drive gate: HQ steers a harness session only while it can state what
//! the session is for and how anyone could tell it is finished. The checks are
//! deterministic on purpose, so the same text always gets the same answer.

const MIN_GOAL_CHARS: usize = 20;
const MIN_GOAL_WORDS: usize = 4;
const MIN_CRITERIA_CHARS: usize = 15;
const MIN_CRITERIA_WORDS: usize = 3;

/// Whole values that say nothing, compared after trimming and lowercasing.
const PLACEHOLDERS: &[&str] = &[
    "tbd",
    "todo",
    "n/a",
    "na",
    "none",
    "null",
    "unknown",
    "test",
    "testing",
    "asdf",
    "xxx",
    "...",
    "fix it",
    "fix this",
    "do it",
    "do the task",
    "do the work",
    "work on it",
    "work on this",
    "when done",
    "when it is done",
    "when it's done",
    "it works",
    "looks good",
    "as needed",
    "finish the task",
    "complete the task",
    "the task",
    "the goal",
    "done",
];

fn normalized(text: &str) -> String {
    text.trim()
        .trim_end_matches(|c: char| c == '.' || c.is_whitespace())
        .to_lowercase()
}

fn is_placeholder(text: &str) -> bool {
    let n = normalized(text);
    n.is_empty() || PLACEHOLDERS.contains(&n.as_str())
}

fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

fn field_gaps(name: &str, text: &str, min_chars: usize, min_words: usize, out: &mut Vec<String>) {
    if text.trim().is_empty() {
        out.push(format!("{name} is missing"));
    } else if is_placeholder(text) {
        out.push(format!(
            "{name} is a placeholder, not a statement anyone could check"
        ));
    } else if text.trim().chars().count() < min_chars || word_count(text) < min_words {
        out.push(format!(
            "{name} is too short to judge completion against (need at least {min_words} words and {min_chars} characters)"
        ));
    }
}

/// Why this goal and definition of done cannot yet license driving. Empty means the gate passes.
pub fn gaps(goal: Option<&str>, done_criteria: Option<&str>) -> Vec<String> {
    let (goal, criteria) = (goal.unwrap_or_default(), done_criteria.unwrap_or_default());
    let mut out = Vec::new();
    field_gaps("goal", goal, MIN_GOAL_CHARS, MIN_GOAL_WORDS, &mut out);
    field_gaps(
        "definition of done",
        criteria,
        MIN_CRITERIA_CHARS,
        MIN_CRITERIA_WORDS,
        &mut out,
    );
    let same = !goal.trim().is_empty() && normalized(goal) == normalized(criteria);
    if same {
        out.push(
            "definition of done only repeats the goal; say what would be observable when it is met"
                .into(),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOAL: &str = "Add rate limiting to the login endpoint";
    const DONE: &str = "Login returns 429 after 5 failed attempts and cargo test passes";

    #[test]
    fn a_specific_goal_and_definition_of_done_pass() {
        assert!(gaps(Some(GOAL), Some(DONE)).is_empty());
    }

    #[test]
    fn missing_fields_are_named() {
        let g = gaps(None, Some("  "));
        assert!(g.iter().any(|m| m.starts_with("goal is missing")), "{g:?}");
        assert!(
            g.iter()
                .any(|m| m.starts_with("definition of done is missing")),
            "{g:?}"
        );
    }

    #[test]
    fn placeholders_and_thin_text_are_ambiguous() {
        assert!(!gaps(Some("TBD"), Some(DONE)).is_empty());
        assert!(!gaps(Some(GOAL), Some("When done.")).is_empty());
        assert!(
            !gaps(Some("fix the bug"), Some(DONE)).is_empty(),
            "too short"
        );
        assert!(
            !gaps(Some(GOAL), Some("it works fine")).is_empty(),
            "too short"
        );
    }

    #[test]
    fn a_definition_of_done_that_repeats_the_goal_is_refused() {
        let g = gaps(Some(GOAL), Some("add rate limiting to the login endpoint."));
        assert!(
            g.iter().any(|m| m.contains("only repeats the goal")),
            "{g:?}"
        );
    }
}
