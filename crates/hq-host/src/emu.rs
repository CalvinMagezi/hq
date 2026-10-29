//! Screen emulation behind a trait, so the backend (vt100 today) can change
//! without touching the host.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

/// One screen row. `wrapped` means its text continues on the next row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub text: String,
    pub wrapped: bool,
}

pub trait Emulator: Send {
    fn process(&mut self, bytes: &[u8]);
    fn resize(&mut self, rows: u16, cols: u16);
    /// `(rows, cols)`.
    fn size(&self) -> (u16, u16);
    /// The rows currently on screen, top first, without trailing blank rows.
    fn visible(&mut self) -> Vec<Row>;
    /// Scrollback followed by the screen, oldest first, without trailing blank rows.
    fn history(&mut self) -> Vec<Row>;
    /// The newest `limit` rows of `history`, read only as far back as needed.
    fn recent(&mut self, limit: usize) -> Vec<Row> {
        let mut rows = self.history();
        rows.drain(..rows.len().saturating_sub(limit));
        rows
    }
    /// The newest `limit` rows of `history` (all when 0) with color and style as ANSI escape sequences,
    /// wrapped rows joined back into logical lines like the unwrapped plain read. Emulators that keep no
    /// style give plain text.
    fn styled_history(&mut self, limit: usize) -> Vec<String> {
        let mut lines: Vec<String> = self.history().into_iter().map(|r| r.text).collect();
        if limit > 0 && lines.len() > limit {
            lines.drain(..lines.len() - limit);
        }
        lines
    }
    fn alt_screen(&self) -> bool;
    fn bracketed_paste(&self) -> bool;
    /// The terminal title the program set, or empty.
    fn title(&self) -> String;
}

/// Keeps the latest window title the program set.
#[derive(Clone, Default)]
struct TitleCapture(Arc<Mutex<String>>);

/// Longest title kept, in bytes. A program sets it freely and every agent reply
/// carries it, so an unbounded one could make replies too big to read.
pub const MAX_TITLE_BYTES: usize = 256;

impl vt100::Callbacks for TitleCapture {
    fn set_window_title(&mut self, _: &mut vt100::Screen, title: &[u8]) {
        let mut text =
            String::from_utf8_lossy(&title[..title.len().min(MAX_TITLE_BYTES)]).into_owned();
        // A cut in the middle of a character leaves a replacement mark at the end.
        while text.ends_with('\u{fffd}') && text.len() > 1 {
            text.pop();
        }
        let mut slot = self.0.lock().unwrap_or_else(|p| p.into_inner());
        *slot = text;
    }
}

pub struct VtEmulator {
    parser: vt100::Parser<TitleCapture>,
    title: TitleCapture,
}

impl VtEmulator {
    pub fn new(rows: u16, cols: u16, scrollback_rows: usize) -> Self {
        let title = TitleCapture::default();
        let parser = vt100::Parser::new_with_callbacks(rows, cols, scrollback_rows, title.clone());
        Self { parser, title }
    }

    fn page_rows(&self) -> Vec<Row> {
        let screen = self.parser.screen();
        let (rows, cols) = screen.size();
        let ghost_row = ghost_text_start(screen);
        screen
            .rows(0, cols)
            .enumerate()
            .map(|(i, text)| {
                let wrapped = screen.row_wrapped(i as u16);
                let text = match ghost_row {
                    Some((row, col)) if row as usize == i => cells_before(screen, row, col),
                    _ => text,
                };
                // A wrapped row's trailing spaces belong to the line it continues.
                let text = if wrapped {
                    text
                } else {
                    text.trim_end().to_string()
                };
                Row { text, wrapped }
            })
            .take(rows as usize)
            .collect()
    }
}

/// Where a suggestion the program drew but nobody typed begins: dim text from the visible cursor to the
/// end of its row on the live page (Claude Code's suggested next prompt, a shell's autosuggestion). Typed
/// input is never dim, so a plain read leaves it out instead of reporting it as typed. `None` when the
/// cursor is hidden, or its row has nothing after it or anything there that is not dim.
fn ghost_text_start(screen: &vt100::Screen) -> Option<(u16, u16)> {
    // A hidden cursor is parked wherever the program left it, so what follows it means nothing.
    if screen.scrollback() != 0 || screen.hide_cursor() {
        return None;
    }
    let (row, col) = screen.cursor_position();
    let (_, cols) = screen.size();
    let mut seen = false;
    for c in col..cols {
        let Some(cell) = screen.cell(row, c) else {
            break;
        };
        let blank = cell.contents().trim().is_empty();
        if !blank && !cell.dim() {
            return None;
        }
        seen |= !blank;
    }
    seen.then_some((row, col))
}

/// The text of `row` left of column `col`.
fn cells_before(screen: &vt100::Screen, row: u16, col: u16) -> String {
    (0..col)
        .filter_map(|c| screen.cell(row, c))
        .filter(|cell| !cell.is_wide_continuation())
        .map(|cell| match cell.contents() {
            "" => " ",
            text => text,
        })
        .collect()
}

/// The widest run of blanks one cursor-forward code may stand for: a row is never wider.
const MAX_FORWARD: usize = 1000;

/// `rows_formatted` skips blank cells with cursor-forward codes (`ESC [ n C`). A reader that only draws
/// text and colors would glue the words together, so they become the spaces they stand for.
fn expand_cursor_forward(row: &str) -> String {
    let mut out = String::with_capacity(row.len());
    let mut chars = row.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' || chars.peek() != Some(&'[') {
            out.push(c);
            continue;
        }
        chars.next();
        let mut params = String::new();
        while let Some(&p) = chars
            .peek()
            .filter(|p| p.is_ascii_digit() || matches!(p, ';' | '?'))
        {
            params.push(p);
            chars.next();
        }
        match chars.next() {
            Some('C') => out.extend(std::iter::repeat_n(
                ' ',
                params.parse::<usize>().unwrap_or(1).clamp(1, MAX_FORWARD),
            )),
            Some(last) => {
                out.push_str("\x1b[");
                out.push_str(&params);
                out.push(last);
            }
            None => {}
        }
    }
    out
}

fn trim_trailing_blank(rows: &mut Vec<Row>) {
    while rows.last().is_some_and(|r| r.text.is_empty() && !r.wrapped) {
        rows.pop();
    }
}

impl Emulator for VtEmulator {
    fn process(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
    }

    fn resize(&mut self, rows: u16, cols: u16) {
        self.parser.screen_mut().set_size(rows, cols);
    }

    fn size(&self) -> (u16, u16) {
        self.parser.screen().size()
    }

    fn visible(&mut self) -> Vec<Row> {
        self.parser.screen_mut().set_scrollback(0);
        let mut rows = self.page_rows();
        trim_trailing_blank(&mut rows);
        rows
    }

    fn history(&mut self) -> Vec<Row> {
        let (page, _) = self.parser.screen().size();
        let page = page as usize;
        // The scrollback offset clamps to the real history length.
        self.parser.screen_mut().set_scrollback(usize::MAX);
        let history_len = self.parser.screen().scrollback();
        let mut by_index: BTreeMap<usize, Row> = BTreeMap::new();
        let mut start = 0;
        while start < history_len + page {
            let offset = history_len.saturating_sub(start);
            self.parser.screen_mut().set_scrollback(offset);
            let first_index = history_len - offset;
            for (i, row) in self.page_rows().into_iter().enumerate() {
                by_index.entry(first_index + i).or_insert(row);
            }
            start += page;
        }
        self.parser.screen_mut().set_scrollback(0);
        let mut rows: Vec<Row> = by_index.into_values().collect();
        trim_trailing_blank(&mut rows);
        rows
    }

    fn recent(&mut self, limit: usize) -> Vec<Row> {
        let (page, _) = self.parser.screen().size();
        let page = page as usize;
        self.parser.screen_mut().set_scrollback(usize::MAX);
        let history_len = self.parser.screen().scrollback();
        let mut by_index: BTreeMap<usize, Row> = BTreeMap::new();
        let mut back = 0;
        // Newest page first, stopping once `limit` rows (and the screen) are in hand.
        loop {
            let offset = back.min(history_len);
            self.parser.screen_mut().set_scrollback(offset);
            let first_index = history_len - offset;
            for (i, row) in self.page_rows().into_iter().enumerate() {
                by_index.entry(first_index + i).or_insert(row);
            }
            if offset == history_len || by_index.len() >= limit.saturating_add(page) {
                break;
            }
            back += page.max(1);
        }
        self.parser.screen_mut().set_scrollback(0);
        let mut rows: Vec<Row> = by_index.into_values().collect();
        trim_trailing_blank(&mut rows);
        rows.drain(..rows.len().saturating_sub(limit));
        rows
    }

    fn styled_history(&mut self, limit: usize) -> Vec<String> {
        let (page, cols) = self.parser.screen().size();
        let page = page as usize;
        self.parser.screen_mut().set_scrollback(usize::MAX);
        let history_len = self.parser.screen().scrollback();
        // Newest page first, so a long scrollback is read only as far back as the caller wants.
        let want = if limit == 0 {
            usize::MAX
        } else {
            limit.saturating_add(page)
        };
        let mut by_index: BTreeMap<usize, (Row, String)> = BTreeMap::new();
        let mut back = 0;
        loop {
            let offset = back.min(history_len);
            self.parser.screen_mut().set_scrollback(offset);
            let first_index = history_len - offset;
            let styled = self
                .parser
                .screen()
                .rows_formatted(0, cols)
                .map(|r| expand_cursor_forward(&String::from_utf8_lossy(&r)));
            for (i, (row, styled)) in self.page_rows().into_iter().zip(styled).enumerate() {
                by_index.entry(first_index + i).or_insert((row, styled));
            }
            if offset == history_len || by_index.len() >= want {
                break;
            }
            back += page.max(1);
        }
        self.parser.screen_mut().set_scrollback(0);
        let mut rows: Vec<(Row, String)> = by_index.into_values().collect();
        while rows
            .last()
            .is_some_and(|(r, _)| r.text.is_empty() && !r.wrapped)
        {
            rows.pop();
        }
        let mut lines: Vec<String> = Vec::new();
        let mut joining = false;
        for (row, styled) in rows {
            match (joining, lines.last_mut()) {
                (true, Some(open)) => open.push_str(&styled),
                _ => lines.push(styled),
            }
            joining = row.wrapped;
        }
        if limit > 0 && lines.len() > limit {
            lines.drain(..lines.len() - limit);
        }
        lines
    }

    fn alt_screen(&self) -> bool {
        self.parser.screen().alternate_screen()
    }

    fn bracketed_paste(&self) -> bool {
        self.parser.screen().bracketed_paste()
    }

    fn title(&self) -> String {
        self.title
            .0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fed(rows: u16, cols: u16, text: &str) -> VtEmulator {
        let mut e = VtEmulator::new(rows, cols, 1000);
        e.process(text.as_bytes());
        e
    }

    #[test]
    fn a_dim_suggestion_after_the_cursor_is_not_read_as_typed() {
        // Claude Code's prompt line with its suggested next prompt, cursor back at the start of it.
        let mut emu = fed(5, 40, "\u{276f}\u{a0}\x1b[2mrun it\x1b[0m\x1b[1;3H");
        let visible = emu.visible();
        assert_eq!(visible[0].text, "\u{276f}", "{visible:?}");
        let history = emu.history();
        assert_eq!(history[0].text, "\u{276f}");
        assert!(
            emu.styled_history(0)[0].contains("run it"),
            "the styled read still shows what is on screen"
        );
    }

    #[test]
    fn claude_codes_own_suggestion_frame_reads_as_an_empty_prompt() {
        // The bytes Claude Code 2.1 wrote for "run it": prompt row 21, the suggestion drawn dim from
        // home, then the visible cursor parked at its first column.
        let frame = "\x1b[21;1H\u{276f}\u{a0}\x1b[?25l\x1b[H\r\x1b[2C\x1b[20B\x1b[2mrun it\x1b[22m\x1b[24;1H\x1b[21;3H\x1b[?25h";
        let mut emu = fed(24, 80, frame);
        let prompt = emu.visible().pop().unwrap();
        assert_eq!(prompt.text, "\u{276f}");
    }

    #[test]
    fn typed_or_partly_bright_text_after_the_cursor_is_kept() {
        let mut typed = fed(5, 40, "\u{276f} run it\x1b[1;3H");
        assert_eq!(typed.visible()[0].text, "\u{276f} run it");
        let mut mixed = fed(5, 40, "\u{276f} \x1b[2mrun\x1b[0m it\x1b[1;3H");
        assert_eq!(mixed.visible()[0].text, "\u{276f} run it");
        // Dim text that is not after the cursor is ordinary output.
        let mut before = fed(5, 40, "\x1b[2mthinking\x1b[0m\r\n\u{276f} ");
        assert_eq!(before.visible()[0].text, "thinking");
        // Nor is dim text after a cursor the program hid.
        let mut hidden = fed(5, 40, "\x1b[2mstatus line\x1b[0m\x1b[1;1H\x1b[?25l");
        assert_eq!(hidden.visible()[0].text, "status line");
    }

    #[test]
    fn history_includes_scrolled_off_rows_in_order() {
        let text: String = (1..=30).map(|n| format!("line{n}\r\n")).collect();
        let mut e = fed(5, 40, &text);
        let all: Vec<String> = e.history().into_iter().map(|r| r.text).collect();
        assert_eq!(all.first().map(String::as_str), Some("line1"));
        assert_eq!(all.last().map(String::as_str), Some("line30"));
        assert_eq!(all.len(), 30);
        assert!(e.visible().len() <= 5);
    }

    #[test]
    fn styled_history_keeps_colors_and_matches_the_plain_rows() {
        let mut emu = fed(
            3,
            20,
            "plain\r\n\x1b[31mred text\x1b[0m\r\nthree\r\nfour\r\nfive",
        );
        let styled = emu.styled_history(0);
        let plain: Vec<String> = emu.history().into_iter().map(|r| r.text).collect();
        assert_eq!(styled.len(), plain.len());
        for (s, p) in styled.iter().zip(&plain) {
            assert_eq!(
                strip_sgr(s).trim_end(),
                p.trim_end(),
                "styled and plain rows differ"
            );
        }
        let red = styled
            .iter()
            .position(|r| r.contains("red text"))
            .expect("red row");
        assert!(
            styled[red].contains("\x1b[31m") || styled[red].contains("\x1b[3"),
            "{:?}",
            styled[red]
        );
        assert!(!styled[0].contains("red"));
    }

    fn strip_sgr(row: &str) -> String {
        let mut out = String::new();
        let mut chars = row.chars();
        while let Some(c) = chars.next() {
            if c == '\x1b' {
                for e in chars.by_ref() {
                    if e.is_ascii_alphabetic() {
                        break;
                    }
                }
            } else {
                out.push(c);
            }
        }
        out
    }

    #[test]
    fn styled_history_joins_wrapped_rows_and_keeps_only_the_newest_lines() {
        let mut emu = fed(
            4,
            10,
            "short\r\n\x1b[32mabcdefghijklmnopqrst\x1b[0m\r\nlast\r\n1\r\n2\r\n3\r\n4",
        );
        let unwrapped: Vec<String> = emu.history().into_iter().map(|r| r.text).collect();
        let styled = emu.styled_history(0);
        let long = styled
            .iter()
            .find(|l| l.contains("abcdefghij"))
            .expect("long line");
        assert!(strip_sgr(long).contains("abcdefghijklmnopqrst"), "{long:?}");
        assert!(
            unwrapped.len() > styled.len(),
            "physical rows are joined into fewer lines"
        );
        let newest = emu.styled_history(2);
        assert_eq!(newest.len(), 2);
        assert!(strip_sgr(&newest[1]).starts_with('4'), "{newest:?}");
    }

    #[test]
    fn cursor_forward_codes_become_the_spaces_they_skip() {
        assert_eq!(expand_cursor_forward("a\x1b[Cb\x1b[3Cc"), "a b   c");
        assert_eq!(
            expand_cursor_forward("\x1b[31mred\x1b[0m"),
            "\x1b[31mred\x1b[0m"
        );
        assert_eq!(
            expand_cursor_forward("x\x1b[99999999Cy").len(),
            2 + MAX_FORWARD
        );
        assert_eq!(expand_cursor_forward("cut\x1b"), "cut\x1b");
    }

    #[test]
    fn recent_rows_are_the_tail_of_history() {
        let text: String = (1..=50).map(|n| format!("line{n}\r\n")).collect();
        let mut e = fed(5, 40, &text);
        let all = e.history();
        let tail: Vec<String> = e.recent(12).into_iter().map(|r| r.text).collect();
        let want: Vec<String> = all[all.len() - 12..]
            .iter()
            .map(|r| r.text.clone())
            .collect();
        assert_eq!(tail, want);
        assert_eq!(e.recent(1000).len(), all.len());
        assert_eq!(e.visible().len(), 4, "the screen is left at the bottom");
    }

    #[test]
    fn long_line_is_wrapped_across_rows() {
        let mut e = fed(10, 20, &"x".repeat(50));
        let rows = e.history();
        assert_eq!(rows.len(), 3);
        assert!(rows[0].wrapped && rows[1].wrapped && !rows[2].wrapped);
    }

    #[test]
    fn spaces_at_a_wrap_boundary_are_kept() {
        let mut e = fed(5, 4, "foo bar");
        let rows = e.history();
        assert_eq!(rows[0].text, "foo ");
        let joined: String = rows.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(joined, "foo bar");
    }

    #[test]
    fn the_window_title_is_captured() {
        let mut e = fed(5, 20, "\x1b]0;first\x07\x1b]2;⠋ second\x1b\\");
        assert_eq!(e.title(), "⠋ second");
        e.process(b"\x1b]0;third\x07");
        assert_eq!(e.title(), "third");
    }

    #[test]
    fn modes_are_reported() {
        let mut e = fed(5, 20, "\x1b[?2004h\x1b[?1049h");
        assert!(e.bracketed_paste());
        assert!(e.alt_screen());
        e.process(b"\x1b[?2004l\x1b[?1049l");
        assert!(!e.bracketed_paste() && !e.alt_screen());
    }

    #[test]
    fn a_huge_title_is_cut_to_a_small_one() {
        let mut emu = VtEmulator::new(24, 80, 100);
        let mut bytes = b"\x1b]0;".to_vec();
        bytes.extend(std::iter::repeat_n(b'x', 100_000));
        bytes.push(0x07);
        emu.process(&bytes);
        assert!(
            emu.title().len() <= MAX_TITLE_BYTES,
            "{}",
            emu.title().len()
        );
        assert!(emu.title().starts_with("xxxx"));
    }

    #[test]
    fn a_title_cut_inside_a_character_does_not_end_in_a_replacement_mark() {
        let mut emu = VtEmulator::new(24, 80, 100);
        let mut bytes = b"\x1b]0;".to_vec();
        bytes.extend("é".repeat(300).as_bytes());
        bytes.push(0x07);
        emu.process(&bytes);
        assert!(!emu.title().contains('\u{fffd}'));
    }
}
