//! Spreadsheets (`.xlsx`, `.xls`, `.ods`) to Markdown tables, one per visible sheet.
//!
//! Values come from `calamine` for Excel files and from the shown text for
//! OpenDocument files. Dates come out as dates, not serial numbers, and number
//! formats are applied: a stored 0.097 under a `0.0%` format reads "9.7%".
//! Excel's old binary `.xls` has no readable format table here, so its numbers
//! are shown as stored unless LibreOffice re-saves the file first.

use std::io::Cursor;

use calamine::{Data, Reader, SheetVisible, open_workbook_auto_from_rs};
use chrono::Timelike;

use super::ir::{Block, Inline, Parsed, Table};
use super::numfmt;
use super::package::{Limits, Package};
use super::report::{LossKind, Losses};
use super::sniff::Kind;
use super::xlsx_formats::{self, CellFormat, CellFormats};
use crate::types::ConvertError;

const MAX_COLUMNS: usize = 100;

/// One visible sheet as rows of cell text.
pub struct Sheet {
    pub name: String,
    pub rows: Vec<Vec<String>>,
}

/// A workbook's visible sheets and how many hidden ones were left out.
#[derive(Default)]
pub struct Book {
    pub sheets: Vec<Sheet>,
    pub hidden: usize,
}

pub fn read(
    bytes: &[u8],
    kind: Kind,
    max_rows: usize,
    limits: &Limits,
) -> Result<Parsed, ConvertError> {
    let mut losses = Losses::default();
    let book = match kind {
        Kind::Ods => {
            let mut pkg = Package::open(bytes, limits)?;
            pkg.check_sizes()?;
            super::ods::read(&mut pkg)?
        }
        Kind::Xlsx => {
            let mut pkg = Package::open(bytes, limits)?;
            pkg.check_sizes()?;
            let formats = xlsx_formats::read(&mut pkg);
            read_calamine(bytes, Some(&formats))?
        }
        _ => {
            losses.note(
                LossKind::NumberFormatsNotApplied,
                "old .xls file: percentages, currency and thousands separators show as plain numbers (LibreOffice, when installed, re-saves the file so they are kept)",
            );
            read_calamine(bytes, None)?
        }
    };
    assemble(book, max_rows, losses)
}

fn read_calamine(bytes: &[u8], formats: Option<&CellFormats>) -> Result<Book, ConvertError> {
    let mut wb = open_workbook_auto_from_rs(Cursor::new(bytes.to_vec()))
        .map_err(|e| ConvertError::InboundFailed(format!("cannot open spreadsheet: {e}")))?;
    let sheets: Vec<(String, bool)> = wb
        .sheets_metadata()
        .iter()
        .map(|s| (s.name.clone(), s.visible == SheetVisible::Visible))
        .collect();
    let mut book = Book::default();
    for (name, is_visible) in &sheets {
        if !is_visible {
            book.hidden += 1;
            continue;
        }
        let range = wb
            .worksheet_range(name)
            .map_err(|e| ConvertError::InboundFailed(format!("cannot read sheet {name}: {e}")))?;
        // The range starts at its first filled cell; styles are keyed by position on the sheet.
        let (top, left) = range.start().unwrap_or((0, 0));
        let rows = range
            .rows()
            .enumerate()
            .map(|(i, r)| {
                r.iter()
                    .enumerate()
                    .map(|(j, d)| {
                        let format =
                            formats.and_then(|f| f.get(name, top + i as u32, left + j as u32));
                        cell_text(d, format)
                    })
                    .collect()
            })
            .collect();
        book.sheets.push(Sheet {
            name: name.clone(),
            rows,
        });
    }
    Ok(book)
}

/// The sheets as Markdown blocks: a heading per sheet when there is more than one,
/// a lone title cell above a table as bold text, and long sheets cut and counted.
fn assemble(book: Book, max_rows: usize, mut losses: Losses) -> Result<Parsed, ConvertError> {
    losses.hit_n(LossKind::HiddenContentSkipped, book.hidden);
    let visible = book.sheets.len();
    let mut blocks: Vec<Block> = Vec::new();
    for Sheet { name, mut rows } in book.sheets {
        trim_empty(&mut rows);
        if rows.is_empty() {
            continue;
        }
        if visible > 1 || !generic_name(&name) {
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

fn cell_text(d: &Data, format: Option<&CellFormat>) -> String {
    let formatted = |f: f64| match format {
        Some(CellFormat::Number(code)) => numfmt::render(code, f),
        _ => None,
    };
    match d {
        Data::Empty => String::new(),
        Data::String(s) => s.trim().to_string(),
        Data::Int(i) => formatted(*i as f64).unwrap_or_else(|| i.to_string()),
        Data::Float(f) => formatted(*f).unwrap_or_else(|| number(*f)),
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
                    Some(t) => date_text(t, dt.as_f64(), format),
                    None => number(dt.as_f64()),
                }
            }
        }
        Data::DateTimeIso(s) | Data::DurationIso(s) => s.clone(),
        Data::Error(e) => format!("#{e:?}"),
    }
}

/// A date and time the way the cell's format shows them: only the date, only the time, or both.
/// Without a format, a value under one day is a time of day, midnight is a plain date.
fn date_text(t: chrono::NaiveDateTime, serial: f64, format: Option<&CellFormat>) -> String {
    let (date, time, seconds) = match format {
        Some(CellFormat::Date(k)) => (k.date, k.time, k.seconds),
        _ if serial.abs() < 1.0 => (false, true, t.time().second() != 0),
        _ => (
            true,
            t.time() != chrono::NaiveTime::MIN,
            t.time().second() != 0,
        ),
    };
    let clock = if seconds { "%H:%M:%S" } else { "%H:%M" };
    match (date, time) {
        (true, true) => t.format(&format!("%Y-%m-%d {clock}")).to_string(),
        (false, true) => t.format(clock).to_string(),
        _ => t.format("%Y-%m-%d").to_string(),
    }
}

/// A number without float noise: 0.30000000000000004 prints as 0.3, 42.0 as 42.
pub(super) fn number(f: f64) -> String {
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
        for kind in [Kind::Xlsx, Kind::Xls, Kind::Ods] {
            assert!(read(b"not a spreadsheet", kind, 10, &Limits::default()).is_err());
        }
    }

    const CT: &str = r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/></Types>"#;

    /// A one-sheet workbook whose data starts at C3, with five number formats.
    fn workbook() -> Vec<u8> {
        let main = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
        let rel = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
        let parts: Vec<(&str, Vec<u8>)> = vec![
            ("[Content_Types].xml", CT.as_bytes().to_vec()),
            (
                "_rels/.rels",
                format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{rel}/officeDocument" Target="xl/workbook.xml"/></Relationships>"#).into_bytes(),
            ),
            (
                "xl/workbook.xml",
                format!(r#"<workbook xmlns="{main}" xmlns:r="{rel}"><sheets><sheet name="Budget" sheetId="1" r:id="rId1"/></sheets></workbook>"#).into_bytes(),
            ),
            (
                "xl/_rels/workbook.xml.rels",
                format!(r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="{rel}/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#).into_bytes(),
            ),
            (
                "xl/styles.xml",
                format!(r##"<styleSheet xmlns="{main}"><numFmts count="2"><numFmt numFmtId="164" formatCode="&quot;UGX&quot; #,##0"/><numFmt numFmtId="165" formatCode="#,##0;(#,##0)"/></numFmts><cellXfs count="5"><xf numFmtId="0"/><xf numFmtId="9"/><xf numFmtId="164"/><xf numFmtId="165"/><xf numFmtId="2"/></cellXfs></styleSheet>"##).into_bytes(),
            ),
            (
                "xl/worksheets/sheet1.xml",
                format!(
                    r#"<worksheet xmlns="{main}"><sheetData>
                    <row r="3"><c r="C3" t="inlineStr"><is><t>Item</t></is></c><c r="D3" t="inlineStr"><is><t>Value</t></is></c></row>
                    <row r="4"><c r="C4" t="inlineStr"><is><t>Margin</t></is></c><c r="D4" s="1"><v>0.256</v></c></row>
                    <row r="5"><c r="C5" t="inlineStr"><is><t>Rent</t></is></c><c r="D5" s="2"><v>1500000</v></c></row>
                    <row r="6"><c r="C6" t="inlineStr"><is><t>Loss</t></is></c><c r="D6" s="3"><v>-2500</v></c></row>
                    <row r="7"><c r="C7" t="inlineStr"><is><t>Rate</t></is></c><c r="D7" s="4"><v>3.14159</v></c></row>
                    <row r="8"><c r="C8" t="inlineStr"><is><t>Plain</t></is></c><c r="D8" s="0"><v>12.5</v></c></row>
                    </sheetData></worksheet>"#
                )
                .into_bytes(),
            ),
        ];
        crate::ingest::testutil::zip_of(&parts)
    }

    #[test]
    fn formats_are_found_by_position_even_when_the_data_starts_below_and_right_of_a1() {
        let parsed = read(&workbook(), Kind::Xlsx, 100, &Limits::default()).unwrap();
        let md = crate::ingest::ir::render(parsed.blocks);
        for want in [
            "| Margin | 26% |",
            "| Rent | UGX 1,500,000 |",
            "| Loss | (2,500) |",
            "| Rate | 3.14 |",
            "| Plain | 12.5 |",
        ] {
            assert!(md.contains(want), "{want}:\n{md}");
        }
    }

    #[test]
    fn a_workbook_that_claims_to_unpack_past_the_limit_is_refused_before_calamine_sees_it() {
        let tiny = Limits {
            max_total_bytes: 200,
            ..Limits::default()
        };
        let err = read(&workbook(), Kind::Xlsx, 100, &tiny).unwrap_err();
        assert!(matches!(err, ConvertError::TooLarge(_)), "{err}");
    }

    #[test]
    fn a_time_of_day_is_not_shown_as_1899_and_a_format_decides_what_is_shown() {
        use crate::ingest::numfmt::DateKind;
        let at = |y, mo, d, h, mi| {
            chrono::NaiveDate::from_ymd_opt(y, mo, d)
                .unwrap()
                .and_hms_opt(h, mi, 0)
                .unwrap()
        };
        // 0.65 of a day, no format: a time, not 1899-12-30 15:36.
        assert_eq!(date_text(at(1899, 12, 30, 15, 36), 0.65, None), "15:36");
        // Midnight with no format is a plain date; a clock time on a date shows both.
        assert_eq!(date_text(at(2026, 7, 1, 0, 0), 46204.0, None), "2026-07-01");
        assert_eq!(
            date_text(at(2026, 7, 1, 9, 30), 46204.4, None),
            "2026-07-01 09:30"
        );
        // A date format shows the date even when the stored value has a time.
        let date_only = CellFormat::Date(DateKind {
            date: true,
            time: false,
            seconds: false,
        });
        assert_eq!(
            date_text(at(2026, 7, 1, 9, 30), 46204.4, Some(&date_only)),
            "2026-07-01"
        );
        let time_only = CellFormat::Date(DateKind {
            date: false,
            time: true,
            seconds: false,
        });
        assert_eq!(
            date_text(at(2026, 7, 1, 9, 30), 46204.4, Some(&time_only)),
            "09:30"
        );
    }
}
