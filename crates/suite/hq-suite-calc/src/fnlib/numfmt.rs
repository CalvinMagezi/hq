//! `TEXT(value, format)`: number, percent, currency-like and date formats.
//!
//! Anything this does not reproduce exactly (times, colours, `[$...]` currency tags, fractions,
//! scientific, scaling commas, fill characters, conditions) returns `None`, which the caller turns
//! into "cannot calculate". A format is never approximated.

use super::dates::{decode, weekday_index};
use super::rounding::fixed;
use super::{CalcError, Value, num};
use crate::value::format_number;

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const DAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

/// Characters Excel prints as they are without quotes (besides the ones that mean something).
fn plain_literal(c: char) -> bool {
    matches!(
        c,
        '$' | '-'
            | '+'
            | '('
            | ')'
            | ':'
            | ' '
            | '!'
            | '^'
            | '&'
            | '\''
            | '~'
            | '{'
            | '}'
            | '<'
            | '>'
            | '='
            | '/'
            | '€'
            | '£'
            | '¥'
            | '₹'
            | '¢'
    )
}

enum Tok {
    Lit(String),
    /// A run of a date letter: `y`, `m` or `d` and how many.
    Date(char, usize),
    /// A run of `0 # , .` placeholders.
    Digits(String),
    /// A `%`: prints itself and multiplies by 100.
    Percent,
}

/// A section's tokens, or `None` for anything not reproduced.
fn tokens(section: &str, date: bool) -> Option<Vec<Tok>> {
    let chars: Vec<char> = section.chars().collect();
    let mut out: Vec<Tok> = Vec::new();
    let mut i = 0;
    let push_lit = |out: &mut Vec<Tok>, s: &str| {
        if let Some(Tok::Lit(last)) = out.last_mut() {
            last.push_str(s);
        } else {
            out.push(Tok::Lit(s.to_string()));
        }
    };
    while i < chars.len() {
        let c = chars[i];
        match c {
            '"' => {
                let end = chars[i + 1..].iter().position(|x| *x == '"')? + i + 1;
                push_lit(&mut out, &chars[i + 1..end].iter().collect::<String>());
                i = end + 1;
            }
            '\\' => {
                push_lit(&mut out, &chars.get(i + 1)?.to_string());
                i += 2;
            }
            '_' => {
                chars.get(i + 1)?;
                push_lit(&mut out, " ");
                i += 2;
            }
            ',' | '.' if date => {
                push_lit(&mut out, &c.to_string());
                i += 1;
            }
            '0' | '#' if date => return None,
            '0' | '#' | ',' | '.' => {
                let start = i;
                while i < chars.len() && matches!(chars[i], '0' | '#' | ',' | '.') {
                    i += 1;
                }
                let run: String = chars[start..i].iter().collect();
                out.push(Tok::Digits(run));
            }
            '%' => {
                out.push(Tok::Percent);
                i += 1;
            }
            'y' | 'Y' | 'm' | 'M' | 'd' | 'D' => {
                let l = c.to_ascii_lowercase();
                let start = i;
                while i < chars.len() && chars[i].to_ascii_lowercase() == l {
                    i += 1;
                }
                out.push(Tok::Date(l, i - start));
            }
            c if plain_literal(c) => {
                push_lit(&mut out, &c.to_string());
                i += 1;
            }
            _ => return None,
        }
    }
    Some(out)
}

/// Splits on `;` outside quotes and escapes.
fn sections(format: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    let (mut quoted, mut escaped) = (false, false);
    for c in format.chars() {
        let cur = out.last_mut().expect("one section at least");
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == '"' {
            quoted = !quoted;
        } else if c == ';' && !quoted {
            out.push(String::new());
            continue;
        }
        cur.push(c);
    }
    out
}

pub(super) fn text(value: &Value, format: &str) -> Result<String, CalcError> {
    match value {
        Value::Err(e) => return Err(*e),
        Value::Bool(_) => return Ok(value.to_string()),
        _ => {}
    }
    if format.eq_ignore_ascii_case("general") {
        return Ok(match value {
            Value::Text(t) => t.clone(),
            v => format_number(num(v)?),
        });
    }
    if format == "@" {
        return Ok(value.to_string());
    }
    let n = match num(value) {
        Ok(n) => n,
        // A text that is not a number comes back as it is (formats with `@` are not reproduced).
        Err(_) if !format.contains('@') => return Ok(value.to_string()),
        Err(_) => return Err(CalcError::Name),
    };
    let secs = sections(format);
    if secs.len() > 3 {
        return Err(CalcError::Name);
    }
    let (section, own_sign) = match (secs.len(), n) {
        (1, _) => (&secs[0], false),
        (_, x) if x < 0.0 => (&secs[1], true),
        (3, 0.0) => (&secs[2], true),
        _ => (&secs[0], true),
    };
    let mut toks = tokens(section, false).ok_or(CalcError::Name)?;
    let is_date = toks.iter().any(|t| matches!(t, Tok::Date(..)));
    if is_date {
        toks = tokens(section, true).ok_or(CalcError::Name)?;
    }
    let shown = if is_date {
        if secs.len() > 1 {
            return Err(CalcError::Name);
        }
        date_text(&toks, n)?
    } else {
        number_text(&toks, n, own_sign)?
    };
    Ok(shown)
}

fn date_text(toks: &[Tok], n: f64) -> Result<String, CalcError> {
    if toks
        .iter()
        .any(|t| matches!(t, Tok::Digits(_) | Tok::Percent))
    {
        return Err(CalcError::Name);
    }
    if !(0.0..2_958_466.0).contains(&n) {
        return Err(CalcError::Value);
    }
    let serial = n.floor() as i64;
    let (y, m, d) = decode(serial);
    let mut out = String::new();
    for t in toks {
        match t {
            Tok::Lit(s) => out.push_str(s),
            Tok::Date('y', 2) => out.push_str(&format!("{:02}", y % 100)),
            Tok::Date('y', 4) => out.push_str(&format!("{y:04}")),
            Tok::Date('m', 1) => out.push_str(&m.to_string()),
            Tok::Date('m', 2) => out.push_str(&format!("{m:02}")),
            Tok::Date('m', 3) => out.push_str(&MONTHS[m as usize - 1][..3]),
            Tok::Date('m', 4) => out.push_str(MONTHS[m as usize - 1]),
            Tok::Date('m', 5) => out.push_str(&MONTHS[m as usize - 1][..1]),
            Tok::Date('d', 1) => out.push_str(&d.to_string()),
            Tok::Date('d', 2) => out.push_str(&format!("{d:02}")),
            Tok::Date('d', 3) => out.push_str(&DAYS[weekday_index(serial) as usize][..3]),
            Tok::Date('d', 4) => out.push_str(DAYS[weekday_index(serial) as usize]),
            _ => return Err(CalcError::Name),
        }
    }
    Ok(out)
}

fn number_text(toks: &[Tok], n: f64, own_sign: bool) -> Result<String, CalcError> {
    let digit_runs = toks.iter().filter(|t| matches!(t, Tok::Digits(_))).count();
    if digit_runs > 1 {
        return Err(CalcError::Name);
    }
    let percents = toks.iter().filter(|t| matches!(t, Tok::Percent)).count();
    let scaled = n.abs() * 100f64.powi(percents as i32);
    let mut body = String::new();
    for (i, t) in toks.iter().enumerate() {
        match t {
            Tok::Lit(s) => body.push_str(s),
            Tok::Percent => body.push('%'),
            Tok::Digits(pat) => {
                // `0/0` would be a fraction, which is not reproduced.
                let next_is_slash =
                    matches!(toks.get(i + 1), Some(Tok::Lit(s)) if s.starts_with('/'));
                if next_is_slash {
                    return Err(CalcError::Name);
                }
                body.push_str(&digits(pat, scaled)?);
            }
            Tok::Date(..) => return Err(CalcError::Name),
        }
    }
    if n < 0.0 && !own_sign {
        // A small negative that shows as zero (`-0.001` as `0.00`) is not reproduced.
        if digit_runs == 1 && body.chars().all(|c| !c.is_ascii_digit() || c == '0') {
            return Err(CalcError::Name);
        }
        return Ok(format!("-{body}"));
    }
    Ok(body)
}

/// One run of `0 # , .` placeholders filled with `v` (never negative).
fn digits(pattern: &str, v: f64) -> Result<String, CalcError> {
    let (int_pat, frac_pat) = match pattern.split_once('.') {
        Some((a, b)) => (a, Some(b)),
        None => (pattern, None),
    };
    if frac_pat.is_some_and(|f| !f.chars().all(|c| matches!(c, '0' | '#'))) {
        return Err(CalcError::Name);
    }
    // A comma must sit between digit placeholders; one at the end scales by 1,000 (not reproduced).
    if int_pat.starts_with(',') || int_pat.ends_with(',') {
        return Err(CalcError::Name);
    }
    if int_pat.contains(",,") {
        return Err(CalcError::Name);
    }
    let grouping = int_pat.contains(',');
    let frac_pat = frac_pat.unwrap_or("");
    let shown = fixed(v, frac_pat.len());
    let (whole, frac) = shown.split_once('.').unwrap_or((shown.as_str(), ""));
    let min_digits = int_pat.chars().filter(|c| *c == '0').count();
    let mut whole = if whole == "0" && min_digits == 0 {
        String::new()
    } else {
        whole.to_string()
    };
    while whole.len() < min_digits {
        whole.insert(0, '0');
    }
    if grouping {
        let bytes: Vec<char> = whole.chars().collect();
        let mut grouped = String::new();
        for (i, c) in bytes.iter().enumerate() {
            if i > 0 && (bytes.len() - i).is_multiple_of(3) {
                grouped.push(',');
            }
            grouped.push(*c);
        }
        whole = grouped;
    }
    let mut frac: Vec<char> = frac.chars().collect();
    let pat: Vec<char> = frac_pat.chars().collect();
    while frac.last() == Some(&'0') && pat.get(frac.len() - 1) == Some(&'#') {
        frac.pop();
    }
    let dot = if pattern.contains('.') { "." } else { "" };
    Ok(format!("{whole}{dot}{}", frac.iter().collect::<String>()))
}
