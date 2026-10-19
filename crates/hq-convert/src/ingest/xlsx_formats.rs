//! Which number format each cell of an `.xlsx` workbook shows.
//!
//! The values come from the spreadsheet reader; this reads only the styles
//! (`xl/styles.xml`) and each cell's style number (`s="3"` on `<c r="B7">`),
//! so that a stored 0.097 can be shown as "9.7%". It is best effort: a part it
//! cannot read leaves those cells in their plain form.

use std::collections::HashMap;
use std::sync::Arc;

use super::numfmt::{self, DateKind};
use super::package::{Package, attr, is, parse_xml, read_rels, resolve_part};

const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// A sheet part bigger than this is left alone; scanning it would cost more than it is worth.
const MAX_SHEET_BYTES: usize = 24 * 1024 * 1024;

/// What a cell's format asks for.
#[derive(Debug, Clone, PartialEq)]
pub enum CellFormat {
    /// A number shown by this format code.
    Number(Arc<str>),
    /// A date, a time or both.
    Date(DateKind),
}

#[derive(Default)]
pub struct CellFormats {
    by_sheet: HashMap<String, HashMap<(u32, u32), Arc<CellFormat>>>,
}

impl CellFormats {
    /// The format of the cell at zero-based (`row`, `col`), when it has one worth applying.
    pub fn get(&self, sheet: &str, row: u32, col: u32) -> Option<&CellFormat> {
        self.by_sheet.get(sheet)?.get(&(row, col)).map(|c| &**c)
    }
}

pub fn read(pkg: &mut Package<'_>) -> CellFormats {
    try_read(pkg).unwrap_or_default()
}

fn try_read(pkg: &mut Package<'_>) -> Option<CellFormats> {
    let max = pkg.nodes_limit();

    // Style number -> format code, for the styles that have one worth applying.
    let styles_xml = pkg.read_text("xl/styles.xml").ok()??;
    let styles = parse_xml(&styles_xml, false, max).ok()?;
    let custom: HashMap<u32, String> = styles
        .descendants()
        .filter(|n| is(*n, MAIN, "numFmt"))
        .filter_map(|n| {
            Some((
                n.attribute("numFmtId")?.parse().ok()?,
                n.attribute("formatCode")?.to_string(),
            ))
        })
        .collect();
    let xf_codes: Vec<Option<Arc<CellFormat>>> = styles
        .descendants()
        .find(|n| is(*n, MAIN, "cellXfs"))?
        .children()
        .filter(|n| is(*n, MAIN, "xf"))
        .map(|xf| {
            let id: u32 = xf
                .attribute("numFmtId")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            if let Some(kind) = numfmt::builtin_date(id) {
                return Some(Arc::new(CellFormat::Date(kind)));
            }
            let code = custom
                .get(&id)
                .map(String::as_str)
                .or_else(|| numfmt::builtin(id))?;
            if let Some(kind) = numfmt::date_kind(code) {
                return Some(Arc::new(CellFormat::Date(kind)));
            }
            // Keep it only if it formats an ordinary number: General and text do not.
            numfmt::render(code, 1.5).map(|_| Arc::new(CellFormat::Number(Arc::from(code))))
        })
        .collect();
    if xf_codes.iter().all(Option::is_none) {
        return Some(CellFormats::default());
    }

    // Sheet name -> part.
    let workbook_xml = pkg.read_text("xl/workbook.xml").ok()??;
    let rels_xml = pkg.read_text("xl/_rels/workbook.xml.rels").ok()??;
    let rels = read_rels(&rels_xml, max);
    let workbook = parse_xml(&workbook_xml, false, max).ok()?;
    let sheets: Vec<(String, String)> = workbook
        .descendants()
        .filter(|n| is(*n, MAIN, "sheet"))
        .filter_map(|n| {
            let name = n.attribute("name")?.to_string();
            let (target, _) = rels.get(attr(n, REL, "id")?)?;
            Some((name, resolve_part("xl", target)))
        })
        .collect();

    let mut out = CellFormats::default();
    for (name, part) in sheets {
        let Ok(Some(bytes)) = pkg.read(&part) else {
            continue;
        };
        if bytes.len() > MAX_SHEET_BYTES {
            continue;
        }
        let text = super::package::text_of(bytes);
        out.by_sheet.insert(name, scan_cells(&text, &xf_codes));
    }
    Some(out)
}

/// Walks the sheet's `<c r="B7" s="3">` tags. A scan is enough here: only two attributes of
/// one tag are wanted, and a sheet can hold millions of cells.
fn scan_cells(
    xml: &str,
    xf_codes: &[Option<Arc<CellFormat>>],
) -> HashMap<(u32, u32), Arc<CellFormat>> {
    let mut out = HashMap::new();
    let mut rest = xml;
    while let Some(i) = rest.find('<') {
        rest = &rest[i + 1..];
        let end = rest.find('>').unwrap_or(rest.len());
        let tag = &rest[..end];
        let name_end = tag.find(|c: char| c.is_whitespace()).unwrap_or(tag.len());
        let name = &tag[..name_end];
        if name == "c" || name.ends_with(":c") {
            let (r, s) = (attr_of(tag, "r"), attr_of(tag, "s"));
            if let (Some(r), Some(s)) = (r, s)
                && let (Some(code), Some(pos)) = (
                    s.parse::<usize>()
                        .ok()
                        .and_then(|s| xf_codes.get(s))
                        .and_then(Option::as_ref),
                    cell_ref(r),
                )
            {
                out.insert(pos, code.clone());
            }
        }
        rest = &rest[end..];
    }
    out
}

/// The value of attribute `name` in a tag's text.
fn attr_of<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    for quote in ['"', '\''] {
        let key = format!(" {name}={quote}");
        if let Some(i) = tag.find(&key) {
            let value = &tag[i + key.len()..];
            return value.find(quote).map(|e| &value[..e]);
        }
    }
    None
}

/// `B7` as zero-based (row, column) = (6, 1).
fn cell_ref(r: &str) -> Option<(u32, u32)> {
    let letters = r.chars().take_while(char::is_ascii_alphabetic).count();
    if letters == 0 || letters > 3 {
        return None;
    }
    let col = r[..letters].bytes().fold(0u32, |acc, b| {
        acc * 26 + u32::from(b.to_ascii_uppercase() - b'A' + 1)
    }) - 1;
    let row: u32 = r[letters..].parse().ok()?;
    Some((row.checked_sub(1)?, col))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_references_become_row_and_column() {
        assert_eq!(cell_ref("A1"), Some((0, 0)));
        assert_eq!(cell_ref("B7"), Some((6, 1)));
        assert_eq!(cell_ref("AA10"), Some((9, 26)));
        assert_eq!(cell_ref("7"), None);
        assert_eq!(cell_ref("A0"), None);
    }

    #[test]
    fn the_scan_finds_styled_cells_with_or_without_a_namespace_prefix() {
        let codes: Vec<Option<Arc<CellFormat>>> =
            vec![None, Some(Arc::new(CellFormat::Number("0.0%".into())))];
        let xml = r#"<sheetData><row r="1"><c r="A1" s="1"><v>0.5</v></c><c r="B1" s="0"/><c r="C1"><v>1</v></c></row><x:row><x:c r='D2' s='1'/></x:row></sheetData>"#;
        let found = scan_cells(xml, &codes);
        let pct = CellFormat::Number("0.0%".into());
        assert_eq!(found.get(&(0, 0)).map(|c| &**c), Some(&pct));
        assert_eq!(found.get(&(1, 3)).map(|c| &**c), Some(&pct));
        assert_eq!(found.len(), 2);
    }

    #[test]
    fn an_attribute_is_not_confused_with_one_that_ends_the_same_way() {
        assert_eq!(attr_of(r#"c r="A1" xs="9" s="2""#, "s"), Some("2"));
        assert_eq!(attr_of(r#"c r="A1""#, "s"), None);
    }
}
