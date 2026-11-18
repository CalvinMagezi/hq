//! The `where` of a tile or a plot: `qty>30 and item=Sugar`, a few conditions on columns named in
//! the range's first row.

use anyhow::{Result, bail};
use hq_suite_calc::Value;

/// Most conditions one `where` may hold.
const MAX_CONDITIONS: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Eq,
    Ne,
    Gt,
    Ge,
    Lt,
    Le,
}

#[derive(Debug, Clone, PartialEq)]
enum Rhs {
    Num(f64),
    Text(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Condition {
    /// The column's name in the range's first row, lowercase.
    pub column: String,
    op: Op,
    rhs: Rhs,
}

/// Longest operators first, so `>=` is not read as `>` and a value starting with `=`.
const OPERATORS: &[(&str, Op)] = &[
    (">=", Op::Ge),
    ("<=", Op::Le),
    ("!=", Op::Ne),
    ("<>", Op::Ne),
    ("=", Op::Eq),
    (">", Op::Gt),
    ("<", Op::Lt),
];

fn split_and(text: &str) -> Vec<&str> {
    let lower = text.to_lowercase();
    let mut parts = Vec::new();
    let mut start = 0;
    while let Some(at) = lower[start..].find(" and ") {
        parts.push(&text[start..start + at]);
        start += at + 5;
    }
    parts.push(&text[start..]);
    parts
}

fn parse_one(part: &str) -> Result<Condition> {
    let found = part.char_indices().find_map(|(i, _)| {
        OPERATORS
            .iter()
            .find(|(s, _)| part[i..].starts_with(s))
            .map(|(s, op)| (i, *s, *op))
    });
    let Some((at, symbol, op)) = found else {
        bail!(
            "`{}` needs a comparison such as qty>30 or item=Sugar",
            part.trim()
        );
    };
    let column = part[..at].trim().to_lowercase();
    let raw = part[at + symbol.len()..].trim();
    if column.is_empty() || raw.is_empty() {
        bail!(
            "`{}` needs a column on the left and a value on the right",
            part.trim()
        );
    }
    let quoted = raw.len() >= 2 && raw.starts_with('"') && raw.ends_with('"');
    let rhs = match raw.parse::<f64>() {
        Ok(n) if !quoted => Rhs::Num(n),
        _ => Rhs::Text(raw.trim_matches('"').to_string()),
    };
    Ok(Condition { column, op, rhs })
}

/// The conditions in `text`, joined by `and`.
pub fn parse_where(text: &str) -> Result<Vec<Condition>> {
    let parts = split_and(text);
    if parts.len() > MAX_CONDITIONS {
        bail!("a where holds at most {MAX_CONDITIONS} conditions");
    }
    parts.into_iter().map(parse_one).collect()
}

impl Condition {
    /// Whether a cell passes. A comparison only holds between values of the same kind; `!=` also
    /// holds between different kinds, as in a spreadsheet.
    pub fn passes(&self, cell: &Value) -> bool {
        use std::cmp::Ordering;
        let ord = match (cell, &self.rhs) {
            (Value::Num(a), Rhs::Num(b)) => a.partial_cmp(b),
            (Value::Text(a), Rhs::Text(b)) => Some(a.to_lowercase().cmp(&b.to_lowercase())),
            _ => None,
        };
        match (self.op, ord) {
            (Op::Ne, None) => true,
            (_, None) => false,
            (Op::Eq, Some(o)) => o == Ordering::Equal,
            (Op::Ne, Some(o)) => o != Ordering::Equal,
            (Op::Gt, Some(o)) => o == Ordering::Greater,
            (Op::Ge, Some(o)) => o != Ordering::Less,
            (Op::Lt, Some(o)) => o == Ordering::Less,
            (Op::Le, Some(o)) => o != Ordering::Greater,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(x: f64) -> Value {
        Value::Num(x)
    }

    fn t(s: &str) -> Value {
        Value::Text(s.into())
    }

    #[test]
    fn conditions_read_columns_operators_numbers_and_text() {
        let c = parse_where("Qty >= 30 AND item = \"Sugar\" and price<>0").unwrap();
        assert_eq!(c.len(), 3);
        assert_eq!(c[0].column, "qty");
        assert!(c[0].passes(&n(30.0)) && c[0].passes(&n(31.0)) && !c[0].passes(&n(29.0)));
        assert!(c[1].passes(&t("sugar")) && !c[1].passes(&t("salt")));
        assert!(c[2].passes(&n(5.0)) && !c[2].passes(&n(0.0)));
    }

    #[test]
    fn kinds_do_not_compare_but_not_equal_holds_across_them() {
        let c = parse_where("a>5 and b=x and c!=7").unwrap();
        assert!(!c[0].passes(&t("9")), "text is not greater than a number");
        assert!(!c[1].passes(&n(1.0)));
        assert!(c[2].passes(&t("seven")), "a different kind is not equal");
        assert!(!c[2].passes(&n(7.0)));
        assert!(!c[0].passes(&Value::Empty));
    }

    #[test]
    fn a_where_that_cannot_be_read_is_refused() {
        for bad in [
            "",
            "qty",
            "qty>",
            ">5",
            "a=1 and b=2 and c=3 and d=4 and e=5",
        ] {
            assert!(parse_where(bad).is_err(), "{bad}");
        }
    }
}
