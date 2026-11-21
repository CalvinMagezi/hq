//! Moving cell references when rows are inserted or deleted, the way Excel and Sheets do it.
//!
//! This module only reads and writes text. It knows nothing about notes or blocks: the caller says
//! which sheet a formula sits on, which sheet changed and how, and gets the formula back.
//!
//! The rules, for a change that touches the rows of one sheet:
//!
//! * A reference is moved only when it points at the changed sheet: an unqualified reference in a
//!   formula on that sheet, or one written `Name!A1` or `'Quoted name'!A1` (any case) anywhere.
//! * Inserting rows at row `at` moves every row from `at` down. A range whose last row is at or
//!   below `at` grows when its first row is above `at`, and moves whole when its first row is at or
//!   below it. A range ending just above `at` does not grow.
//! * Deleting rows shrinks the ranges that cross them and moves what is below. A reference to a
//!   deleted row, or a range with nothing left, becomes `#REF!`.
//! * `$` does not stop a move: absolute rows move too (only copy and fill keep them fixed).
//! * Text in quotes, function names, numbers and anything that is not a cell stay exactly as
//!   written. A whole-row reference (`3:5`) moves like a range for a row change; a whole-column
//!   reference (`B:D`) stays put for it.
//!
//! A column change ([`ColChange`]) follows the same rules along the columns: refs at or right of an
//! insert move, ranges that straddle it grow, a deleted column's refs become `#REF!`, and whole
//! columns (`B:B`, `$B:D`) move like ranges while whole rows stay put.
//!
//! The same rules apply to the `from=` range of a `tile` or `plot` block ([`shift_from`]).

use hq_suite_md::RangeRef;

/// Excel's last row. A reference pushed past it becomes `#REF!`.
const MAX_ROW: u32 = 1_048_576;
/// Excel's last column, XFD.
const MAX_COL: u32 = 16_384;

/// One insert or delete along an axis, in one-based positions.
#[derive(Debug, Clone, Copy)]
enum Line {
    Insert { at: u32, n: u32 },
    Delete { first: u32, count: u32 },
}

impl Line {
    /// Where a single position goes, or `None` when it is deleted or pushed off the sheet.
    fn point(self, v: u32, max: u32) -> Option<u32> {
        match self {
            Line::Insert { at, n } => {
                let new = if v >= at { v.checked_add(n)? } else { v };
                (new <= max).then_some(new)
            }
            Line::Delete { first, count } => {
                let last = first.saturating_add(count).saturating_sub(1);
                if v < first {
                    Some(v)
                } else if v <= last {
                    None
                } else {
                    Some(v - count)
                }
            }
        }
    }

    /// Where the positions `lo..=hi` go, or `None` when nothing of them is left.
    fn span(self, lo: u32, hi: u32, max: u32) -> Option<(u32, u32)> {
        match self {
            Line::Insert { at, n } => {
                let lo2 = if lo >= at { lo.checked_add(n)? } else { lo };
                let hi2 = if hi >= at { hi.checked_add(n)? } else { hi };
                (hi2 <= max).then_some((lo2, hi2))
            }
            Line::Delete { first, count } => {
                let last = first.saturating_add(count).saturating_sub(1);
                let lo2 = if lo < first {
                    lo
                } else if lo <= last {
                    first
                } else {
                    lo - count
                };
                let hi2 = if hi < first {
                    hi
                } else if hi <= last {
                    first - 1
                } else {
                    hi - count
                };
                (hi2 >= lo2 && hi2 >= 1).then_some((lo2, hi2))
            }
        }
    }
}

/// Rows inserted or deleted in one sheet. Rows are one-based sheet rows (the header is row 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowChange {
    /// `n` empty rows go in before `at_row`; the rows from `at_row` down move down by `n`.
    Insert { at_row: u32, n: u32 },
    /// `count` rows from `first_row` down are removed.
    Delete { first_row: u32, count: u32 },
}

impl RowChange {
    fn line(self) -> Line {
        match self {
            RowChange::Insert { at_row, n } => Line::Insert { at: at_row, n },
            RowChange::Delete { first_row, count } => Line::Delete {
                first: first_row,
                count,
            },
        }
    }

    /// Where a single row goes, or `None` when it is deleted or pushed off the sheet.
    pub fn point(self, row: u32) -> Option<u32> {
        self.line().point(row, MAX_ROW)
    }

    /// Where a range of rows `lo..=hi` goes, or `None` when nothing of it is left.
    pub fn span(self, lo: u32, hi: u32) -> Option<(u32, u32)> {
        self.line().span(lo, hi, MAX_ROW)
    }
}

/// Columns inserted or deleted in one sheet. Columns are one-based (`A` is 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColChange {
    /// `n` empty columns go in before `at_col`; the columns from `at_col` right move right by `n`.
    Insert { at_col: u32, n: u32 },
    /// `count` columns from `first_col` rightwards are removed.
    Delete { first_col: u32, count: u32 },
}

impl ColChange {
    fn line(self) -> Line {
        match self {
            ColChange::Insert { at_col, n } => Line::Insert { at: at_col, n },
            ColChange::Delete { first_col, count } => Line::Delete {
                first: first_col,
                count,
            },
        }
    }

    /// Where a single column goes, or `None` when it is deleted or pushed off the sheet.
    pub fn point(self, col: u32) -> Option<u32> {
        self.line().point(col, MAX_COL)
    }

    /// Where a range of columns `lo..=hi` goes, or `None` when nothing of it is left.
    pub fn span(self, lo: u32, hi: u32) -> Option<(u32, u32)> {
        self.line().span(lo, hi, MAX_COL)
    }
}

/// A change along either axis. Whole-column references (`B:B`) move with a column change and stay
/// put for a row change; whole-row references (`3:5`) the other way round.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Row(RowChange),
    Col(ColChange),
}

impl From<RowChange> for Change {
    fn from(c: RowChange) -> Self {
        Change::Row(c)
    }
}

impl From<ColChange> for Change {
    fn from(c: ColChange) -> Self {
        Change::Col(c)
    }
}

fn column_letters(col: u32) -> String {
    let mut out = Vec::new();
    let mut n = col;
    while n > 0 {
        n -= 1;
        out.push(b'A' + (n % 26) as u8);
        n /= 26;
    }
    out.reverse();
    String::from_utf8_lossy(&out).into_owned()
}

// ---------------------------------------------------------------- formulas

fn word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '$')
}

fn scan_word(ch: &[char], mut i: usize) -> usize {
    while i < ch.len() && word_char(ch[i]) {
        i += 1;
    }
    i
}

/// What kind of end a reference has.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// `$B$3`: a column and a row.
    Cell,
    /// `$3`: a row alone, one end of `3:5`.
    Row,
    /// `$B`: a column alone, one end of `B:D`.
    Col,
}

/// One end of a reference, kept as written: a leading `$`, the column letters, the `$` before the
/// row, and the row. A part a kind does not have is empty or 0.
struct End {
    kind: Kind,
    lead: String,
    letters: String,
    col: u32,
    mid: String,
    row: u32,
}

impl End {
    fn render(&self, col: u32, row: u32) -> String {
        let letters = if col == self.col {
            self.letters.clone()
        } else {
            column_letters(col)
        };
        let row = if self.kind == Kind::Col {
            String::new()
        } else {
            row.to_string()
        };
        format!("{}{letters}{}{row}", self.lead, self.mid)
    }
}

fn parse_row(digits: &[char]) -> Option<u32> {
    if digits.is_empty() || digits.len() > 7 || digits[0] == '0' {
        return None;
    }
    if !digits.iter().all(char::is_ascii_digit) {
        return None;
    }
    let n: u32 = digits.iter().collect::<String>().parse().ok()?;
    (1..=MAX_ROW).contains(&n).then_some(n)
}

/// `B3`, `$B3`, `B$3`, `$B$3`, `$3` and `3`, or `B` and `$B`, as a whole word.
fn parse_end(word: &[char]) -> Option<End> {
    let mut i = 0;
    if word.get(i) == Some(&'$') {
        i += 1;
    }
    let lead: String = word[..i].iter().collect();
    let letters_at = i;
    while i < word.len() && word[i].is_ascii_alphabetic() {
        i += 1;
    }
    let letters: String = word[letters_at..i].iter().collect();
    if letters.is_empty() {
        let row = parse_row(&word[i..])?;
        return Some(End {
            kind: Kind::Row,
            lead,
            letters,
            col: 0,
            mid: String::new(),
            row,
        });
    }
    if letters.len() > 3 {
        return None;
    }
    let col = letters.chars().fold(0u32, |n, c| {
        n * 26 + (c.to_ascii_uppercase() as u32 - 'A' as u32) + 1
    });
    if col > MAX_COL {
        return None;
    }
    if i == word.len() {
        return Some(End {
            kind: Kind::Col,
            lead,
            letters,
            col,
            mid: String::new(),
            row: 0,
        });
    }
    let mut mid = String::new();
    if word.get(i) == Some(&'$') {
        mid.push('$');
        i += 1;
    }
    let row = parse_row(&word[i..])?;
    Some(End {
        kind: Kind::Cell,
        lead,
        letters,
        col,
        mid,
        row,
    })
}

/// A reference starting at `i`: one end, or two joined by `:`. Returns the ends and where it stops.
fn parse_operand(ch: &[char], i: usize) -> Option<(End, Option<End>, usize)> {
    let e1 = scan_word(ch, i);
    let first = parse_end(&ch[i..e1])?;
    if ch.get(e1) == Some(&':') {
        let e2 = scan_word(ch, e1 + 1);
        if let Some(second) = parse_end(&ch[e1 + 1..e2])
            && second.kind == first.kind
        {
            return Some((first, Some(second), e2));
        }
    }
    (first.kind == Kind::Cell).then_some((first, None, e1))
}

/// The moved text of an operand, or `None` when it is gone.
fn moved(first: &End, second: Option<&End>, change: Change) -> Option<String> {
    let (cols, line) = match change {
        Change::Row(c) => (false, c.line()),
        Change::Col(c) => (true, c.line()),
    };
    let max = if cols { MAX_COL } else { MAX_ROW };
    // The axis this change moves, of an end that has it.
    let pos = |e: &End| match (cols, e.kind) {
        (true, Kind::Cell | Kind::Col) => Some(e.col),
        (false, Kind::Cell | Kind::Row) => Some(e.row),
        _ => None,
    };
    let put = |e: &End, v: u32| {
        if cols {
            e.render(v, e.row)
        } else {
            e.render(e.col, v)
        }
    };
    match second {
        None => Some(put(first, line.point(pos(first)?, max)?)),
        Some(second) => {
            let (Some(p1), Some(p2)) = (pos(first), pos(second)) else {
                // A reference on the other axis (`A:A` for a row change): not touched.
                return Some(format!(
                    "{}:{}",
                    first.render(first.col, first.row),
                    second.render(second.col, second.row)
                ));
            };
            let reversed = p1 > p2;
            let (lo, hi) = if reversed { (p2, p1) } else { (p1, p2) };
            let (lo, hi) = line.span(lo, hi, max)?;
            let (a, b) = if reversed { (hi, lo) } else { (lo, hi) };
            Some(format!("{}:{}", put(first, a), put(second, b)))
        }
    }
}

/// A quoted sheet name starting at the `'` at `i`: its text and the index just past the closing `'`.
fn read_quoted(ch: &[char], i: usize) -> Option<(String, usize)> {
    let mut name = String::new();
    let mut j = i + 1;
    while j < ch.len() {
        if ch[j] == '\'' {
            if ch.get(j + 1) == Some(&'\'') {
                name.push('\'');
                j += 2;
                continue;
            }
            return Some((name, j + 1));
        }
        name.push(ch[j]);
        j += 1;
    }
    None
}

fn same_name(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

/// Where a reference after a `!` (or without a sheet) stops being text to copy.
struct Walk<'a> {
    ch: &'a [char],
    out: String,
    change: Change,
}

impl Walk<'_> {
    fn copy(&mut self, from: usize, to: usize) {
        self.out.extend(&self.ch[from..to]);
    }

    /// An operand at `i` that points at the changed sheet when `applies`. `qualifier` is the
    /// text already read for `Sheet!`, which goes away when the reference becomes `#REF!`.
    /// Returns where to go on from.
    fn operand(&mut self, i: usize, applies: bool, qualifier: &str) -> usize {
        match parse_operand(self.ch, i) {
            Some((first, second, end)) if applies => {
                match moved(&first, second.as_ref(), self.change) {
                    Some(text) => {
                        self.out.push_str(qualifier);
                        self.out.push_str(&text);
                    }
                    None => self.out.push_str("#REF!"),
                }
                end
            }
            Some((_, _, end)) => {
                self.out.push_str(qualifier);
                self.copy(i, end);
                end
            }
            None => {
                // Something that is not a cell after the `!`, such as `A:A` or a name: copy it
                // whole so its letters are not read as a reference on their own.
                self.out.push_str(qualifier);
                let mut end = scan_word(self.ch, i);
                if self.ch.get(end) == Some(&':') {
                    end = scan_word(self.ch, end + 1);
                }
                self.copy(i, end);
                end
            }
        }
    }
}

/// `formula` (with or without its leading `=`), written on the sheet `formula_sheet`, with every
/// reference to a row of `edited_sheet` moved for `change`. Names are compared without case.
/// Everything else is returned byte for byte.
pub fn shift_formula(
    formula: &str,
    formula_sheet: &str,
    edited_sheet: &str,
    change: impl Into<Change>,
) -> String {
    let change = change.into();
    let ch: Vec<char> = formula.chars().collect();
    let on_edited = same_name(formula_sheet, edited_sheet);
    let mut w = Walk {
        ch: &ch,
        out: String::with_capacity(formula.len() + 8),
        change,
    };
    let mut i = 0;
    while i < ch.len() {
        let c = ch[i];
        if c == '"' {
            // Text: to the closing quote; a doubled quote is a quote inside the text.
            let mut j = i + 1;
            while j < ch.len() {
                if ch[j] == '"' {
                    if ch.get(j + 1) == Some(&'"') {
                        j += 2;
                        continue;
                    }
                    j += 1;
                    break;
                }
                j += 1;
            }
            w.copy(i, j);
            i = j;
        } else if c == '\'' {
            match read_quoted(&ch, i) {
                Some((name, end)) if ch.get(end) == Some(&'!') => {
                    let qualifier: String = ch[i..=end].iter().collect();
                    i = w.operand(end + 1, same_name(&name, edited_sheet), &qualifier);
                }
                _ => {
                    w.copy(i, i + 1);
                    i += 1;
                }
            }
        } else if word_char(c) {
            let end = scan_word(&ch, i);
            match ch.get(end) {
                Some('!') => {
                    let name: String = ch[i..end].iter().collect();
                    let qualifier: String = ch[i..=end].iter().collect();
                    i = w.operand(end + 1, same_name(&name, edited_sheet), &qualifier);
                }
                Some('(') => {
                    w.copy(i, end);
                    i = end;
                }
                _ => match parse_operand(&ch, i) {
                    Some(_) => i = w.operand(i, on_edited, ""),
                    None => {
                        w.copy(i, end);
                        i = end;
                    }
                },
            }
        } else {
            w.copy(i, i + 1);
            i += 1;
        }
    }
    w.out
}

// ---------------------------------------------------------------- tile and plot ranges

/// What a row change does to a block's `from=` range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FromShift {
    /// The rows it reads did not move.
    Same,
    /// The range, rewritten.
    Moved(String),
    /// Every row it reads is deleted.
    Gone,
}

/// `from` (`A1:C6`, `[[Note]]!A1:C6`, `[[Note#Sheet]]!A1:C6`) with its rows moved. The caller
/// decides whether the range reads the changed sheet. A range that cannot be read is left alone.
pub fn shift_from(from: &str, change: impl Into<Change>) -> FromShift {
    let Ok(mut r) = RangeRef::parse(from) else {
        return FromShift::Same;
    };
    let change = change.into();
    let (old, new) = match change {
        Change::Row(c) => ((r.start.row, r.end.row), c.span(r.start.row, r.end.row)),
        Change::Col(c) => ((r.start.col, r.end.col), c.span(r.start.col, r.end.col)),
    };
    let Some((lo, hi)) = new else {
        return FromShift::Gone;
    };
    if (lo, hi) == old {
        return FromShift::Same;
    }
    match change {
        Change::Row(_) => (r.start.row, r.end.row) = (lo, hi),
        Change::Col(_) => (r.start.col, r.end.col) = (lo, hi),
    }
    FromShift::Moved(r.render())
}

#[cfg(test)]
#[path = "ref_shift_tests.rs"]
mod tests;
