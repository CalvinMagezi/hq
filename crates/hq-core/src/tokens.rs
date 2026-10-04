//! Heuristic token counters and token-budget truncation.

/// Estimate token count using the bytes/3.5 heuristic.
///
/// This intentionally over-counts slightly to stay within budget.
#[inline]
pub fn count_tokens_fast(text: &str) -> usize {
    // bytes / 3.5 ≈ (bytes * 2) / 7, rounded up
    (text.len() * 2).div_ceil(7)
}

/// Estimate token count with a character-class weighted heuristic: the
/// session's own counter for compaction thresholds and usage estimates.
///
/// Heavier than [`count_tokens_fast`]. The two must not be mixed in one
/// comparison, and swapping either would shift every threshold tuned on it.
pub fn count_tokens_char_weighted(text: &str) -> usize {
    // Units are twelfths of a token, so each class's rate is an exact integer.
    let mut twelfths = 0usize;
    for ch in text.chars() {
        let cp = ch as u32;
        twelfths += match cp {
            // CJK characters: ~1 token per 1.5 chars
            0x4E00..=0x9FFF | 0x3400..=0x4DBF => 8,
            // ASCII: ~1 token per 3 chars (prose runs nearer 4; code and JSON nearer 3)
            0x0000..=0x007F => 4,
            // Other Unicode: ~1 token per 3 chars
            _ => 4,
        };
    }
    twelfths.div_ceil(12).max(1)
}

/// Truncation strategy for different content types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TruncationStrategy {
    /// Cut at nearest word/line boundary (default, fast)
    Hard,
    /// Cut at nearest markdown section boundary (## or ###)
    SectionBoundary,
    /// Cut at nearest code block boundary (``` fences)
    CodeBlock,
    /// Cut at nearest paragraph boundary (double newline)
    ParagraphBoundary,
    /// Middle-out: keep head + tail, drop middle (best for long outputs)
    MiddleOut,
}

/// Truncate `text` to fit within `max_tokens`, using the given strategy.
///
/// Returns the original string unchanged if it already fits.
pub fn truncate_to_tokens(text: &str, max_tokens: usize) -> String {
    truncate_with_strategy(text, max_tokens, TruncationStrategy::Hard)
}

/// Truncate with a specific strategy.
pub fn truncate_with_strategy(
    text: &str,
    max_tokens: usize,
    strategy: TruncationStrategy,
) -> String {
    if count_tokens_fast(text) <= max_tokens {
        return text.to_string();
    }

    // Approximate byte budget (tokens * 3.5 ≈ tokens * 7 / 2)
    let byte_budget = (max_tokens * 7 / 2).min(text.len());

    // Snap to a valid UTF-8 char boundary to avoid panics on multi-byte text
    let safe_budget = snap_to_char_boundary(text, byte_budget);

    if strategy == TruncationStrategy::MiddleOut {
        return truncate_middle_out(text, max_tokens);
    }

    let cut = match strategy {
        TruncationStrategy::SectionBoundary => find_section_boundary(text, safe_budget)
            .or_else(|| text[..safe_budget].rfind('\n'))
            .or_else(|| text[..safe_budget].rfind(' '))
            .unwrap_or(safe_budget),
        TruncationStrategy::CodeBlock => find_code_block_boundary(text, safe_budget)
            .or_else(|| text[..safe_budget].rfind('\n'))
            .unwrap_or(safe_budget),
        TruncationStrategy::ParagraphBoundary => find_paragraph_boundary(text, safe_budget)
            .or_else(|| text[..safe_budget].rfind('\n'))
            .unwrap_or(safe_budget),
        TruncationStrategy::Hard => text[..safe_budget]
            .rfind('\n')
            .or_else(|| text[..safe_budget].rfind(' '))
            .unwrap_or(safe_budget),
        TruncationStrategy::MiddleOut => unreachable!(),
    };

    let mut result = text[..cut].to_string();
    result.push_str("\n... (truncated)");
    result
}

/// Snap a byte offset to the nearest valid UTF-8 char boundary at or before it.
pub fn snap_to_char_boundary(text: &str, offset: usize) -> usize {
    text.floor_char_boundary(offset)
}

/// Find the last markdown section boundary (line starting with ## or ###)
/// within the byte budget. Returns the byte offset of the start of that line.
fn find_section_boundary(text: &str, byte_budget: usize) -> Option<usize> {
    let safe = snap_to_char_boundary(text, byte_budget);
    let search_area = &text[..safe];
    let mut last_section = None;

    for (i, line) in search_area.lines().enumerate() {
        if i == 0 {
            continue; // Don't cut at the very first line
        }
        let trimmed = line.trim_start();
        let is_marker = trimmed.starts_with("## ") || trimmed.starts_with("### ");
        if is_marker && let Some(offset) = search_area.rfind(line) {
            last_section = Some(offset);
        }
    }

    last_section
}

/// Find the last code block boundary (``` fence) within byte budget.
fn find_code_block_boundary(text: &str, byte_budget: usize) -> Option<usize> {
    let safe = snap_to_char_boundary(text, byte_budget);
    let search_area = &text[..safe];
    // Find the last ``` that starts a line (closing fence)
    let mut last_fence = None;
    for (offset, _) in search_area.match_indices("\n```") {
        last_fence = Some(offset);
    }
    last_fence
}

/// Find the last paragraph boundary (double newline) within byte budget.
fn find_paragraph_boundary(text: &str, byte_budget: usize) -> Option<usize> {
    let safe = snap_to_char_boundary(text, byte_budget);
    let search_area = &text[..safe];
    search_area.rfind("\n\n").map(|pos| pos + 1) // include one newline
}

/// Middle-out truncation: keep head + tail, replace middle with a gap marker.
/// Preserves context from the start (setup) and end (most recent) of text.
fn truncate_middle_out(text: &str, max_tokens: usize) -> String {
    let total_tokens = count_tokens_fast(text);
    if total_tokens <= max_tokens {
        return text.to_string();
    }

    // Allocate 60% to head, 40% to tail (head has more setup context)
    let head_budget = (max_tokens * 7 / 2) * 6 / 10; // bytes
    let tail_budget = (max_tokens * 7 / 2) * 4 / 10;

    let head_end = snap_to_char_boundary(text, head_budget.min(text.len()));
    let head_cut = text[..head_end].rfind('\n').unwrap_or(head_end);

    let tail_start_approx = text.len().saturating_sub(tail_budget);
    let tail_start = snap_to_char_boundary(text, tail_start_approx);
    let tail_cut = text[tail_start..]
        .find('\n')
        .map(|p| tail_start + p + 1)
        .unwrap_or(tail_start);

    if tail_cut <= head_cut {
        // Overlap: just do hard truncation
        return truncate_to_tokens(text, max_tokens);
    }

    let dropped_tokens = count_tokens_fast(&text[head_cut..tail_cut]);
    format!(
        "{}\n\n... ({dropped_tokens} tokens omitted) ...\n\n{}",
        &text[..head_cut],
        &text[tail_cut..]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_string() {
        assert_eq!(count_tokens_fast(""), 0);
    }

    // Pins the weights the session's compaction thresholds were tuned on.
    #[test]
    fn char_weighted_keeps_its_weights() {
        assert_eq!(count_tokens_char_weighted(""), 1);
        // ASCII counts ~3 chars per token; this used to count 1 token per char.
        assert_eq!(count_tokens_char_weighted("abc"), 1);
        assert_eq!(count_tokens_char_weighted(&"a".repeat(300)), 100);
        assert_eq!(count_tokens_char_weighted("\u{4e2d}\u{6587}\u{4e2d}"), 2);
        assert_eq!(count_tokens_char_weighted("\u{e9}\u{e9}\u{e9}"), 1);
    }

    #[test]
    fn short_text() {
        // 11 bytes -> (22 + 6) / 7 = 4
        assert_eq!(count_tokens_fast("hello world"), 4);
    }

    #[test]
    fn truncate_noop_when_fits() {
        let text = "short text";
        assert_eq!(truncate_to_tokens(text, 100), text);
    }

    #[test]
    fn truncate_snaps_to_boundary() {
        let text = "word1 word2 word3 word4 word5 word6 word7 word8";
        let result = truncate_to_tokens(text, 5);
        assert!(result.ends_with("... (truncated)"));
        assert!(result.len() < text.len() + 20);
    }

    #[test]
    fn section_boundary_truncation() {
        let text = "## Section 1\n\nContent for section 1 is here.\n\n## Section 2\n\nContent for section 2 is here.\n\n## Section 3\n\nContent for section 3.";
        // Budget enough for ~2 sections but not 3
        let result = truncate_with_strategy(text, 20, TruncationStrategy::SectionBoundary);
        assert!(result.contains("Section 1"));
        assert!(result.ends_with("... (truncated)"));
    }
}
