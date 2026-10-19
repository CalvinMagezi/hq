//! Tables in PDFs, rebuilt from `pdftotext -layout` output.
//!
//! A text-layer extractor gives a statement or an invoice as a heap of lines.
//! The layout text keeps the columns, so a block of lines that share vertical
//! gaps ("rivers" of whitespace) is a table: the gaps give the columns, a line
//! with an empty first column continues the row above, and a header repeated at
//! the top of the next page extends the same table.

use super::ir::{Block, Inline, ListItem, Table};

/// Narrowest run of spaces that separates two columns.
const MIN_GAP: usize = 2;
const MIN_ROWS: usize = 3;
const MAX_COLUMNS: usize = 24;

pub struct Layout {
    pub blocks: Vec<Block>,
    pub tables: usize,
}

pub fn from_layout_text(text: &str) -> Layout {
    let mut blocks: Vec<Block> = Vec::new();
    let mut tables = 0;
    // The latest table and its columns, for rows that continue it on a later page.
    let mut last_table: Option<usize> = None;
    let mut last_bounds: Vec<(usize, usize)> = Vec::new();
    for page in text.split('\u{c}') {
        let owned: Vec<String> = page.lines().map(bullet_to_dash).collect();
        let lines: Vec<&str> = owned.iter().map(String::as_str).collect();
        let mut i = 0;
        let mut para: Vec<String> = Vec::new();
        let mut at_top = true;
        while i < lines.len() {
            let line = lines[i];
            if line.trim().is_empty() {
                flush_para(&mut para, &mut blocks);
                i += 1;
                continue;
            }
            let first_on_page = std::mem::take(&mut at_top);
            // A page that carries on a table without repeating its header: the columns
            // are laid out afresh on every page, so match by shape, not by position.
            if let (true, Some(idx)) = (first_on_page, last_table)
                && let Block::Table(prev) = &mut blocks[idx]
            {
                let ncols = prev.rows[0].len();
                let shape = prev
                    .rows
                    .last()
                    .and_then(|r| r.first())
                    .map(|c| shape_of(&super::ir::plain_text(c)));
                if let Some((rows, bounds, used)) =
                    headerless_rows(&lines[i..], ncols, shape.as_deref())
                {
                    for row in rows {
                        prev.rows
                            .push(row.into_iter().map(|c| vec![Inline::text(c)]).collect());
                    }
                    last_bounds = bounds;
                    i += used;
                    continue;
                }
            }
            // Rows that carry on the previous table's columns, with or without a repeated header.
            if let Some(idx) = last_table
                && fits(line, &last_bounds)
            {
                let run = continuation_run(&lines[i..], &last_bounds);
                if let Block::Table(prev) = &mut blocks[idx] {
                    let header: Vec<String> = prev.rows[0]
                        .iter()
                        .map(|c| super::ir::plain_text(c))
                        .collect();
                    for row in assemble(&run, &last_bounds, Some(&header)) {
                        prev.rows
                            .push(row.into_iter().map(|c| vec![Inline::text(c)]).collect());
                    }
                    i += run.len();
                    continue;
                }
            }
            if cells_of(line).len() >= 2
                && let Some((table, bounds, used)) = table_at(&lines[i..])
            {
                flush_para(&mut para, &mut blocks);
                i += used;
                blocks.push(Block::Table(table));
                last_table = Some(blocks.len() - 1);
                last_bounds = bounds;
                tables += 1;
                continue;
            }
            para.push(line.trim().to_string());
            i += 1;
        }
        flush_para(&mut para, &mut blocks);
    }
    Layout { blocks, tables }
}

/// Consecutive text lines become paragraphs; bullets become list items.
fn flush_para(para: &mut Vec<String>, blocks: &mut Vec<Block>) {
    if para.is_empty() {
        return;
    }
    let is_bullet = |l: &str| {
        l.starts_with(['•', '▪', '◦', '-', '*', '·'])
            && l.chars().nth(1).is_some_and(char::is_whitespace)
    };
    let mut text = String::new();
    let mut items: Vec<ListItem> = Vec::new();
    let lines = std::mem::take(para);
    for l in lines {
        if is_bullet(&l) {
            if !text.trim().is_empty() {
                blocks.push(Block::Para(vec![Inline::text(std::mem::take(&mut text))]));
            }
            items.push(ListItem {
                level: 0,
                ordered: false,
                content: vec![Inline::text(
                    l[l.chars().next().map_or(0, char::len_utf8)..]
                        .trim()
                        .to_string(),
                )],
            });
        } else if !items.is_empty() && l.starts_with(char::is_whitespace) {
            // never reached: lines are trimmed; kept for clarity of intent
        } else {
            if !items.is_empty() {
                blocks.push(Block::List(std::mem::take(&mut items)));
            }
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(&l);
        }
    }
    if !items.is_empty() {
        blocks.push(Block::List(items));
    }
    if !text.trim().is_empty() {
        blocks.push(Block::Para(vec![Inline::text(text)]));
    }
}

/// Text of a line split where two or more spaces separate it, with start offsets.
fn cells_of(line: &str) -> Vec<(usize, String)> {
    let mut cells = Vec::new();
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == ' ' {
            i += 1;
            continue;
        }
        let start = i;
        let mut end = i;
        let mut j = i;
        while j < chars.len() {
            if chars[j] == ' ' {
                let mut k = j;
                while k < chars.len() && chars[k] == ' ' {
                    k += 1;
                }
                if k - j >= MIN_GAP || k == chars.len() {
                    break;
                }
                j = k;
            } else {
                end = j + 1;
                j += 1;
            }
        }
        cells.push((start, chars[start..end].iter().collect()));
        i = j;
    }
    cells
}

/// A line that starts with a bullet glyph (often a private-use symbol-font
/// character) followed by wide spacing reads as a list item, not as a cell.
fn bullet_to_dash(line: &str) -> String {
    let trimmed = line.trim_start();
    let indent = line.len() - trimmed.len();
    let mut chars = trimmed.chars();
    match (chars.next(), chars.next()) {
        (Some(c), Some(' ')) if is_bullet_glyph(c) => {
            format!("{}- {}", &line[..indent], chars.as_str().trim_start())
        }
        _ => line.to_string(),
    }
}

fn is_bullet_glyph(c: char) -> bool {
    ('\u{e000}'..='\u{f8ff}').contains(&c) || "\u{2022}\u{25aa}\u{25ab}\u{25e6}\u{b7}\u{25cb}\u{25cf}\u{25a0}\u{25a1}\u{25c6}\u{25c7}\u{2023}\u{2043}".contains(c)
}

/// Whether a line's cells sit on the given columns, in at least two of them.
fn fits(line: &str, bounds: &[(usize, usize)]) -> bool {
    if bounds.len() < 2 {
        return false;
    }
    let cells = cells_of(line);
    if cells.len() < 2 {
        return false;
    }
    let mut hit = std::collections::BTreeSet::new();
    for (start, _) in &cells {
        match bounds
            .iter()
            .position(|(s, e)| start + 1 >= *s && *start < *e)
        {
            Some(c) => {
                hit.insert(c);
            }
            None => return false,
        }
    }
    hit.len() >= 2 && hit.len() == cells.len()
}

/// The lines from the start of `lines` that are rows of the table with these columns.
fn continuation_run<'a>(lines: &[&'a str], bounds: &[(usize, usize)]) -> Vec<&'a str> {
    let mut run = Vec::new();
    for l in lines {
        if l.trim().is_empty() {
            break;
        }
        let cells = cells_of(l);
        // A single indented cell is a wrapped cell of the row above.
        let wrapped = cells.len() == 1 && cells[0].0 > 0 && !run.is_empty();
        if fits(l, bounds) || wrapped {
            run.push(*l);
        } else {
            break;
        }
    }
    run
}

/// Column intervals found in the whitespace left by the given lines: each is a
/// stretch of positions where some line has text, with gaps of `MIN_GAP` or more between.
fn data_columns(all: &[&str]) -> Vec<(usize, usize)> {
    // Columns come from the typical row. A totals row or a note that straddles
    // two columns would otherwise join them.
    let mut counts: std::collections::BTreeMap<usize, usize> = std::collections::BTreeMap::new();
    for l in all {
        let n = cells_of(l).len();
        if n >= 2 {
            *counts.entry(n).or_default() += 1;
        }
    }
    let typical = counts
        .iter()
        .max_by_key(|(n, c)| (**c, **n))
        .map(|(n, _)| *n)
        .unwrap_or(0);
    let lines: Vec<&str> = all
        .iter()
        .copied()
        .filter(|l| typical == 0 || cells_of(l).len() == typical)
        .collect();
    let width = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0);
    let mut occupied = vec![false; width];
    for l in lines {
        for (x, c) in l.chars().enumerate() {
            if c != ' ' {
                occupied[x] = true;
            }
        }
    }
    let mut cols: Vec<(usize, usize)> = Vec::new();
    let mut x = 0;
    while x < width {
        if !occupied[x] {
            x += 1;
            continue;
        }
        let start = x;
        let mut last = x;
        while x < width {
            if occupied[x] {
                last = x;
                x += 1;
            } else {
                let gap = occupied[x..].iter().take_while(|o| !**o).count();
                if gap >= MIN_GAP || x + gap >= width {
                    break;
                }
                x += gap;
            }
        }
        cols.push((start, last + 1));
        x = last + 1;
    }
    // Each column owns the space up to the next column's start, so right-aligned
    // numbers that begin left of their header still land in the right cell.
    cols.iter()
        .enumerate()
        .map(|(i, (s, _))| (*s, cols.get(i + 1).map_or(usize::MAX, |(n, _)| *n)))
        .collect()
}

fn slice_row(line: &str, bounds: &[(usize, usize)]) -> Vec<String> {
    let chars: Vec<char> = line.chars().collect();
    bounds
        .iter()
        .map(|(s, e)| {
            let hi = (*e).min(chars.len());
            if *s >= hi {
                String::new()
            } else {
                chars[*s..hi].iter().collect::<String>().trim().to_string()
            }
        })
        .collect()
}

/// Rows from lines: a line whose first cell is empty and that fills under half the
/// columns continues the row above. Repeats of `header` are dropped.
fn assemble(
    lines: &[&str],
    bounds: &[(usize, usize)],
    header: Option<&[String]>,
) -> Vec<Vec<String>> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    for l in lines {
        let cells = slice_row(l, bounds);
        if header.is_some_and(|h| {
            h.iter()
                .map(|c| c.to_lowercase())
                .eq(cells.iter().map(|c| c.to_lowercase()))
        }) {
            continue;
        }
        let filled = cells.iter().filter(|c| !c.is_empty()).count();
        let continuation =
            cells[0].is_empty() && filled * 2 < cells.len() && !rows.is_empty() && bounds.len() > 2;
        if continuation {
            let last = rows.last_mut().expect("checked above");
            for (i, c) in cells.iter().enumerate() {
                if !c.is_empty() {
                    if !last[i].is_empty() {
                        last[i].push(' ');
                    }
                    last[i].push_str(c);
                }
            }
        } else {
            rows.push(cells);
        }
    }
    rows
}

/// Header text for each column. With as many header cells as columns they pair
/// in order, which survives a left-aligned heading over right-aligned numbers.
fn header_for(line: &str, bounds: &[(usize, usize)]) -> Vec<String> {
    let cells = cells_of(line);
    if cells.len() == bounds.len() {
        return cells.into_iter().map(|(_, t)| t).collect();
    }
    let mut out = vec![String::new(); bounds.len()];
    for (start, text) in cells {
        let col = bounds.iter().rposition(|(s, _)| *s <= start).unwrap_or(0);
        if !out[col].is_empty() {
            out[col].push(' ');
        }
        out[col].push_str(&text);
    }
    out
}

/// Digit runs as `9` and letter runs as `a`: "30/09/2026" is "9/9/9".
fn shape_of(s: &str) -> String {
    let mut out = String::new();
    let mut last = ' ';
    for c in s.chars() {
        let k = if c.is_ascii_digit() {
            '9'
        } else if c.is_alphabetic() {
            'a'
        } else {
            c
        };
        if !(k == last && (k == '9' || k == 'a')) {
            out.push(k);
        }
        last = k;
    }
    out
}

/// Column bounds as `(start, end)` character positions within a line.
type Bounds = Vec<(usize, usize)>;

/// Rows at the top of a page that continue a table of `ncols` columns.
fn headerless_rows(
    lines: &[&str],
    ncols: usize,
    shape: Option<&str>,
) -> Option<(Vec<Vec<String>>, Bounds, usize)> {
    if cells_of(lines[0]).len() != ncols || ncols < 3 {
        return None;
    }
    let mut end = 1;
    while end < lines.len() && !lines[end].trim().is_empty() {
        let cells = cells_of(lines[end]);
        let indented = cells.first().is_some_and(|(s, _)| *s > 0);
        if cells.len() >= 2 || (cells.len() == 1 && indented) {
            end += 1;
        } else {
            break;
        }
    }
    let group = &lines[..end];
    let bounds = data_columns(group);
    if bounds.len() != ncols {
        return None;
    }
    let rows = assemble(group, &bounds, None);
    let first = rows.first()?.first()?;
    if shape.is_some_and(|s| shape_of(first) != s) {
        return None;
    }
    Some((rows, bounds, end))
}

/// A table starting at `lines[0]`: its columns, and how many lines it uses.
fn table_at(lines: &[&str]) -> Option<(Table, Bounds, usize)> {
    let header_cells = cells_of(lines[0]);
    let mut end = 1;
    while end < lines.len() {
        let l = lines[end];
        if l.trim().is_empty() {
            break;
        }
        let cells = cells_of(l);
        let indented = cells.first().is_some_and(|(s, _)| *s > 0);
        if cells.len() >= 2 || (cells.len() == 1 && indented) {
            end += 1;
        } else {
            break;
        }
    }
    if end < MIN_ROWS {
        return None;
    }
    let group = &lines[..end];
    let data = &group[1..];
    let bounds = data_columns(data);
    if bounds.len() < 2 || bounds.len() > MAX_COLUMNS {
        return None;
    }
    // Two-column text blocks are tables only when the second column starts at the
    // same place on every line; three or more columns are evidence enough on their own.
    if bounds.len() == 2 {
        let second: Vec<usize> = group
            .iter()
            .filter_map(|l| cells_of(l).get(1).map(|(s, _)| *s))
            .collect();
        let aligned = second.len() >= MIN_ROWS && second.iter().all(|s| s.abs_diff(second[0]) <= 1);
        if !aligned {
            return None;
        }
    } else if header_cells.len() < 2 {
        return None;
    }
    let header = header_for(group[0], &bounds);
    let mut rows = vec![header.clone()];
    rows.extend(assemble(data, &bounds, Some(&header)));
    if rows.len() < MIN_ROWS {
        return None;
    }
    // A real table fills most of its cells; prose with ragged gaps does not.
    let total = rows.len() * bounds.len();
    let filled: usize = rows
        .iter()
        .map(|r| r.iter().filter(|c| !c.is_empty()).count())
        .sum();
    if filled * 100 < total * 55 {
        return None;
    }
    Some((
        Table {
            rows: rows
                .into_iter()
                .map(|r| r.into_iter().map(|c| vec![Inline::text(c)]).collect())
                .collect(),
        },
        bounds,
        end,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn md(text: &str) -> String {
        super::super::ir::render(from_layout_text(text).blocks)
    }

    const STATEMENT: &str = "\
Mobile Money Statement
Account: 0700 000 000    Period: September

Date           Transaction ID   Type         Details             Amount (UGX)   Balance (UGX)
01/09/2026     MP26090000       Deposit      Sample              148,000        648,000
                                             Cooperative
01/09/2026     MP26090037       Payment      Example Shop        -31,000        617,000
02/09/2026     MP26090074       Transfer     Demo Pharmacy       -100,000       517,000
";

    #[test]
    fn a_statement_becomes_a_table_with_wrapped_cells_joined() {
        assert_eq!(
            md(STATEMENT),
            "Mobile Money Statement Account: 0700 000 000 Period: September\n\n\
| Date | Transaction ID | Type | Details | Amount (UGX) | Balance (UGX) |\n\
| --- | --- | --- | --- | --- | --- |\n\
| 01/09/2026 | MP26090000 | Deposit | Sample Cooperative | 148,000 | 648,000 |\n\
| 01/09/2026 | MP26090037 | Payment | Example Shop | -31,000 | 617,000 |\n\
| 02/09/2026 | MP26090074 | Transfer | Demo Pharmacy | -100,000 | 517,000 |\n"
        );
    }

    #[test]
    fn a_header_repeated_on_the_next_page_extends_the_same_table() {
        let page2 = "Date           Transaction ID   Type         Details             Amount (UGX)   Balance (UGX)
03/09/2026     MP26090111       Airtime      Example Shop        -145,000       372,000
04/09/2026     MP26090148       Withdrawal   Demo Pharmacy       -149,000       223,000
05/09/2026     MP26090185       Deposit      Test Station        57,000         280,000
";
        let layout = from_layout_text(&format!("{STATEMENT}\u{c}{page2}"));
        assert_eq!(layout.tables, 1);
        let t = layout
            .blocks
            .iter()
            .find_map(|b| {
                if let Block::Table(t) = b {
                    Some(t)
                } else {
                    None
                }
            })
            .unwrap();
        assert_eq!(t.rows.len(), 7);
    }

    #[test]
    fn rows_without_a_repeated_header_continue_the_table() {
        let page2 = "01/10/2026     MP26100001       Payment      Example Shop        -10,000        500,000
01/10/2026     MP26100002       Deposit      Demo Pharmacy       20,000         520,000
";
        let layout = from_layout_text(&format!("{STATEMENT}\u{c}{page2}"));
        assert_eq!(layout.tables, 1);
        let t = layout
            .blocks
            .iter()
            .find_map(|b| {
                if let Block::Table(t) = b {
                    Some(t)
                } else {
                    None
                }
            })
            .unwrap();
        assert_eq!(t.rows.len(), 6);
    }

    #[test]
    fn columns_laid_out_afresh_on_a_new_page_still_continue_the_table() {
        // The same statement, but page two is narrower, so every column sits elsewhere.
        let page2 = "01/10/2026   MP26100001   Payment      Example Shop        -10,000    500,000
                                       Cooperative
01/10/2026   MP26100002   Deposit      Demo Pharmacy       20,000     520,000
";
        let layout = from_layout_text(&format!("{STATEMENT}\u{c}{page2}"));
        assert_eq!(layout.tables, 1);
        let t = layout
            .blocks
            .iter()
            .find_map(|b| {
                if let Block::Table(t) = b {
                    Some(t)
                } else {
                    None
                }
            })
            .unwrap();
        assert_eq!(t.rows.len(), 6);
    }

    #[test]
    fn a_left_aligned_heading_over_right_aligned_numbers_keeps_its_column() {
        let invoice = "\
Description             Qty                    Unit price (UGX)     Amount (UGX)
Maize seed, 25 kg bag                     10                 85,000           850,000
NPK fertiliser, 50 kg                      6                160,000           960,000
Transport to Mukono                        1                120,000           120,000
";
        assert_eq!(
            md(invoice),
            "| Description | Qty | Unit price (UGX) | Amount (UGX) |\n| --- | --- | --- | --- |\n\
| Maize seed, 25 kg bag | 10 | 85,000 | 850,000 |\n\
| NPK fertiliser, 50 kg | 6 | 160,000 | 960,000 |\n\
| Transport to Mukono | 1 | 120,000 | 120,000 |\n"
        );
    }

    #[test]
    fn a_two_column_block_with_an_aligned_second_column_is_a_table() {
        let block = "\
Bill to                                        Invoice details
Sample Farmers Cooperative                     Invoice No: INV-0042
Mukono District                                Date: 05 October 2026
";
        assert_eq!(
            md(block),
            "| Bill to | Invoice details |\n| --- | --- |\n| Sample Farmers Cooperative | Invoice No: INV-0042 |\n| Mukono District | Date: 05 October 2026 |\n"
        );
    }

    #[test]
    fn a_totals_row_that_straddles_columns_does_not_merge_them() {
        let invoice = "\
Description             Qty                    Unit price (UGX)     Amount (UGX)
Maize seed, 25 kg bag                     10                 85,000           850,000
NPK fertiliser, 50 kg                      6                160,000           960,000
Transport to Mukono                        1                120,000           120,000
TOTAL                                                               2,430,000
";
        let out = md(invoice);
        assert!(
            out.contains("| Description | Qty | Unit price (UGX) | Amount (UGX) |"),
            "{out}"
        );
        assert!(
            out.contains("| TOTAL |  |  | 2,430,000 |") || out.contains("| TOTAL |  | 2,430,000 |"),
            "{out}"
        );
    }

    #[test]
    fn symbol_font_bullets_are_list_items_not_columns() {
        let text = "Payment details\n\u{f0b7}   Bank: Example Bank\n\u{f0b7}   Mobile Money: 0700 000 000\n\u{f0b7}   Reference: INV-0042\n";
        assert_eq!(
            md(text),
            "Payment details\n\n- Bank: Example Bank\n- Mobile Money: 0700 000 000\n- Reference: INV-0042\n"
        );
    }

    #[test]
    fn prose_with_a_few_wide_gaps_is_not_a_table() {
        let prose = "This is a paragraph of ordinary text   with one wide gap in it\nand it wraps onto a second line   with another gap\nthen a third line of prose that ends the block";
        assert_eq!(from_layout_text(prose).tables, 0);
    }

    #[test]
    fn bullets_become_a_list() {
        assert_eq!(
            md("Steps:\n- survey\n- install\n"),
            "Steps:\n\n- survey\n- install\n"
        );
    }

    #[test]
    fn cells_split_on_wide_gaps_only() {
        let cells = cells_of("  Maize seed, 25 kg     10    85,000");
        assert_eq!(
            cells,
            vec![
                (2, "Maize seed, 25 kg".to_string()),
                (24, "10".to_string()),
                (30, "85,000".to_string())
            ]
        );
    }
}
