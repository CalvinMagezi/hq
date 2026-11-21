//! Dates as spreadsheet serial numbers, with Excel's 1900 quirks: serial 0 is "0 January 1900",
//! serial 60 is the 29 February 1900 that never existed, and day 1 is called a Sunday.

use super::*;

/// 31 December 9999, the last date a spreadsheet has.
const LAST_SERIAL: i64 = 2_958_465;

/// A serial as (year, month, day).
pub(super) fn decode(serial: i64) -> (i64, i64, i64) {
    const SHIFT: i64 = 25569;
    match serial {
        0 => (1900, 1, 0),
        60 => (1900, 2, 29),
        s if s < 60 => civil_from_days(s - SHIFT + 1),
        s => civil_from_days(s - SHIFT),
    }
}

/// The serial of the first day of a month.
fn first_of_month(y: i64, m: i64) -> i64 {
    spreadsheet_serial(days_from_civil(y, m, 1))
}

/// 0 is Sunday. Day 1 (1 January 1900) is a Sunday in a spreadsheet, whatever the calendar says.
pub(super) fn weekday_index(serial: i64) -> i64 {
    (serial - 1).rem_euclid(7)
}

/// A date argument: a number, or a text that is a date. A text that is neither a number nor an
/// ISO date but has digits might be a date in some other style, which is not read: it stays
/// "cannot calculate" instead of becoming `#VALUE!`.
fn date_arg(e: &Expr, ctx: &mut dyn Cells) -> Result<i64, CalcError> {
    let v = run(e, ctx)?;
    let n = match num(&v) {
        Ok(n) => n,
        Err(CalcError::Value) => {
            return match &v {
                Value::Text(t) if t.chars().any(|c| c.is_ascii_digit()) => Err(CalcError::Name),
                _ => Err(CalcError::Value),
            };
        }
        Err(e) => return Err(e),
    };
    let serial = n.floor();
    if !(0.0..=LAST_SERIAL as f64).contains(&serial) {
        return Err(CalcError::Num);
    }
    Ok(serial as i64)
}

fn int_arg(e: &Expr, ctx: &mut dyn Cells) -> Result<i64, CalcError> {
    Ok(num(&run(e, ctx)?)?.trunc() as i64)
}

fn in_range(serial: i64) -> Result<f64, CalcError> {
    if (0..=LAST_SERIAL).contains(&serial) {
        Ok(serial as f64)
    } else {
        Err(CalcError::Num)
    }
}

pub(super) fn call(name: &str, args: &[Expr], ctx: &mut dyn Cells) -> R {
    match name {
        "DATE" => {
            arity(args, 3, 3)?;
            let (mut y, m, d) = (
                int_arg(&args[0], ctx)?,
                int_arg(&args[1], ctx)?,
                int_arg(&args[2], ctx)?,
            );
            if (0..=1899).contains(&y) {
                y += 1900;
            }
            if !(0..=9999).contains(&y) {
                return Err(CalcError::Num);
            }
            let total = y * 12 + (m - 1);
            let (ny, nm) = (total.div_euclid(12), total.rem_euclid(12) + 1);
            if !(1900..=9999).contains(&ny) {
                return Err(CalcError::Num);
            }
            Ok(Value::Num(in_range(first_of_month(ny, nm) + d - 1)?))
        }
        "YEAR" | "MONTH" | "DAY" => {
            arity(args, 1, 1)?;
            let (y, m, d) = decode(date_arg(&args[0], ctx)?);
            Ok(Value::Num(match name {
                "YEAR" => y,
                "MONTH" => m,
                _ => d,
            } as f64))
        }
        "WEEKDAY" => {
            arity(args, 1, 2)?;
            let serial = date_arg(&args[0], ctx)?;
            let kind = if args.len() == 2 {
                int_arg(&args[1], ctx)?
            } else {
                1
            };
            let sunday = weekday_index(serial);
            let out = match kind {
                1 => sunday + 1,
                2 => (sunday + 6) % 7 + 1,
                3 => (sunday + 6) % 7,
                // 11 is Monday first, 12 Tuesday first, up to 17 Sunday first.
                11..=17 => (sunday + 6 - (kind - 11) + 7) % 7 + 1,
                _ => return Err(CalcError::Num),
            };
            Ok(Value::Num(out as f64))
        }
        "EOMONTH" => {
            arity(args, 2, 2)?;
            let (y, m, _) = decode(date_arg(&args[0], ctx)?);
            let months = int_arg(&args[1], ctx)?;
            let total = y * 12 + (m - 1) + months + 1;
            let (ny, nm) = (total.div_euclid(12), total.rem_euclid(12) + 1);
            if !(1900..=10000).contains(&ny) {
                return Err(CalcError::Num);
            }
            Ok(Value::Num(in_range(first_of_month(ny, nm) - 1)?))
        }
        "DATEDIF" => {
            arity(args, 3, 3)?;
            let (a, b) = (date_arg(&args[0], ctx)?, date_arg(&args[1], ctx)?);
            let Value::Text(unit) = run(&args[2], ctx)? else {
                return Err(CalcError::Value);
            };
            if a > b {
                return Err(CalcError::Num);
            }
            let ((sy, sm, sd), (ey, em, ed)) = (decode(a), decode(b));
            let months = (ey - sy) * 12 + (em - sm) - i64::from(ed < sd);
            Ok(Value::Num(match unit.to_ascii_uppercase().as_str() {
                "Y" => months.div_euclid(12),
                "M" => months,
                "D" => b - a,
                "YM" => months.rem_euclid(12),
                // Excel's own "MD" and "YD" give odd answers around month ends and leap days, and
                // are not reproduced.
                "MD" | "YD" => return Err(CalcError::Name),
                _ => return Err(CalcError::Num),
            } as f64))
        }
        _ => Err(CalcError::Name),
    }
}
