//! The slices of a screen a rule looks at. A region name picks part of the
//! screen text (or the terminal title) so a rule can say "this phrase in the
//! last twelve lines" or "below the last horizontal rule".

/// What detection sees of a pane.
#[derive(Debug, Clone, Copy)]
pub struct Input<'a> {
    /// The screen as text, one line per row, trailing blank rows removed.
    pub screen: &'a str,
    /// The terminal title set by the program (OSC 0 or 2), or empty.
    pub osc_title: &'a str,
}

/// The text of `spec` within `input`. An unknown region is empty text, so a
/// rule that names one never matches.
pub(super) fn region<'a>(input: Input<'a>, spec: &str) -> &'a str {
    let spec = spec.trim();
    if spec == "osc_title" {
        return input.osc_title;
    }
    let content = input.screen;
    match spec {
        "whole_recent" => content,
        "after_last_prompt_marker" => after_last_prompt_marker(content),
        "before_current_prompt_marker" => before_current_prompt_marker(content),
        "whole_recent_without_current_prompt_marker" => {
            if current_prompt_index(&lines_of(content)).is_some() {
                ""
            } else {
                content
            }
        }
        "current_prompt_block_marker" => current_prompt_block_marker(content).unwrap_or(""),
        "after_current_prompt_block_marker" => {
            after_current_prompt_block_marker(content).unwrap_or("")
        }
        "prompt_box_body" => prompt_box_body(content).unwrap_or(""),
        "above_prompt_box" => above_prompt_box(content),
        "last_non_empty_above_prompt_box" => last_non_empty_line(above_prompt_box(content)),
        "after_last_horizontal_rule" => after_last_horizontal_rule(content),
        other => counted(content, other).unwrap_or(""),
    }
}

fn lines_of(content: &str) -> Vec<&str> {
    content.lines().collect()
}

/// `bottom_lines(N)`, `bottom_non_empty_lines(N)` and `top_non_empty_lines(N)`.
fn counted<'a>(content: &'a str, spec: &str) -> Option<&'a str> {
    let (name, count) = spec.strip_suffix(')')?.split_once('(')?;
    let count: usize = count.parse().ok()?;
    match name {
        "bottom_lines" => Some(bottom_lines(content, count)),
        "bottom_non_empty_lines" => Some(bottom_non_empty_lines(content, count)),
        "top_non_empty_lines" => Some(top_non_empty_lines(content, count)),
        _ => None,
    }
}

/// Byte offset where line `index` starts (lines are separated by one `\n`).
fn line_start(content: &str, lines: &[&str], index: usize) -> usize {
    let end = index.min(lines.len());
    let offset: usize = lines[..end].iter().map(|l| l.len() + 1).sum();
    offset.min(content.len())
}

fn from_line<'a>(content: &'a str, lines: &[&str], index: usize) -> &'a str {
    &content[line_start(content, lines, index)..]
}

fn bottom_lines(content: &str, count: usize) -> &str {
    let lines = lines_of(content);
    from_line(content, &lines, lines.len().saturating_sub(count))
}

fn bottom_non_empty_lines(content: &str, count: usize) -> &str {
    let lines = lines_of(content);
    let first = lines
        .iter()
        .enumerate()
        .rev()
        .filter(|(_, l)| !l.trim().is_empty())
        .take(count)
        .last()
        .map(|(i, _)| i);
    match first {
        Some(i) => from_line(content, &lines, i),
        None => "",
    }
}

fn top_non_empty_lines(content: &str, count: usize) -> &str {
    let lines = lines_of(content);
    let last = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
        .take(count)
        .last()
        .map(|(i, _)| i);
    match last {
        Some(i) => &content[..line_start(content, &lines, i + 1)],
        None => "",
    }
}

fn last_non_empty_line(content: &str) -> &str {
    content
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
}

/// A line of box-drawing dashes, alone or followed by a label.
fn is_horizontal_rule(line: &str) -> bool {
    let trimmed = line.trim();
    let dashes = trimmed.chars().take_while(|&c| c == '─').count();
    if dashes == 0 {
        return false;
    }
    let rest: String = trimmed.chars().skip(dashes).collect();
    rest.trim_start().is_empty() || dashes >= 3
}

fn after_last_horizontal_rule(content: &str) -> &str {
    let lines = lines_of(content);
    match lines.iter().rposition(|l| is_horizontal_rule(l)) {
        Some(i) => from_line(content, &lines, i + 1),
        None => content,
    }
}

/// Index of the upper border of the input box: the second horizontal rule
/// counting from the bottom.
fn prompt_box_top(lines: &[&str]) -> Option<usize> {
    lines
        .iter()
        .enumerate()
        .rev()
        .filter(|(_, l)| is_horizontal_rule(l))
        .nth(1)
        .map(|(i, _)| i)
}

fn above_prompt_box(content: &str) -> &str {
    let lines = lines_of(content);
    match prompt_box_top(&lines) {
        Some(top) => &content[..line_start(content, &lines, top)],
        None => content,
    }
}

fn prompt_box_body(content: &str) -> Option<&str> {
    let lines = lines_of(content);
    let top = prompt_box_top(&lines)?;
    let start = line_start(content, &lines, top + 1);
    let end_line = lines[top + 1..]
        .iter()
        .position(|l| is_horizontal_rule(l))
        .map_or(lines.len(), |rel| top + 1 + rel);
    Some(&content[start..line_start(content, &lines, end_line)])
}

fn is_prompt_line(line: &str) -> bool {
    line == "›" || line.starts_with("› ")
}

fn is_block_marker(line: &str) -> bool {
    line.starts_with(['•', '■', '✗', '✓'])
}

fn after_last_prompt_marker(content: &str) -> &str {
    let lines = lines_of(content);
    match lines.iter().rposition(|l| is_prompt_line(l)) {
        Some(i) => from_line(content, &lines, i + 1),
        None => content,
    }
}

/// The last prompt line, unless a response block starts after it (then that
/// prompt is stale and there is no current one).
fn current_prompt_index(lines: &[&str]) -> Option<usize> {
    let index = lines.iter().rposition(|l| is_prompt_line(l))?;
    if lines[index + 1..].iter().any(|l| is_block_marker(l)) {
        return None;
    }
    Some(index)
}

fn before_current_prompt_marker(content: &str) -> &str {
    let lines = lines_of(content);
    match current_prompt_index(&lines) {
        Some(i) => &content[..line_start(content, &lines, i)],
        None => content,
    }
}

fn current_prompt_block_marker(content: &str) -> Option<&str> {
    let lines = lines_of(content);
    let prompt = current_prompt_index(&lines)?;
    lines[..prompt]
        .iter()
        .rev()
        .find(|l| is_block_marker(l))
        .copied()
}

fn after_current_prompt_block_marker(content: &str) -> Option<&str> {
    let lines = lines_of(content);
    let prompt = current_prompt_index(&lines)?;
    let block = lines[..prompt].iter().rposition(|l| is_block_marker(l))?;
    Some(from_line(content, &lines, block))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at<'a>(screen: &'a str, spec: &str) -> &'a str {
        region(
            Input {
                screen,
                osc_title: "T",
            },
            spec,
        )
    }

    #[test]
    fn counted_regions_pick_lines() {
        let s = "a\n\nb\nc\n\nd";
        assert_eq!(at(s, "bottom_lines(2)"), "\nd");
        assert_eq!(at(s, "bottom_non_empty_lines(2)"), "c\n\nd");
        assert_eq!(at(s, "top_non_empty_lines(2)"), "a\n\nb\n");
        assert_eq!(at(s, "bottom_non_empty_lines(0)"), "");
        assert_eq!(at("", "bottom_non_empty_lines(3)"), "");
    }

    #[test]
    fn title_and_unknown_regions() {
        assert_eq!(at("x", "osc_title"), "T");
        assert_eq!(at("x", "no_such_region"), "");
        assert_eq!(at("x", "whole_recent"), "x");
    }

    #[test]
    fn rules_and_prompt_boxes() {
        let s = "log\n──────\n❯ hi\n──────\nstatus";
        assert_eq!(at(s, "after_last_horizontal_rule"), "status");
        assert_eq!(at(s, "above_prompt_box"), "log\n");
        assert_eq!(at(s, "prompt_box_body"), "❯ hi\n");
        assert_eq!(at(s, "last_non_empty_above_prompt_box"), "log");
        assert!(is_horizontal_rule("─"));
        assert!(!is_horizontal_rule("─ label"));
        assert!(is_horizontal_rule("───abc"));
        assert!(!is_horizontal_rule("─x"));
    }

    #[test]
    fn prompt_markers_follow_response_blocks() {
        let s = "• did a thing\n› next";
        assert_eq!(at(s, "after_last_prompt_marker"), "");
        assert_eq!(at(s, "before_current_prompt_marker"), "• did a thing\n");
        assert_eq!(at(s, "current_prompt_block_marker"), "• did a thing");
        assert_eq!(at(s, "whole_recent_without_current_prompt_marker"), "");
        let stale = "› old\n• answer";
        assert_eq!(
            at(stale, "whole_recent_without_current_prompt_marker"),
            stale
        );
        assert_eq!(at(stale, "current_prompt_block_marker"), "");
    }
}
