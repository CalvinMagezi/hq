//! Sheet edits beyond one cell: a range of cells, rows, a column, a column's type and a new sheet.
//!
//! Each action works on the lines of one `sheet` block's table and writes the block back through an
//! hqmd patch, so the rest of the note is byte for byte what it was and a block that changed since
//! it was read is refused. Only the lines an action must change are rebuilt; the other rows keep
//! their own spacing.
//!
//! Formulas that read rows: adding rows in the middle of a sheet and deleting rows move every row
//! below them, and this editor does not rewrite references. It refuses instead, and says which
//! formula (or tile, or plot) would end up pointing at the wrong row. Adding rows at the end and
//! adding a column on the right move nothing, so they are always allowed.

use anyhow::{Context, Result, bail};
use hq_suite_calc::column_name;
use hq_suite_md::block::Severity;
use hq_suite_md::{Attr, BlockType, ColType, Document, Info, Op, RangeRef, TypedBlock};
use serde_json::{Value, json};

use crate::edit_plan::{
    Planned, addr, opt_str, opt_u64, patch, place_after, plural, req_str, req_u64,
};
use crate::sheets::{MAX_CELL_CHARS, NoteBook, sheet_of_range};

/// Most cells one `set_cells` call may carry, and most rows one `add_rows` call may add.
const MAX_CELLS_PER_CALL: usize = 10_000;
const MAX_ROWS_PER_CALL: usize = 5_000;
/// Longest sheet name, and the characters a sheet name may not hold (Excel's own list).
const MAX_SHEET_NAME: usize = 31;
const BAD_NAME_CHARS: &[char] = &['[', ']', ':', '*', '?', '/', '\\', '!', '\'', '"', '`'];

/// One `sheet` block's table, as lines.
struct Table {
    /// The sheet's number among the note's `sheet` blocks.
    sheet: usize,
    /// The block's place in the note.
    node: usize,
    hash: String,
    tb: TypedBlock,
    /// The block's source, line by line (fences included, no `\r`).
    lines: Vec<String>,
    /// Line numbers of the header and the rule under it.
    head: usize,
    rule: usize,
    /// Line numbers of the data rows.
    rows: Vec<usize>,
    /// Every table row's cells, header first.
    cells: Vec<Vec<String>>,
}

impl Table {
    fn columns(&self) -> usize {
        self.cells[0].len()
    }

    fn rows_total(&self) -> usize {
        self.cells.len()
    }

    /// The line number of table row `r` (0 is the header).
    fn line_of(&self, r: usize) -> usize {
        if r == 0 { self.head } else { self.rows[r - 1] }
    }
}

fn split_cells(line: &str) -> Vec<String> {
    line.trim()
        .trim_start_matches('|')
        .trim_end_matches('|')
        .split('|')
        .map(|c| c.trim().to_string())
        .collect()
}

fn load(doc: &Document, sheet: usize, hash: Option<&str>) -> Result<Table> {
    let (node, tb) = doc
        .typed_blocks()
        .filter_map(|(i, b)| b.ok().map(|b| (i, b)))
        .filter(|(_, b)| b.ty == BlockType::Sheet)
        .nth(sheet)
        .with_context(|| format!("that note has no sheet number {}", sheet + 1))?;
    let n = doc.node(node).context("that sheet is not in the note")?;
    if let Some(h) = hash
        && h != n.hash()
    {
        bail!("that sheet changed since you read it; read the note again and retry");
    }
    if tb.info.get("data").is_some() {
        bail!("this sheet keeps its rows in a data= sidecar, which this editor does not change");
    }
    if !tb.closed {
        bail!("this sheet's block has no closing fence; fix it before editing");
    }
    let lines: Vec<String> = n
        .src()
        .split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l).to_string())
        .collect();
    let shape =
        "this sheet's table is not in the plain `| a | b |` form with a rule under the header";
    let head = (1..lines.len())
        .find(|&i| !lines[i].trim().is_empty())
        .filter(|&i| lines[i].trim_start().starts_with('|'))
        .context(shape)?;
    let rule = head + 1;
    if !lines
        .get(rule)
        .is_some_and(|l| l.trim_start().starts_with('|'))
    {
        bail!("{shape}");
    }
    let mut rows = Vec::new();
    for (i, l) in lines
        .iter()
        .enumerate()
        .take(lines.len() - 1)
        .skip(rule + 1)
    {
        if l.trim().is_empty() {
            continue;
        }
        if !l.trim_start().starts_with('|') {
            bail!("{shape}");
        }
        rows.push(i);
    }
    if lines.iter().any(|l| l.contains("\\|")) {
        bail!("this sheet has a cell with an escaped pipe, which this editor does not rewrite");
    }
    let mut cells = vec![split_cells(&lines[head])];
    cells.extend(rows.iter().map(|&i| split_cells(&lines[i])));
    if cells.iter().any(|r| r.len() != cells[0].len()) {
        bail!("a row of this sheet has a different number of cells than its header");
    }
    Ok(Table {
        sheet,
        node,
        hash: n.hash(),
        tb,
        lines,
        head,
        rule,
        rows,
        cells,
    })
}

fn a1(row: usize, col: usize) -> String {
    format!("{}{}", column_name(col as u32), row + 1)
}

/// A value from the request as the text of a cell.
fn cell_text(v: &Value) -> Result<String> {
    let s = match v {
        Value::Null => String::new(),
        Value::String(s) => s.trim().to_string(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        _ => bail!("a cell holds text, a number or true/false"),
    };
    if s.chars().count() > MAX_CELL_CHARS {
        bail!("a cell holds at most {MAX_CELL_CHARS} characters");
    }
    if s.contains('|') || s.contains('\n') || s.contains('\r') {
        bail!("a cell cannot hold a | or a line break");
    }
    Ok(s)
}

fn grid_arg(args: &Value, key: &str) -> Result<Vec<Vec<Value>>> {
    let rows = args
        .get(key)
        .and_then(Value::as_array)
        .with_context(|| format!("`{key}` (a list of rows, each a list of cells) is needed"))?;
    rows.iter()
        .map(|r| {
            r.as_array()
                .cloned()
                .with_context(|| format!("each row of `{key}` must be a list of cells"))
        })
        .collect()
}

fn line_of_cells(cells: &[String]) -> String {
    format!("| {} |", cells.join(" | "))
}

/// Writes the block back with `lines` (and a new header when `info` is given).
fn finish(
    doc: &Document,
    t: &Table,
    mut lines: Vec<String>,
    info: Option<&Info>,
) -> Result<Document> {
    if let Some(info) = info {
        let fence: String = lines[0]
            .chars()
            .take_while(|c| *c == '`' || *c == '~')
            .collect();
        lines[0] = format!("{fence}{}", info.render());
    }
    let markdown = lines.join("\n");
    let block = TypedBlock::parse(&markdown).map_err(|e| anyhow::anyhow!("{e}"))?;
    if let Some(p) = block
        .validate()
        .into_iter()
        .find(|p| p.severity == Severity::Error)
    {
        bail!("that change would break the sheet: {}", p.message);
    }
    patch(
        doc,
        vec![Op::Replace {
            at: addr(doc, t.node, Some(&t.hash))?,
            markdown,
        }],
    )
}

/// Every formula among `texts` must be one the sheet can read and the file writers may write.
fn check_formulas(doc: &Document, sheet: usize, texts: &[(String, String)]) -> Result<()> {
    if !texts.iter().any(|(_, t)| t.trim_start().starts_with('=')) {
        return Ok(());
    }
    let blocks: Vec<TypedBlock> = doc.typed_blocks().filter_map(|(_, b)| b.ok()).collect();
    let book = NoteBook::new(&blocks).book;
    for (label, text) in texts {
        let Some(body) = text.trim_start().strip_prefix('=') else {
            continue;
        };
        let body = body.trim();
        if body.is_empty() {
            bail!("{label}: a formula needs something after the =");
        }
        if !hq_suite_xlsx::formula_is_safe(body) {
            bail!(
                "{label}: `{text}` uses something a sheet does not allow (a function the sheet does not have, a link to another file, or a very long formula)"
            );
        }
        if !book.formula_parses(sheet, body) {
            bail!("{label}: `{text}` is not a formula the sheet can read");
        }
    }
    Ok(())
}

fn after(t: &Table, new: &Document) -> Value {
    let hash = new.node(t.node).map(|n| n.hash()).unwrap_or_default();
    json!({"sheet": t.sheet, "block_index": t.node, "block_hash": hash})
}

// ---------------------------------------------------------------- formulas that read rows

/// The row (one-based) a word such as `D12` or `$B$3` names, if it is a cell.
fn ref_row(word: &str) -> Option<u32> {
    let w: String = word.chars().filter(|c| *c != '$').collect();
    let letters = w.chars().take_while(|c| c.is_ascii_alphabetic()).count();
    let digits = &w[letters..];
    ((1..=3).contains(&letters) && !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()))
        .then(|| digits.parse().ok())
        .flatten()
}

/// Whether `formula` names any cell on row `first_row` (one-based) or below. Text in quotes and
/// function names are not cells; a whole-column reference has no row.
pub(crate) fn mentions_row_from(formula: &str, first_row: u32) -> bool {
    let chars: Vec<char> = formula.chars().collect();
    let (mut i, mut in_text) = (0, false);
    while i < chars.len() {
        let c = chars[i];
        if c == '"' {
            in_text = !in_text;
            i += 1;
        } else if in_text {
            i += 1;
        } else if c.is_ascii_alphanumeric() || matches!(c, '_' | '$' | '.') {
            let start = i;
            while i < chars.len()
                && (chars[i].is_ascii_alphanumeric() || matches!(chars[i], '_' | '$' | '.'))
            {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            if chars.get(i) != Some(&'(') && ref_row(&word).is_some_and(|r| r >= first_row) {
                return true;
            }
        } else {
            i += 1;
        }
    }
    false
}

/// Refuses a change that moves rows from `first_row` (one-based) down when anything reads them:
/// a formula in this sheet (other than in the rows being deleted, `gone`), a formula in another
/// sheet that names this one, or a tile or plot range over it.
fn guard_row_shift(
    doc: &Document,
    note: &str,
    t: &Table,
    first_row: u32,
    gone: std::ops::Range<usize>,
) -> Result<()> {
    let blocks: Vec<TypedBlock> = doc.typed_blocks().filter_map(|(_, b)| b.ok()).collect();
    let nb = NoteBook::new(&blocks);
    let sheets: Vec<&TypedBlock> = blocks.iter().filter(|b| b.ty == BlockType::Sheet).collect();
    let name_of = |i: usize| {
        sheets[i]
            .info
            .get("name")
            .map_or_else(|| format!("Sheet{}", i + 1), str::to_string)
    };
    let mine = name_of(t.sheet).to_lowercase();
    let advice = "This editor does not rewrite references, so rows are only added at the end or deleted when nothing reads the rows below. Rewrite those formulas with set_cells first, or add the rows at the end.";
    for (s, table) in nb.tables.iter().enumerate() {
        for (r, row) in table.iter().enumerate() {
            for (c, text) in row.iter().enumerate() {
                if !text.trim_start().starts_with('=') {
                    continue;
                }
                if s == t.sheet {
                    if !gone.contains(&r) && mentions_row_from(text, first_row) {
                        bail!(
                            "{} holds the formula `{text}`, which reads row {first_row} or below, and those rows would move. {advice}",
                            a1(r, c)
                        );
                    }
                } else if text.to_lowercase().contains(&format!("{mine}!"))
                    && mentions_row_from(text, first_row)
                {
                    bail!(
                        "{} in sheet `{}` holds the formula `{text}`, which reads this sheet from row {first_row} down, and those rows would move. {advice}",
                        a1(r, c),
                        name_of(s)
                    );
                }
            }
        }
    }
    for b in blocks
        .iter()
        .filter(|b| matches!(b.ty, BlockType::Tile | BlockType::Plot))
    {
        let Ok(from) = RangeRef::parse(b.info.get("from").unwrap_or_default()) else {
            continue;
        };
        if sheet_of_range(&blocks, note, &from, "a block").ok() == Some(t.sheet)
            && from.end.row >= first_row
        {
            bail!(
                "a {} block reads `{}`, which reaches row {first_row} or below, and those rows would move. {advice}",
                b.ty.name(),
                b.info.get("from").unwrap_or_default()
            );
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- set_cells

/// Sets a rectangle of cells. `expected` holds what each cell holds now; any difference refuses
/// the whole call.
pub fn set_cells(doc: &Document, args: &Value) -> Result<Planned> {
    let sheet = opt_u64(args, "sheet")?.unwrap_or(0) as usize;
    let (row0, col0) = (
        req_u64(args, "row")? as usize,
        req_u64(args, "col")? as usize,
    );
    let values = grid_arg(args, "values")?;
    let expected = grid_arg(args, "expected")?;
    let width = values.first().map_or(0, Vec::len);
    if values.is_empty() || width == 0 || values.iter().any(|r| r.len() != width) {
        bail!("`values` must be a rectangle of at least one cell");
    }
    if expected.len() != values.len() || expected.iter().any(|r| r.len() != width) {
        bail!("`expected` must be the same size as `values`: what each cell holds now");
    }
    if values.len() * width > MAX_CELLS_PER_CALL {
        bail!("set at most {MAX_CELLS_PER_CALL} cells in one call");
    }
    let t = load(doc, sheet, opt_str(args, "hash"))?;
    if row0 + values.len() > t.rows_total() || col0 + width > t.columns() {
        bail!(
            "that range reaches outside the sheet, which is {} rows by {} columns; add rows or a column first",
            t.rows_total(),
            t.columns()
        );
    }
    let mut cells = t.cells.clone();
    let mut changed: Vec<(String, String, String)> = Vec::new();
    let mut formulas: Vec<(String, String)> = Vec::new();
    for (dr, (vrow, erow)) in values.iter().zip(&expected).enumerate() {
        for (dc, (v, e)) in vrow.iter().zip(erow).enumerate() {
            let (r, c) = (row0 + dr, col0 + dc);
            let label = a1(r, c);
            let want = cell_text(e)?;
            if cells[r][c] != want {
                bail!(
                    "{label} changed since you read it: it holds `{}`, you expected `{want}`. Read the sheet again and retry",
                    cells[r][c]
                );
            }
            let new = cell_text(v)?;
            if new != cells[r][c] {
                changed.push((label.clone(), cells[r][c].clone(), new.clone()));
            }
            formulas.push((label, new.clone()));
            cells[r][c] = new;
        }
    }
    let mut lines = t.lines.clone();
    for r in row0..row0 + values.len() {
        lines[t.line_of(r)] = line_of_cells(&cells[r]);
    }
    let new = finish(doc, &t, lines, None)?;
    check_formulas(&new, sheet, &formulas)?;
    let range = format!(
        "{}:{}",
        a1(row0, col0),
        a1(row0 + values.len() - 1, col0 + width - 1)
    );
    let mut summary = format!(
        "Set {} in {range} ({} changed)",
        plural(values.len() * width, "cell", "cells"),
        changed.len()
    );
    for (label, old, new) in changed.iter().take(6) {
        summary.push_str(&format!("; {label}: `{old}` -> `{new}`"));
    }
    if changed.len() > 6 {
        summary.push_str(&format!("; and {} more", changed.len() - 6));
    }
    let extra = after(&t, &new);
    Ok(Planned::new(new, summary)
        .touching(values.len() * width)
        .with(extra))
}

// ---------------------------------------------------------------- rows

/// Adds rows at the end, or before table row `at` (1 is the first row under the header).
pub fn add_rows(doc: &Document, note: &str, args: &Value) -> Result<Planned> {
    let sheet = opt_u64(args, "sheet")?.unwrap_or(0) as usize;
    let t = load(doc, sheet, Some(req_str(args, "hash")?))?;
    let rows = grid_arg(args, "rows")?;
    if rows.is_empty() || rows.len() > MAX_ROWS_PER_CALL {
        bail!("add between 1 and {MAX_ROWS_PER_CALL} rows in one call");
    }
    let cols = t.columns();
    let mut texts = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        if row.len() > cols {
            bail!(
                "row {} has {} cells but the sheet has {cols} columns",
                i + 1,
                row.len()
            );
        }
        let mut cells = row.iter().map(cell_text).collect::<Result<Vec<_>>>()?;
        cells.resize(cols, String::new());
        texts.push(cells);
    }
    let total = t.rows_total();
    let at = match opt_u64(args, "at")? {
        None => total,
        Some(a) if (1..=total as u64).contains(&a) => a as usize,
        Some(a) => {
            bail!("`at` must be from 1 (first row under the header) to {total} (the end); got {a}")
        }
    };
    if at < total {
        guard_row_shift(doc, note, &t, at as u32 + 1, 0..0)?;
    }
    let mut lines = t.lines.clone();
    let place = if at < total {
        t.line_of(at)
    } else {
        t.rows.last().copied().unwrap_or(t.rule) + 1
    };
    for (k, cells) in texts.iter().enumerate() {
        lines.insert(place + k, line_of_cells(cells));
    }
    let new = finish(doc, &t, lines, None)?;
    let formulas: Vec<(String, String)> = texts
        .iter()
        .enumerate()
        .flat_map(|(k, row)| {
            row.iter()
                .enumerate()
                .map(move |(c, text)| (a1(at + k, c), text.clone()))
        })
        .collect();
    check_formulas(&new, sheet, &formulas)?;
    let summary = format!(
        "Added {} as sheet rows {} to {}",
        plural(texts.len(), "row", "rows"),
        at + 1,
        at + texts.len()
    );
    let extra = after(&t, &new);
    Ok(Planned::new(new, summary)
        .touching(texts.len() * cols)
        .with(extra))
}

/// Deletes `count` rows starting at table row `row` (1 is the first row under the header).
pub fn delete_rows(doc: &Document, note: &str, args: &Value) -> Result<Planned> {
    let sheet = opt_u64(args, "sheet")?.unwrap_or(0) as usize;
    let t = load(doc, sheet, Some(req_str(args, "hash")?))?;
    let row = req_u64(args, "row")? as usize;
    let count = opt_u64(args, "count")?.unwrap_or(1) as usize;
    if row == 0 {
        bail!("the header row cannot be deleted; `row` 1 is the first row under it");
    }
    if count == 0 || row + count > t.rows_total() {
        bail!(
            "there are {} rows under the header; cannot delete {count} from row {row}",
            t.rows_total() - 1
        );
    }
    if args.get("expected").is_some() {
        let expected = grid_arg(args, "expected")?;
        if expected.len() != count {
            bail!("`expected` must list the {count} rows being deleted");
        }
        for (k, erow) in expected.iter().enumerate() {
            let want: Vec<String> = erow.iter().map(cell_text).collect::<Result<_>>()?;
            if want != t.cells[row + k] {
                bail!(
                    "row {} changed since you read it; read the sheet again and retry",
                    row + k + 1
                );
            }
        }
    }
    guard_row_shift(doc, note, &t, row as u32 + 1, row..row + count)?;
    let mut lines = t.lines.clone();
    for r in (row..row + count).rev() {
        lines.remove(t.line_of(r));
    }
    let new = finish(doc, &t, lines, None)?;
    let shown: Vec<String> = (row..(row + count).min(row + 3))
        .map(|r| format!("row {}: {}", r + 1, t.cells[r].join(" | ")))
        .collect();
    let summary = format!(
        "Deleted {} (sheet rows {} to {}): {}{}",
        plural(count, "row", "rows"),
        row + 1,
        row + count,
        shown.join("; "),
        if count > 3 { "; and more" } else { "" }
    );
    let extra = after(&t, &new);
    Ok(Planned::new(new, summary)
        .touching(count * t.columns())
        .removing()
        .with(extra))
}

// ---------------------------------------------------------------- columns

fn parse_type(text: &str) -> Result<ColType> {
    ColType::parse(&text.trim().to_ascii_lowercase()).with_context(|| {
        format!("`{text}` is not a column type; use text, int, num, bool, date, pct or a currency code such as ugx")
    })
}

/// The block's header with its `types=` list replaced by `types`.
fn with_types(tb: &TypedBlock, types: &[ColType]) -> Info {
    let mut info = tb.info.clone();
    let value = types
        .iter()
        .map(ColType::token)
        .collect::<Vec<_>>()
        .join(",");
    match info.attrs.iter_mut().find(|a| a.key == "types") {
        Some(a) => {
            a.value = value;
            a.quoted = true;
        }
        None => info.attrs.push(Attr::new("types", value)),
    }
    info
}

/// The types the block declares, or `text` for every column when it declares none.
fn declared_types(t: &Table) -> Result<Vec<ColType>> {
    match t.tb.info.get("types") {
        Some(list) => hq_suite_md::sheet::parse_types(list).map_err(|bad| {
            anyhow::anyhow!("the sheet's types list has `{bad}`, which is not a column type")
        }),
        None => Ok(vec![ColType::Text; t.columns()]),
    }
}

fn append_cell(line: &str, text: &str) -> Result<String> {
    let l = line.trim_end();
    if !l.ends_with('|') {
        bail!("a line of this sheet's table does not end with a |");
    }
    Ok(format!("{l} {text} |"))
}

/// Adds a column on the right with a name, an optional type and an optional default for every row.
pub fn add_column(doc: &Document, args: &Value) -> Result<Planned> {
    let sheet = opt_u64(args, "sheet")?.unwrap_or(0) as usize;
    let t = load(doc, sheet, Some(req_str(args, "hash")?))?;
    let name = cell_text(&json!(req_str(args, "name")?))?;
    if name.is_empty() {
        bail!("the column needs a name");
    }
    let default = match args.get("default") {
        None | Some(Value::Null) => String::new(),
        Some(v) => cell_text(v)?,
    };
    if default.starts_with('=') {
        bail!(
            "a default cannot be a formula, since every row would get the same one; fill the cells with set_cells"
        );
    }
    let kind = opt_str(args, "type").map(parse_type).transpose()?;
    let mut lines = t.lines.clone();
    lines[t.head] = append_cell(&lines[t.head], &name)?;
    lines[t.rule] = format!("{}---|", lines[t.rule].trim_end());
    for &i in &t.rows {
        lines[i] = append_cell(&lines[i], &default)?;
    }
    let info = if kind.is_some() || t.tb.info.get("types").is_some() {
        let mut types = declared_types(&t)?;
        types.push(kind.unwrap_or(ColType::Text));
        Some(with_types(&t.tb, &types))
    } else {
        None
    };
    let new = finish(doc, &t, lines, info.as_ref())?;
    let col = t.columns();
    let summary = format!(
        "Added column {} `{name}`{}{}",
        column_name(col as u32),
        opt_str(args, "type").map_or(String::new(), |k| format!(" of type {k}")),
        if default.is_empty() {
            String::new()
        } else {
            format!(", every row set to `{default}`")
        }
    );
    let extra = after(&t, &new);
    Ok(Planned::new(new, summary)
        .touching(t.rows_total())
        .with(extra))
}

/// Declares a column's type. The cells are not converted; only how they are read and shown changes.
pub fn set_column_type(doc: &Document, args: &Value) -> Result<Planned> {
    let sheet = opt_u64(args, "sheet")?.unwrap_or(0) as usize;
    let t = load(doc, sheet, Some(req_str(args, "hash")?))?;
    let col = req_u64(args, "col")? as usize;
    if col >= t.columns() {
        bail!(
            "the sheet has {} columns; column {col} is not one",
            t.columns()
        );
    }
    let kind = parse_type(req_str(args, "type")?)?;
    let mut types = declared_types(&t)?;
    if types.len() != t.columns() {
        bail!("the sheet's types list does not match its columns; fix the block's header first");
    }
    let token = kind.token();
    types[col] = kind;
    let info = with_types(&t.tb, &types);
    let new = finish(doc, &t, t.lines.clone(), Some(&info))?;
    let summary = format!(
        "Column {} ({}) is now `{token}`; its cells are not converted",
        column_name(col as u32),
        t.cells[0][col]
    );
    let extra = after(&t, &new);
    Ok(Planned::new(new, summary).with(extra))
}

// ---------------------------------------------------------------- a new sheet

/// Adds a `sheet` block: a name, the columns, optional types and optional rows. It goes after the
/// block `after_index` (which must still hash to `hash`), or at the end of the note.
pub fn add_sheet(doc: &Document, args: &Value) -> Result<Planned> {
    let name = req_str(args, "name")?.trim().to_string();
    if name.is_empty() || name.chars().count() > MAX_SHEET_NAME {
        bail!("a sheet name is 1 to {MAX_SHEET_NAME} characters");
    }
    if name.contains(BAD_NAME_CHARS) || name.chars().any(char::is_control) {
        bail!("a sheet name cannot hold any of [ ] : * ? / \\ ! ' \" ` or a line break");
    }
    let blocks: Vec<TypedBlock> = doc.typed_blocks().filter_map(|(_, b)| b.ok()).collect();
    let existing: Vec<String> = blocks
        .iter()
        .filter(|b| b.ty == BlockType::Sheet)
        .enumerate()
        .map(|(i, b)| {
            b.info
                .get("name")
                .map_or_else(|| format!("Sheet{}", i + 1), str::to_string)
        })
        .collect();
    if existing.iter().any(|n| n.eq_ignore_ascii_case(&name)) {
        bail!("this note already has a sheet called `{name}`");
    }
    let columns: Vec<String> = args
        .get("columns")
        .and_then(Value::as_array)
        .context("`columns` (a list of column names) is needed")?
        .iter()
        .map(cell_text)
        .collect::<Result<_>>()?;
    if columns.is_empty() || columns.iter().any(String::is_empty) {
        bail!("a sheet needs at least one column, and every column a name");
    }
    let types: Option<Vec<ColType>> = match args.get("types").and_then(Value::as_array) {
        None => None,
        Some(list) => {
            if list.len() != columns.len() {
                bail!("`types` must list one type per column ({})", columns.len());
            }
            Some(
                list.iter()
                    .map(|v| parse_type(v.as_str().unwrap_or_default()))
                    .collect::<Result<_>>()?,
            )
        }
    };
    let rows = match args.get("rows") {
        None | Some(Value::Null) => Vec::new(),
        Some(_) => grid_arg(args, "rows")?,
    };
    if rows.len() > MAX_ROWS_PER_CALL {
        bail!("add at most {MAX_ROWS_PER_CALL} rows in one call");
    }
    let mut body = vec![
        line_of_cells(&columns),
        format!("|{}", "---|".repeat(columns.len())),
    ];
    let mut texts = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        if row.len() > columns.len() {
            bail!(
                "row {} has {} cells but there are {} columns",
                i + 1,
                row.len(),
                columns.len()
            );
        }
        let mut cells = row.iter().map(cell_text).collect::<Result<Vec<_>>>()?;
        cells.resize(columns.len(), String::new());
        body.push(line_of_cells(&cells));
        texts.push(cells);
    }
    let mut attrs = vec![("name", name.clone())];
    if let Some(types) = &types {
        attrs.push((
            "types",
            types
                .iter()
                .map(ColType::token)
                .collect::<Vec<_>>()
                .join(","),
        ));
    }
    let attr_refs: Vec<(&str, &str)> = attrs.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let block = TypedBlock::new(BlockType::Sheet, &attr_refs, &body.join("\n")).render("\n");
    let checked = TypedBlock::parse(&block).map_err(|e| anyhow::anyhow!("{e}"))?;
    if let Some(p) = checked
        .validate()
        .into_iter()
        .find(|p| p.severity == Severity::Error)
    {
        bail!("that sheet would not be valid: {}", p.message);
    }
    let (place, at_index) = place_after(doc, args)?;
    let new = patch(
        doc,
        vec![Op::InsertAfter {
            at: place,
            markdown: block,
        }],
    )?;
    let ordinal = new
        .typed_blocks()
        .filter(|(i, b)| *i < at_index && b.as_ref().is_ok_and(|b| b.ty == BlockType::Sheet))
        .count();
    let formulas: Vec<(String, String)> = texts
        .iter()
        .enumerate()
        .flat_map(|(r, row)| {
            row.iter()
                .enumerate()
                .map(move |(c, text)| (a1(r + 1, c), text.clone()))
        })
        .collect();
    check_formulas(&new, ordinal, &formulas)?;
    let hash = new.node(at_index).map(|n| n.hash()).unwrap_or_default();
    let summary = format!(
        "Added sheet `{name}` with {} and {}",
        plural(columns.len(), "column", "columns"),
        plural(texts.len(), "row", "rows")
    );
    Ok(Planned::new(new, summary)
        .touching((texts.len() + 1) * columns.len())
        .with(json!({"sheet": ordinal, "block_index": at_index, "block_hash": hash})))
}

#[cfg(test)]
#[path = "sheet_ops_tests.rs"]
mod tests;
