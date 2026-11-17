//! What an app draws for a `sheet` block: a bounded window of calculated cells, already formatted.
//!
//! The engine, the formats and the window live here once; the phone and the web only draw what this
//! says (plan section 12.3), so neither holds formula or format logic of its own.

use std::path::Path;

use anyhow::{Context, Result, bail};
use hq_suite_calc::{Sheet, Value, column_name};
use hq_suite_md::{BlockType, Document};
use hq_suite_store::SuitePaths;
use serde::Serialize;

/// Rows and columns one view may hold, so a large sheet is read a window at a time.
pub const MAX_ROWS: u32 = 30;
pub const MAX_COLS: u32 = 10;
/// Most cells of a table the view reads.
const MAX_TABLE_ROWS: usize = 100_000;

#[derive(Debug, Clone, Serialize)]
pub struct Column {
    pub letter: String,
    /// The column's declared type (`text`, `int`, `ugx`, ...), or empty.
    pub kind: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CellView {
    pub text: String,
    /// The cell holds a formula.
    pub formula: bool,
    /// The cell is a number, so the app aligns it to the right.
    pub number: bool,
    /// The cell shows an error such as `#DIV/0!`.
    pub error: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SheetView {
    pub name: String,
    pub columns: Vec<Column>,
    pub rows: Vec<Vec<CellView>>,
    /// The window: first row and column shown (zero-based) and the sheet's whole size.
    pub row0: u32,
    pub col0: u32,
    pub row_count: u32,
    pub col_count: u32,
    /// A plain summary for readers that cannot draw a grid.
    pub text: String,
}

/// The rows of a markdown table body. The separator row (`|---|---|`) is dropped.
fn table_rows(body: &str) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    for line in body.lines() {
        let t = line.trim();
        if !t.starts_with('|') {
            continue;
        }
        let inner = t.trim_start_matches('|').trim_end_matches('|');
        let cells: Vec<String> = inner.split('|').map(|c| c.trim().to_string()).collect();
        let separator = cells
            .iter()
            .all(|c| !c.is_empty() && c.chars().all(|ch| matches!(ch, '-' | ':' | ' ')));
        if separator {
            continue;
        }
        out.push(cells);
        if out.len() >= MAX_TABLE_ROWS {
            break;
        }
    }
    out
}

/// `1234567` as `1,234,567`.
fn grouped(n: f64) -> String {
    let neg = n < 0.0;
    let digits = format!("{:.0}", n.abs());
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    if neg { format!("-{out}") } else { out }
}

fn show(value: &Value, kind: &str) -> String {
    match (value, kind) {
        (Value::Num(n), "ugx") => format!("UGX {}", grouped(*n)),
        (Value::Num(n), "int") => grouped(*n),
        (v, _) => v.to_string(),
    }
}

/// The `index`th `sheet` block of the note at `note`, calculated, from (`row0`, `col0`).
pub fn view(vault: &Path, note: &str, index: usize, row0: u32, col0: u32) -> Result<SheetView> {
    let paths = SuitePaths::new(vault);
    let text = std::fs::read_to_string(paths.resolve(note)?)
        .with_context(|| format!("{note} cannot be read"))?;
    let doc = Document::parse(&text);
    let block = doc
        .typed_blocks()
        .filter_map(|(_, b)| b.ok())
        .filter(|b| b.ty == BlockType::Sheet)
        .nth(index);
    let Some(block) = block else {
        bail!("that note has no sheet number {}", index + 1);
    };
    let rows = table_rows(&block.body);
    if rows.is_empty() {
        bail!("that sheet has no table");
    }
    let sheet = Sheet::from_table(&rows);
    let calculated = sheet.calculate();
    let (cols, count) = (
        rows.iter().map(Vec::len).max().unwrap_or(0) as u32,
        rows.len() as u32,
    );
    let kinds: Vec<String> = block
        .info
        .get("types")
        .unwrap_or_default()
        .split(',')
        .map(|s| s.trim().to_ascii_lowercase())
        .collect();
    let kind_of = |c: u32| kinds.get(c as usize).cloned().unwrap_or_default();
    let (r_end, c_end) = ((row0 + MAX_ROWS).min(count), (col0 + MAX_COLS).min(cols));
    let columns = (col0..c_end)
        .map(|c| Column {
            letter: column_name(c),
            kind: kind_of(c),
        })
        .collect();
    let window = (row0..r_end)
        .map(|r| {
            (col0..c_end)
                .map(|c| {
                    let raw = rows.get(r as usize).and_then(|row| row.get(c as usize));
                    let value = calculated.get(&(c, r)).cloned().unwrap_or(Value::Empty);
                    CellView {
                        text: show(&value, &kind_of(c)),
                        formula: raw.is_some_and(|t| t.trim_start().starts_with('=')),
                        number: matches!(value, Value::Num(_)),
                        error: matches!(value, Value::Err(_)),
                    }
                })
                .collect()
        })
        .collect();
    let name = block.info.get("name").unwrap_or("Sheet").to_string();
    Ok(SheetView {
        text: format!("{name}: {count} rows, {cols} columns"),
        name,
        columns,
        rows: window,
        row0,
        col0,
        row_count: count,
        col_count: cols,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOTE: &str = "# Stock\n\n```sheet name=\"Stock\" types=\"text,int,ugx,ugx\"\n| item  | qty | price | total  |\n|-------|-----|-------|--------|\n| Sugar | 40  | 4200  | =B2*C2 |\n| Salt  | 25  | 1800  | =B3*C3 |\n| Total |     |       | =SUM(D2:D3) |\n```\n\nSome text.\n";

    fn vault_with(note: &str) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("Notebooks")).unwrap();
        std::fs::write(d.path().join("Notebooks/stock.md"), note).unwrap();
        d
    }

    #[test]
    fn the_plan_example_is_calculated_and_formatted_by_column_type() {
        let d = vault_with(NOTE);
        let v = view(d.path(), "Notebooks/stock.md", 0, 0, 0).unwrap();
        assert_eq!(v.name, "Stock");
        assert_eq!((v.row_count, v.col_count), (4, 4));
        let cell = |r: usize, c: usize| &v.rows[r][c];
        assert_eq!(cell(0, 0).text, "item");
        assert_eq!(cell(1, 3).text, "UGX 168,000");
        assert!(cell(1, 3).formula && cell(1, 3).number);
        assert_eq!(cell(2, 3).text, "UGX 45,000");
        assert_eq!(cell(3, 3).text, "UGX 213,000");
        assert_eq!(cell(1, 1).text, "40");
        assert!(!cell(1, 0).number);
        assert_eq!(v.columns[3].letter, "D");
        assert_eq!(v.columns[3].kind, "ugx");
    }

    #[test]
    fn a_large_sheet_is_read_a_window_at_a_time() {
        let mut t = String::from("```sheet name=big\n| a | b |\n|---|---|\n");
        for i in 0..200 {
            t.push_str(&format!("| {i} | =A{}*2 |\n", i + 2));
        }
        t.push_str("```\n");
        let d = vault_with(&t);
        let first = view(d.path(), "Notebooks/stock.md", 0, 0, 0).unwrap();
        assert_eq!(first.rows.len(), MAX_ROWS as usize);
        assert_eq!(first.row_count, 201);
        let later = view(d.path(), "Notebooks/stock.md", 0, 100, 0).unwrap();
        assert_eq!(later.rows[0][0].text, "99");
        assert_eq!(later.rows[0][1].text, "198");
        let end = view(d.path(), "Notebooks/stock.md", 0, 190, 0).unwrap();
        assert_eq!(end.rows.len(), 11);
    }

    #[test]
    fn errors_and_missing_blocks_are_said_plainly() {
        let d = vault_with("```sheet\n| a |\n|---|\n| =1/0 |\n```\n");
        let v = view(d.path(), "Notebooks/stock.md", 0, 0, 0).unwrap();
        assert!(v.rows[1][0].error);
        assert_eq!(v.rows[1][0].text, "#DIV/0!");
        assert!(view(d.path(), "Notebooks/stock.md", 1, 0, 0).is_err());
        assert!(view(d.path(), "Notebooks/none.md", 0, 0, 0).is_err());
        assert!(view(d.path(), "../etc/passwd", 0, 0, 0).is_err());
        assert_eq!(grouped(-1234567.4), "-1,234,567");
        assert_eq!(grouped(999.0), "999");
    }
}
