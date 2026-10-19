//! OpenDocument spreadsheets (`.ods`) to sheets of text.
//!
//! An ODS cell stores its value and, beside it, the text the sheet shows for
//! it ("9.7%", "UGX 2,700,000", "01/07/2026"). The shown text is what is read,
//! so number formats come out right without interpreting them.

use roxmltree::Node;

use super::package::{Package, attr, child, is, parse_xml};
use super::sheet::{Book, Sheet, number};
use crate::types::ConvertError;

const OFFICE: &str = "urn:oasis:names:tc:opendocument:xmlns:office:1.0";
const TEXT: &str = "urn:oasis:names:tc:opendocument:xmlns:text:1.0";
const TABLE: &str = "urn:oasis:names:tc:opendocument:xmlns:table:1.0";
const STYLE: &str = "urn:oasis:names:tc:opendocument:xmlns:style:1.0";

/// Most columns kept per row, and the most times one non-empty cell or row is repeated.
const MAX_COLUMNS: usize = 200;
const MAX_REPEAT: usize = 50;

pub fn read(pkg: &mut Package<'_>) -> Result<Book, ConvertError> {
    let max = pkg.nodes_limit();
    if pkg
        .read_text("META-INF/manifest.xml")?
        .is_some_and(|m| m.contains("encryption-data"))
    {
        return Err(ConvertError::Encrypted(
            "this OpenDocument file is encrypted".into(),
        ));
    }
    let content = pkg.read_text("content.xml")?.ok_or_else(|| {
        ConvertError::InboundFailed("no content.xml: not an OpenDocument file".into())
    })?;
    let doc = parse_xml(&content, false, max)?;

    let hidden_styles: Vec<&str> = doc
        .descendants()
        .filter(|n| is(*n, STYLE, "style") && attr(*n, STYLE, "family") == Some("table"))
        .filter(|n| {
            child(*n, STYLE, "table-properties")
                .is_some_and(|p| attr(p, TABLE, "display") == Some("false"))
        })
        .filter_map(|n| attr(n, STYLE, "name"))
        .collect();

    let mut book = Book::default();
    for table in doc.descendants().filter(|n| is(*n, TABLE, "table")) {
        // Tables nested in a cell are not sheets.
        if table.parent().is_some_and(|p| is(p, TABLE, "table-cell")) {
            continue;
        }
        let name = attr(table, TABLE, "name").unwrap_or("Sheet").to_string();
        if attr(table, TABLE, "style-name").is_some_and(|s| hidden_styles.contains(&s)) {
            book.hidden += 1;
            continue;
        }
        let mut rows = Vec::new();
        collect_rows(table, &mut rows);
        book.sheets.push(Sheet { name, rows });
    }
    if book.sheets.is_empty() && book.hidden == 0 {
        return Err(ConvertError::InboundFailed(
            "no sheets found in this file".into(),
        ));
    }
    Ok(book)
}

fn repeat(n: Node<'_, '_>, name: &str) -> usize {
    attr(n, TABLE, name)
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(1)
        .max(1)
}

/// Rows of a table, looking through header-row and row-group wrappers.
fn collect_rows(parent: Node<'_, '_>, rows: &mut Vec<Vec<String>>) {
    for n in parent.children().filter(Node::is_element) {
        if is(n, TABLE, "table-row") {
            let cells = row_cells(n);
            let empty = cells.iter().all(String::is_empty);
            // A long run of identical blank rows is the sheet's padding, not data.
            let times = if empty {
                1
            } else {
                repeat(n, "number-rows-repeated").min(MAX_REPEAT)
            };
            for _ in 0..times {
                rows.push(cells.clone());
            }
        } else if is(n, TABLE, "table-header-rows")
            || is(n, TABLE, "table-rows")
            || is(n, TABLE, "table-row-group")
        {
            collect_rows(n, rows);
        }
    }
}

fn row_cells(row: Node<'_, '_>) -> Vec<String> {
    let mut cells: Vec<String> = Vec::new();
    for c in row.children().filter(Node::is_element) {
        let covered = is(c, TABLE, "covered-table-cell");
        if !covered && !is(c, TABLE, "table-cell") {
            continue;
        }
        let text = if covered {
            String::new()
        } else {
            cell_display(c)
        };
        let times = if text.is_empty() {
            1
        } else {
            repeat(c, "number-columns-repeated").min(MAX_REPEAT)
        };
        // Skipped empty cells still occupy their columns.
        let skipped = if text.is_empty() {
            repeat(c, "number-columns-repeated").min(MAX_COLUMNS)
        } else {
            0
        };
        for _ in 0..times.max(skipped) {
            if cells.len() >= MAX_COLUMNS {
                return cells;
            }
            cells.push(text.clone());
        }
    }
    cells
}

fn paragraph_text(p: Node<'_, '_>, out: &mut String) {
    for n in p.children() {
        if n.is_text() {
            out.push_str(n.text().unwrap_or(""));
        } else if is(n, TEXT, "s") {
            let c = attr(n, TEXT, "c")
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(1)
                .min(100);
            out.extend(std::iter::repeat_n(' ', c));
        } else if is(n, TEXT, "tab") {
            out.push(' ');
        } else if is(n, TEXT, "line-break") {
            out.push('\n');
        } else if n.is_element() && !matches!(n.tag_name().name(), "note" | "annotation") {
            paragraph_text(n, out);
        }
    }
}

/// What the sheet shows for a cell; the stored value when it shows nothing.
fn cell_display(cell: Node<'_, '_>) -> String {
    // A date is written the unambiguous way: "01/07/2026" reads two ways, 2026-07-01 one.
    if attr(cell, OFFICE, "value-type") == Some("date")
        && let Some(d) = attr(cell, OFFICE, "date-value")
    {
        return d
            .replace('T', " ")
            .trim_end_matches(":00")
            .trim_end_matches(" 00:00")
            .to_string();
    }
    let paragraphs: Vec<String> = cell
        .children()
        .filter(|c| is(*c, TEXT, "p"))
        .map(|p| {
            let mut s = String::new();
            paragraph_text(p, &mut s);
            s
        })
        .collect();
    let text = paragraphs.join("\n").trim().to_string();
    if !text.is_empty() {
        return text;
    }
    if let Some(v) = attr(cell, OFFICE, "value").and_then(|v| v.parse::<f64>().ok()) {
        return number(v);
    }
    if let Some(d) = attr(cell, OFFICE, "date-value") {
        return d.replace('T', " ");
    }
    match attr(cell, OFFICE, "boolean-value") {
        Some("true") => "TRUE".into(),
        Some("false") => "FALSE".into(),
        _ => attr(cell, OFFICE, "string-value").unwrap_or("").to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::package::Limits;
    use crate::ingest::testutil::zip_of;

    fn ods(body: &str) -> Vec<u8> {
        let content = format!(
            r#"<office:document-content xmlns:office="{OFFICE}" xmlns:text="{TEXT}" xmlns:table="{TABLE}" xmlns:style="{STYLE}">
            <office:automatic-styles><style:style style:name="ta_hidden" style:family="table"><style:table-properties table:display="false"/></style:style></office:automatic-styles>
            <office:body><office:spreadsheet>{body}</office:spreadsheet></office:body></office:document-content>"#
        );
        zip_of(&[
            (
                "mimetype",
                b"application/vnd.oasis.opendocument.spreadsheet".to_vec(),
            ),
            ("content.xml", content.into_bytes()),
        ])
    }

    fn read_book(bytes: &[u8]) -> Result<Book, ConvertError> {
        let mut pkg = Package::open(bytes, &Limits::default())?;
        read(&mut pkg)
    }

    #[test]
    fn cells_are_read_as_shown_with_dates_unambiguous_and_blanks_kept_in_place() {
        let bytes = ods(r#"<table:table table:name="Sales">
            <table:table-row>
              <table:table-cell office:value-type="string"><text:p>Date</text:p></table:table-cell>
              <table:table-cell office:value-type="string"><text:p>Growth</text:p></table:table-cell>
              <table:table-cell office:value-type="string"><text:p>Amount</text:p></table:table-cell>
            </table:table-row>
            <table:table-row>
              <table:table-cell office:value-type="date" office:date-value="2026-07-01"><text:p>01/07/2026</text:p></table:table-cell>
              <table:table-cell office:value-type="percentage" office:value="0.097"><text:p>9.7%</text:p></table:table-cell>
              <table:table-cell office:value-type="currency" office:currency="UGX" office:value="2700000"><text:p>UGX 2,700,000</text:p></table:table-cell>
            </table:table-row>
            <table:table-row>
              <table:table-cell table:number-columns-repeated="2"/>
              <table:table-cell office:value-type="float" office:value="12.5"/>
            </table:table-row>
            <table:table-row table:number-rows-repeated="1048000"><table:table-cell table:number-columns-repeated="1024"/></table:table-row>
            </table:table>"#);
        let book = read_book(&bytes).unwrap();
        assert_eq!(book.sheets.len(), 1);
        let rows = &book.sheets[0].rows;
        assert_eq!(rows[1], ["2026-07-01", "9.7%", "UGX 2,700,000"]);
        assert_eq!(
            rows[2],
            ["", "", "12.5"],
            "a value with no shown text is not lost"
        );
        assert!(
            rows.len() <= 4,
            "padding rows are not expanded: {}",
            rows.len()
        );
    }

    #[test]
    fn hidden_sheets_are_counted_and_not_read() {
        let bytes = ods(
            r#"<table:table table:name="Visible"><table:table-row><table:table-cell office:value-type="string"><text:p>a</text:p></table:table-cell></table:table-row></table:table>
               <table:table table:name="Secret" table:style-name="ta_hidden"><table:table-row><table:table-cell office:value-type="string"><text:p>hidden</text:p></table:table-cell></table:table-row></table:table>"#,
        );
        let book = read_book(&bytes).unwrap();
        assert_eq!((book.sheets.len(), book.hidden), (1, 1));
        assert_eq!(book.sheets[0].name, "Visible");
    }

    #[test]
    fn line_breaks_spaces_and_several_paragraphs_in_a_cell() {
        let bytes = ods(
            r#"<table:table table:name="T"><table:table-row><table:table-cell office:value-type="string"><text:p>one<text:line-break/>two<text:s text:c="2"/>x</text:p><text:p>three</text:p></table:table-cell></table:table-row></table:table>"#,
        );
        assert_eq!(
            read_book(&bytes).unwrap().sheets[0].rows[0][0],
            "one\ntwo  x\nthree"
        );
    }

    #[test]
    fn a_file_without_content_is_refused() {
        let bytes = zip_of(&[("mimetype", b"x".to_vec())]);
        assert!(read_book(&bytes).is_err());
    }
}
