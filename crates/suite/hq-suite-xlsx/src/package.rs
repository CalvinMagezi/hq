//! The zip package: content types, relationships, styles, the workbook and one part per sheet.

use std::io::{Cursor, Write};

use crate::XlsxError;
use crate::cells::{Built, BuiltSheet, CachedValue, Cell, Style, cell_ref};

const MAIN_NS: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const REL_NS: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const PKG_REL_NS: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
/// Custom number format ids start here; lower ids are built in.
const FIRST_CUSTOM_FORMAT: u32 = 164;
const WHOLE_CURRENCIES: &[&str] = &["ugx", "kes", "rwf", "tzs", "jpy"];

pub(crate) fn pack(built: &Built) -> Result<Vec<u8>, XlsxError> {
    let styles = StyleTable::from(&built.sheets);
    let mut parts: Vec<(String, String)> = vec![
        (
            "[Content_Types].xml".into(),
            content_types(built.sheets.len()),
        ),
        ("_rels/.rels".into(), root_rels()),
        ("xl/workbook.xml".into(), workbook(&built.sheets)),
        (
            "xl/_rels/workbook.xml.rels".into(),
            workbook_rels(built.sheets.len()),
        ),
        ("xl/styles.xml".into(), styles.xml()),
    ];
    for (i, sheet) in built.sheets.iter().enumerate() {
        parts.push((
            format!("xl/worksheets/sheet{}.xml", i + 1),
            sheet_xml(sheet, &styles),
        ));
    }
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (name, xml) in parts {
        let pack = |e: &dyn std::fmt::Display| XlsxError::Pack(e.to_string());
        zip.start_file(name, options).map_err(|e| pack(&e))?;
        zip.write_all(xml.as_bytes()).map_err(|e| pack(&e))?;
    }
    let out = zip.finish().map_err(|e| XlsxError::Pack(e.to_string()))?;
    Ok(out.into_inner())
}

fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            // XML 1.0 cannot hold most control characters.
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
            c => out.push(c),
        }
    }
    out
}

fn content_types(sheets: usize) -> String {
    let mut s = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>"#,
    );
    for i in 1..=sheets {
        s.push_str(&format!(r#"<Override PartName="/xl/worksheets/sheet{i}.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>"#));
    }
    s.push_str("</Types>");
    s
}

fn root_rels() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="{PKG_REL_NS}"><Relationship Id="rId1" Type="{REL_NS}/officeDocument" Target="xl/workbook.xml"/></Relationships>"#
    )
}

fn workbook(sheets: &[BuiltSheet]) -> String {
    let mut s = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><workbook xmlns="{MAIN_NS}" xmlns:r="{REL_NS}"><sheets>"#
    );
    for (i, sh) in sheets.iter().enumerate() {
        s.push_str(&format!(
            r#"<sheet name="{}" sheetId="{n}" r:id="rId{n}"/>"#,
            esc(&sh.name),
            n = i + 1
        ));
    }
    // Ask the program to recalculate on open, so a formula stored without a value is filled in.
    s.push_str(r#"</sheets><calcPr fullCalcOnLoad="1"/></workbook>"#);
    s
}

fn workbook_rels(sheets: usize) -> String {
    let mut s = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="{PKG_REL_NS}">"#
    );
    for i in 1..=sheets {
        s.push_str(&format!(r#"<Relationship Id="rId{i}" Type="{REL_NS}/worksheet" Target="worksheets/sheet{i}.xml"/>"#));
    }
    s.push_str(&format!(
        r#"<Relationship Id="rId{}" Type="{REL_NS}/styles" Target="styles.xml"/></Relationships>"#,
        sheets + 1
    ));
    s
}

/// The distinct cell styles, in the order their `s=` numbers are given out.
struct StyleTable(Vec<Style>);

impl StyleTable {
    fn from(sheets: &[BuiltSheet]) -> Self {
        let mut all = vec![Style::Plain, Style::Header];
        for sh in sheets {
            for row in &sh.rows {
                for (_, cell) in row {
                    let style = match cell {
                        Cell::Number(_, s) | Cell::Formula { style: s, .. } => s,
                        _ => continue,
                    };
                    if !all.contains(style) {
                        all.push(style.clone());
                    }
                }
            }
        }
        StyleTable(all)
    }

    fn index(&self, style: &Style) -> usize {
        self.0.iter().position(|s| s == style).unwrap_or(0)
    }

    fn format_code(style: &Style) -> Option<String> {
        match style {
            Style::Int => Some("#,##0".into()),
            Style::Pct => Some("0.0%".into()),
            Style::Date => Some("yyyy-mm-dd".into()),
            Style::Money(code) => {
                let decimals = if WHOLE_CURRENCIES.contains(&code.as_str()) {
                    ""
                } else {
                    ".00"
                };
                Some(format!("\"{}\" #,##0{decimals}", code.to_uppercase()))
            }
            Style::Plain | Style::Header => None,
        }
    }

    fn xml(&self) -> String {
        let codes: Vec<Option<String>> = self.0.iter().map(Self::format_code).collect();
        let mut custom: Vec<&String> = Vec::new();
        for c in codes.iter().flatten() {
            if !custom.contains(&c) {
                custom.push(c);
            }
        }
        let mut s = format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><styleSheet xmlns="{MAIN_NS}">"#
        );
        if !custom.is_empty() {
            s.push_str(&format!(r#"<numFmts count="{}">"#, custom.len()));
            for (i, c) in custom.iter().enumerate() {
                s.push_str(&format!(
                    r#"<numFmt numFmtId="{}" formatCode="{}"/>"#,
                    FIRST_CUSTOM_FORMAT as usize + i,
                    esc(c)
                ));
            }
            s.push_str("</numFmts>");
        }
        s.push_str(r#"<fonts count="2"><font><sz val="11"/><name val="Calibri"/></font><font><b/><sz val="11"/><name val="Calibri"/></font></fonts><fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills><borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders><cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>"#);
        s.push_str(&format!(r#"<cellXfs count="{}">"#, self.0.len()));
        for (style, code) in self.0.iter().zip(&codes) {
            let fmt = code
                .as_ref()
                .and_then(|c| custom.iter().position(|x| *x == c))
                .map_or(0, |i| FIRST_CUSTOM_FORMAT as usize + i);
            let font = usize::from(*style == Style::Header);
            s.push_str(&format!(r#"<xf numFmtId="{fmt}" fontId="{font}" fillId="0" borderId="0" xfId="0" applyNumberFormat="1" applyFont="1"/>"#));
        }
        s.push_str(r#"</cellXfs><cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles></styleSheet>"#);
        s
    }
}

fn sheet_xml(sheet: &BuiltSheet, styles: &StyleTable) -> String {
    let mut s = format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><worksheet xmlns="{MAIN_NS}"><sheetViews><sheetView workbookViewId="0"><pane ySplit="1" topLeftCell="A2" activePane="bottomLeft" state="frozen"/></sheetView></sheetViews><sheetFormatPr defaultRowHeight="15"/>"#
    );
    if !sheet.widths.is_empty() {
        s.push_str("<cols>");
        for (i, w) in sheet.widths.iter().enumerate() {
            s.push_str(&format!(
                r#"<col min="{n}" max="{n}" width="{w}" customWidth="1"/>"#,
                n = i + 1
            ));
        }
        s.push_str("</cols>");
    }
    s.push_str("<sheetData>");
    for (r, row) in sheet.rows.iter().enumerate() {
        if row.is_empty() {
            continue;
        }
        s.push_str(&format!(r#"<row r="{}">"#, r + 1));
        for (c, cell) in row {
            s.push_str(&cell_xml(&cell_ref(*c, r), cell, r == 0, styles));
        }
        s.push_str("</row>");
    }
    s.push_str("</sheetData></worksheet>");
    s
}

fn cell_xml(at: &str, cell: &Cell, header: bool, styles: &StyleTable) -> String {
    let style_attr = |style: &Style| match styles.index(style) {
        0 => String::new(),
        n => format!(r#" s="{n}""#),
    };
    match cell {
        Cell::Text(t) => {
            let style = if header {
                style_attr(&Style::Header)
            } else {
                String::new()
            };
            format!(
                r#"<c r="{at}"{style} t="inlineStr"><is><t xml:space="preserve">{}</t></is></c>"#,
                esc(t)
            )
        }
        Cell::Number(n, style) => {
            format!(r#"<c r="{at}"{}><v>{}</v></c>"#, style_attr(style), num(*n))
        }
        Cell::Bool(b) => format!(r#"<c r="{at}" t="b"><v>{}</v></c>"#, u8::from(*b)),
        Cell::Formula { text, value, style } => {
            let (kind, v) = match value {
                Some(CachedValue::Number(n)) => (String::new(), format!("<v>{}</v>", num(*n))),
                Some(CachedValue::Text(t)) => (r#" t="str""#.into(), format!("<v>{}</v>", esc(t))),
                Some(CachedValue::Bool(b)) => {
                    (r#" t="b""#.into(), format!("<v>{}</v>", u8::from(*b)))
                }
                Some(CachedValue::Error(e)) => (r#" t="e""#.into(), format!("<v>{}</v>", esc(e))),
                None => (String::new(), String::new()),
            };
            format!(
                r#"<c r="{at}"{}{kind}><f>{}</f>{v}</c>"#,
                style_attr(style),
                esc(text)
            )
        }
    }
}

/// A number as the file stores it: shortest text that reads back as the same value.
fn num(n: f64) -> String {
    format!("{n}")
}
