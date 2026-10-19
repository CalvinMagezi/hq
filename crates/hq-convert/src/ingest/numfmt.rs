//! Excel number formats applied to a number: `0.0%`, `#,##0`, `"UGX" #,##0`,
//! `(#,##0)`, `0.00E+00` and the rest of the common ones.
//!
//! A spreadsheet stores 0.097 and a format code; what a person sees is "9.7%".
//! This turns the pair into that text. Dates and times are the spreadsheet
//! reader's job, and formats this does not model (fractions, conditions,
//! engineering notation) return `None`, so the caller falls back to the plain
//! number instead of showing something wrong.

/// The built-in format codes a style can name by number (ECMA-376, 18.8.30).
/// `None` for General, dates and times, which are handled elsewhere.
pub fn builtin(id: u32) -> Option<&'static str> {
    Some(match id {
        1 => "0",
        2 => "0.00",
        3 => "#,##0",
        4 => "#,##0.00",
        5 => "$#,##0_);($#,##0)",
        6 => "$#,##0_);[Red]($#,##0)",
        7 => "$#,##0.00_);($#,##0.00)",
        8 => "$#,##0.00_);[Red]($#,##0.00)",
        9 => "0%",
        10 => "0.00%",
        11 => "0.00E+00",
        37 => "#,##0_);(#,##0)",
        38 => "#,##0_);[Red](#,##0)",
        39 => "#,##0.00_);(#,##0.00)",
        40 => "#,##0.00_);[Red](#,##0.00)",
        41 => r#"_(* #,##0_);_(* \(#,##0\);_(* "-"_);_(@_)"#,
        42 => r#"_("$"* #,##0_);_("$"* \(#,##0\);_("$"* "-"_);_(@_)"#,
        43 => r#"_(* #,##0.00_);_(* \(#,##0.00\);_(* "-"??_);_(@_)"#,
        44 => r#"_("$"* #,##0.00_);_("$"* \(#,##0.00\);_("$"* "-"??_);_(@_)"#,
        _ => return None,
    })
}

/// Which parts of a date and time a format shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateKind {
    pub date: bool,
    pub time: bool,
    pub seconds: bool,
}

/// What a built-in date or time format number shows (ECMA-376, 18.8.30), or `None` for a number format.
pub fn builtin_date(id: u32) -> Option<DateKind> {
    let (date, time, seconds) = match id {
        14..=17 | 27..=36 | 50..=58 => (true, false, false),
        18 | 20 => (false, true, false),
        19 | 21 | 45 | 47 => (false, true, true),
        22 => (true, true, false),
        _ => return None,
    };
    Some(DateKind {
        date,
        time,
        seconds,
    })
}

/// Which parts a custom date or time format shows, or `None` when it is not one.
pub fn date_kind(code: &str) -> Option<DateKind> {
    let first = sections(code).into_iter().next()?;
    let (mut date, mut time, mut seconds) = (false, false, false);
    let (mut quoted, mut escaped, mut bracket) = (false, false, false);
    let lower = first.to_lowercase();
    // AM/PM is not a month, and means a clock time.
    let meridiem = lower.contains("am/pm") || lower.contains("a/p");
    let lower = lower.replace("am/pm", "").replace("a/p", "");
    time |= meridiem;
    let chars: Vec<char> = lower.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if escaped {
            escaped = false;
        } else if quoted {
            quoted = c != '"';
        } else if bracket {
            // [h] [mm] [ss] are elapsed time; [Red] and [$-409] are not parts at all.
            if matches!(c, 'h' | 's') {
                time = true;
            }
            bracket = c != ']';
        } else {
            match c {
                '\\' | '_' | '*' => escaped = true,
                '"' => quoted = true,
                '[' => bracket = true,
                'y' | 'd' => date = true,
                'h' => time = true,
                's' => {
                    time = true;
                    seconds = true;
                }
                // m is a month, unless it sits next to hours or seconds.
                'm' => {
                    // The whole run (m, mm, mmm) is judged by what sits either side of it.
                    let start = chars[..i]
                        .iter()
                        .rposition(|c| *c != 'm')
                        .map_or(0, |p| p + 1);
                    let end = chars[i..]
                        .iter()
                        .position(|c| *c != 'm')
                        .map_or(chars.len(), |p| i + p);
                    let near_time = chars[..start].iter().rev().find(|c| c.is_alphabetic())
                        == Some(&'h')
                        || chars[end..].iter().find(|c| c.is_alphabetic()) == Some(&'s');
                    if near_time {
                        time = true;
                    } else {
                        date = true;
                    }
                }
                _ => {}
            }
        }
    }
    (date || time).then_some(DateKind {
        date,
        time,
        seconds,
    })
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Lit(String),
    /// `0`, `#` or `?`.
    Digit(char),
    Point,
    Comma,
    Percent,
    /// `E+` or `E-`; the digit placeholders after it are the exponent.
    Exp {
        always_sign: bool,
    },
    /// The word General inside a longer format.
    General,
}

/// Splits a format into its `;` sections, ignoring semicolons inside quotes,
/// brackets and after a backslash.
fn sections(code: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    let (mut quoted, mut bracket, mut escaped) = (false, false, false);
    for c in code.chars() {
        if escaped {
            escaped = false;
        } else if quoted {
            quoted = c != '"';
        } else {
            match c {
                '\\' | '_' | '*' => escaped = true,
                '"' => quoted = true,
                '[' => bracket = true,
                ']' => bracket = false,
                ';' if !bracket => {
                    out.push(String::new());
                    continue;
                }
                _ => {}
            }
        }
        if let Some(cur) = out.last_mut() {
            cur.push(c);
        }
    }
    out
}

/// Turns one section into tokens, or `None` for anything this does not model.
fn tokenize(section: &str) -> Option<Vec<Tok>> {
    let chars: Vec<char> = section.chars().collect();
    let mut toks: Vec<Tok> = Vec::new();
    let mut lit = String::new();
    let flush = |lit: &mut String, toks: &mut Vec<Tok>| {
        if !lit.is_empty() {
            toks.push(Tok::Lit(std::mem::take(lit)));
        }
    };
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '"' => {
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    lit.push(chars[i]);
                    i += 1;
                }
            }
            '\\' => {
                i += 1;
                if let Some(&n) = chars.get(i) {
                    lit.push(n);
                }
            }
            '_' => {
                i += 1;
                lit.push(' ');
            }
            // Fill: repeat a character to the cell's width. Nothing to show in text.
            '*' => i += 1,
            '[' => {
                let end = chars[i..].iter().position(|&c| c == ']')? + i;
                let inner: String = chars[i + 1..end].iter().collect();
                if let Some(cur) = inner.strip_prefix('$') {
                    // [$UGX-409] is the currency text before the locale code.
                    lit.push_str(cur.split('-').next().unwrap_or(""));
                } else if inner.starts_with(['<', '>', '=']) {
                    return None; // a condition
                }
                i = end;
            }
            '0' | '#' | '?' => {
                flush(&mut lit, &mut toks);
                toks.push(Tok::Digit(c));
            }
            '.' => {
                flush(&mut lit, &mut toks);
                toks.push(Tok::Point);
            }
            ',' => {
                flush(&mut lit, &mut toks);
                toks.push(Tok::Comma);
            }
            '%' => {
                flush(&mut lit, &mut toks);
                toks.push(Tok::Percent);
            }
            'E' | 'e' if matches!(chars.get(i + 1), Some('+' | '-')) => {
                flush(&mut lit, &mut toks);
                toks.push(Tok::Exp {
                    always_sign: chars[i + 1] == '+',
                });
                i += 1;
            }
            'G' | 'g'
                if section[char_offset(section, i)..]
                    .to_lowercase()
                    .starts_with("general") =>
            {
                flush(&mut lit, &mut toks);
                toks.push(Tok::General);
                i += "general".len() - 1;
            }
            // Dates, times, AM/PM, fractions and the text placeholder are not numbers.
            'd' | 'D' | 'm' | 'M' | 'y' | 'Y' | 'h' | 'H' | 's' | 'S' | 'A' | 'a' | 'P' | 'p'
            | '@' => return None,
            '/' if toks.iter().any(|t| matches!(t, Tok::Digit(_))) => return None,
            other => lit.push(other),
        }
        i += 1;
    }
    flush(&mut lit, &mut toks);
    Some(toks)
}

fn char_offset(s: &str, char_index: usize) -> usize {
    s.char_indices().nth(char_index).map_or(s.len(), |(b, _)| b)
}

/// The decimal digits of a non-negative number, rounded half up at `frac`
/// places: (integer digits without leading zeros, `frac` fraction digits).
/// Works from the shortest text that reads back as the same number, so 2.675
/// rounds to 2.68 as a spreadsheet shows it.
fn decimal(v: f64, frac: usize) -> Option<(Vec<u8>, Vec<u8>)> {
    if !(0.0..1e15).contains(&v) {
        return None;
    }
    // A spreadsheet keeps 15 significant digits, which hides float noise such as 7.249999999999999.
    let v: f64 = format!("{v:.14e}").parse().ok()?;
    let sci = format!("{v:e}");
    let (mant, exp) = sci.split_once('e')?;
    let exp: i32 = exp.parse().ok()?;
    let digits: Vec<u8> = mant
        .bytes()
        .filter(u8::is_ascii_digit)
        .map(|b| b - b'0')
        .collect();
    let point = exp + 1; // digits before the decimal point
    let (mut int, mut fr): (Vec<u8>, Vec<u8>) = if point <= 0 {
        let mut f = vec![0; (-point) as usize];
        f.extend(&digits);
        (Vec::new(), f)
    } else if (point as usize) >= digits.len() {
        let mut i = digits.clone();
        i.resize(point as usize, 0);
        (i, Vec::new())
    } else {
        (
            digits[..point as usize].to_vec(),
            digits[point as usize..].to_vec(),
        )
    };
    if fr.len() > frac {
        let up = fr[frac] >= 5;
        fr.truncate(frac);
        if up {
            let mut carry = true;
            for d in fr.iter_mut().rev().chain(int.iter_mut().rev()) {
                if !carry {
                    break;
                }
                if *d == 9 {
                    *d = 0;
                } else {
                    *d += 1;
                    carry = false;
                }
            }
            if carry {
                int.insert(0, 1);
            }
        }
    }
    fr.resize(frac, 0);
    while int.first() == Some(&0) {
        int.remove(0);
    }
    Some((int, fr))
}

fn group(digits: &str) -> String {
    let n = digits.len();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (n - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The number as the plain text a spreadsheet shows for General.
fn general(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        return format!("{}", v as i64);
    }
    let s = format!("{v:.10}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// `value` shown the way `code` says, or `None` when the code is General or
/// something this does not model.
pub fn render(code: &str, value: f64) -> Option<String> {
    if !value.is_finite() || code.trim().is_empty() || code.trim().eq_ignore_ascii_case("general") {
        return None;
    }
    let secs = sections(code);
    let negative = value < 0.0;
    let (section, own_sign) = match (secs.len(), negative, value == 0.0) {
        (1, ..) => (&secs[0], false),
        (_, true, _) if secs.len() >= 2 => (&secs[1], true),
        (n, _, true) if n >= 3 => (&secs[2], true),
        _ => (&secs[0], false),
    };
    let toks = tokenize(section)?;
    // An empty section shows nothing (it is how a format hides zeros).
    if toks.is_empty() {
        return Some(String::new());
    }
    let body = layout(&toks, value.abs())?;
    let out = if negative && !own_sign {
        format!("-{body}")
    } else {
        body
    };
    Some(out.trim().to_string())
}

fn layout(toks: &[Tok], abs: f64) -> Option<String> {
    let percents = toks.iter().filter(|t| **t == Tok::Percent).count() as i32;
    let exp_at = toks.iter().position(|t| matches!(t, Tok::Exp { .. }));
    let main = &toks[..exp_at.unwrap_or(toks.len())];

    // Placeholders either side of the decimal point.
    let point_at = main.iter().position(|t| *t == Tok::Point);
    let int_toks = &main[..point_at.unwrap_or(main.len())];
    let int_ph: Vec<char> = int_toks
        .iter()
        .filter_map(|t| {
            if let Tok::Digit(c) = t {
                Some(*c)
            } else {
                None
            }
        })
        .collect();
    let frac_ph: Vec<char> = point_at
        .map(|p| {
            main[p + 1..]
                .iter()
                .filter_map(|t| {
                    if let Tok::Digit(c) = t {
                        Some(*c)
                    } else {
                        None
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    // A comma between integer placeholders groups thousands; one after the last scales by 1000.
    let first_digit = int_toks.iter().position(|t| matches!(t, Tok::Digit(_)));
    let last_digit = int_toks.iter().rposition(|t| matches!(t, Tok::Digit(_)));
    let grouping = match (first_digit, last_digit) {
        (Some(a), Some(b)) => int_toks[a..b].contains(&Tok::Comma),
        _ => false,
    };
    let scale_commas = match main.iter().rposition(|t| matches!(t, Tok::Digit(_))) {
        Some(b) => main[b + 1..].iter().filter(|t| **t == Tok::Comma).count(),
        None => 0,
    } as i32;

    // Literals sitting between integer placeholders (a phone-number style `0000 000 000`)
    // spread the digits over the placeholders instead of keeping them together.
    let interleaved = match (first_digit, last_digit) {
        (Some(a), Some(b)) => int_toks[a..b].iter().any(|t| matches!(t, Tok::Lit(_))),
        _ => false,
    };
    if interleaved && (!frac_ph.is_empty() || exp_at.is_some()) {
        return None;
    }

    let mut v = abs * 100f64.powi(percents) / 1000f64.powi(scale_commas);

    let mut exp_text = String::new();
    if let Some(e) = exp_at {
        if int_ph.len() != 1 {
            return None; // engineering notation
        }
        let always_sign = matches!(toks[e], Tok::Exp { always_sign: true });
        let exp_digits = toks[e + 1..]
            .iter()
            .filter(|t| matches!(t, Tok::Digit(_)))
            .count();
        let mut exp10 = if v == 0.0 {
            0
        } else {
            v.log10().floor() as i32
        };
        v /= 10f64.powi(exp10);
        // Rounding the mantissa can carry it to 10.
        if let Some((int, _)) = decimal(v, frac_ph.len())
            && int.len() > 1
        {
            v /= 10.0;
            exp10 += 1;
        }
        let sign = if exp10 < 0 {
            "-"
        } else if always_sign {
            "+"
        } else {
            ""
        };
        exp_text = format!("E{sign}{:0>width$}", exp10.abs(), width = exp_digits.max(1));
    }

    let (int_digits, frac_digits) = decimal(v, frac_ph.len())?;
    let mut int: String = int_digits.iter().map(|d| (b'0' + d) as char).collect();
    let min_int = int_ph.iter().filter(|c| **c == '0').count();
    if int.len() < min_int {
        int = format!("{}{int}", "0".repeat(min_int - int.len()));
    }

    // Fraction: required digits always, optional ones only up to the last that is not zero.
    let last_required = frac_ph.iter().rposition(|c| *c == '0').map_or(0, |i| i + 1);
    let last_nonzero = frac_digits
        .iter()
        .rposition(|d| *d != 0)
        .map_or(0, |i| i + 1);
    let shown = last_required.max(last_nonzero).min(frac_ph.len());
    let frac: String = frac_digits[..shown]
        .iter()
        .map(|d| (b'0' + d) as char)
        .collect();

    let number = if interleaved {
        spread(&int, int_toks)
    } else {
        let int = if grouping { group(&int) } else { int };
        if frac.is_empty() {
            int
        } else {
            format!("{int}.{frac}")
        }
    };

    let run = if interleaved {
        first_digit.zip(last_digit)
    } else {
        None
    };
    let mut out = String::new();
    let mut placed = false;
    let mut after_exp = false;
    for (i, t) in toks.iter().enumerate() {
        if let Some((a, b)) = run
            && (a..=b).contains(&i)
        {
            if i == a {
                out.push_str(&number);
            }
            continue;
        }
        match t {
            Tok::Lit(s) => out.push_str(s),
            Tok::Digit(_) | Tok::Point | Tok::Comma if !after_exp => {
                if !placed {
                    out.push_str(&number);
                    placed = true;
                }
            }
            Tok::Percent => out.push('%'),
            Tok::Exp { .. } => {
                out.push_str(&exp_text);
                after_exp = true;
            }
            Tok::General => out.push_str(&general(abs)),
            _ => {}
        }
    }
    Some(out)
}

/// Digits over placeholders from the right, with the literals between them: `0000 000 000`.
/// Digits beyond the placeholders stay in front.
fn spread(int: &str, int_toks: &[Tok]) -> String {
    let a = int_toks
        .iter()
        .position(|t| matches!(t, Tok::Digit(_)))
        .unwrap_or(0);
    let b = int_toks
        .iter()
        .rposition(|t| matches!(t, Tok::Digit(_)))
        .unwrap_or(0);
    let mut digits: Vec<char> = int.chars().collect();
    let mut out: Vec<String> = Vec::new();
    for t in int_toks[a..=b].iter().rev() {
        match t {
            Tok::Digit(c) => match digits.pop() {
                Some(d) => out.push(d.to_string()),
                None if *c == '0' => out.push("0".into()),
                None if *c == '?' => out.push(" ".into()),
                None => {}
            },
            Tok::Lit(s) => out.push(s.clone()),
            _ => {}
        }
    }
    let rest: String = digits.into_iter().collect();
    let tail: String = out.into_iter().rev().collect();
    format!("{rest}{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(code: &str, v: f64) -> String {
        render(code, v).unwrap_or_else(|| panic!("{code} not modelled"))
    }

    #[test]
    fn percentages() {
        assert_eq!(r("0%", 0.097), "10%");
        assert_eq!(r("0.0%", 0.097), "9.7%");
        assert_eq!(r("0.00%", 0.12345), "12.35%");
        assert_eq!(r("0%", 1.0), "100%");
        assert_eq!(r("0.0%", -0.05), "-5.0%");
    }

    #[test]
    fn thousands_and_decimals() {
        assert_eq!(r("#,##0", 2_700_000.0), "2,700,000");
        assert_eq!(r("#,##0", 999.0), "999");
        assert_eq!(r("#,##0.00", 1234.5), "1,234.50");
        assert_eq!(r("0.00", 2.675), "2.68");
        assert_eq!(r("0", 2.5), "3");
        assert_eq!(r("0", 0.4), "0");
        assert_eq!(r("0.0", 9.96), "10.0");
        assert_eq!(r("#,##0", 999_999.6), "1,000,000");
        assert_eq!(r("000", 7.0), "007");
        assert_eq!(r("#.##", 3.5), "3.5");
        assert_eq!(r("#.##", 3.0), "3");
        assert_eq!(r("0.0#", 3.25), "3.25");
        assert_eq!(r("0.0#", 3.2), "3.2");
    }

    #[test]
    fn currency_text_and_symbols() {
        assert_eq!(r(r#""UGX" #,##0"#, 2_700_000.0), "UGX 2,700,000");
        assert_eq!(r(r#"#,##0 "UGX""#, 85_000.0), "85,000 UGX");
        assert_eq!(r("[$UGX-409] #,##0", 1500.0), "UGX 1,500");
        assert_eq!(r("[$₦-466]#,##0.00", 1500.0), "₦1,500.00");
        assert_eq!(r("$#,##0.00", 1234.5), "$1,234.50");
        assert_eq!(r("€#,##0", 12.0), "€12");
        assert_eq!(r(r"\$#,##0", 12.0), "$12");
    }

    #[test]
    fn negatives_zero_and_sections() {
        assert_eq!(r("#,##0;(#,##0)", -1500.0), "(1,500)");
        assert_eq!(r("#,##0;(#,##0)", 1500.0), "1,500");
        assert_eq!(r("#,##0;-#,##0;\"-\"", 0.0), "-");
        assert_eq!(r("#,##0.00;[Red](#,##0.00)", -3.0), "(3.00)");
        assert_eq!(r("#,##0", -1500.0), "-1,500");
        assert_eq!(r("0.0%;-0.0%", -0.25), "-25.0%");
        assert_eq!(
            r("#,##0;(#,##0)", 0.0),
            "0",
            "zero uses the first of two sections"
        );
        assert_eq!(
            r("0.0%", 0.0725),
            "7.3%",
            "float noise does not change the rounding"
        );
    }

    #[test]
    fn builtin_accounting_and_padding() {
        assert_eq!(r(builtin(3).unwrap(), 1234.0), "1,234");
        assert_eq!(r(builtin(10).unwrap(), 0.256), "25.60%");
        assert_eq!(r(builtin(37).unwrap(), -1234.0), "(1,234)");
        assert_eq!(r(builtin(37).unwrap(), 1234.0), "1,234");
        assert_eq!(r(builtin(41).unwrap(), 1234.0), "1,234");
        assert_eq!(r(builtin(41).unwrap(), -1234.0), "(1,234)");
        assert_eq!(r(builtin(41).unwrap(), 0.0), "-");
        assert_eq!(r(builtin(43).unwrap(), 1234.5), "1,234.50");
        assert_eq!(r(builtin(2).unwrap(), 3.14159), "3.14");
        assert!(builtin(14).is_none() && builtin(0).is_none() && builtin(49).is_none());
    }

    #[test]
    fn scaling_and_scientific() {
        assert_eq!(r("#,##0,", 1_234_567.0), "1,235");
        assert_eq!(r("0.0,,", 2_500_000.0), "2.5");
        assert_eq!(r("0.00E+00", 12345.0), "1.23E+04");
        assert_eq!(r("0.00E+00", 0.00012), "1.20E-04");
        assert_eq!(r("0E+0", 9.9e5), "1E+6");
    }

    #[test]
    fn digits_spread_over_a_pattern() {
        assert_eq!(r("0000 000 000", 700_123_456.0), "0700 123 456");
        assert_eq!(r("000-00-0000", 123_456_789.0), "123-45-6789");
        assert_eq!(r("00-00", 5.0), "00-05");
    }

    #[test]
    fn general_and_unmodelled_formats_fall_back() {
        assert_eq!(render("General", 5.5), None);
        assert_eq!(render("", 5.5), None);
        assert_eq!(render("yyyy-mm-dd", 45000.0), None);
        assert_eq!(render("h:mm AM/PM", 0.5), None);
        assert_eq!(render("# ?/?", 0.5), None);
        assert_eq!(render("[>=100]0;0.0", 5.0), None);
        assert_eq!(render("##0.0E+0", 12345.0), None);
        assert_eq!(render("@", 5.0), None);
        assert_eq!(render("0.00", f64::NAN), None);
        assert_eq!(r(r#"General" kg""#, 12.5), "12.5 kg");
    }

    #[test]
    fn dates_and_times_are_told_apart() {
        let k = |c: &str| date_kind(c).map(|k| (k.date, k.time, k.seconds));
        assert_eq!(k("dd/mm/yyyy"), Some((true, false, false)));
        assert_eq!(k("d-mmm-yy"), Some((true, false, false)));
        assert_eq!(k("mmm yyyy"), Some((true, false, false)));
        assert_eq!(k("h:mm AM/PM"), Some((false, true, false)));
        assert_eq!(k("h:mm:ss"), Some((false, true, true)));
        assert_eq!(k("mm:ss"), Some((false, true, true)));
        assert_eq!(k("yyyy-mm-dd hh:mm"), Some((true, true, false)));
        assert_eq!(k("[h]:mm"), Some((false, true, false)));
        assert_eq!(k(r#""Due" dd/mm"#), Some((true, false, false)));
        assert_eq!(k("#,##0.00"), None);
        assert_eq!(k("0.0%"), None);
        assert_eq!(k("General"), None);
        assert_eq!(k(r#"0" days""#), None);
        assert_eq!(builtin_date(14).map(|k| k.date), Some(true));
        assert_eq!(
            builtin_date(20).map(|k| (k.date, k.time)),
            Some((false, true))
        );
        assert!(builtin_date(3).is_none());
    }

    #[test]
    fn an_empty_section_hides_the_value() {
        assert_eq!(r("0;-0;;@", 0.0), "");
        assert_eq!(r("0;-0;;@", 4.0), "4");
    }
}
