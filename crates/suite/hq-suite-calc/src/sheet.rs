//! A grid of cells and the order they are worked out in.

use std::collections::{HashMap, HashSet};

use crate::eval::{Cells, eval};
use crate::parse::{
    CellRef, Expr, MAX_COLUMNS, RefContext, cell_ref, parse_failure, parse_formula,
    parse_formula_in,
};
use crate::value::{CalcError, Value};

/// Most cells one sheet may hold, so a hostile file cannot ask for unbounded memory.
const MAX_CELLS: usize = 2_000_000;

#[derive(Debug, Clone)]
enum Cell {
    Literal(Value),
    Formula(Result<Expr, ()>),
}

#[derive(Debug, Clone, Default)]
pub struct Sheet {
    cells: HashMap<(u32, u32), Cell>,
}

/// `0` is `A`, `26` is `AA`.
pub fn column_name(mut col: u32) -> String {
    let mut out = Vec::new();
    loop {
        out.push((b'A' + (col % 26) as u8) as char);
        if col < 26 {
            break;
        }
        col = col / 26 - 1;
    }
    out.iter().rev().collect()
}

/// `B7` as a zero-based (column, row).
pub fn parse_cell_ref(text: &str) -> Option<(u32, u32)> {
    cell_ref(text).map(|c| (c.col, c.row))
}

/// What a person typed into a cell, as a value: a number, `TRUE` or `FALSE`, or text.
fn literal(input: &str) -> Value {
    let t = input.trim();
    if t.is_empty() {
        return Value::Empty;
    }
    if let Ok(n) = t.parse::<f64>()
        && n.is_finite()
    {
        return Value::Num(n);
    }
    match t.to_ascii_uppercase().as_str() {
        "TRUE" => Value::Bool(true),
        "FALSE" => Value::Bool(false),
        _ => Value::Text(input.to_string()),
    }
}

impl Sheet {
    pub fn new() -> Self {
        Sheet::default()
    }

    /// Puts what a person typed at (`col`, `row`), both zero-based. Starting with `=` it is a
    /// formula. Returns false if the sheet is full.
    pub fn set(&mut self, col: u32, row: u32, input: &str) -> bool {
        if self.cells.len() >= MAX_CELLS && !self.cells.contains_key(&(col, row)) {
            return false;
        }
        let cell = match input.trim_start().strip_prefix('=') {
            Some(f) => Cell::Formula(parse_formula(f).map_err(|_| ())),
            None => Cell::Literal(literal(input)),
        };
        self.cells.insert((col, row), cell);
        true
    }

    /// Puts a value that is already typed, as loaded from a file: text that looks like a number
    /// stays text, an error stays an error. Returns false if the sheet is full.
    pub fn set_value(&mut self, col: u32, row: u32, value: Value) -> bool {
        if self.cells.len() >= MAX_CELLS && !self.cells.contains_key(&(col, row)) {
            return false;
        }
        self.cells.insert((col, row), Cell::Literal(value));
        true
    }

    /// A table as a sheet: the first row is row 1, the first column is A.
    pub fn from_table(rows: &[Vec<String>]) -> Sheet {
        let mut s = Sheet::new();
        for (r, row) in rows.iter().enumerate() {
            for (c, text) in row.iter().enumerate() {
                if !text.trim().is_empty() {
                    s.set(c as u32, r as u32, text);
                }
            }
        }
        s
    }

    /// The (columns, rows) the sheet reaches.
    pub fn size(&self) -> (u32, u32) {
        self.cells
            .keys()
            .fold((0, 0), |(c, r), &(cc, rr)| (c.max(cc + 1), r.max(rr + 1)))
    }

    /// The value of one cell, calculated.
    pub fn get(&self, col: u32, row: u32) -> Value {
        Run::new(self).cell(CellRef { col, row })
    }

    /// Every cell that holds anything, calculated, each worked out once.
    pub fn calculate(&self) -> HashMap<(u32, u32), Value> {
        let mut run = Run::new(self);
        let keys: Vec<(u32, u32)> = self.cells.keys().copied().collect();
        for (col, row) in keys {
            run.cell(CellRef { col, row });
        }
        run.done
    }
}

struct Run<'a> {
    sheet: &'a Sheet,
    done: HashMap<(u32, u32), Value>,
}

impl<'a> Run<'a> {
    fn new(sheet: &'a Sheet) -> Self {
        Run {
            sheet,
            done: HashMap::new(),
        }
    }

    /// The cells a formula reads that hold something. A range names every cell in it, but only the
    /// ones the sheet has can matter, so a huge range over a small sheet costs the sheet's size.
    fn depends_on(&self, e: &Expr, out: &mut Vec<(u32, u32)>) {
        match e {
            Expr::Ref(r) => out.push((r.col, r.row)),
            Expr::Range(a, b) => {
                let (c0, c1) = (a.col.min(b.col), a.col.max(b.col));
                let (r0, r1) = (a.row.min(b.row), a.row.max(b.row));
                let area = u64::from(c1 - c0 + 1) * u64::from(r1 - r0 + 1);
                if area <= self.sheet.cells.len() as u64 {
                    for row in r0..=r1 {
                        for col in c0..=c1 {
                            if self.sheet.cells.contains_key(&(col, row)) {
                                out.push((col, row));
                            }
                        }
                    }
                } else {
                    out.extend(
                        self.sheet
                            .cells
                            .keys()
                            .filter(|(c, r)| (c0..=c1).contains(c) && (r0..=r1).contains(r))
                            .copied(),
                    );
                }
            }
            Expr::Neg(x) | Expr::Percent(x) => self.depends_on(x, out),
            Expr::Binary(_, x, y) | Expr::Compare(_, x, y) => {
                self.depends_on(x, out);
                self.depends_on(y, out);
            }
            Expr::Call(_, args) => args.iter().for_each(|a| self.depends_on(a, out)),
            Expr::Num(_) | Expr::Text(_) | Expr::Bool(_) => {}
        }
    }

    /// Works `start` out after everything it reads, with an explicit stack so a long chain of
    /// formulas cannot overflow the machine's. A formula reached again while it is still being
    /// worked out is a cycle.
    fn work(&mut self, start: (u32, u32)) {
        let mut stack = vec![start];
        // Formulas whose inputs have been queued: the ones above a cell on the stack. A cell that
        // is waiting only as someone's sibling is not in this set, so reading it is not a cycle.
        let mut expanded: HashSet<(u32, u32)> = HashSet::new();
        while let Some(&key) = stack.last() {
            if self.done.contains_key(&key) {
                stack.pop();
                continue;
            }
            let value = match self.sheet.cells.get(&key) {
                None => Some(Value::Empty),
                Some(Cell::Literal(v)) => Some(v.clone()),
                Some(Cell::Formula(Err(()))) => Some(Value::Err(parse_failure())),
                Some(Cell::Formula(Ok(expr))) => {
                    let mut deps = Vec::new();
                    self.depends_on(expr, &mut deps);
                    deps.sort_unstable();
                    deps.dedup();
                    let mut cycle = false;
                    let mut waiting = false;
                    for d in deps {
                        if self.done.contains_key(&d) {
                            continue;
                        }
                        if d == key || expanded.contains(&d) {
                            cycle = true;
                        } else {
                            // Always queued on top, even if it is already waiting lower down, so
                            // it is worked out before this cell is looked at again.
                            stack.push(d);
                            waiting = true;
                        }
                    }
                    if cycle {
                        Some(Value::Err(CalcError::Circular))
                    } else if waiting {
                        expanded.insert(key);
                        None
                    } else {
                        Some(eval(expr, self))
                    }
                }
            };
            if let Some(v) = value {
                self.done.insert(key, v);
                stack.pop();
                expanded.remove(&key);
            }
        }
    }
}

impl Cells for Run<'_> {
    fn cell(&mut self, at: CellRef) -> Value {
        let key = (at.col, at.row);
        if !self.done.contains_key(&key) {
            self.work(key);
        }
        self.done.get(&key).cloned().unwrap_or(Value::Empty)
    }
}

/// Most sheets one workbook may hold.
const MAX_SHEETS: usize = 256;

/// Several sheets that may read each other: `=Stock!B2*2` or `='Price list'!A1:A9`.
///
/// The sheets sit side by side in one grid, each at its own column offset, so the evaluator,
/// the cycle check and the work stack that already handle one sheet handle all of them.
#[derive(Debug, Clone, Default)]
pub struct Workbook {
    grid: Sheet,
    names: Vec<String>,
}

impl Workbook {
    pub fn new() -> Self {
        Workbook::default()
    }

    /// Adds a sheet and returns its number, or the existing number if the name is taken (names
    /// are compared without regard to case, as in a spreadsheet). `None` once the book is full.
    /// Every sheet must be added before formulas are set, so a formula can name any of them.
    pub fn add_sheet(&mut self, name: &str) -> Option<usize> {
        let lower = name.to_lowercase();
        if let Some(i) = self.names.iter().position(|n| *n == lower) {
            return Some(i);
        }
        if self.names.len() >= MAX_SHEETS {
            return None;
        }
        self.names.push(lower);
        Some(self.names.len() - 1)
    }

    fn offset(sheet: usize) -> u32 {
        sheet as u32 * MAX_COLUMNS
    }

    fn context(&self, sheet: usize) -> RefContext {
        RefContext {
            base: Self::offset(sheet),
            sheets: self
                .names
                .iter()
                .enumerate()
                .map(|(i, n)| (n.clone(), Self::offset(i)))
                .collect(),
        }
    }

    /// Puts what a person typed at (`col`, `row`) of sheet number `sheet`, both zero-based.
    pub fn set(&mut self, sheet: usize, col: u32, row: u32, input: &str) -> bool {
        if sheet >= self.names.len() || col >= MAX_COLUMNS {
            return false;
        }
        let at = (col + Self::offset(sheet), row);
        if self.grid.cells.len() >= MAX_CELLS && !self.grid.cells.contains_key(&at) {
            return false;
        }
        let cell = match input.trim_start().strip_prefix('=') {
            Some(f) => Cell::Formula(parse_formula_in(f, &self.context(sheet)).map_err(|_| ())),
            None => Cell::Literal(literal(input)),
        };
        self.grid.cells.insert(at, cell);
        true
    }

    /// Puts a value that is already typed, as loaded from a file.
    pub fn set_value(&mut self, sheet: usize, col: u32, row: u32, value: Value) -> bool {
        if sheet >= self.names.len() || col >= MAX_COLUMNS {
            return false;
        }
        self.grid.set_value(col + Self::offset(sheet), row, value)
    }

    /// The value of one cell, calculated.
    pub fn get(&self, sheet: usize, col: u32, row: u32) -> Value {
        if sheet >= self.names.len() || col >= MAX_COLUMNS {
            return Value::Empty;
        }
        self.grid.get(col + Self::offset(sheet), row)
    }
}
