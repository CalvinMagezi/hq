//! The ledger is an hqmd note: one `sheet` block of rows plus `tile` and `plot` blocks the HQ Suite
//! computes and draws. This module only writes and reads that text. It does not depend on the
//! Suite, so the Suite's extraction guard stays true; `hq-suite-adapter/tests/finance_ledger.rs`
//! proves the Suite reads what is written here.
//!
//! The owner edits the note in Sheets (category, flow, status), so import never overwrites a row
//! that is already there.

use crate::parse::{Draft, Kind, ReviewState};
use std::collections::HashSet;

pub const SHEET_NAME: &str = "Ledger";
pub const COLUMNS: [&str; 11] = [
    "date", "time", "provider", "kind", "txn_id", "category", "status", "flow", "amount", "fee",
    "balance",
];
const TYPES: &str = "date,text,text,text,text,text,text,text,ugx,ugx,ugx";
pub const STATUS_DRAFT: &str = "draft";
pub const STATUS_CONFIRMED: &str = "confirmed";
pub const CATEGORY_UNKNOWN: &str = "unknown";
const CENTS: i64 = 100;
/// Marks the tiles and chart this module wrote, so a re-import stretches those and no other.
const OWN_ID_PREFIX: &str = "fin_";
const FIRST_STATUS_COL: char = 'G';
const AMOUNT_COL: char = 'I';
const FEE_COL: char = 'J';
const CATEGORY_COL: char = 'F';

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub cells: [String; 11],
}

/// What a message did to the owner's money, before the owner says otherwise. Transfers between the
/// owner's own wallets are not in this list on purpose: only the owner can know them.
pub fn flow_of(kind: Kind) -> Option<&'static str> {
    match kind {
        Kind::Received => Some("inflow"),
        Kind::Sent | Kind::Paid | Kind::Utility | Kind::Purchase => Some("outflow"),
        Kind::Withdrawal => Some("cashout"),
        Kind::LoanDisbursed => Some("loan_in"),
        Kind::LoanRepayment => Some("loan_out"),
        Kind::Failed | Kind::WithdrawalRequested => None,
    }
}

/// `1234500` cents as `12345`, `5035` as `50.35`: a plain number a spreadsheet cell can hold.
fn money(cents: i64) -> String {
    let (whole, frac) = (cents / CENTS, cents % CENTS);
    match frac {
        0 => whole.to_string(),
        f if f % 10 == 0 => format!("{whole}.{}", f / 10),
        f => format!("{whole}.{f:02}"),
    }
}

fn cell_text(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('|', "\\|")
        .replace('\n', " ")
}

pub fn row_from(d: &Draft) -> Option<Row> {
    let flow = flow_of(d.kind)?;
    let (date, time) = match &d.occurred_at {
        Some(t) => (t[..10].to_string(), t[11..].to_string()),
        None => (d.received_on.clone().unwrap_or_default(), String::new()),
    };
    let status = match d.review {
        ReviewState::Confirmed => STATUS_CONFIRMED,
        _ => STATUS_DRAFT,
    };
    let opt = |c: Option<i64>| c.map(money).unwrap_or_default();
    Some(Row {
        cells: [
            date,
            time,
            format!("{:?}", d.provider).to_lowercase(),
            format!("{:?}", d.kind).to_lowercase(),
            d.txn_id.clone().unwrap_or_default(),
            CATEGORY_UNKNOWN.to_string(),
            status.to_string(),
            flow.to_string(),
            money(d.amount_cents),
            opt(d.fee_cents),
            opt(d.balance_cents),
        ],
    })
}

/// The same transaction by its id and kind; a message with no id by when it happened and how much.
fn key_of(r: &Row) -> String {
    let c = &r.cells;
    if c[4].is_empty() {
        format!("noid|{}|{}|{}|{}|{}", c[0], c[1], c[3], c[8], c[2])
    } else {
        format!("{}|{}|{}", c[2], c[4], c[3])
    }
}

/// The first `sheet` block of a note: where it sits and what it holds.
struct SheetBlock {
    /// The line of the closing fence.
    close_at: usize,
    header: Vec<String>,
    rows: Vec<Row>,
}

fn is_rule(cells: &[String]) -> bool {
    cells
        .iter()
        .all(|c| c.chars().all(|ch| matches!(ch, '-' | ':' | ' ')))
}

/// Reads the sheet by its header names, so a column the owner added, moved or removed does not
/// make a row unreadable. A column the ledger does not know is left alone; a missing one reads empty.
fn find_sheet(lines: &[&str]) -> Option<SheetBlock> {
    let open = lines.iter().position(|l| l.starts_with("```sheet"))?;
    let close = open
        + 1
        + lines[open + 1..]
            .iter()
            .position(|l| l.starts_with("```"))?;
    let mut header: Option<Vec<String>> = None;
    let mut rows = Vec::new();
    for line in lines.iter().take(close).skip(open + 1) {
        if !line.starts_with('|') {
            continue;
        }
        let cells = split_cells(line);
        match &header {
            None => header = Some(cells),
            Some(h) if !is_rule(&cells) => rows.push(row_by_header(h, &cells)),
            Some(_) => {}
        }
    }
    Some(SheetBlock {
        close_at: close,
        header: header?,
        rows,
    })
}

fn row_by_header(header: &[String], cells: &[String]) -> Row {
    let cell = |name: &str| {
        header
            .iter()
            .position(|h| h == name)
            .and_then(|i| cells.get(i))
            .cloned()
            .unwrap_or_default()
    };
    Row {
        cells: COLUMNS.map(cell),
    }
}

/// Rows of the first `sheet` block, in order. Everything outside the table is ignored.
pub fn parse_rows(note: &str) -> Vec<Row> {
    let lines: Vec<&str> = note.lines().collect();
    find_sheet(&lines).map(|b| b.rows).unwrap_or_default()
}

fn split_cells(line: &str) -> Vec<String> {
    let inner = line.trim().trim_start_matches('|');
    let inner = inner.strip_suffix('|').unwrap_or(inner);
    let (mut cells, mut cur, mut esc) = (Vec::new(), String::new(), false);
    for ch in inner.chars() {
        match (esc, ch) {
            (true, c) => {
                cur.push(c);
                esc = false;
            }
            (false, '\\') => esc = true,
            (false, '|') => cells.push(std::mem::take(&mut cur).trim().to_string()),
            (false, c) => cur.push(c),
        }
    }
    cells.push(cur.trim().to_string());
    cells
}

/// Adds the drafts that are not in the note yet. A note that exists is edited, never rewritten: new
/// rows go in before the sheet's closing fence and the ranges of the tiles are stretched to cover
/// them, so everything else the owner changed comes back byte for byte. Returns the note and how
/// many rows were added.
pub fn import(existing: Option<&str>, drafts: &[Draft]) -> (String, usize) {
    let fresh_rows = |seen: &mut HashSet<String>| -> Vec<Row> {
        drafts
            .iter()
            .filter_map(row_from)
            .filter(|r| seen.insert(key_of(r)))
            .collect()
    };
    let lines: Vec<&str> = existing.map(|n| n.lines().collect()).unwrap_or_default();
    let (Some(note), Some(sheet)) = (existing, find_sheet(&lines)) else {
        let rows = fresh_rows(&mut HashSet::new());
        let added = rows.len();
        return (render(&rows), added);
    };
    let mut seen: HashSet<String> = sheet.rows.iter().map(key_of).collect();
    let fresh = fresh_rows(&mut seen);
    if fresh.is_empty() {
        return (note.to_string(), 0);
    }
    let eol = if note.contains("\r\n") { "\r\n" } else { "\n" };
    let last = sheet.rows.len() + fresh.len() + 1;
    let mut out: Vec<String> = Vec::with_capacity(lines.len() + fresh.len());
    for (i, line) in lines.iter().enumerate() {
        if i == sheet.close_at {
            out.extend(fresh.iter().map(|r| row_line(&sheet.header, r)));
        }
        out.push(stretch_range(line, last));
    }
    let mut text = out.join(eol);
    if note.ends_with('\n') {
        text.push_str(eol);
    }
    (text, fresh.len())
}

fn row_line(header: &[String], r: &Row) -> String {
    let cells: Vec<String> = header
        .iter()
        .map(|h| {
            COLUMNS
                .iter()
                .position(|c| c == h)
                .map_or(String::new(), |i| cell_text(&r.cells[i]))
        })
        .collect();
    format!("| {} |", cells.join(" | "))
}

/// A tile or plot reads a fixed range, so one that ends above the last row is stretched down.
fn stretch_range(line: &str, last: usize) -> String {
    let ours = format!(" id={OWN_ID_PREFIX}");
    let is_block = line.starts_with("```tile") || line.starts_with("```plot");
    if !is_block || !line.contains(&ours) {
        return line.to_string();
    }
    RANGE_END
        .replace(line, |c: &regex::Captures| {
            let end: usize = c[3].parse().unwrap_or(0);
            format!("from={}1:{}{}", &c[1], &c[2], end.max(last))
        })
        .into_owned()
}

static RANGE_END: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
    regex::Regex::new(r"from=([A-Z]{1,3})1:([A-Z]{1,3})(\d+)").expect("static pattern")
});

fn last_row(n: usize) -> usize {
    n + 1
}

pub fn render(rows: &[Row]) -> String {
    let mut s = String::from("---\nsuite_kind: sheet\nfinance_ledger: true\n---\n\n# Ledger\n\n");
    s.push_str("Figures count only rows with status `confirmed`. Change `category`, `flow` or `status` here; a re-import never overwrites a row.\n\n");
    s.push_str(&format!(
        "```sheet name={SHEET_NAME} types=\"{TYPES}\" id=ledger\n"
    ));
    s.push_str(&format!("| {} |\n", COLUMNS.join(" | ")));
    s.push_str(&format!("|{}\n", "---|".repeat(COLUMNS.len())));
    for r in rows {
        let cells: Vec<String> = r.cells.iter().map(|c| cell_text(c)).collect();
        s.push_str(&format!("| {} |\n", cells.join(" | ")));
    }
    s.push_str("```\n\n");
    s.push_str(&tiles(last_row(rows.len())));
    s
}

fn tile(label: &str, func: &str, from: &str, cond: &str) -> String {
    let format = if func == "sum" { " format=ugx" } else { "" };
    let id = label.to_lowercase().replace(' ', "_");
    format!(
        "```tile id={OWN_ID_PREFIX}{id} from={from} fn={func} label=\"{label}\"{format} where=\"{cond}\"\n```\n\n"
    )
}

fn tiles(last: usize) -> String {
    let (s, a, f, c) = (FIRST_STATUS_COL, AMOUNT_COL, FEE_COL, CATEGORY_COL);
    let amounts = format!("{s}1:{a}{last}");
    let fees = format!("{s}1:{f}{last}");
    let confirmed = "status=confirmed";
    let mut out = String::new();
    for (label, flow) in [
        ("Money in", "inflow"),
        ("Money out", "outflow"),
        ("Cash withdrawn", "cashout"),
        ("Loan received", "loan_in"),
        ("Loan repaid", "loan_out"),
    ] {
        out.push_str(&tile(
            label,
            "sum",
            &amounts,
            &format!("flow={flow} and {confirmed}"),
        ));
    }
    out.push_str(&tile("Fees paid", "sum", &fees, confirmed));
    let drafts = format!("status={STATUS_DRAFT}");
    out.push_str(&tile("Waiting for review", "count", &amounts, &drafts));
    out.push_str(&format!(
        "```plot id={OWN_ID_PREFIX}spending type=bar from={c}1:{a}{last} x=category y=amount title=\"Spending by category\" where=\"flow=outflow and {confirmed}\"\n```\n"
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse_at;

    const SENT: &str = "You have sent UGX 32,000 to <NAME>, 0700000000 on 2026-03-04 10:11:12, fee: 218. Reason: . New balance: 80,455. ID :47152250553.";

    fn draft() -> Draft {
        parse_at("MTNMobMoney", SENT, RECEIVED_MS).unwrap()
    }

    #[test]
    fn money_cells_are_plain_numbers() {
        assert_eq!(
            (money(3_200_000), money(5035), money(21_800), money(50)),
            ("32000".into(), "50.35".into(), "218".into(), "0.5".into())
        );
    }

    #[test]
    fn rows_round_trip_through_the_note() {
        let (note, added) = import(None, &[draft()]);
        assert_eq!(added, 1);
        let rows = parse_rows(&note);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].cells[0], "2026-03-04");
        assert_eq!(rows[0].cells[7], "outflow");
        assert_eq!(rows[0].cells[8], "32000");
    }

    #[test]
    fn import_twice_adds_nothing_and_keeps_the_owners_edits() {
        let (note, _) = import(None, &[draft()]);
        let edited = note
            .replace("| unknown |", "| transport |")
            .replace("| draft |", "| confirmed |");
        let (again, added) = import(Some(&edited), &[draft()]);
        assert_eq!(added, 0);
        let row = &parse_rows(&again)[0];
        assert_eq!(
            (row.cells[5].as_str(), row.cells[6].as_str()),
            ("transport", "confirmed")
        );
    }

    #[test]
    fn a_pipe_in_a_cell_does_not_break_the_row() {
        assert_eq!(split_cells("| a \\| b | c |"), vec!["a | b", "c"]);
    }

    const RECEIVED_MS: i64 = 1_772_600_000_000;
    const SECOND: &str = "You have received UGX 5,000 from <NAME> on 2026-03-05 09:00:00. fee:0. Reason: . New balance: UGX 9,000. ID: 47152250554.";

    #[test]
    fn a_column_a_note_and_a_tile_the_owner_added_survive_an_import() {
        let (note, _) = import(None, &[draft()]);
        let edited = note
            .replacen("| date |", "| mood | date |", 1)
            .replacen("|---|", "|---|---|", 1)
            .replace("| 2026-03-04 |", "| happy | 2026-03-04 |")
            .replace("# Ledger\n", "# Ledger\n\nMy own note about March.\n")
            + "\n```tile from=A1:A2 fn=count label=\"Mine\"\n```\n";
        let second = parse_at("MTNMobMoney", SECOND, RECEIVED_MS).unwrap();
        let (after, added) = import(Some(&edited), &[draft(), second]);
        assert_eq!(added, 1);
        assert!(
            after.contains("My own note about March.") && after.contains("| happy | 2026-03-04 |")
        );
        assert!(
            after.contains("```tile from=A1:A2 fn=count label=\"Mine\""),
            "an unrelated tile is untouched"
        );
        assert_eq!(
            parse_rows(&after).len(),
            2,
            "the extra column did not hide a row"
        );
    }

    #[test]
    fn tiles_stretch_to_cover_appended_rows_and_nothing_else_moves() {
        let (note, _) = import(None, &[draft()]);
        let second = parse_at("MTNMobMoney", SECOND, RECEIVED_MS).unwrap();
        let (after, _) = import(Some(&note), &[second]);
        assert!(
            after.contains("from=G1:I3 fn=sum"),
            "range now ends at row 3"
        );
        assert!(note.contains("from=G1:I2 fn=sum"));
    }

    #[test]
    fn crlf_notes_keep_their_line_endings() {
        let (note, _) = import(None, &[draft()]);
        let crlf = note.replace('\n', "\r\n");
        let second = parse_at("MTNMobMoney", SECOND, RECEIVED_MS).unwrap();
        let (after, _) = import(Some(&crlf), &[second]);
        assert!(
            !after.replace("\r\n", "").contains('\n'),
            "no bare line feed was introduced"
        );
    }

    #[test]
    fn failed_and_requested_messages_are_not_ledger_rows() {
        assert!(flow_of(Kind::Failed).is_none() && flow_of(Kind::WithdrawalRequested).is_none());
    }
}
