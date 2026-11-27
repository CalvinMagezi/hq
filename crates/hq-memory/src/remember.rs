//! "Remember that ...": the owner asking HQ to keep one thing.
//!
//! Read from the owner's own words only, never from an assistant block or a tool result, so
//! nothing a model writes can make HQ keep a sentence for good. A reminder ("remember to call
//! Sam") is not a fact and is left to the tasks and reminders features.

const OPENERS: &[&str] = &[
    "please remember that ",
    "please remember: ",
    "remember that ",
    "remember this: ",
    "remember: ",
    "don't forget that ",
    "dont forget that ",
    "keep in mind that ",
];

/// What the shortest useful request must say.
const MIN_CHARS: usize = 8;
const MAX_CHARS: usize = 300;

/// The sentence the owner asked HQ to remember, if one of their lines opens that way.
pub fn explicit_request(owner_text: &str) -> Option<String> {
    for line in owner_text.lines() {
        let line = line.trim();
        let lower = line.to_lowercase();
        for opener in OPENERS {
            if lower.starts_with(opener) {
                // The openers are ASCII, so their length is the same in the original line.
                let rest = line.get(opener.len()..)?.trim();
                let rest: String = rest.chars().take(MAX_CHARS).collect();
                let rest = rest.trim().trim_end_matches(['.', '!']).trim().to_string();
                if rest.chars().count() >= MIN_CHARS {
                    return Some(rest);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sentence_after_the_opener_is_the_request() {
        assert_eq!(
            explicit_request("Remember that Acme Feeds delivers on Thursdays.").as_deref(),
            Some("Acme Feeds delivers on Thursdays")
        );
        assert_eq!(
            explicit_request("hi\nPlease remember: the shop closes at 6pm on Saturdays").as_deref(),
            Some("the shop closes at 6pm on Saturdays")
        );
    }

    #[test]
    fn a_reminder_or_a_question_is_not_a_request() {
        assert_eq!(explicit_request("Remember to call Sam tomorrow"), None);
        assert_eq!(explicit_request("Do you remember that day?"), None);
        assert_eq!(explicit_request("remember that"), None);
    }

    #[test]
    fn it_only_reads_the_start_of_a_line() {
        assert_eq!(
            explicit_request("The doc says remember that nothing is final here"),
            None
        );
    }

    #[test]
    fn a_long_request_is_cut() {
        let long = format!("Remember that {}", "x".repeat(500));
        assert_eq!(explicit_request(&long).unwrap().chars().count(), MAX_CHARS);
    }
}
