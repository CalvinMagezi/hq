//! Char-safe truncation shared by every crate that clips text for display or prompts, and the
//! cleaning of short labels that are shown to people and agents.

/// The first `max` characters of `s`, borrowed. Never splits a UTF-8 character.
pub fn truncate_chars(s: &str, max: usize) -> &str {
    match s.char_indices().nth(max) {
        Some((cut, _)) => &s[..cut],
        None => s,
    }
}

/// The first `max` characters of `s`, followed by `suffix` only when something was cut.
pub fn truncate_chars_with(s: &str, max: usize, suffix: &str) -> String {
    let kept = truncate_chars(s, max);
    if kept.len() == s.len() {
        return s.to_string();
    }
    format!("{kept}{suffix}")
}

/// At most `max` characters of `s` in all: whole when it fits, else cut one short and ended
/// with an ellipsis.
pub fn truncate_chars_ellipsis(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    format!("{}…", truncate_chars(s, max.saturating_sub(1)))
}

/// Cut tool output to `max_bytes` on a char boundary and append the truncation note.
///
/// `hint` lands inside the note's brackets, so a caller can add guidance.
pub fn truncate_output(output: &str, max_bytes: usize, hint: &str) -> String {
    if output.len() <= max_bytes {
        return output.to_string();
    }
    let kept = &output[..output.floor_char_boundary(max_bytes)];
    format!(
        "{kept}\n\n[Output truncated at 50KB ({} bytes total){hint}]",
        output.len()
    )
}

/// Longest label (actor, harness, host, cwd, branch, session ref) kept.
const MAX_LABEL_CHARS: usize = 120;

/// A label with control characters dropped and its length capped. Lease text is
/// shown to other agents and written to the task thread, which is read back as
/// trusted input, so it is kept short and single-line.
pub fn clean_label(text: &str) -> String {
    let kept: String = text
        .chars()
        .filter(|c| !c.is_control() && !is_invisible(*c))
        .take(MAX_LABEL_CHARS)
        .collect();
    kept.trim().to_string()
}

/// Whether `text` has a control or invisible character, which could make shown text read as
/// something else.
pub fn has_hidden_chars(text: &str) -> bool {
    text.chars().any(|c| c.is_control() || is_invisible(c))
}

/// Zero-width, bidirectional-override and byte-order characters: they print as
/// nothing or reorder what is around them, so a label could look like another.
pub fn is_invisible(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{115F}'..='\u{1160}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{2028}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{2069}'
            | '\u{3164}'
            | '\u{FE00}'..='\u{FE0F}'
            | '\u{FEFF}'
            | '\u{FFA0}'
            | '\u{E0000}'..='\u{E007F}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_chars_counts_characters_not_bytes() {
        assert_eq!(truncate_chars("héllo wörld", 5), "héllo");
        assert_eq!(truncate_chars("中文中文", 3), "中文中");
        assert_eq!(truncate_chars("🦀🦀", 1), "🦀");
    }

    #[test]
    fn truncate_chars_returns_short_text_whole() {
        assert_eq!(truncate_chars("héllo", 5), "héllo");
        assert_eq!(truncate_chars("héllo", 50), "héllo");
        assert_eq!(truncate_chars("", 3), "");
        assert_eq!(truncate_chars("abc", 0), "");
    }

    #[test]
    fn truncate_chars_with_only_suffixes_when_cut() {
        assert_eq!(truncate_chars_with("héllo", 5, "..."), "héllo");
        assert_eq!(truncate_chars_with("héllo wörld", 7, "…"), "héllo w…");
    }

    #[test]
    fn truncate_chars_ellipsis_never_exceeds_max() {
        assert_eq!(truncate_chars_ellipsis("héllo", 10), "héllo");
        assert_eq!(truncate_chars_ellipsis("héllo", 5), "héllo");
        assert_eq!(truncate_chars_ellipsis("héllo wörld", 5), "héll…");
        assert_eq!(truncate_chars_ellipsis("abc", 0), "…");
    }

    #[test]
    fn truncate_output_keeps_a_char_boundary_and_the_note() {
        let output = "€".repeat(30);
        let cut = truncate_output(&output, 50, ". Hint.");
        assert!(cut.starts_with(&"€".repeat(16)));
        assert!(cut.ends_with("[Output truncated at 50KB (90 bytes total). Hint.]"));
        assert_eq!(truncate_output("short", 50, ""), "short");
    }

    #[test]
    fn clean_label_drops_what_cannot_be_seen_and_caps_the_rest() {
        assert_eq!(clean_label("  relay\u{200B}\n\tbot  "), "relaybot");
        assert_eq!(clean_label("\u{202E}evil\u{FEFF}"), "evil");
        assert_eq!(clean_label(&"x".repeat(500)).chars().count(), 120);
        assert_eq!(clean_label("héllo wörld"), "héllo wörld");
    }

    #[test]
    fn has_hidden_chars_sees_control_and_invisible_characters() {
        assert!(has_hidden_chars("a\u{200D}b"));
        assert!(has_hidden_chars("a\nb"));
        assert!(!has_hidden_chars("plain text"));
        assert!(is_invisible('\u{00AD}'));
        assert!(!is_invisible('a'));
    }
}
