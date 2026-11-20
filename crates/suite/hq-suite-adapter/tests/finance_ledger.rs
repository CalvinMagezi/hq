//! The finance ledger is an hqmd note the Suite computes and draws. This proves the Suite reads
//! exactly what `hq-finance` writes, that the figures are right, and that the note sits where agents
//! cannot read it. Synthetic messages only.

use hq_finance::ledger::{STATUS_CONFIRMED, STATUS_DRAFT, import};
use hq_finance::parse::parse_at;
use hq_suite_adapter::sheets;

const NOTE: &str = "_data/finance/ledger.md";
const RECEIVED_MS: i64 = 1_772_600_000_000;

const MESSAGES: [&str; 4] = [
    "You have sent UGX 32,000 to <NAME>, 0700000000 on 2026-03-04 10:11:12, fee: 218. Reason: . New balance: 80,455. ID :47152250553.",
    "You have received UGX 145,200 from <NAME> on 2026-03-05 09:00:00. fee:0. Reason: . New balance: UGX 225,655. ID: 47152250554.",
    "You have paid <NAME> 901368 UGX 25,771.  Fee: UGX 0.  New balance: UGX 199,884.10. ID: 47152250555.",
    "You have sent UGX 10,000 to <NAME>, 0700000001 on 2026-03-06 11:00:00, fee: 100. Reason: . New balance: 189,784. ID :47152250556.",
];

fn vault_with_ledger() -> tempfile::TempDir {
    let drafts: Vec<_> = MESSAGES
        .iter()
        .map(|m| parse_at("MTNMobMoney", m, RECEIVED_MS).expect("parses"))
        .collect();
    let (note, added) = import(None, &drafts);
    assert_eq!(added, 4);
    // The owner confirms the first three and leaves the last for review.
    let mut confirmed = 0;
    let note: String = note
        .lines()
        .map(|l| {
            if l.starts_with("| 2026") && confirmed < 3 {
                confirmed += 1;
                l.replacen(STATUS_DRAFT, STATUS_CONFIRMED, 1)
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(NOTE);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, note + "\n").unwrap();
    dir
}

fn tile_text(dir: &tempfile::TempDir, i: usize) -> String {
    let t = sheets::tile(dir.path(), NOTE, i).unwrap();
    format!("{}={}", t.label, t.text)
}

#[test]
fn the_suite_totals_only_confirmed_rows() {
    let dir = vault_with_ledger();
    // Order: in, out, cash, loan in, loan out, fees, waiting.
    assert_eq!(tile_text(&dir, 0), "Money in=UGX 145,200");
    assert_eq!(tile_text(&dir, 1), "Money out=UGX 57,771");
    assert_eq!(tile_text(&dir, 5), "Fees paid=UGX 218");
    assert_eq!(tile_text(&dir, 6), "Waiting for review=1");
}

#[test]
fn the_suite_draws_the_ledger_as_a_sheet_and_a_chart() {
    let dir = vault_with_ledger();
    let view = sheets::view(dir.path(), NOTE, 0, 0, 0).unwrap();
    assert_eq!(view.row_count, 5);
    assert!(sheets::plot(dir.path(), NOTE, 0).is_ok());
}

#[test]
fn the_ledger_sits_where_agent_tools_cannot_read_it() {
    let hit = hq_vault::rel_path::protected_folder(NOTE, hq_vault::rel_path::PRIVATE_FOLDERS);
    assert_eq!(hit.unwrap(), Some("_data"));
}
