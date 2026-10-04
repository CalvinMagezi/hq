//! Char-safe truncation shared by every crate that clips text for display or prompts.

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
    fn truncate_output_keeps_a_char_boundary_and_the_note() {
        let output = "€".repeat(30);
        let cut = truncate_output(&output, 50, ". Hint.");
        assert!(cut.starts_with(&"€".repeat(16)));
        assert!(cut.ends_with("[Output truncated at 50KB (90 bytes total). Hint.]"));
        assert_eq!(truncate_output("short", 50, ""), "short");
    }
}
