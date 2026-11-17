//! What an app draws for a `sheet` block: a bounded window of calculated cells, already formatted.
//!
//! The engine, the formats and the window live here once; the phone and the web only draw what this
//! says (plan section 12.3), so neither holds formula or format logic of its own.

use std::path::Path;

use anyhow::{Context, Result, bail};
use hq_suite_calc::{Value, Workbook, column_name};
use hq_suite_md::{BlockType, Document, RangeRef, TypedBlock};
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

/// Every `sheet` block of a note as one workbook, so a formula can read another block by its name
/// (`=Prices!B2`). A block without a `name` is `Sheet1`, `Sheet2` and so on, in order.
struct NoteBook {
    book: Workbook,
    /// The table of each sheet block, in note order; its number here is its sheet number.
    tables: Vec<Vec<Vec<String>>>,
}

impl NoteBook {
    fn new(blocks: &[TypedBlock]) -> NoteBook {
        let sheets: Vec<&TypedBlock> = blocks.iter().filter(|b| b.ty == BlockType::Sheet).collect();
        let names: Vec<String> = sheets
            .iter()
            .enumerate()
            .map(|(i, b)| {
                b.info
                    .get("name")
                    .map_or_else(|| format!("Sheet{}", i + 1), str::to_string)
            })
            .collect();
        let mut book = Workbook::new();
        for name in &names {
            book.add_sheet(name);
        }
        let tables: Vec<_> = sheets.iter().map(|b| table_rows(&b.body)).collect();
        for (id, table) in tables.iter().enumerate() {
            for (r, row) in table.iter().enumerate() {
                for (c, text) in row.iter().enumerate().filter(|(_, t)| !t.trim().is_empty()) {
                    book.set(id, c as u32, r as u32, text);
                }
            }
        }
        NoteBook { book, tables }
    }
}

/// The `index`th `sheet` block of the note at `note`, calculated, from (`row0`, `col0`).
pub fn view(vault: &Path, note: &str, index: usize, row0: u32, col0: u32) -> Result<SheetView> {
    let paths = SuitePaths::new(vault);
    let text = std::fs::read_to_string(paths.resolve(note)?)
        .with_context(|| format!("{note} cannot be read"))?;
    let doc = Document::parse(&text);
    let blocks: Vec<TypedBlock> = doc.typed_blocks().filter_map(|(_, b)| b.ok()).collect();
    let note_book = NoteBook::new(&blocks);
    let Some(rows) = note_book.tables.get(index) else {
        bail!("that note has no sheet number {}", index + 1);
    };
    if rows.is_empty() {
        bail!("that sheet has no table");
    }
    let block = blocks
        .iter()
        .filter(|b| b.ty == BlockType::Sheet)
        .nth(index)
        .context("that sheet is not in the note")?;
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
                    let value = note_book.book.get(index, c, r);
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

#[derive(Debug, Clone, Serialize)]
pub struct TileView {
    pub label: String,
    /// The number, already formatted (`UGX 213,000`).
    pub text: String,
    /// How many numeric cells the function read.
    pub cells: u32,
}

/// The `index`th `tile` block of the note, computed over a range of a sheet in the same note.
pub fn tile(vault: &Path, note: &str, index: usize) -> Result<TileView> {
    let paths = SuitePaths::new(vault);
    let text = std::fs::read_to_string(paths.resolve(note)?)
        .with_context(|| format!("{note} cannot be read"))?;
    let doc = Document::parse(&text);
    let blocks: Vec<_> = doc.typed_blocks().filter_map(|(_, b)| b.ok()).collect();
    let Some(tile) = blocks.iter().filter(|b| b.ty == BlockType::Tile).nth(index) else {
        bail!("that note has no tile number {}", index + 1);
    };
    let from = RangeRef::parse(tile.info.get("from").unwrap_or_default())
        .map_err(|e| anyhow::anyhow!("from=: {e}"))?;
    let stem = note
        .rsplit('/')
        .next()
        .unwrap_or(note)
        .trim_end_matches(".md");
    if from.note.as_deref().is_some_and(|n| n != stem && n != note) {
        bail!("a tile reads a sheet in its own note for now");
    }
    let note_book = NoteBook::new(&blocks);
    let wanted = from.block.as_deref();
    let sheet_id = blocks
        .iter()
        .filter(|b| b.ty == BlockType::Sheet)
        .position(|b| {
            wanted.is_none_or(|w| b.info.get("name") == Some(w) || b.info.get("id") == Some(w))
        })
        .context("the tile's sheet is not in this note")?;
    let mut numbers = Vec::new();
    for r in from.start.row..=from.end.row {
        for c in from.start.col..=from.end.col {
            if let Value::Num(n) = note_book.book.get(sheet_id, c - 1, r - 1) {
                numbers.push(n);
            }
        }
    }
    let function = tile.info.get("fn").unwrap_or("sum");
    let result = match function {
        "count" => numbers.len() as f64,
        _ if numbers.is_empty() => bail!("the range has no numbers"),
        "sum" => numbers.iter().sum(),
        "avg" => numbers.iter().sum::<f64>() / numbers.len() as f64,
        "min" => numbers.iter().copied().fold(f64::INFINITY, f64::min),
        "max" => numbers.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        "median" => {
            numbers.sort_by(f64::total_cmp);
            let mid = numbers.len() / 2;
            if numbers.len() % 2 == 1 {
                numbers[mid]
            } else {
                (numbers[mid - 1] + numbers[mid]) / 2.0
            }
        }
        other => bail!("`{other}` is not a tile function"),
    };
    let kind = tile.info.get("format").unwrap_or_default();
    Ok(TileView {
        label: tile.info.get("label").unwrap_or(function).to_string(),
        text: show(&Value::Num(result), kind),
        cells: numbers.len() as u32,
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
    fn a_tile_computes_one_number_from_a_range_and_formats_it() {
        let note = format!(
            "{NOTE}\n```tile from=\"[[stock]]!D2:D3\" fn=sum label=\"Stock value\" format=ugx\n```\n\n```tile from=\"D2:D3\" fn=avg\n```\n\n```tile from=\"B2:B3\" fn=max label=Largest\n```\n"
        );
        let d = vault_with(&note);
        let t0 = tile(d.path(), "Notebooks/stock.md", 0).unwrap();
        assert_eq!(
            (t0.label.as_str(), t0.text.as_str(), t0.cells),
            ("Stock value", "UGX 213,000", 2)
        );
        assert_eq!(
            tile(d.path(), "Notebooks/stock.md", 1).unwrap().text,
            "106500"
        );
        assert_eq!(tile(d.path(), "Notebooks/stock.md", 2).unwrap().text, "40");
        assert!(tile(d.path(), "Notebooks/stock.md", 3).is_err());
    }

    #[test]
    fn sheet_blocks_of_one_note_read_each_other_by_name() {
        let note = "```sheet name=Prices\n| item | price |\n|---|---|\n| Sugar | 4200 |\n| Salt | 1800 |\n```\n\n```sheet name=\"Stock take\" types=\"text,int,ugx\"\n| item | qty | value |\n|---|---|---|\n| Sugar | 40 | =B2*Prices!B2 |\n| Salt | 25 | =B3*prices!B3 |\n```\n\n```sheet\n| x |\n|---|\n| =SUM('Stock take'!C2:C3) |\n```\n\n```tile from=\"[[stock]]!C2:C3\" fn=sum label=Value\n```\n";
        let d = vault_with(note);
        let second = view(d.path(), "Notebooks/stock.md", 1, 0, 0).unwrap();
        assert_eq!(second.rows[1][2].text, "UGX 168,000");
        assert_eq!(second.rows[2][2].text, "UGX 45,000");
        let third = view(d.path(), "Notebooks/stock.md", 2, 0, 0).unwrap();
        assert_eq!(
            third.rows[1][0].text, "213000",
            "an unnamed block is Sheet3 and can read the others"
        );
        assert!(
            tile(d.path(), "Notebooks/stock.md", 0).is_err(),
            "a tile over a range with no numbers in the first sheet says so"
        );
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
