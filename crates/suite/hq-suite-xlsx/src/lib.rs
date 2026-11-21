//! A native `.xlsx` writer for `sheet` blocks.
//!
//! Numbers go in as numbers with the number format their column type asks for, dates as date
//! serials, and a cell starting with `=` as a real formula with the value this crate's engine
//! worked out stored beside it. A formula the engine cannot read, or one that names a function
//! outside its list, is written as plain text: a workbook built from content an agent gathered
//! from the web must not carry `HYPERLINK`, `WEBSERVICE`, external references or DDE.
//! A formula whose result is unknown is stored without a value and the file asks the spreadsheet
//! program to recalculate on open, so nothing is ever guessed.

mod cells;
mod chart;
mod package;

pub use chart::{ChartKind, ChartSeries, ChartSpec, MAX_SERIES};

use hq_suite_md::ColType;

pub use cells::formula_is_safe;

/// Longest sheet name Excel accepts.
pub const MAX_SHEET_NAME: usize = 31;

/// One sheet to write: a header row, one type per column and the rows as written in the note.
#[derive(Debug, Clone)]
pub struct SheetSpec {
    pub name: String,
    pub header: Vec<String>,
    pub types: Vec<ColType>,
    pub rows: Vec<Vec<String>>,
}

#[derive(Debug, thiserror::Error)]
pub enum XlsxError {
    #[error("there is no sheet to write")]
    NoSheets,
    #[error("the workbook could not be packed: {0}")]
    Pack(String),
}

/// What the writer did, so a caller can tell the owner in one plain line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct XlsxReport {
    pub sheets: usize,
    pub formulas: usize,
    /// Formulas written as text because the engine cannot read them or they are not allowed.
    pub formulas_as_text: usize,
    /// Formulas stored with no value, left for the spreadsheet program to calculate.
    pub formulas_without_value: usize,
    /// Charts written as native charts.
    pub charts: usize,
}

impl XlsxReport {
    /// One plain line for the person: what was kept and what was re-created or left out.
    pub fn fidelity_line(&self) -> String {
        let mut line = if self.formulas > 0 {
            "Formulas, number formats and the header row were kept.".to_string()
        } else {
            "Values, number formats and the header row were kept.".to_string()
        };
        if self.formulas_as_text > 0 {
            line.push_str(&format!(
                " {} formula{} could not be carried over and {} saved as text.",
                self.formulas_as_text,
                plural(self.formulas_as_text),
                if self.formulas_as_text == 1 {
                    "is"
                } else {
                    "are"
                }
            ));
        }
        if self.charts > 0 {
            line.push_str(&format!(
                " {} chart{} kept as native chart{}. Pictures are not included.",
                self.charts,
                plural(self.charts),
                plural(self.charts)
            ));
        } else {
            line.push_str(" Charts and pictures are not included.");
        }
        line
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

/// The workbook's bytes and a report of what went in.
pub fn write_xlsx(sheets: &[SheetSpec]) -> Result<(Vec<u8>, XlsxReport), XlsxError> {
    write_xlsx_with_charts(sheets, &[])
}

/// As [`write_xlsx`], with native charts. A chart that cannot be drawn
/// ([`ChartSpec::is_drawable`]) or names a sheet that is not there is left out and not counted.
pub fn write_xlsx_with_charts(
    sheets: &[SheetSpec],
    charts: &[ChartSpec],
) -> Result<(Vec<u8>, XlsxReport), XlsxError> {
    if sheets.is_empty() {
        return Err(XlsxError::NoSheets);
    }
    let mut built = cells::build(sheets);
    let drawable: Vec<ChartSpec> = charts
        .iter()
        .filter(|c| c.is_drawable() && c.sheet < built.sheets.len())
        .cloned()
        .collect();
    built.report.charts = drawable.len();
    let bytes = package::pack(&built, &drawable)?;
    Ok((bytes, built.report))
}

#[cfg(test)]
mod tests;
