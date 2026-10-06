//! Screen emulation behind a trait, so the backend (vt100 today) can change
//! without touching the host.

use std::collections::BTreeMap;

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
    fn alt_screen(&self) -> bool;
    fn bracketed_paste(&self) -> bool;
}

pub struct VtEmulator {
    parser: vt100::Parser,
}

impl VtEmulator {
    pub fn new(rows: u16, cols: u16, scrollback_rows: usize) -> Self {
        Self {
            parser: vt100::Parser::new(rows, cols, scrollback_rows),
        }
    }

    fn page_rows(&self) -> Vec<Row> {
        let screen = self.parser.screen();
        let (rows, cols) = screen.size();
        screen
            .rows(0, cols)
            .enumerate()
            .map(|(i, text)| Row {
                text: text.trim_end().to_string(),
                wrapped: screen.row_wrapped(i as u16),
            })
            .take(rows as usize)
            .collect()
    }
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

    fn alt_screen(&self) -> bool {
        self.parser.screen().alternate_screen()
    }

    fn bracketed_paste(&self) -> bool {
        self.parser.screen().bracketed_paste()
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
    fn long_line_is_wrapped_across_rows() {
        let mut e = fed(10, 20, &"x".repeat(50));
        let rows = e.history();
        assert_eq!(rows.len(), 3);
        assert!(rows[0].wrapped && rows[1].wrapped && !rows[2].wrapped);
    }

    #[test]
    fn modes_are_reported() {
        let mut e = fed(5, 20, "\x1b[?2004h\x1b[?1049h");
        assert!(e.bracketed_paste());
        assert!(e.alt_screen());
        e.process(b"\x1b[?2004l\x1b[?1049l");
        assert!(!e.bracketed_paste() && !e.alt_screen());
    }
}
