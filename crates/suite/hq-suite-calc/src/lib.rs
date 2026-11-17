//! Sheets: a small spreadsheet formula engine.
//!
//! Cells hold a number, text, a truth value or a formula. A formula is parsed once and evaluated
//! with the usual spreadsheet rules (A1 references, ranges, operator precedence, errors that
//! travel through a calculation, a cycle reported instead of looping). It is a subset on purpose:
//! the functions people use in office sheets, written to give the same answers Excel stores in the
//! file. How close it gets is measured against real workbooks by `hq suite calc-check`, not
//! assumed.

mod eval;
mod parse;
mod sheet;
mod value;

pub use eval::FUNCTIONS;
pub use parse::{CellRef, Expr, ParseError, parse_formula};
pub use sheet::{Sheet, column_name, parse_cell_ref};
pub use value::{CalcError, Value, format_number};

#[cfg(test)]
mod tests;
