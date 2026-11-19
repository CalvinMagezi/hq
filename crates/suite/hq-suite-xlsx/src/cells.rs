//! Turning the rows of a sheet block into typed cells, formulas and styles.

use hq_suite_calc::{CalcError, FUNCTIONS, Value, Workbook, column_name};
use hq_suite_md::ColType;

use crate::{MAX_SHEET_NAME, SheetSpec, XlsxReport};

/// Formulas longer than this are written as text.
const MAX_FORMULA_CHARS: usize = 8_192;
/// Column width limits, in characters.
const MIN_WIDTH: usize = 8;
const MAX_WIDTH: usize = 60;
/// Days between 1899-12-30 (the spreadsheet epoch) and 1970-01-01.
const EPOCH_OFFSET_DAYS: i64 = 25_569;

/// How a cell is drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Style {
    Plain,
    Header,
    Int,
    Pct,
    Date,
    Money(String),
}

#[derive(Debug, Clone)]
pub(crate) enum Cell {
    Text(String),
    Number(f64, Style),
    Bool(bool),
    Formula {
        text: String,
        value: Option<CachedValue>,
        style: Style,
    },
}

#[derive(Debug, Clone)]
pub(crate) enum CachedValue {
    Number(f64),
    Text(String),
    Bool(bool),
    Error(String),
}

pub(crate) struct BuiltSheet {
    pub name: String,
    /// Rows of (column, cell), header first. Blank cells are absent.
    pub rows: Vec<Vec<(u32, Cell)>>,
    pub widths: Vec<usize>,
}

pub(crate) struct Built {
    pub sheets: Vec<BuiltSheet>,
    pub report: XlsxReport,
}

pub(crate) fn build(specs: &[SheetSpec]) -> Built {
    let names = unique_names(specs);
    let mut book = Workbook::new();
    let indices: Vec<usize> = names
        .iter()
        .map(|n| book.add_sheet(n).unwrap_or(0))
        .collect();
    let mut report = XlsxReport {
        sheets: specs.len(),
        ..XlsxReport::default()
    };
    // First pass: decide which cells are formulas and put every cell in the engine, so a formula
    // may read any other cell or sheet before anything is evaluated.
    let mut kinds: Vec<Vec<Vec<Option<String>>>> = Vec::new();
    for (spec, &si) in specs.iter().zip(&indices) {
        let mut sheet_kinds = Vec::new();
        for (r, row) in spec.rows.iter().enumerate() {
            let mut row_kinds = Vec::new();
            for (c, text) in row.iter().enumerate() {
                let formula = formula_body(text).filter(|f| {
                    let ok = formula_is_safe(f) && book.formula_parses(si, f);
                    if !ok {
                        report.formulas_as_text += 1;
                    }
                    ok
                });
                let input = match &formula {
                    Some(_) => text.trim().to_string(),
                    None => String::new(),
                };
                if formula.is_some() {
                    book.set(si, c as u32, r as u32 + 1, &input);
                } else if let Some(v) = literal_value(text, spec.types.get(c)) {
                    book.set_value(si, c as u32, r as u32 + 1, v);
                }
                row_kinds.push(formula);
            }
            sheet_kinds.push(row_kinds);
        }
        kinds.push(sheet_kinds);
    }
    // Second pass: read the values back and make the cells.
    let mut sheets = Vec::new();
    for ((spec, &si), sheet_kinds) in specs.iter().zip(&indices).zip(&kinds) {
        let name = names[sheets.len()].clone();
        let mut rows: Vec<Vec<(u32, Cell)>> = Vec::new();
        let mut widths: Vec<usize> = spec.header.iter().map(|h| h.chars().count()).collect();
        rows.push(
            spec.header
                .iter()
                .enumerate()
                .filter(|(_, h)| !h.is_empty())
                .map(|(c, h)| (c as u32, Cell::Text(h.clone())))
                .collect(),
        );
        for (r, row) in spec.rows.iter().enumerate() {
            let mut out = Vec::new();
            for (c, text) in row.iter().enumerate() {
                let style = style_for(spec.types.get(c));
                let cell = match &sheet_kinds[r][c] {
                    Some(f) => {
                        report.formulas += 1;
                        let value = cached(book.get(si, c as u32, r as u32 + 1));
                        if value.is_none() {
                            report.formulas_without_value += 1;
                        }
                        Some(Cell::Formula {
                            text: f.clone(),
                            value,
                            style,
                        })
                    }
                    None => plain_cell(text, spec.types.get(c), style),
                };
                if let Some(cell) = cell {
                    if widths.len() <= c {
                        widths.resize(c + 1, 0);
                    }
                    widths[c] = widths[c].max(shown_len(text));
                    out.push((c as u32, cell));
                }
            }
            rows.push(out);
        }
        sheets.push(BuiltSheet {
            name,
            rows,
            widths: widths
                .into_iter()
                .map(|w| w.clamp(MIN_WIDTH, MAX_WIDTH))
                .collect(),
        });
    }
    Built { sheets, report }
}

fn shown_len(text: &str) -> usize {
    text.chars().count() + 2
}

/// The formula's body when the cell is a formula: `=B2*C2` gives `B2*C2`.
fn formula_body(text: &str) -> Option<String> {
    let body = text.trim_start().strip_prefix('=')?.trim();
    (!body.is_empty()).then(|| body.to_string())
}

/// Whether a formula may be written as one: short, no external or DDE syntax, and every function
/// it calls is on the engine's list.
pub fn formula_is_safe(formula: &str) -> bool {
    if formula.chars().count() > MAX_FORMULA_CHARS {
        return false;
    }
    let mut in_text = false;
    let mut word = String::new();
    for c in formula.chars() {
        if c == '"' {
            in_text = !in_text;
            word.clear();
            continue;
        }
        if in_text {
            continue;
        }
        if matches!(c, '[' | ']' | '|' | '{' | '}' | '\\') {
            return false;
        }
        if c.is_alphanumeric() || c == '_' || c == '.' {
            word.push(c);
            continue;
        }
        if c == '(' && !word.is_empty() && !is_function(&word) {
            return false;
        }
        word.clear();
    }
    true
}

fn is_function(name: &str) -> bool {
    FUNCTIONS.iter().any(|f| f.eq_ignore_ascii_case(name))
}

fn style_for(ty: Option<&ColType>) -> Style {
    match ty {
        Some(ColType::Int) => Style::Int,
        Some(ColType::Pct) => Style::Pct,
        Some(ColType::Date) => Style::Date,
        Some(ColType::Money(code)) => Style::Money(code.clone()),
        _ => Style::Plain,
    }
}

/// A literal as the engine should hold it, so formulas can read it. `None` for a blank cell.
fn literal_value(text: &str, ty: Option<&ColType>) -> Option<Value> {
    if text.trim().is_empty() {
        return None;
    }
    Some(match typed_literal(text, ty) {
        Typed::Number(n) => Value::Num(n),
        Typed::Bool(b) => Value::Bool(b),
        Typed::Text => Value::Text(text.to_string()),
    })
}

enum Typed {
    Number(f64),
    Bool(bool),
    Text,
}

fn typed_literal(text: &str, ty: Option<&ColType>) -> Typed {
    let t = text.trim();
    match ty {
        Some(ColType::Int | ColType::Num | ColType::Money(_)) => {
            parse_number(t).map_or(Typed::Text, Typed::Number)
        }
        Some(ColType::Pct) => match t.strip_suffix('%') {
            Some(p) => parse_number(p).map_or(Typed::Text, |n| Typed::Number(n / 100.0)),
            None => parse_number(t).map_or(Typed::Text, Typed::Number),
        },
        Some(ColType::Date) => parse_date(t).map_or(Typed::Text, Typed::Number),
        Some(ColType::Bool) => match t.to_ascii_uppercase().as_str() {
            "TRUE" => Typed::Bool(true),
            "FALSE" => Typed::Bool(false),
            _ => Typed::Text,
        },
        _ => Typed::Text,
    }
}

fn plain_cell(text: &str, ty: Option<&ColType>, style: Style) -> Option<Cell> {
    if text.trim().is_empty() {
        return None;
    }
    Some(match typed_literal(text, ty) {
        Typed::Number(n) => Cell::Number(n, style),
        Typed::Bool(b) => Cell::Bool(b),
        Typed::Text => Cell::Text(text.to_string()),
    })
}

/// `2,700,000`, `UGX 2,700,000`, `$12.50`, `(40)` and `-3` as numbers; anything else is not.
fn parse_number(text: &str) -> Option<f64> {
    let mut t = text.trim().to_string();
    let negative = t.starts_with('(') && t.ends_with(')');
    if negative {
        t = t[1..t.len() - 1].to_string();
    }
    let t = t.trim_start_matches(|c: char| c.is_ascii_alphabetic() || c == '$' || c == ' ');
    let cleaned: String = t.chars().filter(|c| *c != ',' && *c != ' ').collect();
    let n: f64 = cleaned.parse().ok().filter(|n: &f64| n.is_finite())?;
    Some(if negative { -n } else { n })
}

/// An ISO date (`2026-11-18`) as a spreadsheet serial number.
fn parse_date(text: &str) -> Option<f64> {
    let mut parts = text.get(..10)?.split('-');
    let (y, m, d): (i64, i64, i64) = (
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    );
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || !(1900..=9999).contains(&y) {
        return None;
    }
    // Days from the civil date to 1970-01-01, then to the spreadsheet's epoch.
    let (y2, m2) = if m <= 2 { (y - 1, m + 9) } else { (y, m - 3) };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * m2 + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days_since_unix = era * 146_097 + doe - 719_468;
    Some((days_since_unix + EPOCH_OFFSET_DAYS) as f64)
}

fn cached(v: Value) -> Option<CachedValue> {
    match v {
        Value::Num(n) if n.is_finite() => Some(CachedValue::Number(n)),
        Value::Num(_) | Value::Empty => None,
        Value::Text(t) => Some(CachedValue::Text(t)),
        Value::Bool(b) => Some(CachedValue::Bool(b)),
        Value::Err(CalcError::Circular | CalcError::Name) => None,
        Value::Err(e) => Some(CachedValue::Error(e.to_string())),
    }
}

/// Sheet names Excel accepts: at most 31 characters, none of `[]:*?/\`, no blank, unique.
fn unique_names(specs: &[SheetSpec]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (i, spec) in specs.iter().enumerate() {
        let mut base: String = spec
            .name
            .chars()
            .map(|c| {
                if "[]:*?/\\".contains(c) || c.is_control() {
                    '-'
                } else {
                    c
                }
            })
            .collect();
        base = base.trim().trim_matches('\'').to_string();
        if base.is_empty() {
            base = format!("Sheet{}", i + 1);
        }
        let mut name: String = base.chars().take(MAX_SHEET_NAME).collect();
        let mut n = 2;
        while out.iter().any(|o| o.eq_ignore_ascii_case(&name)) {
            let suffix = format!(" {n}");
            let keep = MAX_SHEET_NAME - suffix.chars().count();
            name = format!("{}{suffix}", base.chars().take(keep).collect::<String>());
            n += 1;
        }
        out.push(name);
    }
    out
}

/// A1-style reference for a zero-based column and row.
pub(crate) fn cell_ref(col: u32, row: usize) -> String {
    format!("{}{}", column_name(col), row + 1)
}
