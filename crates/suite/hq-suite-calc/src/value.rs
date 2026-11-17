use std::fmt;

/// What a cell holds once calculated.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Empty,
    Num(f64),
    Text(String),
    Bool(bool),
    Err(CalcError),
}

/// The errors a formula can produce, spelled the way a spreadsheet shows them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalcError {
    /// `#DIV/0!`
    Div0,
    /// `#VALUE!`: a text where a number was needed, or the wrong kind of argument.
    Value,
    /// `#REF!`
    Ref,
    /// `#NAME?`: a function this engine does not have.
    Name,
    /// `#NUM!`
    Num,
    /// `#N/A`
    Na,
    /// A formula that depends on itself.
    Circular,
}

impl fmt::Display for CalcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            CalcError::Div0 => "#DIV/0!",
            CalcError::Value => "#VALUE!",
            CalcError::Ref => "#REF!",
            CalcError::Name => "#NAME?",
            CalcError::Num => "#NUM!",
            CalcError::Na => "#N/A",
            CalcError::Circular => "#CIRC!",
        })
    }
}

/// Digits a spreadsheet keeps of a number when it shows or stores it.
const SIGNIFICANT_DIGITS: usize = 15;

/// A number as a person reads it: no trailing zeros, no `.0`, at most 15 significant digits.
pub fn format_number(n: f64) -> String {
    if n == 0.0 {
        return "0".into();
    }
    if !n.is_finite() {
        return CalcError::Num.to_string();
    }
    let text = format!("{:.*e}", SIGNIFICANT_DIGITS - 1, n);
    let rounded: f64 = text.parse().unwrap_or(n);
    let out = format!("{rounded}");
    out
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Empty => Ok(()),
            Value::Num(n) => f.write_str(&format_number(*n)),
            Value::Text(t) => f.write_str(t),
            Value::Bool(true) => f.write_str("TRUE"),
            Value::Bool(false) => f.write_str("FALSE"),
            Value::Err(e) => e.fmt(f),
        }
    }
}
