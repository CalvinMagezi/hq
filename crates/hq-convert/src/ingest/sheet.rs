//! Spreadsheets (`.xlsx`, `.xls`, `.ods`) to Markdown tables, one per visible
//! sheet, through the `calamine` reader.
//!
//! Dates come out as dates, not serial numbers. Values are shown as stored:
//! number formats such as percentages and currency symbols are not applied,
//! because the reader does not expose them.

use std::io::Cursor;

use calamine::{Data, Reader, SheetVisible, open_workbook_auto_from_rs};

use super::ir::{Block, Inline, Parsed, Table};
use super::report::{LossKind, Losses};
use crate::types::ConvertError;

const MAX_COLUMNS: usize = 100;

pub fn read(bytes: &[u8], max_rows: usize) -> Result<Parsed, ConvertError> {
    let mut wb = open_workbook_auto_from_rs(Cursor::new(bytes.to_vec()))
        .map_err(|e| ConvertError::InboundFailed(format!("cannot open spreadsheet: {e}")))?;
    let sheets: Vec<(String, bool)> = wb
        .sheets_metadata()
        .iter()
        .map(|s| (s.name.clone(), s.visible == SheetVisible::Visible))
        .collect();
    let visible = sheets.iter().filter(|(_, v)| *v).count();
    let mut losses = Losses::default();
    let mut blocks: Vec<Block> = Vec::new();
    for (name, is_visible) in &sheets {
        if !is_visible {
            losses.hit(LossKind::HiddenContentSkipped);
            continue;
        }
        let range = wb
            .worksheet_range(name)
            .map_err(|e| ConvertError::InboundFailed(format!("cannot read sheet {name}: {e}")))?;
        let mut rows: Vec<Vec<String>> = range
            .rows()
            .map(|r| r.iter().map(cell_text).collect())
            .collect();
        trim_empty(&mut rows);
        if rows.is_empty() {
            continue;
        }
        if visible > 1 || !generic_name(name) {
            blocks.push(Block::Heading(2, vec![Inline::text(name.clone())]));
        }
        // A lone cell above the table is a title, not a header.
        if rows.len() > 2 && non_empty(&rows[0]) == 1 && non_empty(&rows[1]) >= 2 {
            let title = rows
                .remove(0)
                .into_iter()
                .find(|c| !c.is_empty())
                .unwrap_or_default();
            blocks.push(Block::Para(vec![Inline::Run(super::ir::Run {
                text: title,
                bold: true,
                ..Default::default()
            })]));
        }
        let total = rows.len();
        if total > max_rows + 1 {
            rows.truncate(max_rows + 1);
            losses.note(
                LossKind::Truncated,
                format!(
                    "sheet {name}: showing the first {max_rows} of {} data rows",
                    total - 1
                ),
            );
        }
        if rows.iter().map(Vec::len).max().unwrap_or(0) > MAX_COLUMNS {
            for r in &mut rows {
                r.truncate(MAX_COLUMNS);
            }
            losses.note(
                LossKind::Truncated,
                format!("sheet {name}: showing the first {MAX_COLUMNS} columns"),
            );
        }
        blocks.push(Block::Table(Table {
            rows: rows
                .into_iter()
                .map(|r| r.into_iter().map(|c| text_cell(&c)).collect())
                .collect(),
        }));
    }
    if blocks.is_empty() {
        return Err(ConvertError::InboundFailed(
            "no data found in any sheet".into(),
        ));
    }
    Ok(Parsed {
        blocks,
        title: None,
        losses,
    })
}

fn non_empty(row: &[String]) -> usize {
    row.iter().filter(|c| !c.is_empty()).count()
}

fn generic_name(name: &str) -> bool {
    let n = name.trim().to_lowercase();
    [
        "sheet", "blad", "feuil", "hoja", "tabelle", "munka", "foglio", "planilha",
    ]
    .iter()
    .any(|p| {
        n.strip_prefix(p)
            .is_some_and(|rest| rest.trim().chars().all(|c| c.is_ascii_digit()))
    })
}

/// Drops empty rows at the end and at the start, and empty columns at the end.
fn trim_empty(rows: &mut Vec<Vec<String>>) {
    while rows.last().is_some_and(|r| r.iter().all(String::is_empty)) {
        rows.pop();
    }
    let lead = rows
        .iter()
        .take_while(|r| r.iter().all(String::is_empty))
        .count();
    rows.drain(..lead);
    // Blank rows in the middle only separate blocks of the sheet; keep the rows.
    rows.retain(|r| r.iter().any(|c| !c.is_empty()));
    let width = rows
        .iter()
        .map(|r| r.iter().rposition(|c| !c.is_empty()).map_or(0, |i| i + 1))
        .max()
        .unwrap_or(0);
    for r in rows.iter_mut() {
        r.truncate(width);
        r.resize(width, String::new());
    }
}

/// Cell text; embedded newlines become line breaks in the table cell.
fn text_cell(s: &str) -> Vec<Inline> {
    let mut out = Vec::new();
    for (i, line) in s.split('\n').enumerate() {
        if i > 0 {
            out.push(Inline::Break);
        }
        if !line.is_empty() {
            out.push(Inline::text(line.trim_end_matches('\r')));
        }
    }
    out
}

fn cell_text(d: &Data) -> String {
    match d {
        Data::Empty => String::new(),
        Data::String(s) => s.trim().to_string(),
        Data::Int(i) => i.to_string(),
        Data::Float(f) => number(*f),
        Data::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
        Data::DateTime(dt) => {
            if dt.is_duration() {
                match dt.as_duration() {
                    Some(d) => {
                        let secs = d.num_seconds();
                        format!("{}:{:02}:{:02}", secs / 3600, (secs / 60) % 60, secs % 60)
                    }
                    None => number(dt.as_f64()),
                }
            } else {
                match dt.as_datetime() {
                    Some(t) if t.time() == chrono::NaiveTime::MIN => {
                        t.format("%Y-%m-%d").to_string()
                    }
                    Some(t) => t.format("%Y-%m-%d %H:%M").to_string(),
                    None => number(dt.as_f64()),
                }
            }
        }
        Data::DateTimeIso(s) | Data::DurationIso(s) => s.clone(),
        Data::Error(e) => format!("#{e:?}"),
    }
}

/// A number without float noise: 0.30000000000000004 prints as 0.3, 42.0 as 42.
fn number(f: f64) -> String {
    if !f.is_finite() {
        return f.to_string();
    }
    if f.fract() == 0.0 && f.abs() < 1e15 {
        return format!("{}", f as i64);
    }
    let s = format!("{f:.10}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_print_without_noise() {
        assert_eq!(number(42.0), "42");
        assert_eq!(number(0.1 + 0.2), "0.3");
        assert_eq!(number(1234567.5), "1234567.5");
        assert_eq!(number(-3.0), "-3");
    }

    #[test]
    fn empty_edges_are_trimmed_and_rows_padded() {
        let mut rows = vec![
            vec!["".to_string(), "".to_string(), "".to_string()],
            vec!["a".to_string(), "b".to_string(), "".to_string()],
            vec!["c".to_string(), "".to_string(), "".to_string()],
            vec!["".to_string(), "".to_string(), "".to_string()],
        ];
        trim_empty(&mut rows);
        assert_eq!(rows, vec![vec!["a", "b"], vec!["c", ""]]);
        let mut gap = vec![
            vec!["a".to_string()],
            vec![String::new()],
            vec!["b".to_string()],
        ];
        trim_empty(&mut gap);
        assert_eq!(gap, vec![vec!["a"], vec!["b"]]);
    }

    #[test]
    fn default_sheet_names_are_not_worth_a_heading() {
        assert!(generic_name("Sheet1"));
        assert!(generic_name("Sheet 3"));
        assert!(generic_name("Munka1"));
        assert!(!generic_name("Q3 Sales"));
    }

    #[test]
    fn something_that_is_not_a_workbook_is_an_error() {
        assert!(read(b"not a spreadsheet", 10).is_err());
    }
}
