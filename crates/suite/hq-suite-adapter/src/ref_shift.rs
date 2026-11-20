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
//! * Text in quotes, function names, whole-column references (`A:A`), numbers and anything that is
//!   not a cell stay exactly as written. A whole-row reference (`3:5`) moves like a range.
//!
//! The same rules apply to the `from=` range of a `tile` or `plot` block ([`shift_from`]).

use hq_suite_md::RangeRef;

/// Excel's last row. A reference pushed past it becomes `#REF!`.
const MAX_ROW: u32 = 1_048_576;
/// Excel's last column, XFD.
const MAX_COL: u32 = 16_384;

/// Rows inserted or deleted in one sheet. Rows are one-based sheet rows (the header is row 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowChange {
    /// `n` empty rows go in before `at_row`; the rows from `at_row` down move down by `n`.
    Insert { at_row: u32, n: u32 },
    /// `count` rows from `first_row` down are removed.
    Delete { first_row: u32, count: u32 },
}

impl RowChange {
    /// Where a single row goes, or `None` when it is deleted or pushed off the sheet.
    pub fn point(self, row: u32) -> Option<u32> {
        match self {
            RowChange::Insert { at_row, n } => {
                let new = if row >= at_row {
                    row.checked_add(n)?
                } else {
                    row
                };
                (new <= MAX_ROW).then_some(new)
            }
            RowChange::Delete { first_row, count } => {
                let last = first_row.saturating_add(count).saturating_sub(1);
                if row < first_row {
                    Some(row)
                } else if row <= last {
                    None
                } else {
                    Some(row - count)
                }
            }
        }
    }

    /// Where a range of rows `lo..=hi` goes, or `None` when nothing of it is left.
    pub fn span(self, lo: u32, hi: u32) -> Option<(u32, u32)> {
        match self {
            RowChange::Insert { at_row, n } => {
                let lo2 = if lo >= at_row { lo.checked_add(n)? } else { lo };
                let hi2 = if hi >= at_row { hi.checked_add(n)? } else { hi };
                (hi2 <= MAX_ROW).then_some((lo2, hi2))
            }
            RowChange::Delete { first_row, count } => {
                let last = first_row.saturating_add(count).saturating_sub(1);
                let lo2 = if lo < first_row {
                    lo
                } else if lo <= last {
                    first_row
                } else {
                    lo - count
                };
                let hi2 = if hi < first_row {
                    hi
                } else if hi <= last {
                    first_row - 1
                } else {
                    hi - count
                };
                (hi2 >= lo2 && hi2 >= 1).then_some((lo2, hi2))
            }
        }
    }
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
}

/// One end of a reference: the text before the row number, kept as written, and the row.
struct End {
    kind: Kind,
    prefix: String,
    row: u32,
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

/// `B3`, `$B3`, `B$3`, `$B$3` or `$3` and `3`, as a whole word.
fn parse_end(word: &[char]) -> Option<End> {
    let mut i = 0;
    if word.get(i) == Some(&'$') {
        i += 1;
    }
    let letters_at = i;
    while i < word.len() && word[i].is_ascii_alphabetic() {
        i += 1;
    }
    let letters = i - letters_at;
    if letters == 0 {
        let row = parse_row(&word[i..])?;
        return Some(End {
            kind: Kind::Row,
            prefix: word[..i].iter().collect(),
            row,
        });
    }
    if letters > 3 {
        return None;
    }
    let col = word[letters_at..i].iter().fold(0u32, |n, c| {
        n * 26 + (c.to_ascii_uppercase() as u32 - 'A' as u32) + 1
    });
    if col > MAX_COL {
        return None;
    }
    if word.get(i) == Some(&'$') {
        i += 1;
    }
    let row = parse_row(&word[i..])?;
    Some(End {
        kind: Kind::Cell,
        prefix: word[..i].iter().collect(),
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
fn moved(first: &End, second: Option<&End>, change: RowChange) -> Option<String> {
    match second {
        None => Some(format!("{}{}", first.prefix, change.point(first.row)?)),
        Some(second) => {
            let reversed = first.row > second.row;
            let (lo, hi) = if reversed {
                (second.row, first.row)
            } else {
                (first.row, second.row)
            };
            let (lo, hi) = change.span(lo, hi)?;
            let (a, b) = if reversed { (hi, lo) } else { (lo, hi) };
            Some(format!("{}{a}:{}{b}", first.prefix, second.prefix))
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
    change: RowChange,
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
    change: RowChange,
) -> String {
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
pub fn shift_from(from: &str, change: RowChange) -> FromShift {
    let Ok(mut r) = RangeRef::parse(from) else {
        return FromShift::Same;
    };
    let Some((lo, hi)) = change.span(r.start.row, r.end.row) else {
        return FromShift::Gone;
    };
    if (lo, hi) == (r.start.row, r.end.row) {
        return FromShift::Same;
    }
    r.start.row = lo;
    r.end.row = hi;
    FromShift::Moved(r.render())
}

#[cfg(test)]
#[path = "ref_shift_tests.rs"]
mod tests;
