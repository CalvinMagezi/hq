/// Drop ANSI escape sequences and stray control bytes.
///
/// Handles the three forms a harness TUI actually emits: CSI (colour, cursor,
/// erase), OSC (window title, terminated by BEL or ST), and two-byte escapes
/// such as charset selection.
pub(super) fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            if c == '\n' || c == '\t' || !c.is_control() {
                out.push(c);
            }
            continue;
        }
        match chars.peek() {
            Some('[') => {
                chars.next();
                while let Some(&next) = chars.peek() {
                    chars.next();
                    if next.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            Some(']') => {
                chars.next();
                while let Some(&next) = chars.peek() {
                    chars.next();
                    if next == '\x07' {
                        break;
                    }
                    if next == '\x1b' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            Some(_) => {
                chars.next();
            }
            None => {}
        }
    }
    out
}

/// Turn pty output into text a person can read: real newlines, no escapes, no
/// leading or trailing blank screen.
pub(super) fn clean_pty_text(raw: &str) -> String {
    let normalized = raw.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<String> = normalized
        .lines()
        .map(|line| strip_ansi(line).trim_end().to_string())
        .collect();
    let start = lines
        .iter()
        .position(|l| !l.trim().is_empty())
        .unwrap_or(lines.len());
    let end = lines
        .iter()
        .rposition(|l| !l.trim().is_empty())
        .map(|i| i + 1)
        .unwrap_or(start);
    lines[start..end].join("\n")
}

pub(super) fn last_lines(text: &str, max_lines: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= max_lines {
        return text.to_string();
    }
    lines[lines.len() - max_lines..].join("\n")
}

/// Byte-safe excerpt keeping the *end* of the text: a harness writes its answer
/// last, so trimming the front is what preserves the useful part. The cap is on
/// bytes because that is what the delivery surfaces charge for, and the cut
/// walks forward to a char boundary so multibyte output cannot panic or split.
pub(super) fn tail_excerpt(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    let start = text.ceil_char_boundary(text.len() - max_bytes);
    format!("…{}", &text[start..])
}
