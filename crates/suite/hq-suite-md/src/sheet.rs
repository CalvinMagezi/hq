//! The body of a `sheet` block: a GitHub-style table, and the column types in its header.
//!
//! Cells are kept as written. Whether `2,700,000` is a number, and what a formula returns, is
//! the calculation engine's business, not this crate's. What is checked here is shape: the same
//! number of cells in every row, a legal type for every column, and a size that cannot be used
//! to hang a phone.

use std::fmt;

/// Most cells a block may hold inline. Larger sheets keep a preview here and the rest in a
/// compressed sidecar named by `data=`.
pub const MAX_INLINE_CELLS: usize = 100_000;
/// Most columns, matching Excel's own limit.
pub const MAX_COLUMNS: usize = 16_384;

/// What a column holds, as named in `types="text,int,ugx"`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColType {
    Text,
    Int,
    Num,
    Bool,
    Date,
    Pct,
    /// An amount in a currency, by lowercase ISO 4217 code: `ugx`, `usd`, `kes`.
    Money(String),
}

const RESERVED: &[&str] = &["text", "int", "num", "bool", "date", "pct"];

impl ColType {
    pub fn parse(token: &str) -> Option<ColType> {
        match token {
            "text" => Some(ColType::Text),
            "int" => Some(ColType::Int),
            "num" => Some(ColType::Num),
            "bool" => Some(ColType::Bool),
            "date" => Some(ColType::Date),
            "pct" => Some(ColType::Pct),
            t if t.len() == 3
                && t.chars().all(|c| c.is_ascii_lowercase())
                && !RESERVED.contains(&t) =>
            {
                Some(ColType::Money(t.to_string()))
            }
            _ => None,
        }
    }

    pub fn token(&self) -> String {
        match self {
            ColType::Text => "text".into(),
            ColType::Int => "int".into(),
            ColType::Num => "num".into(),
            ColType::Bool => "bool".into(),
            ColType::Date => "date".into(),
            ColType::Pct => "pct".into(),
            ColType::Money(c) => c.clone(),
        }
    }
}

/// Parses `text,int,ugx`. An error names the first token that is not a type.
pub fn parse_types(s: &str) -> Result<Vec<ColType>, String> {
    s.split(',')
        .map(|t| {
            let t = t.trim();
            ColType::parse(t).ok_or_else(|| t.to_string())
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TableError {
    Empty,
    /// The line under the header is not `|---|---|`.
    NoDelimiter,
    /// Row `row` (1-based, header excluded) has `got` cells where the header has `want`.
    RaggedRow {
        row: usize,
        want: usize,
        got: usize,
    },
    /// A blank line inside the table; the rest would be read as a paragraph.
    BlankLine {
        line: usize,
    },
    TooManyColumns(usize),
    TooManyCells(usize),
    DuplicateHeader(String),
}

impl fmt::Display for TableError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TableError::Empty => write!(f, "the sheet has no header row"),
            TableError::NoDelimiter => {
                write!(f, "the line under the header must look like |---|---|")
            }
            TableError::RaggedRow { row, want, got } => {
                write!(f, "row {row} has {got} cells but the header has {want}")
            }
            TableError::BlankLine { line } => {
                write!(f, "a blank line at line {line} cuts the table in two")
            }
            TableError::TooManyColumns(n) => write!(f, "{n} columns is more than a sheet can hold"),
            TableError::TooManyCells(n) => {
                write!(
                    f,
                    "{n} cells is too many to keep in a note; use a data= sidecar"
                )
            }
            TableError::DuplicateHeader(h) => write!(f, "the header `{h}` appears twice"),
        }
    }
}

impl std::error::Error for TableError {}

/// A table read from a block body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grid {
    pub header: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

impl Grid {
    pub fn parse(body: &str) -> Result<Grid, TableError> {
        let mut lines = body.lines().map(|l| l.trim_end_matches('\r'));
        let header = loop {
            match lines.next() {
                None => return Err(TableError::Empty),
                Some(l) if l.trim().is_empty() => continue,
                Some(l) => break split_row(l),
            }
        };
        if header.len() > MAX_COLUMNS {
            return Err(TableError::TooManyColumns(header.len()));
        }
        for (i, h) in header.iter().enumerate() {
            if !h.is_empty() && header[..i].contains(h) {
                return Err(TableError::DuplicateHeader(h.clone()));
            }
        }
        let delim = lines.next().ok_or(TableError::NoDelimiter)?;
        if !is_delimiter(delim, header.len()) {
            return Err(TableError::NoDelimiter);
        }
        let mut rows: Vec<Vec<String>> = Vec::new();
        let mut cells = header.len();
        let mut seen_blank: Option<usize> = None;
        for (i, line) in lines.enumerate() {
            let line_no = i + 3;
            if line.trim().is_empty() {
                seen_blank.get_or_insert(line_no);
                continue;
            }
            if let Some(line) = seen_blank {
                return Err(TableError::BlankLine { line });
            }
            let row = split_row(line);
            if row.len() != header.len() {
                return Err(TableError::RaggedRow {
                    row: rows.len() + 1,
                    want: header.len(),
                    got: row.len(),
                });
            }
            cells += row.len();
            if cells > MAX_INLINE_CELLS {
                return Err(TableError::TooManyCells(cells));
            }
            rows.push(row);
        }
        Ok(Grid { header, rows })
    }

    pub fn columns(&self) -> usize {
        self.header.len()
    }

    /// The table as block body text, columns padded so it reads well in a plain editor. Parsing
    /// the result gives back the same grid.
    pub fn render(&self, eol: &str) -> String {
        let n = self.header.len();
        let esc = |c: &str| {
            c.replace('\\', "\\\\")
                .replace('|', "\\|")
                .replace(['\n', '\r'], " ")
        };
        let header: Vec<String> = self.header.iter().map(|c| esc(c)).collect();
        let rows: Vec<Vec<String>> = self
            .rows
            .iter()
            .map(|r| r.iter().map(|c| esc(c)).collect())
            .collect();
        let mut widths: Vec<usize> = header.iter().map(|c| c.chars().count().max(3)).collect();
        for r in &rows {
            for (w, c) in widths.iter_mut().zip(r) {
                *w = (*w).max(c.chars().count());
            }
        }
        let line = |cells: &[String]| {
            let mut s = String::from("|");
            for (c, w) in cells.iter().zip(&widths) {
                s.push(' ');
                s.push_str(c);
                for _ in c.chars().count()..*w {
                    s.push(' ');
                }
                s.push_str(" |");
            }
            s
        };
        let mut out = line(&header);
        out.push_str(eol);
        let mut delim = String::from("|");
        for w in widths.iter().take(n) {
            delim.push_str(&"-".repeat(w + 2));
            delim.push('|');
        }
        out.push_str(&delim);
        for r in &rows {
            out.push_str(eol);
            out.push_str(&line(r));
        }
        out
    }
}

fn is_delimiter(line: &str, columns: usize) -> bool {
    let cells = split_row(line);
    cells.len() == columns
        && cells.iter().all(|c| {
            let c = c.trim_matches(':');
            !c.is_empty() && c.chars().all(|ch| ch == '-')
        })
}

/// Splits one table line into trimmed cells. `\|` is a pipe inside a cell and `\\` a backslash;
/// the outer pipes are optional.
fn split_row(line: &str) -> Vec<String> {
    let line = line.trim();
    let line = line.strip_prefix('|').unwrap_or(line);
    let mut cells = Vec::new();
    let mut cur = String::new();
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.peek() {
                Some('|') => {
                    cur.push('|');
                    chars.next();
                }
                Some('\\') => {
                    cur.push('\\');
                    chars.next();
                }
                _ => cur.push('\\'),
            },
            '|' => {
                cells.push(cur.trim().to_string());
                cur.clear();
            }
            c => cur.push(c),
        }
    }
    // A closing pipe leaves an empty remainder that is not a cell; text after the last pipe is.
    if !cur.trim().is_empty() {
        cells.push(cur.trim().to_string());
    }
    cells
}

#[cfg(test)]
mod tests {
    use super::*;

    const STOCK: &str = "| item  | qty | price | total  |\n|-------|-----|-------|--------|\n| Sugar | 40  | 4200  | =B2*C2 |\n| Salt  | 25  | 1800  | =B3*C3 |";

    #[test]
    fn reads_the_draft_example() {
        let g = Grid::parse(STOCK).unwrap();
        assert_eq!(g.header, ["item", "qty", "price", "total"]);
        assert_eq!(g.rows.len(), 2);
        assert_eq!(g.rows[0], ["Sugar", "40", "4200", "=B2*C2"]);
    }

    #[test]
    fn render_then_parse_is_the_same_grid() {
        let g = Grid::parse(STOCK).unwrap();
        let text = g.render("\n");
        assert_eq!(Grid::parse(&text).unwrap(), g);
        // Padded output is stable: rendering what we rendered changes nothing.
        assert_eq!(Grid::parse(&text).unwrap().render("\n"), text);
    }

    #[test]
    fn pipes_and_backslashes_survive() {
        let g = Grid {
            header: vec!["a|b".into(), "c".into()],
            rows: vec![
                vec!["x\\y".into(), "p|q".into()],
                vec!["".into(), "z".into()],
            ],
        };
        assert_eq!(Grid::parse(&g.render("\n")).unwrap(), g);
    }

    #[test]
    fn empty_cells_are_kept() {
        let g = Grid::parse("| a | b |\n|---|---|\n|  | 2 |\n| 3 |  |").unwrap();
        assert_eq!(g.rows[0], ["", "2"]);
        assert_eq!(g.rows[1], ["3", ""]);
    }

    #[test]
    fn rejects_malformed_tables() {
        assert_eq!(Grid::parse(""), Err(TableError::Empty));
        assert_eq!(Grid::parse("| a |"), Err(TableError::NoDelimiter));
        assert_eq!(
            Grid::parse("| a | b |\n| x | y |"),
            Err(TableError::NoDelimiter)
        );
        assert_eq!(
            Grid::parse("| a | b |\n|---|---|\n| 1 |"),
            Err(TableError::RaggedRow {
                row: 1,
                want: 2,
                got: 1
            })
        );
        assert!(matches!(
            Grid::parse("| a |\n|---|\n| 1 |\n\n| 2 |"),
            Err(TableError::BlankLine { .. })
        ));
        assert_eq!(
            Grid::parse("| a | a |\n|---|---|"),
            Err(TableError::DuplicateHeader("a".into()))
        );
    }

    #[test]
    fn trailing_blank_lines_are_fine() {
        assert!(Grid::parse("| a |\n|---|\n| 1 |\n\n\n").is_ok());
    }

    #[test]
    fn crlf_bodies_parse() {
        let g = Grid::parse("| a |\r\n|---|\r\n| 1 |\r\n").unwrap();
        assert_eq!(g.rows[0], ["1"]);
    }

    #[test]
    fn hostile_sizes_are_refused() {
        let header = "| a | b | c | d | e | f | g | h | i | j |\n|---|---|---|---|---|---|---|---|---|---|\n";
        let row = "| 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 10 |\n";
        let body = format!("{header}{}", row.repeat(MAX_INLINE_CELLS / 10 + 1));
        assert!(matches!(
            Grid::parse(&body),
            Err(TableError::TooManyCells(_))
        ));
    }

    #[test]
    fn column_types_parse() {
        let t = parse_types("text, int,ugx,pct").unwrap();
        assert_eq!(
            t,
            [
                ColType::Text,
                ColType::Int,
                ColType::Money("ugx".into()),
                ColType::Pct
            ]
        );
        assert_eq!(parse_types("text,water,int"), Err("water".to_string()));
        assert_eq!(parse_types("UGX"), Err("UGX".to_string()));
        assert_eq!(parse_types("text,"), Err(String::new()));
        for t in ["text", "int", "num", "bool", "date", "pct", "ugx"] {
            assert_eq!(ColType::parse(t).unwrap().token(), t);
        }
    }
}
