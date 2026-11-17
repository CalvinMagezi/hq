//! `hq suite calc-check <xlsx>`: how often the suite's formula engine gives the answer Excel stored.
//!
//! Every formula cell in a workbook has the value Excel (or another program) calculated and saved
//! with it. The workbook is loaded into the engine, every formula is calculated, and each answer
//! is compared with the saved one. The report is counts only (no sheet names, cell addresses,
//! formulas or values), so it can be shared; the names of functions the engine lacks are counted
//! because they say what to build next.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result, bail};
use calamine::{Data, Reader, open_workbook_auto};
use hq_suite_calc::{CalcError, Value, Workbook};

/// The largest workbook the check reads.
const MAX_BYTES: u64 = 64 * 1024 * 1024;
/// Numbers agree when they differ by no more than this share of their size.
const RELATIVE_TOLERANCE: f64 = 1e-9;
/// A result this close to the saved one is the same number: chains of money arithmetic differ from
/// Excel's by floating-point noise (a balance of 0.0000000016 against 0).
const ABSOLUTE_TOLERANCE: f64 = 1e-6;
const TOP_UNSUPPORTED: usize = 12;

#[derive(Debug, Default)]
pub struct CalcReport {
    pub sheets: usize,
    pub formulas: usize,
    pub matched: usize,
    pub mismatched: usize,
    /// Formulas using a function the engine does not have, or syntax it cannot read.
    pub unsupported: usize,
    /// How many unsupported formulas named each missing function (or `syntax`).
    pub missing: BTreeMap<String, usize>,
    /// Wrong answers by the kind of value each side gave and the formula's outermost function.
    pub wrong: BTreeMap<String, usize>,
}

fn from_data(d: &Data) -> Value {
    match d {
        Data::Empty => Value::Empty,
        Data::Int(i) => Value::Num(*i as f64),
        Data::Float(f) => Value::Num(*f),
        Data::Bool(b) => Value::Bool(*b),
        Data::String(s) => Value::Text(s.clone()),
        Data::DateTime(dt) => Value::Num(dt.as_f64()),
        Data::DateTimeIso(s) | Data::DurationIso(s) => Value::Text(s.clone()),
        Data::Error(e) => Value::Err(match e {
            calamine::CellErrorType::Div0 => CalcError::Div0,
            calamine::CellErrorType::NA => CalcError::Na,
            calamine::CellErrorType::Name => CalcError::Name,
            calamine::CellErrorType::Num => CalcError::Num,
            calamine::CellErrorType::Ref => CalcError::Ref,
            _ => CalcError::Value,
        }),
    }
}

fn agrees(ours: &Value, theirs: &Data) -> bool {
    match (ours, theirs) {
        (Value::Num(a), Data::Float(_) | Data::Int(_) | Data::DateTime(_)) => {
            let Value::Num(b) = from_data(theirs) else {
                return false;
            };
            let diff = (a - b).abs();
            diff <= ABSOLUTE_TOLERANCE || diff <= RELATIVE_TOLERANCE * a.abs().max(b.abs())
        }
        (Value::Text(a), Data::String(b)) => a == b,
        (Value::Bool(a), Data::Bool(b)) => a == b,
        (Value::Empty, Data::Empty) => true,
        (Value::Empty, Data::String(s)) => s.is_empty(),
        (Value::Text(a), Data::Empty) => a.is_empty(),
        (Value::Err(_), Data::Error(_)) => true,
        _ => false,
    }
}

/// The function names a formula uses (`SUM(` gives `SUM`).
fn functions_in(formula: &str) -> Vec<String> {
    let chars: Vec<char> = formula.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '"' {
            i += 1;
            while i < chars.len() && chars[i] != '"' {
                i += 1;
            }
        } else if chars[i].is_ascii_alphabetic() {
            let start = i;
            while i < chars.len()
                && (chars[i].is_ascii_alphanumeric() || matches!(chars[i], '_' | '.'))
            {
                i += 1;
            }
            if chars.get(i) == Some(&'(') {
                out.push(
                    chars[start..i]
                        .iter()
                        .collect::<String>()
                        .to_ascii_uppercase(),
                );
            }
            continue;
        }
        i += 1;
    }
    out
}

pub fn check(file: &Path) -> Result<CalcReport> {
    let meta =
        std::fs::metadata(file).with_context(|| format!("{} cannot be read", file.display()))?;
    if meta.len() > MAX_BYTES {
        bail!("that workbook is too large to check");
    }
    let mut book =
        open_workbook_auto(file).context("that is not a workbook this build can read")?;
    let mut report = CalcReport::default();
    // Every sheet is added before any formula is set, so a formula can name any of them.
    let mut wb = Workbook::new();
    let mut loaded = Vec::new();
    for name in book.sheet_names() {
        let (Ok(values), Ok(formulas)) =
            (book.worksheet_range(&name), book.worksheet_formula(&name))
        else {
            continue;
        };
        let Some(id) = wb.add_sheet(&name) else {
            continue;
        };
        report.sheets += 1;
        loaded.push((id, values, formulas));
    }
    let mut theirs: Vec<(usize, u32, u32, String, Data)> = Vec::new();
    for (id, values, formulas) in &loaded {
        let (v0, f0) = (
            values.start().unwrap_or((0, 0)),
            formulas.start().unwrap_or((0, 0)),
        );
        for (r, c, d) in values.cells() {
            wb.set_value(*id, c as u32 + v0.1, r as u32 + v0.0, from_data(d));
        }
        for (r, c, f) in formulas.cells() {
            if f.is_empty() {
                continue;
            }
            let (col, row) = (c as u32 + f0.1, r as u32 + f0.0);
            wb.set(*id, col, row, &format!("={f}"));
            // `get_value` takes the position in the sheet, not in the range.
            let saved = values.get_value((row, col)).cloned().unwrap_or(Data::Empty);
            theirs.push((*id, col, row, f.clone(), saved));
        }
    }
    {
        for (id, col, row, formula, saved) in theirs {
            report.formulas += 1;
            let ours = wb.get(id, col, row);
            if let Value::Err(CalcError::Name) = ours {
                report.unsupported += 1;
                let names: Vec<String> = functions_in(&formula)
                    .into_iter()
                    .map(|f| f.strip_prefix("XLFN.").map(str::to_string).unwrap_or(f))
                    .filter(|f| !hq_suite_calc::FUNCTIONS.contains(&f.as_str()))
                    .collect();
                if names.is_empty() {
                    // Its own text reads fine and uses functions the engine has, so it reads a
                    // cell that could not be calculated, or a sheet this check does not link.
                    let label = if hq_suite_calc::parse_formula(&formula).is_ok() {
                        "reads a cell that could not be calculated"
                    } else {
                        "syntax the parser cannot read"
                    };
                    *report.missing.entry(label.into()).or_default() += 1;
                }
                for n in names {
                    *report.missing.entry(n).or_default() += 1;
                }
            } else if agrees(&ours, &saved) {
                report.matched += 1;
            } else {
                report.mismatched += 1;
                let head = functions_in(&formula)
                    .into_iter()
                    .next()
                    .unwrap_or_else(|| "none".into());
                let key = format!(
                    "{head}: ours {}, Excel {}",
                    kind_of(&ours),
                    data_kind(&saved)
                );
                *report.wrong.entry(key).or_default() += 1;
            }
        }
    }
    Ok(report)
}

fn kind_of(v: &Value) -> &'static str {
    match v {
        Value::Num(_) => "number",
        Value::Text(_) => "text",
        Value::Bool(_) => "true/false",
        Value::Empty => "empty",
        Value::Err(_) => "error",
    }
}

fn data_kind(d: &Data) -> &'static str {
    match d {
        Data::Int(_) | Data::Float(_) | Data::DateTime(_) => "number",
        Data::String(_) | Data::DateTimeIso(_) | Data::DurationIso(_) => "text",
        Data::Bool(_) => "true/false",
        Data::Empty => "empty",
        Data::Error(_) => "error",
    }
}

pub fn report_text(r: &CalcReport) -> String {
    if r.formulas == 0 {
        return format!("{} sheet(s), no formulas to check.\n", r.sheets);
    }
    let pct = |n: usize| 100.0 * n as f64 / r.formulas as f64;
    let mut out = format!(
        "{} sheet(s), {} formulas: {} gave Excel's answer ({:.1}%), {} gave another answer, {} could not be calculated ({:.1}%).\n",
        r.sheets,
        r.formulas,
        r.matched,
        pct(r.matched),
        r.mismatched,
        r.unsupported,
        pct(r.unsupported)
    );
    let mut missing: Vec<_> = r.missing.iter().collect();
    missing.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    if !missing.is_empty() {
        out.push_str("Missing most often:\n");
        for (name, n) in missing.into_iter().take(TOP_UNSUPPORTED) {
            out.push_str(&format!("  {name}: {n}\n"));
        }
    }
    let mut wrong: Vec<_> = r.wrong.iter().collect();
    wrong.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    if !wrong.is_empty() {
        out.push_str("Wrong answers, by leading function:\n");
        for (kind, n) in wrong.into_iter().take(TOP_UNSUPPORTED) {
            out.push_str(&format!("  {kind}: {n}\n"));
        }
    }
    out
}
