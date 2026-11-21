//! A tree to a value.

use std::cmp::Ordering;

use crate::parse::{CellRef, Expr, MAX_ROWS};
use crate::value::{CalcError, Value};

#[path = "fnlib/dates.rs"]
mod dates;
#[path = "fnlib/more.rs"]
mod more;
#[path = "fnlib/numfmt.rs"]
mod numfmt;
#[path = "fnlib/rounding.rs"]
mod rounding;

/// Where cells come from while evaluating.
pub(crate) trait Cells {
    fn cell(&mut self, at: CellRef) -> Value;
    /// The last row (zero-based) that holds anything anywhere in the workbook. A whole-column
    /// range is cut back to it, so `$B:$B` never copies a million empty cells.
    fn max_row(&mut self) -> u32;
}

/// Largest range a formula may read, so `A1:ZZZ999999` cannot ask for the world.
const MAX_RANGE_CELLS: u64 = 1_000_000;

/// Every function the engine has. A test keeps this list and `call` in step.
pub const FUNCTIONS: &[&str] = &[
    "SUM",
    "PRODUCT",
    "AVERAGE",
    "MIN",
    "MAX",
    "COUNT",
    "COUNTA",
    "IF",
    "IFERROR",
    "IFS",
    "SWITCH",
    "AND",
    "OR",
    "NOT",
    "ABS",
    "INT",
    "SQRT",
    "ROUND",
    "ROUNDUP",
    "ROUNDDOWN",
    "MOD",
    "LEN",
    "CONCATENATE",
    "CONCAT",
    "ISNUMBER",
    "ISTEXT",
    "ISBLANK",
    "ISERROR",
    "LEFT",
    "RIGHT",
    "UPPER",
    "LOWER",
    "TRIM",
    "ROW",
    "INDEX",
    "VLOOKUP",
    "PMT",
    "EDATE",
    "COUNTIF",
    "SUMIF",
    "SEARCH",
    "FIND",
    "SUBSTITUTE",
    "N",
    "XLOOKUP",
    "TEXT",
    "POWER",
    "QUOTIENT",
    "CHAR",
    "ROWS",
    "TEXTJOIN",
    "LET",
    "TRUE",
    "FALSE",
    "IFNA",
    "COUNTIFS",
    "SUMIFS",
    "AVERAGEIF",
    "AVERAGEIFS",
    "MAXIFS",
    "MINIFS",
    "MEDIAN",
    "LARGE",
    "SMALL",
    "RANK",
    "RANK.EQ",
    "MATCH",
    "LOOKUP",
    "HLOOKUP",
    "VALUE",
    "REPT",
    "PROPER",
    "MID",
    "EXACT",
    "TRUNC",
    "SIGN",
    "CEILING",
    "FLOOR",
    "DATE",
    "YEAR",
    "MONTH",
    "DAY",
    "WEEKDAY",
    "EOMONTH",
    "DATEDIF",
];

/// Functions that are left out on purpose, with the reason. Each one gives `#NAME?` ("cannot
/// calculate"), never a guess, and the calc-check report names it.
pub const NOT_PLANNED: &[(&str, &str)] = &[
    (
        "TODAY",
        "depends on the clock, so a saved answer cannot be repeated",
    ),
    (
        "NOW",
        "depends on the clock, so a saved answer cannot be repeated",
    ),
    ("RAND", "random, so a saved answer cannot be repeated"),
    (
        "RANDBETWEEN",
        "random, so a saved answer cannot be repeated",
    ),
    ("OFFSET", "volatile: what it reads depends on a calculation"),
    ("INDIRECT", "volatile: what it reads depends on a text"),
    (
        "SUMPRODUCT",
        "works on whole arrays; the engine has no array values yet",
    ),
    ("CHOOSE", "not built yet"),
    (
        "UNIQUE",
        "returns an array that spills over cells; a cell here holds one value",
    ),
    (
        "SORT",
        "returns an array that spills over cells; a cell here holds one value",
    ),
    (
        "FILTER",
        "returns an array that spills over cells; a cell here holds one value",
    ),
];

/// Why a function is not calculated, in a few words, or `None` if it is unknown or supported.
pub fn why_unsupported(name: &str) -> Option<&'static str> {
    let upper = name.to_ascii_uppercase();
    NOT_PLANNED
        .iter()
        .find(|(n, _)| *n == upper)
        .map(|(_, why)| *why)
}

type R = Result<Value, CalcError>;

pub(crate) fn eval(e: &Expr, ctx: &mut dyn Cells) -> Value {
    match run(e, ctx) {
        Ok(v) => v,
        Err(err) => Value::Err(err),
    }
}

fn run(e: &Expr, ctx: &mut dyn Cells) -> R {
    match e {
        Expr::Num(n) => Ok(Value::Num(*n)),
        Expr::Text(s) => Ok(Value::Text(s.clone())),
        Expr::Bool(b) => Ok(Value::Bool(*b)),
        Expr::Ref(r) => match ctx.cell(*r) {
            Value::Err(err) => Err(err),
            v => Ok(v),
        },
        // A range on its own is not a value; only a function that takes one can use it.
        Expr::Range(..) => Err(CalcError::Value),
        Expr::Neg(x) => Ok(Value::Num(-num(&run(x, ctx)?)?)),
        Expr::Percent(x) => Ok(Value::Num(num(&run(x, ctx)?)? / 100.0)),
        Expr::Binary(op, a, b) => {
            let (a, b) = (run(a, ctx)?, run(b, ctx)?);
            binary(*op, &a, &b)
        }
        Expr::Compare(op, a, b) => {
            let (a, b) = (run(a, ctx)?, run(b, ctx)?);
            Ok(Value::Bool(compare(*op, &a, &b)))
        }
        Expr::Call(name, args) => call(name, args, ctx),
        // A name nothing bound: a defined name in the workbook, which this engine does not know.
        Expr::Var(_) => Err(CalcError::Name),
    }
}

/// A value as a number: empty is 0, a truth value is 0 or 1, a text that reads as a number is
/// that number, any other text is `#VALUE!`.
pub(crate) fn num(v: &Value) -> Result<f64, CalcError> {
    match v {
        Value::Empty => Ok(0.0),
        Value::Num(n) => Ok(*n),
        Value::Bool(b) => Ok(f64::from(u8::from(*b))),
        Value::Text(t) => t
            .trim()
            .parse::<f64>()
            .ok()
            // Rust reads "inf" and "nan"; a spreadsheet does not.
            .filter(|n| n.is_finite())
            .or_else(|| iso_date_serial(t))
            .ok_or(CalcError::Value),
        Value::Err(e) => Err(*e),
    }
}

/// `2023-06-01` as a spreadsheet date, which Excel also reads from text in arithmetic.
fn iso_date_serial(text: &str) -> Option<f64> {
    let mut parts = text.trim().splitn(3, '-');
    let (y, m, d) = (parts.next()?, parts.next()?, parts.next()?);
    let digits = |s: &str, n: usize| s.len() == n && s.bytes().all(|b| b.is_ascii_digit());
    if !(digits(y, 4) && digits(m, 2) && digits(d, 2)) {
        return None;
    }
    let (y, m, d): (i64, i64, i64) = (y.parse().ok()?, m.parse().ok()?, d.parse().ok()?);
    if !(1900..=9999).contains(&y) || !(1..=12).contains(&m) || d < 1 || d > days_in_month(y, m) {
        return None;
    }
    Some(spreadsheet_serial(days_from_civil(y, m, d)) as f64)
}

fn text_of(v: &Value) -> String {
    v.to_string()
}

/// Excel shows a sum or difference that lands within rounding noise of zero as exactly 0, so
/// `=A1-B1` over two equal-looking numbers is 0 rather than 1.6E-9.
fn snap_to_zero(result: f64, x: f64, y: f64) -> f64 {
    const NOISE: f64 = 1e-15;
    if result.abs() < x.abs().max(y.abs()) * NOISE {
        0.0
    } else {
        result
    }
}

fn binary(op: char, a: &Value, b: &Value) -> R {
    if op == '&' {
        return Ok(Value::Text(format!("{}{}", text_of(a), text_of(b))));
    }
    let (x, y) = (num(a)?, num(b)?);
    let out = match op {
        '+' => snap_to_zero(x + y, x, y),
        '-' => snap_to_zero(x - y, x, y),
        '*' => x * y,
        '/' => {
            if y == 0.0 {
                return Err(CalcError::Div0);
            }
            x / y
        }
        '^' => {
            let p = x.powf(y);
            if !p.is_finite() {
                return Err(CalcError::Num);
            }
            p
        }
        _ => return Err(CalcError::Value),
    };
    if out.is_finite() {
        Ok(Value::Num(out))
    } else {
        Err(CalcError::Num)
    }
}

/// Numbers sort before text, and text before truth values; text compares without regard to case.
fn rank(v: &Value) -> u8 {
    match v {
        Value::Empty | Value::Num(_) => 0,
        Value::Text(_) => 1,
        Value::Bool(_) => 2,
        Value::Err(_) => 3,
    }
}

fn order(a: &Value, b: &Value) -> Ordering {
    match (a, b) {
        (Value::Text(x), Value::Text(y)) => x.to_lowercase().cmp(&y.to_lowercase()),
        (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
        (Value::Text(x), Value::Empty) => {
            if x.is_empty() {
                Ordering::Equal
            } else {
                Ordering::Greater
            }
        }
        (Value::Empty, Value::Text(y)) => {
            if y.is_empty() {
                Ordering::Equal
            } else {
                Ordering::Less
            }
        }
        _ if rank(a) != rank(b) => rank(a).cmp(&rank(b)),
        _ => {
            let (x, y) = (num(a).unwrap_or(0.0), num(b).unwrap_or(0.0));
            x.partial_cmp(&y).unwrap_or(Ordering::Equal)
        }
    }
}

fn compare(op: char, a: &Value, b: &Value) -> bool {
    let o = order(a, b);
    match op {
        '=' => o == Ordering::Equal,
        'n' => o != Ordering::Equal,
        '<' => o == Ordering::Less,
        '>' => o == Ordering::Greater,
        'l' => o != Ordering::Greater,
        'g' => o != Ordering::Less,
        _ => false,
    }
}

/// (first column, last column, first row, last row) of a range. A range that runs to the last row
/// Excel has (`$B:$B`) stops at the last row anything is written in, because the cells below hold
/// nothing; a range that does not keep its own bounds.
fn bounds(a: CellRef, b: CellRef, ctx: &mut dyn Cells) -> (u32, u32, u32, u32) {
    let (c0, c1) = (a.col.min(b.col), a.col.max(b.col));
    let (r0, mut r1) = (a.row.min(b.row), a.row.max(b.row));
    if r1 >= MAX_ROWS - 1 {
        r1 = r1.min(ctx.max_row().max(r0));
    }
    (c0, c1, r0, r1)
}

/// Whether an argument is a range that runs to the bottom of the sheet (`$B:$B`, `B2:B1048576`).
fn open_ended(e: &Expr) -> bool {
    matches!(e, Expr::Range(a, b) if a.row.max(b.row) >= MAX_ROWS - 1)
}

/// Rows and columns a range names, before any cutting back.
fn shape(e: &Expr) -> (u64, u64) {
    match e {
        Expr::Range(a, b) => (
            u64::from(a.row.abs_diff(b.row)) + 1,
            u64::from(a.col.abs_diff(b.col)) + 1,
        ),
        _ => (1, 1),
    }
}

/// Every cell of a range, row by row.
fn range_values(a: CellRef, b: CellRef, ctx: &mut dyn Cells) -> Result<Vec<Value>, CalcError> {
    let (c0, c1, r0, r1) = bounds(a, b, ctx);
    let cells = u64::from(c1 - c0 + 1) * u64::from(r1 - r0 + 1);
    if cells > MAX_RANGE_CELLS {
        return Err(CalcError::Ref);
    }
    let mut out = Vec::with_capacity(cells as usize);
    for row in r0..=r1 {
        for col in c0..=c1 {
            out.push(ctx.cell(CellRef { col, row }));
        }
    }
    Ok(out)
}

/// The numbers an aggregate function sees. In a range only numbers count (text, truth values and
/// empty cells are skipped); an argument written directly is coerced, so `SUM("3", TRUE)` is 4.
/// Any error stops the calculation.
fn numbers(args: &[Expr], ctx: &mut dyn Cells) -> Result<Vec<f64>, CalcError> {
    let mut out = Vec::new();
    for a in args {
        match a {
            Expr::Range(x, y) => {
                for v in range_values(*x, *y, ctx)? {
                    match v {
                        Value::Num(n) => out.push(n),
                        Value::Err(e) => return Err(e),
                        _ => {}
                    }
                }
            }
            Expr::Ref(r) => match ctx.cell(*r) {
                Value::Num(n) => out.push(n),
                Value::Err(e) => return Err(e),
                _ => {}
            },
            other => out.push(num(&run(other, ctx)?)?),
        }
    }
    Ok(out)
}

fn arity(args: &[Expr], min: usize, max: usize) -> Result<(), CalcError> {
    if args.len() < min || args.len() > max {
        Err(CalcError::Value)
    } else {
        Ok(())
    }
}

fn call(name: &str, args: &[Expr], ctx: &mut dyn Cells) -> R {
    match name {
        // Unary plus: the value itself, with an empty cell read as 0.
        "+" => match run(&args[0], ctx)? {
            Value::Empty => Ok(Value::Num(0.0)),
            v => Ok(v),
        },
        "SUM" => Ok(Value::Num(numbers(args, ctx)?.iter().sum())),
        "PRODUCT" => Ok(Value::Num(numbers(args, ctx)?.iter().product())),
        "AVERAGE" => {
            let n = numbers(args, ctx)?;
            if n.is_empty() {
                return Err(CalcError::Div0);
            }
            Ok(Value::Num(n.iter().sum::<f64>() / n.len() as f64))
        }
        "MIN" => Ok(Value::Num(
            numbers(args, ctx)?
                .into_iter()
                .reduce(f64::min)
                .unwrap_or(0.0),
        )),
        "MAX" => Ok(Value::Num(
            numbers(args, ctx)?
                .into_iter()
                .reduce(f64::max)
                .unwrap_or(0.0),
        )),
        "COUNT" => Ok(Value::Num(numbers(args, ctx)?.len() as f64)),
        "COUNTA" => {
            let mut n = 0.0;
            for a in args {
                match a {
                    Expr::Range(x, y) => {
                        n += range_values(*x, *y, ctx)?
                            .iter()
                            .filter(|v| !matches!(v, Value::Empty))
                            .count() as f64;
                    }
                    other => {
                        if !matches!(run(other, ctx)?, Value::Empty) {
                            n += 1.0;
                        }
                    }
                }
            }
            Ok(Value::Num(n))
        }
        "IF" => {
            arity(args, 2, 3)?;
            let c = run(&args[0], ctx)?;
            let truth = match c {
                Value::Bool(b) => b,
                other => num(&other)? != 0.0,
            };
            if truth {
                run(&args[1], ctx)
            } else if args.len() == 3 {
                run(&args[2], ctx)
            } else {
                Ok(Value::Bool(false))
            }
        }
        "IFERROR" => {
            arity(args, 2, 2)?;
            match run(&args[0], ctx) {
                Ok(v) => Ok(v),
                // `#NAME?` means this engine cannot calculate it, not that the sheet has an error to hide.
                Err(CalcError::Name) => Err(CalcError::Name),
                Err(_) => run(&args[1], ctx),
            }
        }
        "AND" | "OR" => {
            let mut all = Vec::new();
            for a in args {
                match a {
                    Expr::Range(x, y) => {
                        for v in range_values(*x, *y, ctx)? {
                            if let Value::Bool(b) = v {
                                all.push(b);
                            } else if let Value::Num(n) = v {
                                all.push(n != 0.0);
                            }
                        }
                    }
                    other => all.push(match run(other, ctx)? {
                        Value::Bool(b) => b,
                        v => num(&v)? != 0.0,
                    }),
                }
            }
            if all.is_empty() {
                return Err(CalcError::Value);
            }
            Ok(Value::Bool(if name == "AND" {
                all.iter().all(|b| *b)
            } else {
                all.iter().any(|b| *b)
            }))
        }
        "NOT" => {
            arity(args, 1, 1)?;
            Ok(Value::Bool(match run(&args[0], ctx)? {
                Value::Bool(b) => !b,
                v => num(&v)? == 0.0,
            }))
        }
        "IFS" => {
            if args.len() < 2 || !args.len().is_multiple_of(2) {
                return Err(CalcError::Value);
            }
            for pair in args.chunks(2) {
                let t = match run(&pair[0], ctx)? {
                    Value::Bool(b) => b,
                    v => num(&v)? != 0.0,
                };
                if t {
                    return run(&pair[1], ctx);
                }
            }
            Err(CalcError::Na)
        }
        "SWITCH" => {
            if args.len() < 3 {
                return Err(CalcError::Value);
            }
            let subject = run(&args[0], ctx)?;
            let rest = &args[1..];
            for pair in rest.chunks(2) {
                if pair.len() == 2 {
                    if order(&subject, &run(&pair[0], ctx)?) == Ordering::Equal {
                        return run(&pair[1], ctx);
                    }
                } else {
                    return run(&pair[0], ctx);
                }
            }
            Err(CalcError::Na)
        }
        "ISNUMBER" | "ISTEXT" | "ISBLANK" | "ISERROR" => {
            arity(args, 1, 1)?;
            let v = match &args[0] {
                Expr::Ref(r) => ctx.cell(*r),
                other => run(other, ctx).unwrap_or_else(Value::Err),
            };
            if v == Value::Err(CalcError::Name) {
                return Err(CalcError::Name);
            }
            Ok(Value::Bool(matches!(
                (name, &v),
                ("ISNUMBER", Value::Num(_))
                    | ("ISTEXT", Value::Text(_))
                    | ("ISBLANK", Value::Empty)
                    | ("ISERROR", Value::Err(_))
            )))
        }
        "ROUNDUP" | "ROUNDDOWN" => {
            arity(args, 2, 2)?;
            let (x, d) = (num(&run(&args[0], ctx)?)?, num(&run(&args[1], ctx)?)?);
            let mode = if name == "ROUNDUP" {
                rounding::Mode::Up
            } else {
                rounding::Mode::Down
            };
            Ok(Value::Num(rounding::round_dec(x, d, mode)))
        }
        "LEFT" | "RIGHT" => {
            arity(args, 1, 2)?;
            let t: Vec<char> = text_of(&run(&args[0], ctx)?).chars().collect();
            let n = if args.len() == 2 {
                num(&run(&args[1], ctx)?)?
            } else {
                1.0
            };
            if n < 0.0 {
                return Err(CalcError::Value);
            }
            let n = (n as usize).min(t.len());
            let part: String = if name == "LEFT" {
                t[..n].iter().collect()
            } else {
                t[t.len() - n..].iter().collect()
            };
            Ok(Value::Text(part))
        }
        "UPPER" | "LOWER" | "TRIM" => {
            arity(args, 1, 1)?;
            let t = text_of(&run(&args[0], ctx)?);
            Ok(Value::Text(match name {
                "UPPER" => t.to_uppercase(),
                "LOWER" => t.to_lowercase(),
                _ => t.split_whitespace().collect::<Vec<_>>().join(" "),
            }))
        }
        "ROW" => match args {
            [Expr::Ref(r)] | [Expr::Range(r, _)] => Ok(Value::Num(f64::from(r.row) + 1.0)),
            _ => Err(CalcError::Value),
        },
        "INDEX" | "VLOOKUP" => table_lookup(name, args, ctx),
        "PMT" => {
            arity(args, 3, 5)?;
            let mut v = Vec::new();
            for a in args {
                v.push(num(&run(a, ctx)?)?);
            }
            let (rate, nper, pv) = (v[0], v[1], v[2]);
            let fv = v.get(3).copied().unwrap_or(0.0);
            let due = v.get(4).copied().unwrap_or(0.0) != 0.0;
            if nper == 0.0 {
                return Err(CalcError::Value);
            }
            if rate == 0.0 {
                return Ok(Value::Num(-(pv + fv) / nper));
            }
            let g = (1.0 + rate).powf(nper);
            let pay = -(rate * (pv * g + fv)) / ((g - 1.0) * if due { 1.0 + rate } else { 1.0 });
            Ok(Value::Num(pay))
        }
        "EDATE" => {
            arity(args, 2, 2)?;
            let serial = num(&run(&args[0], ctx)?)?.trunc();
            let months = num(&run(&args[1], ctx)?)?.trunc() as i64;
            Ok(Value::Num(add_months(serial as i64, months)? as f64))
        }
        "LET" => let_call(args, ctx),
        "POWER" | "QUOTIENT" => {
            arity(args, 2, 2)?;
            let (x, y) = (num(&run(&args[0], ctx)?)?, num(&run(&args[1], ctx)?)?);
            let out = if name == "POWER" {
                x.powf(y)
            } else if y == 0.0 {
                return Err(CalcError::Div0);
            } else {
                (x / y).trunc()
            };
            if out.is_finite() {
                Ok(Value::Num(out))
            } else {
                Err(CalcError::Num)
            }
        }
        "CHAR" => {
            arity(args, 1, 1)?;
            let code = num(&run(&args[0], ctx)?)?.trunc();
            if !(1.0..=255.0).contains(&code) {
                return Err(CalcError::Value);
            }
            Ok(Value::Text(char::from(code as u8).to_string()))
        }
        "ROWS" => match args {
            [Expr::Range(a, b)] => Ok(Value::Num(f64::from(a.row.abs_diff(b.row)) + 1.0)),
            [Expr::Ref(_)] => Ok(Value::Num(1.0)),
            _ => Err(CalcError::Value),
        },
        "TEXTJOIN" => {
            arity(args, 3, 255)?;
            let delimiter = text_of(&run(&args[0], ctx)?);
            let skip_empty = match run(&args[1], ctx)? {
                Value::Bool(b) => b,
                v => num(&v)? != 0.0,
            };
            let mut parts = Vec::new();
            for a in &args[2..] {
                for v in range_list(a, ctx)? {
                    if let Value::Err(e) = v {
                        return Err(e);
                    }
                    let t = text_of(&v);
                    if !(skip_empty && t.is_empty()) {
                        parts.push(t);
                    }
                }
            }
            Ok(Value::Text(parts.join(&delimiter)))
        }
        "COUNTIF" | "SUMIF" => count_or_sum_if(name, args, ctx),
        "SEARCH" | "FIND" => {
            arity(args, 2, 3)?;
            let find = text_of(&run(&args[0], ctx)?);
            let within = text_of(&run(&args[1], ctx)?);
            let start = if args.len() == 3 {
                num(&run(&args[2], ctx)?)?
            } else {
                1.0
            };
            if start < 1.0 {
                return Err(CalcError::Value);
            }
            let (hay, needle): (Vec<char>, Vec<char>) = if name == "SEARCH" {
                (
                    within.to_lowercase().chars().collect(),
                    find.to_lowercase().chars().collect(),
                )
            } else {
                (within.chars().collect(), find.chars().collect())
            };
            let from = start as usize - 1;
            if from > hay.len() {
                return Err(CalcError::Value);
            }
            let at = (from..=hay.len().saturating_sub(needle.len()))
                .find(|&i| hay[i..].starts_with(&needle));
            at.map(|i| Value::Num(i as f64 + 1.0))
                .ok_or(CalcError::Value)
        }
        "SUBSTITUTE" => {
            arity(args, 3, 4)?;
            let text = text_of(&run(&args[0], ctx)?);
            let (old, new) = (text_of(&run(&args[1], ctx)?), text_of(&run(&args[2], ctx)?));
            if old.is_empty() {
                return Ok(Value::Text(text));
            }
            if args.len() == 3 {
                return Ok(Value::Text(text.replace(&old, &new)));
            }
            let which = num(&run(&args[3], ctx)?)?;
            if which < 1.0 {
                return Err(CalcError::Value);
            }
            let mut out = String::new();
            let (mut seen, mut rest) = (0usize, text.as_str());
            while let Some(i) = rest.find(&old) {
                seen += 1;
                out.push_str(&rest[..i]);
                out.push_str(if seen == which as usize { &new } else { &old });
                rest = &rest[i + old.len()..];
            }
            out.push_str(rest);
            Ok(Value::Text(out))
        }
        "N" => {
            arity(args, 1, 1)?;
            Ok(Value::Num(match run(&args[0], ctx)? {
                Value::Num(n) => n,
                Value::Bool(b) => f64::from(u8::from(b)),
                _ => 0.0,
            }))
        }
        "XLOOKUP" => {
            arity(args, 3, 4)?;
            let key = run(&args[0], ctx)?;
            let (look, ret) = (range_list(&args[1], ctx)?, range_list(&args[2], ctx)?);
            match look
                .iter()
                .position(|v| order(v, &key) == Ordering::Equal && rank(v) == rank(&key))
            {
                Some(i) => ret.get(i).cloned().ok_or(CalcError::Ref),
                None if args.len() == 4 => run(&args[3], ctx),
                None => Err(CalcError::Na),
            }
        }
        "TEXT" => {
            arity(args, 2, 2)?;
            let value = run(&args[0], ctx)?;
            let Value::Text(format) = run(&args[1], ctx)? else {
                return Err(CalcError::Value);
            };
            // Number, date and currency-like formats are reproduced; anything else (times, colours,
            // fractions, scientific) stays "cannot calculate".
            numfmt::text(&value, &format).map(Value::Text)
        }
        "ABS" => one(args, ctx, f64::abs),
        "INT" => one(args, ctx, f64::floor),
        "SQRT" => {
            arity(args, 1, 1)?;
            let x = num(&run(&args[0], ctx)?)?;
            if x < 0.0 {
                Err(CalcError::Num)
            } else {
                Ok(Value::Num(x.sqrt()))
            }
        }
        "ROUND" => {
            arity(args, 2, 2)?;
            let (x, d) = (num(&run(&args[0], ctx)?)?, num(&run(&args[1], ctx)?)?);
            Ok(Value::Num(rounding::round_dec(x, d, rounding::Mode::Half)))
        }
        "MOD" => {
            arity(args, 2, 2)?;
            let (x, y) = (num(&run(&args[0], ctx)?)?, num(&run(&args[1], ctx)?)?);
            if y == 0.0 {
                return Err(CalcError::Div0);
            }
            Ok(Value::Num(x - y * (x / y).floor()))
        }
        "LEN" => {
            arity(args, 1, 1)?;
            Ok(Value::Num(
                text_of(&run(&args[0], ctx)?).chars().count() as f64
            ))
        }
        "CONCATENATE" | "CONCAT" => {
            let mut s = String::new();
            for a in args {
                s.push_str(&text_of(&run(a, ctx)?));
            }
            Ok(Value::Text(s))
        }
        _ => more::call(name, args, ctx),
    }
}

fn one(args: &[Expr], ctx: &mut dyn Cells, f: fn(f64) -> f64) -> R {
    arity(args, 1, 1)?;
    Ok(Value::Num(f(num(&run(&args[0], ctx)?)?)))
}

/// The values of a range as rows, for the functions that look inside a table. A range down to the
/// bottom of the sheet stops at the last row in use.
fn table(e: &Expr, ctx: &mut dyn Cells) -> Result<Vec<Vec<Value>>, CalcError> {
    let Expr::Range(a, b) = e else {
        return Err(CalcError::Value);
    };
    let (c0, c1, r0, r1) = bounds(*a, *b, ctx);
    let cells = u64::from(c1 - c0 + 1) * u64::from(r1 - r0 + 1);
    if cells > MAX_RANGE_CELLS {
        return Err(CalcError::Ref);
    }
    Ok((r0..=r1)
        .map(|r| {
            (c0..=c1)
                .map(|c| ctx.cell(CellRef { col: c, row: r }))
                .collect()
        })
        .collect())
}

fn table_lookup(name: &str, args: &[Expr], ctx: &mut dyn Cells) -> R {
    if name == "INDEX" {
        arity(args, 2, 3)?;
        let t = table(&args[0], ctx)?;
        let r = num(&run(&args[1], ctx)?)?;
        let c = if args.len() == 3 {
            num(&run(&args[2], ctx)?)?
        } else {
            1.0
        };
        if r < 1.0 || c < 1.0 {
            return Err(CalcError::Ref);
        }
        let cell = t
            .get(r as usize - 1)
            .and_then(|row| row.get(c as usize - 1));
        if let Some(v) = cell {
            return Ok(v.clone());
        }
        // Inside the range as written but below what the sheet uses: an empty cell.
        let (rows, cols) = shape(&args[0]);
        if (r as u64) <= rows && (c as u64) <= cols && open_ended(&args[0]) {
            return Ok(Value::Empty);
        }
        return Err(CalcError::Ref);
    }
    arity(args, 3, 4)?;
    let key = run(&args[0], ctx)?;
    let t = table(&args[1], ctx)?;
    let col = num(&run(&args[2], ctx)?)?;
    if col < 1.0 {
        return Err(CalcError::Value);
    }
    let approx = match args.get(3) {
        Some(a) => match run(a, ctx)? {
            Value::Bool(b) => b,
            v => num(&v)? != 0.0,
        },
        None => true,
    };
    let first: Vec<Value> = t.iter().map(|row| row[0].clone()).collect();
    let lines: Vec<&[Value]> = t.iter().map(|r| r.as_slice()).collect();
    lookup_row(&key, &first, &lines, col as usize, approx)
}

/// The shared part of `VLOOKUP` and `HLOOKUP`: `first` is the line searched, and the answer is
/// element `pick` (1-based) of the matching entry of `lines`.
fn lookup_row(key: &Value, first: &[Value], lines: &[&[Value]], pick: usize, approx: bool) -> R {
    let mut hit = None;
    for (i, cell) in first.iter().enumerate() {
        match order(cell, key) {
            Ordering::Equal => {
                hit = Some(i);
                if !approx {
                    break;
                }
            }
            // Cells below the data (a whole-column table) are not part of it.
            Ordering::Less if approx && rank(cell) == rank(key) && *cell != Value::Empty => {
                hit = Some(i)
            }
            _ => {}
        }
    }
    let i = hit.ok_or(CalcError::Na)?;
    lines[i].get(pick - 1).cloned().ok_or(CalcError::Ref)
}

/// A spreadsheet date moved by whole months, keeping the day or the month's last day. Excel counts
/// 1900 as a leap year, so serials before 1 March 1900 sit one day off the rest, and serial 0 is
/// the day "0 January 1900"; both are reproduced here so a date column written by Excel agrees.
/// Days since 1970 as a spreadsheet serial, with Excel's missing 29 February 1900.
fn spreadsheet_serial(days_since_1970: i64) -> i64 {
    const SHIFT: i64 = 25569;
    const FIRST_MARCH_1900: i64 = 61;
    let serial = days_since_1970 + SHIFT;
    if serial >= FIRST_MARCH_1900 {
        serial
    } else {
        serial - 1
    }
}

fn add_months(serial: i64, months: i64) -> Result<i64, CalcError> {
    const SHIFT: i64 = 25569;
    const FIRST_MARCH_1900: i64 = 61;
    let to_serial = |days: i64| {
        if days + SHIFT >= FIRST_MARCH_1900 {
            days + SHIFT
        } else {
            days + SHIFT - 1
        }
    };
    let (y, m, d) = if serial == 0 {
        (1900, 1, 0)
    } else if serial < FIRST_MARCH_1900 {
        civil_from_days(serial - SHIFT + 1)
    } else {
        civil_from_days(serial - SHIFT)
    };
    let total = y * 12 + (m - 1) + months;
    let (ny, nm) = (total.div_euclid(12), total.rem_euclid(12) + 1);
    if !(1900..=9999).contains(&ny) {
        return Err(CalcError::Num);
    }
    if d == 0 {
        return Ok(to_serial(days_from_civil(ny, nm, 1)) - 1);
    }
    Ok(to_serial(days_from_civil(
        ny,
        nm,
        d.min(days_in_month(ny, nm)),
    )))
}

fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        2 if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// The cells of a range, or the one value of a single argument, in reading order.
fn range_list(e: &Expr, ctx: &mut dyn Cells) -> Result<Vec<Value>, CalcError> {
    match e {
        Expr::Range(a, b) => range_values(*a, *b, ctx),
        other => Ok(vec![run(other, ctx)?]),
    }
}

/// A `COUNTIF` or `SUMIF` test: `5`, `"text"`, `">5"`, `"<=3"`, `"<>x"`.
fn criterion_matches(cell: &Value, criterion: &Value) -> bool {
    let Value::Text(c) = criterion else {
        // An empty cell is not 0: `COUNTIF(A:A, 0)` does not count blanks.
        return !matches!(cell, Value::Empty)
            && order(cell, criterion) == Ordering::Equal
            && rank(cell) == rank(criterion);
    };
    let (op, rest) = ["<>", "<=", ">=", "<", ">", "="]
        .iter()
        .find_map(|op| c.strip_prefix(op).map(|r| (*op, r)))
        .unwrap_or(("=", c.as_str()));
    // `""` counts blank cells and empty texts, `"="` only blank cells, `"<>"` every cell that is
    // not blank (an empty text included), as in Excel.
    if rest.is_empty() {
        match op {
            "=" if c.is_empty() => {
                return matches!(cell, Value::Empty) || *cell == Value::Text(String::new());
            }
            "=" => return matches!(cell, Value::Empty),
            "<>" => return !matches!(cell, Value::Empty),
            _ => {}
        }
    }
    if (op == "=" || op == "<>") && (rest.contains('*') || rest.contains('?')) {
        let hit = matches!(cell, Value::Text(t) if wildcard_match(&rest.to_lowercase(), &t.to_lowercase()));
        return if op == "=" { hit } else { !hit };
    }
    let wanted = match rest.trim().parse::<f64>() {
        Ok(n) => Value::Num(n),
        Err(_) => Value::Text(rest.to_string()),
    };
    // A comparison only holds between values of the same kind, as in a spreadsheet.
    let same_kind = rank(cell) == rank(&wanted) && !matches!(cell, Value::Empty);
    let ord = order(cell, &wanted);
    match op {
        "<>" => !(same_kind && ord == Ordering::Equal),
        "=" => same_kind && ord == Ordering::Equal,
        "<" => same_kind && ord == Ordering::Less,
        ">" => same_kind && ord == Ordering::Greater,
        "<=" => same_kind && ord != Ordering::Greater,
        _ => same_kind && ord != Ordering::Less,
    }
}

fn count_or_sum_if(name: &str, args: &[Expr], ctx: &mut dyn Cells) -> R {
    let counting = name == "COUNTIF";
    arity(args, 2, if counting { 2 } else { 3 })?;
    let tested = range_list(&args[0], ctx)?;
    let criterion = run(&args[1], ctx)?;
    // A range cut back to the rows in use would count fewer blanks than Excel does.
    if open_ended(&args[0]) && criterion_matches(&Value::Empty, &criterion) {
        return Err(CalcError::Name);
    }
    let summed = if !counting && args.len() == 3 {
        Some(range_list(&args[2], ctx)?)
    } else {
        None
    };
    let mut total = 0.0;
    for (i, cell) in tested.iter().enumerate() {
        if !criterion_matches(cell, &criterion) {
            continue;
        }
        if counting {
            total += 1.0;
        } else if let Value::Num(n) = summed
            .as_ref()
            .map_or(Some(cell), |s| s.get(i))
            .unwrap_or(&Value::Empty)
        {
            total += n;
        }
    }
    Ok(Value::Num(total))
}

/// `*` is any run of characters and `?` any one, as in a `COUNTIF` criterion.
fn wildcard_match(pattern: &str, text: &str) -> bool {
    fn go(p: &[char], t: &[char]) -> bool {
        match p.split_first() {
            None => t.is_empty(),
            Some(('*', rest)) => (0..=t.len()).any(|i| go(rest, &t[i..])),
            Some(('?', rest)) => !t.is_empty() && go(rest, &t[1..]),
            Some((c, rest)) => t.first() == Some(c) && go(rest, &t[1..]),
        }
    }
    let (p, t): (Vec<char>, Vec<char>) = (pattern.chars().collect(), text.chars().collect());
    go(&p, &t)
}

/// Most nodes a `LET` may expand to, so names that build on names cannot grow without limit.
const MAX_LET_NODES: usize = 20_000;

fn size(e: &Expr) -> usize {
    1 + match e {
        Expr::Neg(x) | Expr::Percent(x) => size(x),
        Expr::Binary(_, a, b) | Expr::Compare(_, a, b) => size(a) + size(b),
        Expr::Call(_, args) => args.iter().map(size).sum(),
        _ => 0,
    }
}

/// `e` with every use of the name `name` replaced by `with`.
fn substitute(e: &Expr, name: &str, with: &Expr) -> Expr {
    let sub = |x: &Expr| Box::new(substitute(x, name, with));
    match e {
        Expr::Var(v) if v.eq_ignore_ascii_case(name) => with.clone(),
        Expr::Neg(x) => Expr::Neg(sub(x)),
        Expr::Percent(x) => Expr::Percent(sub(x)),
        Expr::Binary(op, a, b) => Expr::Binary(*op, sub(a), sub(b)),
        Expr::Compare(op, a, b) => Expr::Compare(*op, sub(a), sub(b)),
        Expr::Call(f, args) => Expr::Call(
            f.clone(),
            args.iter().map(|a| substitute(a, name, with)).collect(),
        ),
        other => other.clone(),
    }
}

/// `LET(a, 1, b, a+1, b*2)`: each name stands for its value in what follows. Calculations here
/// are pure, so putting the value expression where the name is gives the same answer.
fn let_call(args: &[Expr], ctx: &mut dyn Cells) -> R {
    if args.len() < 3 || args.len().is_multiple_of(2) {
        return Err(CalcError::Value);
    }
    let (pairs, body) = args.split_at(args.len() - 1);
    let mut rest: Vec<Expr> = pairs.to_vec();
    rest.push(body[0].clone());
    let mut i = 0;
    while i + 1 < rest.len() - 1 {
        let Expr::Var(name) = &rest[i] else {
            return Err(CalcError::Value);
        };
        let (name, value) = (name.clone(), rest[i + 1].clone());
        let mut j = i + 2;
        while j < rest.len() {
            let is_name_slot = j < rest.len() - 1 && (j - i) % 2 == 0;
            if is_name_slot {
                // A name bound again hides this one from there on, but its own value is still
                // worked out with this one in scope.
                let rebinds = matches!(&rest[j], Expr::Var(n) if n.eq_ignore_ascii_case(&name));
                if rebinds {
                    rest[j + 1] = substitute(&rest[j + 1], &name, &value);
                    break;
                }
            } else {
                rest[j] = substitute(&rest[j], &name, &value);
                if size(&rest[j]) > MAX_LET_NODES {
                    return Err(CalcError::Value);
                }
            }
            j += 1;
        }
        i += 2;
    }
    run(&rest[rest.len() - 1], ctx)
}
