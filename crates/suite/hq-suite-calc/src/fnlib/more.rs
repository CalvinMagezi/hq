//! More functions: multi-criteria aggregates, statistics, lookups and text. Each one follows
//! Excel's documented behaviour, and where Excel's behaviour depends on something this engine
//! cannot see (an unsorted lookup column, an unreadable date text) the answer is `#NAME?`, "cannot
//! calculate", never a guess.

use super::rounding::{Mode, round_dec};
use super::*;

/// The values of a one-row or one-column range, or the one value of a single argument.
fn vector(e: &Expr, ctx: &mut dyn Cells) -> Result<Vec<Value>, CalcError> {
    if let Expr::Range(..) = e {
        let (rows, cols) = shape(e);
        if rows > 1 && cols > 1 {
            return Err(CalcError::Na);
        }
    }
    range_list(e, ctx)
}

fn truth(v: Value) -> Result<bool, CalcError> {
    match v {
        Value::Bool(b) => Ok(b),
        other => Ok(num(&other)? != 0.0),
    }
}

/// One `range, criterion` pair of `COUNTIFS` and friends, read.
struct Pair {
    cells: Vec<Value>,
    criterion: Value,
}

fn is_range(e: &Expr) -> bool {
    matches!(e, Expr::Range(..) | Expr::Ref(_))
}

/// The rows (indexes) where every pair holds, over equally shaped ranges.
fn matching_rows(
    pairs: &[Expr],
    target: Option<&Expr>,
    strict: bool,
    ctx: &mut dyn Cells,
) -> Result<(Vec<usize>, Option<Vec<Value>>), CalcError> {
    let first_shape = shape(&pairs[0]);
    let mut read = Vec::new();
    for p in pairs.chunks(2) {
        if !is_range(&p[0]) || shape(&p[0]) != first_shape {
            return Err(CalcError::Value);
        }
        let criterion = run(&p[1], ctx)?;
        // A range cut back to the rows in use would count fewer blanks than Excel does.
        if open_ended(&p[0]) && criterion_matches(&Value::Empty, &criterion) {
            return Err(CalcError::Name);
        }
        read.push(Pair {
            cells: range_list(&p[0], ctx)?,
            criterion,
        });
    }
    let values = match target {
        Some(t) => {
            if !is_range(t) || shape(t) != first_shape {
                // The `...IFS` functions refuse it; `AVERAGEIF` anchors a differently shaped
                // range at its first cell, which is not reproduced.
                return Err(if strict {
                    CalcError::Value
                } else {
                    CalcError::Name
                });
            }
            Some(range_list(t, ctx)?)
        }
        None => None,
    };
    let len = read[0].cells.len();
    let rows = (0..len)
        .filter(|i| {
            read.iter()
                .all(|p| criterion_matches(p.cells.get(*i).unwrap_or(&Value::Empty), &p.criterion))
        })
        .collect();
    Ok((rows, values))
}

fn criteria_family(name: &str, args: &[Expr], ctx: &mut dyn Cells) -> R {
    let (pairs, target): (&[Expr], Option<&Expr>) = match name {
        "COUNTIFS" => {
            if args.len() < 2 || !args.len().is_multiple_of(2) {
                return Err(CalcError::Value);
            }
            (args, None)
        }
        "AVERAGEIF" => {
            arity(args, 2, 3)?;
            (&args[..2], Some(args.get(2).unwrap_or(&args[0])))
        }
        _ => {
            if args.len() < 3 || args.len().is_multiple_of(2) {
                return Err(CalcError::Value);
            }
            (&args[1..], Some(&args[0]))
        }
    };
    let (rows, values) = matching_rows(pairs, target, name != "AVERAGEIF", ctx)?;
    let Some(values) = values else {
        return Ok(Value::Num(rows.len() as f64));
    };
    let mut picked = Vec::new();
    for i in rows {
        match values.get(i) {
            Some(Value::Num(n)) => picked.push(*n),
            Some(Value::Err(e)) => return Err(*e),
            _ => {}
        }
    }
    let sum: f64 = picked.iter().sum();
    match name {
        "SUMIFS" => Ok(Value::Num(sum)),
        "AVERAGEIF" | "AVERAGEIFS" => {
            if picked.is_empty() {
                Err(CalcError::Div0)
            } else {
                Ok(Value::Num(sum / picked.len() as f64))
            }
        }
        "MAXIFS" => Ok(Value::Num(
            picked.into_iter().reduce(f64::max).unwrap_or(0.0),
        )),
        _ => Ok(Value::Num(
            picked.into_iter().reduce(f64::min).unwrap_or(0.0),
        )),
    }
}

/// Checks that the values that count (a trailing run of empty cells is ignored) are all of the key's
/// kind and in order. A binary search over anything else gives an answer that depends on how Excel
/// happens to probe, which is not reproduced.
fn ordered(key: &Value, v: &[Value], ascending: bool) -> Result<usize, CalcError> {
    let used = v
        .iter()
        .rposition(|c| *c != Value::Empty)
        .map_or(0, |i| i + 1);
    let v = &v[..used];
    if v.iter().any(|c| *c == Value::Empty || rank(c) != rank(key)) {
        return Err(CalcError::Name);
    }
    for w in v.windows(2) {
        let o = order(&w[0], &w[1]);
        if (ascending && o == Ordering::Greater) || (!ascending && o == Ordering::Less) {
            return Err(CalcError::Name);
        }
    }
    Ok(used)
}

/// `MATCH`: the zero-based position, or `#N/A`.
fn match_pos(key: &Value, v: &[Value], kind: f64) -> Result<usize, CalcError> {
    if *key == Value::Empty {
        return Err(CalcError::Name);
    }
    if kind == 0.0 {
        if let Value::Text(t) = key {
            if t.contains('~') {
                return Err(CalcError::Name);
            }
            if t.contains('*') || t.contains('?') {
                let pat = t.to_lowercase();
                return v
                    .iter()
                    .position(
                        |c| matches!(c, Value::Text(s) if wildcard_match(&pat, &s.to_lowercase())),
                    )
                    .ok_or(CalcError::Na);
            }
        }
        return v
            .iter()
            .position(|c| {
                *c != Value::Empty && rank(c) == rank(key) && order(c, key) == Ordering::Equal
            })
            .ok_or(CalcError::Na);
    }
    let ascending = kind > 0.0;
    let used = ordered(key, v, ascending)?;
    let mut hit = None;
    for (i, c) in v[..used].iter().enumerate() {
        let o = order(c, key);
        let fits = if ascending {
            o != Ordering::Greater
        } else {
            o != Ordering::Less
        };
        if fits {
            hit = Some(i);
        }
    }
    let i = hit.ok_or(CalcError::Na)?;
    // With equal entries in a descending list Excel's pick is not reproduced.
    if !ascending
        && v[..used]
            .iter()
            .filter(|c| order(c, &v[i]) == Ordering::Equal)
            .count()
            > 1
    {
        return Err(CalcError::Name);
    }
    Ok(i)
}

/// `VALUE`: a number (with thousands commas and a percent sign) or an ISO date.
fn value_of(t: &str) -> R {
    let s = t.trim();
    let (body, percent) = match s.strip_suffix('%') {
        Some(b) => (b.trim_end(), true),
        None => (s, false),
    };
    let digits_only = |p: &str| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit());
    let cleaned = if body.contains(',') {
        let unsigned = body.trim_start_matches(['+', '-']);
        let (int, rest) = unsigned
            .split_once('.')
            .map_or((unsigned, None), |(a, b)| (a, Some(b)));
        let groups: Vec<&str> = int.split(',').collect();
        let well_formed = groups.len() > 1
            && digits_only(groups[0])
            && groups[0].len() <= 3
            && groups[1..].iter().all(|g| g.len() == 3 && digits_only(g))
            && rest.is_none_or(|r| r.is_empty() || digits_only(r));
        if !well_formed {
            return Err(CalcError::Name);
        }
        body.replace(',', "")
    } else {
        body.to_string()
    };
    match num(&Value::Text(cleaned)) {
        Ok(n) => Ok(Value::Num(if percent { n / 100.0 } else { n })),
        // Other date styles, currency signs and times might be numbers in Excel.
        Err(_) if t.chars().any(|c| c.is_ascii_digit()) => Err(CalcError::Name),
        Err(e) => Err(e),
    }
}

fn proper(t: &str) -> String {
    let mut out = String::new();
    let mut after_letter = false;
    for c in t.chars() {
        if c.is_alphabetic() {
            if after_letter {
                out.extend(c.to_lowercase());
            } else {
                out.extend(c.to_uppercase());
            }
            after_letter = true;
        } else {
            out.push(c);
            after_letter = false;
        }
    }
    out
}

pub(super) fn call(name: &str, args: &[Expr], ctx: &mut dyn Cells) -> R {
    match name {
        "TRUE" | "FALSE" => {
            arity(args, 0, 0)?;
            Ok(Value::Bool(name == "TRUE"))
        }
        "IFNA" => {
            arity(args, 2, 2)?;
            match run(&args[0], ctx) {
                Err(CalcError::Na) => run(&args[1], ctx),
                other => other,
            }
        }
        "COUNTIFS" | "SUMIFS" | "AVERAGEIF" | "AVERAGEIFS" | "MAXIFS" | "MINIFS" => {
            criteria_family(name, args, ctx)
        }
        "MEDIAN" => {
            let mut n = numbers(args, ctx)?;
            if n.is_empty() {
                return Err(CalcError::Num);
            }
            n.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
            let mid = n.len() / 2;
            Ok(Value::Num(if n.len() % 2 == 1 {
                n[mid]
            } else {
                (n[mid - 1] + n[mid]) / 2.0
            }))
        }
        "LARGE" | "SMALL" => {
            arity(args, 2, 2)?;
            let mut n = numbers(&args[..1], ctx)?;
            let k = num(&run(&args[1], ctx)?)?;
            if k.fract() != 0.0 {
                // How Excel rounds a fractional k is not reproduced.
                return Err(CalcError::Name);
            }
            if k < 1.0 || k > n.len() as f64 {
                return Err(CalcError::Num);
            }
            n.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
            let i = k as usize - 1;
            Ok(Value::Num(if name == "SMALL" {
                n[i]
            } else {
                n[n.len() - 1 - i]
            }))
        }
        "RANK" | "RANK.EQ" => {
            arity(args, 2, 3)?;
            let x = num(&run(&args[0], ctx)?)?;
            if !is_range(&args[1]) {
                return Err(CalcError::Value);
            }
            let ascending = args.len() == 3 && truth(run(&args[2], ctx)?)?;
            let n = numbers(&args[1..2], ctx)?;
            if !n.contains(&x) {
                return Err(CalcError::Na);
            }
            let ahead = n
                .iter()
                .filter(|v| if ascending { **v < x } else { **v > x })
                .count();
            Ok(Value::Num(ahead as f64 + 1.0))
        }
        "MATCH" => {
            arity(args, 2, 3)?;
            let key = run(&args[0], ctx)?;
            let v = vector(&args[1], ctx)?;
            let kind = if args.len() == 3 {
                num(&run(&args[2], ctx)?)?
            } else {
                1.0
            };
            Ok(Value::Num(match_pos(&key, &v, kind)? as f64 + 1.0))
        }
        "LOOKUP" => {
            arity(args, 2, 3)?;
            let key = run(&args[0], ctx)?;
            let (rows, cols) = shape(&args[1]);
            if rows > 1 && cols > 1 {
                // The array form picks its search direction from the shape; not reproduced.
                return Err(CalcError::Name);
            }
            let look = vector(&args[1], ctx)?;
            let i = match_pos(&key, &look, 1.0)?;
            let answer = match args.get(2) {
                Some(r) => vector(r, ctx)?,
                None => look,
            };
            answer.get(i).cloned().ok_or(CalcError::Na)
        }
        "HLOOKUP" => {
            arity(args, 3, 4)?;
            let key = run(&args[0], ctx)?;
            let t = table(&args[1], ctx)?;
            let row = num(&run(&args[2], ctx)?)?;
            if row < 1.0 {
                return Err(CalcError::Value);
            }
            let approx = match args.get(3) {
                Some(a) => truth(run(a, ctx)?)?,
                None => true,
            };
            let width = t.first().map_or(0, Vec::len);
            let columns: Vec<Vec<Value>> = (0..width)
                .map(|c| t.iter().map(|r| r[c].clone()).collect())
                .collect();
            let lines: Vec<&[Value]> = columns.iter().map(Vec::as_slice).collect();
            lookup_row(&key, &t[0], &lines, row as usize, approx)
        }
        "VALUE" => {
            arity(args, 1, 1)?;
            match run(&args[0], ctx)? {
                n @ Value::Num(_) => Ok(n),
                Value::Empty => Ok(Value::Num(0.0)),
                Value::Bool(_) => Err(CalcError::Value),
                Value::Text(t) => value_of(&t),
                Value::Err(e) => Err(e),
            }
        }
        "REPT" => {
            arity(args, 2, 2)?;
            let t = text_of(&run(&args[0], ctx)?);
            let n = num(&run(&args[1], ctx)?)?.trunc();
            if n < 0.0 || (t.chars().count() as f64) * n > 32_767.0 {
                return Err(CalcError::Value);
            }
            Ok(Value::Text(t.repeat(n as usize)))
        }
        "PROPER" => {
            arity(args, 1, 1)?;
            Ok(Value::Text(proper(&text_of(&run(&args[0], ctx)?))))
        }
        "MID" => {
            arity(args, 3, 3)?;
            let t: Vec<char> = text_of(&run(&args[0], ctx)?).chars().collect();
            let (start, len) = (
                num(&run(&args[1], ctx)?)?.trunc(),
                num(&run(&args[2], ctx)?)?.trunc(),
            );
            if start < 1.0 || len < 0.0 {
                return Err(CalcError::Value);
            }
            let from = (start as usize - 1).min(t.len());
            let to = from.saturating_add(len as usize).min(t.len());
            Ok(Value::Text(t[from..to].iter().collect()))
        }
        "EXACT" => {
            arity(args, 2, 2)?;
            let (a, b) = (run(&args[0], ctx)?, run(&args[1], ctx)?);
            Ok(Value::Bool(text_of(&a) == text_of(&b)))
        }
        "TRUNC" => {
            arity(args, 1, 2)?;
            let x = num(&run(&args[0], ctx)?)?;
            let d = if args.len() == 2 {
                num(&run(&args[1], ctx)?)?
            } else {
                0.0
            };
            Ok(Value::Num(round_dec(x, d, Mode::Down)))
        }
        "SIGN" => {
            arity(args, 1, 1)?;
            let x = num(&run(&args[0], ctx)?)?;
            Ok(Value::Num(if x == 0.0 { 0.0 } else { x.signum() }))
        }
        "CEILING" | "FLOOR" => {
            arity(args, 2, 2)?;
            let (x, sig) = (num(&run(&args[0], ctx)?)?, num(&run(&args[1], ctx)?)?);
            if x > 0.0 && sig < 0.0 {
                return Err(CalcError::Num);
            }
            if sig == 0.0 {
                return if name == "CEILING" {
                    Ok(Value::Num(0.0))
                } else {
                    Err(CalcError::Div0)
                };
            }
            // Snap the quotient to 15 digits so 2.5 / 0.1 is 25, not 25.000000000000004.
            let q = round_dec(x / sig, 14.0, Mode::Half);
            let steps = if name == "CEILING" {
                q.ceil()
            } else {
                q.floor()
            };
            Ok(Value::Num(steps * sig))
        }
        _ => dates::call(name, args, ctx),
    }
}
