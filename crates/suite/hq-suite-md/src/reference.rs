//! References that blocks carry as text: a range of cells in a sheet (`from="[[Stock]]!D2:D40"`)
//! and a relative path to a sidecar (`data="_data/suite/sheets/stock.csv.zst"`).
//!
//! Only the shape is checked here. Whether the note or the file exists is the caller's question.

use std::fmt;

/// Excel's own limits, so a range that fits here fits there when a sheet is rendered to xlsx.
pub const MAX_COL: u32 = 16_384;
pub const MAX_ROW: u32 = 1_048_576;

/// A cell: one-based column and row, as in `D2` (column 4, row 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Cell {
    pub col: u32,
    pub row: u32,
}

impl Cell {
    pub fn parse(s: &str) -> Result<Cell, RefError> {
        let split = s
            .find(|c: char| c.is_ascii_digit())
            .ok_or_else(|| RefError::BadCell(s.to_string()))?;
        let (letters, digits) = s.split_at(split);
        if letters.is_empty()
            || letters.len() > 3
            || !letters.chars().all(|c| c.is_ascii_uppercase())
            || digits.is_empty()
            || digits.starts_with('0')
            || digits.len() > 7
            || !digits.chars().all(|c| c.is_ascii_digit())
        {
            return Err(RefError::BadCell(s.to_string()));
        }
        let col = letters
            .bytes()
            .fold(0u32, |n, b| n * 26 + u32::from(b - b'A') + 1);
        let row: u32 = digits
            .parse()
            .map_err(|_| RefError::BadCell(s.to_string()))?;
        if col > MAX_COL || row > MAX_ROW {
            return Err(RefError::OutOfRange(s.to_string()));
        }
        Ok(Cell { col, row })
    }

    /// The A1 name, such as `AB12`.
    pub fn a1(self) -> String {
        let mut letters = Vec::new();
        let mut n = self.col;
        while n > 0 {
            n -= 1;
            letters.push(b'A' + (n % 26) as u8);
            n /= 26;
        }
        letters.reverse();
        format!("{}{}", String::from_utf8_lossy(&letters), self.row)
    }
}

/// A rectangle of cells, with the note and block it lives in when the reference names them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeRef {
    /// The note's title or path inside `[[ ]]`. `None` means the note holding the reference.
    pub note: Option<String>,
    /// A block inside that note, by `name` or `id`, written `[[Note#block]]`.
    pub block: Option<String>,
    /// Top left, always at or before `end`.
    pub start: Cell,
    pub end: Cell,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefError {
    Empty,
    BadCell(String),
    OutOfRange(String),
    BadTarget(String),
    MissingBang,
    TooLong,
}

impl fmt::Display for RefError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RefError::Empty => write!(f, "the reference is empty"),
            RefError::BadCell(c) => write!(f, "`{c}` is not a cell such as D2"),
            RefError::OutOfRange(c) => write!(f, "`{c}` is outside the grid (XFD1048576)"),
            RefError::BadTarget(t) => write!(f, "`{t}` is not a note reference such as [[Stock]]"),
            RefError::MissingBang => write!(f, "write the cells after the note as [[Note]]!A1:B2"),
            RefError::TooLong => write!(f, "the reference is too long"),
        }
    }
}

impl std::error::Error for RefError {}

impl RangeRef {
    pub fn parse(s: &str) -> Result<RangeRef, RefError> {
        let s = s.trim();
        if s.is_empty() {
            return Err(RefError::Empty);
        }
        if s.len() > 512 {
            return Err(RefError::TooLong);
        }
        let (note, block, cells) = if let Some(rest) = s.strip_prefix("[[") {
            let close = rest
                .find("]]")
                .ok_or_else(|| RefError::BadTarget(s.to_string()))?;
            let target = &rest[..close];
            let after = &rest[close + 2..];
            let cells = after.strip_prefix('!').ok_or(RefError::MissingBang)?;
            let (note, block) = match target.split_once('#') {
                Some((n, b)) => (n, Some(b)),
                None => (target, None),
            };
            let bad = |t: &str| {
                t.is_empty()
                    || t.chars()
                        .any(|c| c.is_control() || c == '[' || c == ']' || c == '|')
            };
            if bad(note) || block.is_some_and(bad) {
                return Err(RefError::BadTarget(format!("[[{target}]]")));
            }
            (Some(note.to_string()), block.map(str::to_string), cells)
        } else {
            (None, None, s)
        };
        let (a, b) = match cells.split_once(':') {
            Some((a, b)) => (Cell::parse(a)?, Cell::parse(b)?),
            None => {
                let c = Cell::parse(cells)?;
                (c, c)
            }
        };
        Ok(RangeRef {
            note,
            block,
            start: Cell {
                col: a.col.min(b.col),
                row: a.row.min(b.row),
            },
            end: Cell {
                col: a.col.max(b.col),
                row: a.row.max(b.row),
            },
        })
    }

    pub fn rows(&self) -> u32 {
        self.end.row - self.start.row + 1
    }

    pub fn cols(&self) -> u32 {
        self.end.col - self.start.col + 1
    }

    /// The canonical text, which parses back to the same value.
    pub fn render(&self) -> String {
        let cells = if self.start == self.end {
            self.start.a1()
        } else {
            format!("{}:{}", self.start.a1(), self.end.a1())
        };
        match (&self.note, &self.block) {
            (Some(n), Some(b)) => format!("[[{n}#{b}]]!{cells}"),
            (Some(n), None) => format!("[[{n}]]!{cells}"),
            _ => cells,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathError {
    Empty,
    Absolute,
    /// `..`, an empty component or a backslash: anything that could leave the folder it is
    /// joined to, or that means something else on another platform.
    Unsafe,
    TooLong,
}

impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PathError::Empty => write!(f, "the path is empty"),
            PathError::Absolute => write!(f, "the path must be relative to the vault"),
            PathError::Unsafe => write!(f, "the path has a `..`, an empty part or a backslash"),
            PathError::TooLong => write!(f, "the path is too long"),
        }
    }
}

impl std::error::Error for PathError {}

/// Checks a vault-relative path from a block header. Returns it unchanged when it is safe to
/// join onto the vault root: forward slashes, no `..`, no empty or `.` parts, no drive letter,
/// no scheme, no control characters.
pub fn safe_rel_path(p: &str) -> Result<&str, PathError> {
    if p.is_empty() {
        return Err(PathError::Empty);
    }
    if p.len() > 1024 {
        return Err(PathError::TooLong);
    }
    if p.starts_with('/') || p.starts_with('~') {
        return Err(PathError::Absolute);
    }
    let bytes = p.as_bytes();
    let drive = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    if drive || p.contains("://") {
        return Err(PathError::Absolute);
    }
    if p.contains('\\') || p.chars().any(char::is_control) {
        return Err(PathError::Unsafe);
    }
    if p.split('/')
        .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(PathError::Unsafe);
    }
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cells_convert_both_ways() {
        let d2 = Cell::parse("D2").unwrap();
        assert_eq!(d2, Cell { col: 4, row: 2 });
        assert_eq!(d2.a1(), "D2");
        assert_eq!(Cell::parse("AA10").unwrap().col, 27);
        assert_eq!(Cell::parse("XFD1048576").unwrap().a1(), "XFD1048576");
        for bad in [
            "", "D", "2", "d2", "D0", "D02", "A1B", "XFE1", "A1048577", "AAAA1",
        ] {
            assert!(Cell::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn ranges_name_their_note() {
        let r = RangeRef::parse("[[Stock]]!D2:D40").unwrap();
        assert_eq!(r.note.as_deref(), Some("Stock"));
        assert_eq!(r.block, None);
        assert_eq!((r.rows(), r.cols()), (39, 1));
        assert_eq!(r.render(), "[[Stock]]!D2:D40");

        let r = RangeRef::parse("[[Notebooks/Shop/Stock#q3]]!B2").unwrap();
        assert_eq!(r.block.as_deref(), Some("q3"));
        assert_eq!(r.render(), "[[Notebooks/Shop/Stock#q3]]!B2");

        let local = RangeRef::parse("B2:A1").unwrap();
        assert_eq!(local.note, None);
        assert_eq!(local.render(), "A1:B2");
    }

    #[test]
    fn rejects_malformed_ranges() {
        assert_eq!(RangeRef::parse(""), Err(RefError::Empty));
        assert_eq!(RangeRef::parse("[[Stock]]D2"), Err(RefError::MissingBang));
        assert!(matches!(
            RangeRef::parse("[[Stock!D2"),
            Err(RefError::BadTarget(_))
        ));
        assert!(matches!(
            RangeRef::parse("[[]]!D2"),
            Err(RefError::BadTarget(_))
        ));
        assert!(matches!(
            RangeRef::parse("[[a|b]]!D2"),
            Err(RefError::BadTarget(_))
        ));
        assert!(RangeRef::parse("D2:").is_err());
        assert!(RangeRef::parse(&"x".repeat(600)).is_err());
    }

    #[test]
    fn paths_must_stay_inside_the_vault() {
        assert!(safe_rel_path("_data/suite/sheets/stock.csv.zst").is_ok());
        for bad in [
            "",
            "/etc/passwd",
            "~/x",
            "C:/x",
            "c:x",
            "https://example.com/x",
            "../x",
            "a/../x",
            "a//b",
            "./a",
            "a/./b",
            "a\\b",
            "a/b/",
            "a\u{0}b",
        ] {
            assert!(safe_rel_path(bad).is_err(), "{bad:?}");
        }
        assert!(safe_rel_path(&"a/".repeat(600)).is_err());
    }
}
