//! A tree to a value.

use std::cmp::Ordering;

use crate::parse::{CellRef, Expr};
use crate::value::{CalcError, Value};

/// Where cells come from while evaluating.
pub(crate) trait Cells {
    fn cell(&mut self, at: CellRef) -> Value;
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
];

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
    }
}

/// A value as a number: empty is 0, a truth value is 0 or 1, a text that reads as a number is
/// that number, any other text is `#VALUE!`.
pub(crate) fn num(v: &Value) -> Result<f64, CalcError> {
    match v {
        Value::Empty => Ok(0.0),
        Value::Num(n) => Ok(*n),
        Value::Bool(b) => Ok(f64::from(u8::from(*b))),
        Value::Text(t) => t.trim().parse::<f64>().map_err(|_| CalcError::Value),
        Value::Err(e) => Err(*e),
    }
}

fn text_of(v: &Value) -> String {
    v.to_string()
}

fn binary(op: char, a: &Value, b: &Value) -> R {
    if op == '&' {
        return Ok(Value::Text(format!("{}{}", text_of(a), text_of(b))));
    }
    let (x, y) = (num(a)?, num(b)?);
    let out = match op {
        '+' => x + y,
        '-' => x - y,
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

/// Every cell of a range, row by row.
fn range_values(a: CellRef, b: CellRef, ctx: &mut dyn Cells) -> Result<Vec<Value>, CalcError> {
    let (c0, c1) = (a.col.min(b.col), a.col.max(b.col));
    let (r0, r1) = (a.row.min(b.row), a.row.max(b.row));
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

/// Rounds half away from zero, as a spreadsheet does (Rust's `round` already does).
fn round_to(x: f64, digits: f64) -> f64 {
    let d = digits.trunc().clamp(-15.0, 15.0) as i32;
    let m = 10f64.powi(d.abs());
    if d >= 0 {
        (x * m).round() / m
    } else {
        (x / m).round() * m
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
            let m = 10f64.powi(d.trunc().clamp(-15.0, 15.0) as i32);
            let scaled = x * m;
            let cut = if name == "ROUNDUP" {
                scaled.signum() * scaled.abs().ceil()
            } else {
                scaled.trunc()
            };
            Ok(Value::Num(cut / m))
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
            Ok(Value::Num(round_to(x, d)))
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
        _ => Err(CalcError::Name),
    }
}

fn one(args: &[Expr], ctx: &mut dyn Cells, f: fn(f64) -> f64) -> R {
    arity(args, 1, 1)?;
    Ok(Value::Num(f(num(&run(&args[0], ctx)?)?)))
}
