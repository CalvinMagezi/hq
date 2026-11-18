//! Whether a skill's hint appears in a piece of text. Shared by the live hint index and by skill
//! fixtures, which use the same rule on purpose.

/// Check if `hint` appears at a word boundary in `text`.
///
/// Prevents "test" from matching "latest" or "contest".
/// Multi-word hints like "dark mode" are matched as a phrase.
pub(crate) fn word_boundary_match(text: &str, hint: &str) -> bool {
    let mut start = 0;
    let text_bytes = text.as_bytes();
    let hint_len = hint.len();

    while let Some(pos) = text[start..].find(hint) {
        let abs_pos = start + pos;
        let before_ok = abs_pos == 0 || !text_bytes[abs_pos - 1].is_ascii_alphanumeric();
        let after_pos = abs_pos + hint_len;
        let after_ok = after_pos >= text.len() || !text_bytes[after_pos].is_ascii_alphanumeric();

        if before_ok && after_ok {
            return true;
        }
        start = abs_pos + 1;
    }
    false
}
