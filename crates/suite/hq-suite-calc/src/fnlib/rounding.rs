//! Rounding the way a spreadsheet does it: on the 15 significant digits it keeps, not on the
//! binary number underneath, so `ROUND(1.005, 2)` is 1.01 and `ROUNDUP(2.3*3, 0)` is 7.

/// What to do with the digits that are cut off.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Mode {
    /// Half away from zero (`ROUND`).
    Half,
    /// Away from zero whenever anything is cut off (`ROUNDUP`).
    Up,
    /// Toward zero (`ROUNDDOWN`, `TRUNC`).
    Down,
}

/// `x` rounded at `digits` places after the point (negative digits round left of it).
pub(super) fn round_dec(x: f64, digits: f64, mode: Mode) -> f64 {
    if x == 0.0 || !x.is_finite() {
        return x;
    }
    let digits = digits.trunc().clamp(-30.0, 30.0) as i32;
    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    let text = format!("{:.14e}", x.abs());
    let Some((mantissa, exp)) = text.split_once('e') else {
        return x;
    };
    let exp: i32 = exp.parse().unwrap_or(0);
    let all: Vec<u8> = mantissa
        .bytes()
        .filter(u8::is_ascii_digit)
        .map(|b| b - b'0')
        .collect();
    // The value is 0.d1d2d3... times 10^point.
    let point = exp + 1;
    let keep = point + digits;
    if keep >= all.len() as i32 {
        return sign * text.parse::<f64>().unwrap_or(x.abs());
    }
    let kept: &[u8] = if keep > 0 { &all[..keep as usize] } else { &[] };
    let dropped_nonzero = if keep >= 0 {
        all[keep as usize..].iter().any(|d| *d != 0)
    } else {
        true
    };
    let round_up = match mode {
        Mode::Down => false,
        Mode::Up => dropped_nonzero,
        Mode::Half => keep >= 0 && all[keep as usize] >= 5,
    };
    let mut n: u64 = kept.iter().fold(0, |acc, d| acc * 10 + u64::from(*d));
    if round_up {
        n += 1;
    }
    let out = if digits >= 0 {
        n as f64 / 10f64.powi(digits)
    } else {
        n as f64 * 10f64.powi(-digits)
    };
    sign * out
}

/// `x` as decimal digits rounded half away from zero at `decimals` places, as text with exactly
/// that many places. Used by `TEXT`, where the digits shown must be the digits a spreadsheet shows.
pub(super) fn fixed(x: f64, decimals: usize) -> String {
    let r = round_dec(x.abs(), decimals as f64, Mode::Half);
    format!("{:.*}", decimals, r)
}
