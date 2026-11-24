//! Export file to ledger note to ledger rows, end to end, on the redacted corpus. The ignored test
//! runs the same checks on a real export you name: `HQ_FINANCE_EXPORT=<file> cargo test -p
//! hq-finance --test pipeline -- --ignored --nocapture`. It prints counts only.

use hq_finance::{export::drafts_from_export, ledger, validate};
use serde::Deserialize;

#[derive(Deserialize)]
struct Msg {
    sender: String,
    body: String,
}

/// Redacted fixtures have no dates, so give each a day apart, in order.
fn fixture_export() -> String {
    const DAY_MS: i64 = 86_400_000;
    const EPOCH_2026_MS: i64 = 1_767_225_600_000;
    let msgs: Vec<Msg> =
        serde_json::from_str(include_str!("fixtures/mobile_money_redacted.json")).unwrap();
    let rows: Vec<_> = msgs
        .iter()
        .enumerate()
        .map(|(i, m)| serde_json::json!({"sender": m.sender, "date": EPOCH_2026_MS + i as i64 * DAY_MS, "body": m.body}))
        .collect();
    serde_json::to_string(&rows).unwrap()
}

#[test]
fn importing_twice_adds_nothing_the_second_time() {
    let drafts = drafts_from_export(&fixture_export()).unwrap();
    assert!(drafts.len() > 50, "the corpus should yield many drafts");
    let (note, added) = ledger::import(None, &drafts);
    assert!(added > 0);
    let (again, added_again) = ledger::import(Some(&note), &drafts);
    assert_eq!(added_again, 0);
    assert_eq!(note, again, "a re-import must not change the note at all");
}

#[test]
fn every_ledger_row_has_a_date_an_amount_and_a_known_flow() {
    let drafts = drafts_from_export(&fixture_export()).unwrap();
    let (note, _) = ledger::import(None, &drafts);
    for row in ledger::parse_rows(&note) {
        assert!(
            !row.cells[0].is_empty(),
            "every row is dated, by the message or by its arrival"
        );
        assert!(
            row.cells[8].parse::<f64>().is_ok(),
            "amount is a plain number: {:?}",
            row.cells[8]
        );
        assert!(
            ["inflow", "outflow", "cashout", "loan_in", "loan_out"]
                .contains(&row.cells[7].as_str())
        );
    }
}

#[test]
#[ignore = "needs a real export: set HQ_FINANCE_EXPORT to a JSON file"]
fn real_export_replays_cleanly() {
    let path = std::env::var("HQ_FINANCE_EXPORT").expect("HQ_FINANCE_EXPORT");
    let drafts = drafts_from_export(&std::fs::read_to_string(path).unwrap()).unwrap();
    let report = validate::replay(&drafts);
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
    assert!(report.explained_share().unwrap_or(0.0) >= 0.5);
}
