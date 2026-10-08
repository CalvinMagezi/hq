//! The tables in a note, as CSV, JSON, JSONL, XML, LaTeX or XLSX.
//!
//! A note may hold several tables. Formats that can carry only one table
//! (CSV, and LaTeX files) return a zip when there are several; the rest keep
//! them together. A note with no tables is an error, not an empty file.

use std::collections::HashSet;

use rust_xlsxwriter::{Format as XlsxFormat, Workbook};

use crate::doc::{Align, Document, Table, plain_text};
use crate::error::ExportError;
use crate::util::{defuse_formula, parse_number, slug, zip_files};

/// One table with its resolved name and cleaned-up cells.
pub(crate) struct Grid {
    pub name: String,
    pub aligns: Vec<Align>,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

pub(crate) struct Packed {
    pub bytes: Vec<u8>,
    pub extension: &'static str,
    pub mime: &'static str,
}

fn cell_text(inlines: &[crate::doc::Inline]) -> String {
    plain_text(inlines)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Every table in reading order. Names come from the nearest heading and are
/// made unique; header cells that are empty or repeated get distinct names so
/// they can serve as record keys.
pub(crate) fn grids(doc: &Document) -> Result<Vec<Grid>, ExportError> {
    let tables = doc.tables();
    if tables.is_empty() {
        return Err(ExportError::Unsupported(
            "this note has no tables to export".into(),
        ));
    }
    let mut used: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    for (i, (heading, table)) in tables.into_iter().enumerate() {
        out.push(grid(i, heading, table, &mut used));
    }
    Ok(out)
}

fn grid(index: usize, heading: Option<String>, t: &Table, used: &mut HashSet<String>) -> Grid {
    let n = t.columns();
    let mut name = heading
        .map(|h| h.trim().to_owned())
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| format!("Table {}", index + 1));
    if !used.insert(name.to_lowercase()) {
        name = format!("{name} {}", index + 1);
        used.insert(name.to_lowercase());
    }

    let mut seen: HashSet<String> = HashSet::new();
    let columns: Vec<String> = (0..n)
        .map(|i| {
            let raw = t.header.get(i).map(|c| cell_text(c)).unwrap_or_default();
            let base = if raw.is_empty() {
                format!("Column {}", i + 1)
            } else {
                raw
            };
            let mut unique = base.clone();
            let mut k = 2;
            while !seen.insert(unique.clone()) {
                unique = format!("{base}_{k}");
                k += 1;
            }
            unique
        })
        .collect();
    let rows = t
        .rows
        .iter()
        .map(|r| {
            (0..n)
                .map(|i| r.get(i).map(|c| cell_text(c)).unwrap_or_default())
                .collect()
        })
        .collect();
    Grid {
        name,
        aligns: (0..n)
            .map(|i| t.aligns.get(i).copied().unwrap_or(Align::Left))
            .collect(),
        columns,
        rows,
    }
}

fn file_name(index: usize, grid: &Grid, ext: &str) -> String {
    format!("{:02}-{}.{ext}", index + 1, slug(&grid.name, "table"))
}

// ---------------------------------------------------------------- CSV

fn csv_bytes(g: &Grid) -> Result<Vec<u8>, ExportError> {
    let mut w = csv::WriterBuilder::new().from_writer(Vec::new());
    let safe = |cells: &[String]| -> Vec<String> { cells.iter().map(|c| defuse_formula(c)).collect() };
    w.write_record(safe(&g.columns))
        .map_err(|e| ExportError::Render(e.to_string()))?;
    for row in &g.rows {
        w.write_record(safe(row))
            .map_err(|e| ExportError::Render(e.to_string()))?;
    }
    w.into_inner()
        .map_err(|e| ExportError::Render(e.to_string()))
}

pub(crate) fn csv(doc: &Document) -> Result<Packed, ExportError> {
    let grids = grids(doc)?;
    if let [only] = grids.as_slice() {
        return Ok(Packed {
            bytes: csv_bytes(only)?,
            extension: "csv",
            mime: "text/csv",
        });
    }
    let files = grids
        .iter()
        .enumerate()
        .map(|(i, g)| Ok((file_name(i, g, "csv"), csv_bytes(g)?)))
        .collect::<Result<Vec<_>, ExportError>>()?;
    Ok(Packed {
        bytes: zip_files(files)?,
        extension: "zip",
        mime: "application/zip",
    })
}

// ---------------------------------------------------------------- JSON

fn json_value(cell: &str) -> String {
    match parse_number(cell) {
        // `{}` keeps integers integral and never emits exponent forms for
        // values that came from at most 15 plain digits.
        Some(n) => format!("{n}"),
        None => serde_json::to_string(cell).unwrap_or_else(|_| "\"\"".into()),
    }
}

fn json_key(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into())
}

/// `{"col":value,...}`, keys in column order (a map type would sort them).
fn json_record(g: &Grid, row: &[String], extra: Option<(&str, &str)>) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some((k, v)) = extra {
        parts.push(format!("{}:{}", json_key(k), json_key(v)));
    }
    for (c, v) in g.columns.iter().zip(row) {
        parts.push(format!("{}:{}", json_key(c), json_value(v)));
    }
    format!("{{{}}}", parts.join(","))
}

fn json_array(g: &Grid, indent: &str) -> String {
    if g.rows.is_empty() {
        return "[]".to_owned();
    }
    let lines: Vec<String> = g
        .rows
        .iter()
        .map(|r| format!("{indent}  {}", json_record(g, r, None)))
        .collect();
    format!("[\n{}\n{indent}]", lines.join(",\n"))
}

/// One table: an array of records. Several: an object of arrays, keyed by name.
pub(crate) fn json(doc: &Document) -> Result<Packed, ExportError> {
    let grids = grids(doc)?;
    let text = if let [only] = grids.as_slice() {
        json_array(only, "")
    } else {
        let members: Vec<String> = grids
            .iter()
            .map(|g| format!("  {}: {}", json_key(&g.name), json_array(g, "  ")))
            .collect();
        format!("{{\n{}\n}}", members.join(",\n"))
    };
    Ok(Packed {
        bytes: format!("{text}\n").into_bytes(),
        extension: "json",
        mime: "application/json",
    })
}

/// One record per line. With several tables each record is tagged `_table`.
pub(crate) fn jsonl(doc: &Document) -> Result<Packed, ExportError> {
    let grids = grids(doc)?;
    let tag = grids.len() > 1;
    let mut out = String::new();
    for g in &grids {
        for row in &g.rows {
            let extra = tag.then_some(("_table", g.name.as_str()));
            out.push_str(&json_record(g, row, extra));
            out.push('\n');
        }
    }
    Ok(Packed {
        bytes: out.into_bytes(),
        extension: "jsonl",
        mime: "application/x-ndjson",
    })
}

// ---------------------------------------------------------------- XML

fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // XML 1.0 forbids most control characters outright.
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
            '\u{fffe}' | '\u{ffff}' => {}
            c => out.push(c),
        }
    }
    out
}

/// Column names go in an attribute rather than becoming element names, so a
/// header like `Cost (UGX)` needs no sanitising.
pub(crate) fn xml(doc: &Document) -> Result<Packed, ExportError> {
    let grids = grids(doc)?;
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<tables>\n");
    for g in &grids {
        out.push_str(&format!("  <table name=\"{}\">\n", xml_escape(&g.name)));
        for row in &g.rows {
            out.push_str("    <row>\n");
            for (c, v) in g.columns.iter().zip(row) {
                out.push_str(&format!(
                    "      <cell column=\"{}\">{}</cell>\n",
                    xml_escape(c),
                    xml_escape(v)
                ));
            }
            out.push_str("    </row>\n");
        }
        out.push_str("  </table>\n");
    }
    out.push_str("</tables>\n");
    Ok(Packed {
        bytes: out.into_bytes(),
        extension: "xml",
        mime: "application/xml",
    })
}

// ---------------------------------------------------------------- LaTeX

fn latex_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\textbackslash{}"),
            '&' | '%' | '$' | '#' | '_' | '{' | '}' => {
                out.push('\\');
                out.push(c);
            }
            '~' => out.push_str("\\textasciitilde{}"),
            '^' => out.push_str("\\textasciicircum{}"),
            c => out.push(c),
        }
    }
    out
}

fn latex_table(g: &Grid) -> String {
    let spec: String = g
        .aligns
        .iter()
        .map(|a| match a {
            Align::Left => 'l',
            Align::Center => 'c',
            Align::Right => 'r',
        })
        .collect();
    let row = |cells: Vec<String>| format!("{} \\\\\n", cells.join(" & "));
    let mut out = format!("% {}\n", g.name.replace('\n', " "));
    out.push_str(&format!("\\begin{{tabular}}{{{spec}}}\n\\hline\n"));
    out.push_str(&row(
        g.columns
            .iter()
            .map(|c| format!("\\textbf{{{}}}", latex_escape(c)))
            .collect(),
    ));
    out.push_str("\\hline\n");
    for r in &g.rows {
        out.push_str(&row(r.iter().map(|c| latex_escape(c)).collect()));
    }
    out.push_str("\\hline\n\\end{tabular}\n");
    out
}

/// `tabular` environments, ready to paste into a document. Several tables are
/// kept in one file, each under a `%` comment naming it.
pub(crate) fn latex(doc: &Document) -> Result<Packed, ExportError> {
    let grids = grids(doc)?;
    let text: Vec<String> = grids.iter().map(latex_table).collect();
    Ok(Packed {
        bytes: text.join("\n").into_bytes(),
        extension: "tex",
        mime: "application/x-tex",
    })
}

// ---------------------------------------------------------------- XLSX

/// Excel sheet names: at most 31 characters, none of `[]:*?/\`, unique
/// ignoring case, never empty.
fn sheet_name(raw: &str, index: usize, used: &mut HashSet<String>) -> String {
    let cleaned: String = raw
        .chars()
        .filter(|c| !matches!(c, '[' | ']' | ':' | '*' | '?' | '/' | '\\'))
        .collect();
    let cleaned = cleaned.trim().trim_matches('\'').trim();
    let mut base: String = cleaned.chars().take(31).collect();
    if base.is_empty() {
        base = format!("Sheet {}", index + 1);
    }
    let mut name = base.clone();
    let mut k = 2;
    while !used.insert(name.to_lowercase()) {
        let suffix = format!(" {k}");
        let keep = 31usize.saturating_sub(suffix.chars().count());
        name = format!("{}{suffix}", base.chars().take(keep).collect::<String>());
        k += 1;
    }
    name
}

pub(crate) fn xlsx(doc: &Document) -> Result<Packed, ExportError> {
    let grids = grids(doc)?;
    let fail = |e: rust_xlsxwriter::XlsxError| ExportError::Render(e.to_string());
    let mut workbook = Workbook::new();
    let header = XlsxFormat::new().set_bold();
    let mut used = HashSet::new();

    for (i, g) in grids.iter().enumerate() {
        let sheet = workbook.add_worksheet();
        sheet.set_name(sheet_name(&g.name, i, &mut used)).map_err(fail)?;
        for (c, name) in g.columns.iter().enumerate() {
            sheet
                .write_string_with_format(0, c as u16, name, &header)
                .map_err(fail)?;
        }
        for (r, row) in g.rows.iter().enumerate() {
            for (c, value) in row.iter().enumerate() {
                let (r, c) = (r as u32 + 1, c as u16);
                // Strings are written as strings, never parsed as formulas, so
                // `=cmd()` in a note stays text in the sheet.
                match parse_number(value) {
                    Some(n) => sheet.write_number(r, c, n).map_err(fail)?,
                    None if value.is_empty() => continue,
                    None => sheet.write_string(r, c, value).map_err(fail)?,
                };
            }
        }
        sheet.set_freeze_panes(1, 0).map_err(fail)?;
        sheet.set_autofit_max_width(60);
        sheet.autofit();
    }
    Ok(Packed {
        bytes: workbook.save_to_buffer().map_err(fail)?,
        extension: "xlsx",
        mime: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(md: &str) -> Document {
        Document::from_markdown("t", md)
    }

    const ONE: &str = "## Sites\n\n| Site | Farmers | Note |\n|:--|--:|---|\n| Mukono | 42 | =HYPERLINK(\"x\") |\n| Wakiso | 007 | has, comma and \"quote\" |\n";
    const TWO: &str = "## Sites\n\n| a | b |\n|---|---|\n| 1 | x |\n\n## Sites\n\n| c |\n|---|\n| 2 |\n";

    fn text(p: Packed) -> String {
        String::from_utf8(p.bytes).unwrap()
    }

    #[test]
    fn no_tables_is_a_clear_error() {
        let err = csv(&doc("just text")).err().expect("error");
        assert!(err.to_string().contains("no tables"));
    }

    #[test]
    fn a_single_table_is_plain_csv_with_quoting_and_defused_formulas() {
        let p = csv(&doc(ONE)).unwrap();
        assert_eq!(p.extension, "csv");
        let t = text(p);
        assert!(t.starts_with("Site,Farmers,Note\n"));
        assert!(t.contains("'=HYPERLINK(\"\"x\"\")"), "formula must be defused: {t}");
        assert!(t.contains("\"has, comma and \"\"quote\"\"\""), "{t}");
        assert!(t.contains("Wakiso,007,"), "leading zeros must survive: {t}");
    }

    #[test]
    fn several_tables_become_a_zip_of_csv_files() {
        let p = csv(&doc(TWO)).unwrap();
        assert_eq!(p.extension, "zip");
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(p.bytes)).unwrap();
        let names: Vec<String> = (0..z.len())
            .map(|i| z.by_index(i).unwrap().name().to_owned())
            .collect();
        assert_eq!(names.len(), 2);
        assert!(names[0].starts_with("01-") && names[1].starts_with("02-"), "{names:?}");
        assert_ne!(names[0], names[1]);
    }

    #[test]
    fn json_keeps_column_order_and_types() {
        let t = text(json(&doc(ONE)).unwrap());
        let first_line = t.lines().nth(1).unwrap().trim().trim_end_matches(',');
        assert_eq!(
            first_line,
            "{\"Site\":\"Mukono\",\"Farmers\":42,\"Note\":\"=HYPERLINK(\\\"x\\\")\"}"
        );
        assert!(t.contains("\"Farmers\":\"007\""), "leading zeros stay text: {t}");
        serde_json::from_str::<serde_json::Value>(&t).expect("valid json");
    }

    #[test]
    fn json_with_several_tables_is_an_object_with_unique_keys() {
        let t = text(json(&doc(TWO)).unwrap());
        let v: serde_json::Value = serde_json::from_str(&t).expect("valid json");
        assert_eq!(v.as_object().unwrap().len(), 2, "{t}");
    }

    #[test]
    fn jsonl_is_one_record_per_line_and_tags_tables_when_there_are_several() {
        let one = text(jsonl(&doc(ONE)).unwrap());
        assert_eq!(one.lines().count(), 2);
        assert!(!one.contains("_table"));
        let two = text(jsonl(&doc(TWO)).unwrap());
        assert!(two.lines().all(|l| l.starts_with("{\"_table\":")), "{two}");
    }

    #[test]
    fn duplicate_and_empty_headers_get_distinct_keys() {
        let t = text(jsonl(&doc("| a | a | |\n|---|---|---|\n| 1 | 2 | 3 |\n")).unwrap());
        assert_eq!(t.trim(), "{\"a\":1,\"a_2\":2,\"Column 3\":3}");
    }

    #[test]
    fn xml_escapes_and_keeps_headers_out_of_element_names() {
        let t = text(xml(&doc("| Cost (UGX) | N&A |\n|---|---|\n| 1 < 2 | \"q\" |\n")).unwrap());
        assert!(t.contains("column=\"Cost (UGX)\">1 &lt; 2<"), "{t}");
        assert!(t.contains("column=\"N&amp;A\">&quot;q&quot;<"), "{t}");
    }

    #[test]
    fn latex_escapes_special_characters_and_sets_alignment() {
        let t = text(latex(&doc("| a_b | c |\n|:--|--:|\n| 50% & $ | # |\n")).unwrap());
        assert!(t.contains("\\begin{tabular}{lr}"), "{t}");
        assert!(t.contains("\\textbf{a\\_b}"), "{t}");
        assert!(t.contains("50\\% \\& \\$ & \\#"), "{t}");
    }

    #[test]
    fn sheet_names_obey_excel_limits() {
        let mut used = HashSet::new();
        let long = "x".repeat(50);
        assert_eq!(sheet_name(&long, 0, &mut used).chars().count(), 31);
        assert_eq!(sheet_name("A/B:C?", 1, &mut used), "ABC");
        assert_eq!(sheet_name("abc", 2, &mut used), "abc 2", "case-insensitive duplicate");
        assert_eq!(sheet_name("   ", 3, &mut used), "Sheet 4");
        // a 31-char name that collides must still fit
        let again = sheet_name(&long, 4, &mut used);
        assert!(again.chars().count() <= 31 && again != long[..31], "{again}");
    }

    #[test]
    fn xlsx_is_a_valid_zip_with_one_sheet_per_table() {
        let p = xlsx(&doc(TWO)).unwrap();
        assert_eq!(p.extension, "xlsx");
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(p.bytes)).unwrap();
        assert!(z.by_name("xl/worksheets/sheet1.xml").is_ok());
        assert!(z.by_name("xl/worksheets/sheet2.xml").is_ok());
        assert!(z.by_name("xl/worksheets/sheet3.xml").is_err());
    }

    #[test]
    fn xlsx_formula_text_is_stored_as_a_string_not_a_formula() {
        use std::io::Read;
        let p = xlsx(&doc(ONE)).unwrap();
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(p.bytes)).unwrap();
        let mut sheet = String::new();
        z.by_name("xl/worksheets/sheet1.xml").unwrap().read_to_string(&mut sheet).unwrap();
        assert!(!sheet.contains("<f>"), "no formula element may be written: {sheet}");
        let mut shared = String::new();
        z.by_name("xl/sharedStrings.xml").unwrap().read_to_string(&mut shared).unwrap();
        assert!(shared.contains("HYPERLINK"), "the text must be present as a string");
    }
}
