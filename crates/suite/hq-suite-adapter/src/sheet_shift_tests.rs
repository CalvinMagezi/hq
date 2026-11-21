//! Row inserts and deletes that move the references reading them.

use crate::edit::{Gate, run};
use crate::sheets::view;
use serde_json::{Value, json};

const NOTE: &str = "---\nsuite_kind: sheet\n---\n# Stock\n\n```sheet name=\"Stock\" types=\"text,int,ugx,ugx\"\n| item  | qty | price | total  |\n|-------|-----|-------|--------|\n| Sugar | 40  | 4200  | =B2*C2 |\n| Salt  | 25  | 1800  | =B3*C3 |\n| Rice  | 10  | 3000  | =B4*C4 |\n| Total |     |       | =SUM(D2:D4) |\n```\n";

const REL: &str = "Notebooks/n.md";

fn vault(note: &str) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(d.path().join("Notebooks")).unwrap();
    std::fs::write(d.path().join(REL), note).unwrap();
    d
}

fn text(d: &tempfile::TempDir) -> String {
    std::fs::read_to_string(d.path().join(REL)).unwrap()
}

fn hash(d: &tempfile::TempDir, n: usize) -> String {
    let doc = hq_suite_md::Document::parse(&text(d));
    let (i, _) = doc
        .typed_blocks()
        .filter_map(|(i, b)| b.ok().map(|b| (i, b)))
        .filter(|(_, b)| b.ty == hq_suite_md::BlockType::Sheet)
        .nth(n)
        .unwrap();
    doc.node(i).unwrap().hash()
}

fn go(d: &tempfile::TempDir, mut args: Value) -> anyhow::Result<Value> {
    args["note"] = json!(REL);
    run(d.path(), &args, Gate::Owner)
}

/// The calculated text of every cell of sheet `n`, row by row.
fn shown(d: &tempfile::TempDir, n: usize) -> Vec<Vec<String>> {
    let v = view(d.path(), REL, n, 0, 0).unwrap();
    v.rows
        .iter()
        .map(|r| r.iter().map(|c| c.text.clone()).collect())
        .collect()
}

#[test]
fn an_insert_inside_a_range_keeps_every_value_and_the_total_covers_the_new_row() {
    let d = vault(NOTE);
    let before = shown(&d, 0);
    let h = hash(&d, 0);
    let out = go(
        &d,
        json!({"action": "add_rows", "hash": h, "at": 2, "rows": [["Tea", 2, 1000, "=B3*C3"]]}),
    )
    .unwrap();
    assert!(
        out["summary"]
            .as_str()
            .unwrap()
            .contains("Adjusted 3 formulas"),
        "{out}"
    );
    let t = text(&d);
    assert!(t.contains("| Tea | 2 | 1000 | =B3*C3 |"), "{t}");
    assert!(t.contains("=SUM(D2:D5)"), "{t}");
    assert!(t.contains("| Salt  | 25  | 1800  | =B4*C4 |"), "{t}");
    let after = shown(&d, 0);
    // Rows above and below the new one show what they showed.
    assert_eq!(after[..2], before[..2]);
    assert_eq!(after[3..5], before[2..4]);
    // The new row is worth 2 * 1000, and the total grew by it.
    assert_eq!(after[2][3], "UGX 2,000");
    let total = |rows: &Vec<Vec<String>>| rows.last().unwrap()[3].clone();
    assert_eq!(total(&before), "UGX 243,000");
    assert_eq!(total(&after), "UGX 245,000");
}

#[test]
fn an_insert_below_the_last_row_of_a_range_does_not_widen_it() {
    let d = vault(NOTE);
    let before = shown(&d, 0);
    let h = hash(&d, 0);
    // Before the Total row, which is table row 4: the range D2:D4 ends just above it.
    go(
        &d,
        json!({"action": "add_rows", "hash": h, "at": 4, "rows": [["Tea", 2, 1000, "=B5*C5"]]}),
    )
    .unwrap();
    let t = text(&d);
    assert!(t.contains("=SUM(D2:D4)"), "{t}");
    let after = shown(&d, 0);
    assert_eq!(after[..4], before[..4]);
    assert_eq!(after.last().unwrap()[3], before.last().unwrap()[3]);
}

#[test]
fn deleting_a_row_shrinks_the_total_and_keeps_the_other_values() {
    let d = vault(NOTE);
    let before = shown(&d, 0);
    let h = hash(&d, 0);
    go(&d, json!({"action": "delete_rows", "hash": h, "row": 2})).unwrap();
    let t = text(&d);
    assert!(t.contains("=SUM(D2:D3)"), "{t}");
    assert!(t.contains("| Rice  | 10  | 3000  | =B3*C3 |"), "{t}");
    let after = shown(&d, 0);
    assert_eq!(after[1], before[1]);
    assert_eq!(after[2], before[3]);
    assert_eq!(after[3][3], "UGX 198,000");
}

#[test]
fn a_formula_that_read_a_deleted_row_says_so_and_the_edit_still_goes_through() {
    let note = "```sheet name=S\n| a | b |\n|---|---|\n| 1 | 2 |\n| 3 | =A2+A3 |\n| 5 | 6 |\n```\n";
    let d = vault(note);
    let h = hash(&d, 0);
    let out = go(&d, json!({"action": "delete_rows", "hash": h, "row": 1})).unwrap();
    let t = text(&d);
    assert!(t.contains("| 3 | =#REF!+A2 |"), "{t}");
    assert!(out["summary"].as_str().unwrap().contains("#REF!"), "{out}");
}

#[test]
fn tiles_plots_and_other_sheets_follow_and_another_notes_range_does_not() {
    let note = "# n\n\n```sheet name=Names\n| who | n |\n|-----|---|\n| Ann | 1 |\n| Bob | 2 |\n| Cy  | 3 |\n```\n\n```tile from=\"B2:B4\" fn=sum label=Total\n```\n\n```plot from=\"[[n]]!A1:B4\" kind=bar\n```\n\n```tile from=\"[[Elsewhere]]!B2:B4\" fn=sum label=Far\n```\n\n```sheet name=Other\n| v |\n|---|\n| =SUM(names!B2:B4) |\n| =SUM(B2:B3) |\n```\n";
    let d = vault(note);
    let h = hash(&d, 0);
    let out = go(
        &d,
        json!({"action": "add_rows", "hash": h, "at": 1, "rows": [["Zed", 0]]}),
    )
    .unwrap();
    let after = text(&d);
    assert!(after.contains("from=\"B3:B5\""), "{after}");
    assert!(after.contains("[[n]]!A1:B5"), "{after}");
    assert!(after.contains("[[Elsewhere]]!B2:B4"), "{after}");
    assert!(after.contains("=SUM(names!B3:B5)"), "{after}");
    assert!(after.contains("| =SUM(B2:B3) |"), "{after}");
    let s = out["summary"].as_str().unwrap();
    assert!(s.contains("Adjusted 1 formula, 1 tile and 1 chart"), "{s}");
}

#[test]
fn deleting_every_row_a_tile_reads_is_refused_in_one_plain_sentence() {
    let note = "```sheet name=Names\n| who | n |\n|-----|---|\n| Ann | 1 |\n| Bob | 2 |\n```\n\n```tile from=\"B3:B3\" fn=sum label=Total\n```\n";
    let d = vault(note);
    let h = hash(&d, 0);
    let before = text(&d);
    let e = go(&d, json!({"action": "delete_rows", "hash": h, "row": 2}))
        .unwrap_err()
        .to_string();
    assert!(
        e.contains("a tile block reads only the rows you are deleting"),
        "{e}"
    );
    assert_eq!(text(&d), before);
}

#[test]
fn spacing_and_other_blocks_are_untouched_and_a_formula_on_a_sidecar_sheet_is_not_guessed() {
    let note = "# n\n\n```sheet name=S\n|  a |   b   |\n|----|-------|\n|  1 | =A2+1 |\n|  2 | =A3+1 |\n```\n\nText with A3 in it.\n";
    let d = vault(note);
    let h = hash(&d, 0);
    go(
        &d,
        json!({"action": "add_rows", "hash": h, "at": 1, "rows": [["0"]]}),
    )
    .unwrap();
    assert_eq!(
        text(&d),
        "# n\n\n```sheet name=S\n|  a |   b   |\n|----|-------|\n| 0 |  |\n|  1 | =A3+1 |\n|  2 | =A4+1 |\n```\n\nText with A3 in it.\n"
    );
}

// ---------------------------------------------------------------- columns

fn without_col(rows: &[Vec<String>], col: usize) -> Vec<Vec<String>> {
    rows.iter()
        .map(|r| {
            let mut r = r.clone();
            r.remove(col);
            r
        })
        .collect()
}

#[test]
fn a_column_inserted_before_b_keeps_every_value_and_moves_the_formulas() {
    let d = vault(NOTE);
    let before = shown(&d, 0);
    let h = hash(&d, 0);
    let out = go(
        &d,
        json!({"action": "add_column", "hash": h, "at": 1, "name": "sku", "type": "text"}),
    )
    .unwrap();
    let s = out["summary"].as_str().unwrap();
    assert!(s.contains("Added column B `sku`"), "{s}");
    assert!(s.contains("Adjusted 4 formulas"), "{s}");
    let t = text(&d);
    assert!(t.contains("types=\"text,text,int,ugx,ugx\""), "{t}");
    assert!(t.contains("=C2*D2"), "{t}");
    assert!(t.contains("=SUM(E2:E4)"), "{t}");
    let after = shown(&d, 0);
    assert_eq!(without_col(&after, 1), before);
    assert_eq!(after[0][1], "sku");
}

#[test]
fn add_column_without_at_still_goes_on_the_right_and_at_past_the_end_is_refused() {
    let d = vault(NOTE);
    let h = hash(&d, 0);
    let e = go(
        &d,
        json!({"action": "add_column", "hash": h, "at": 9, "name": "x"}),
    )
    .unwrap_err()
    .to_string();
    assert!(e.contains("`at` must be from 0"), "{e}");
    let out = go(
        &d,
        json!({"action": "add_column", "hash": h, "at": 4, "name": "x"}),
    )
    .unwrap();
    assert!(!out["summary"].as_str().unwrap().contains("Adjusted"));
    // The header keeps its own padding; the new cell is appended after it.
    assert!(
        text(&d).contains("| item  | qty | price | total  | x |"),
        "{}",
        text(&d)
    );
}

#[test]
fn deleting_a_column_a_formula_reads_shows_ref_and_keeps_the_others() {
    let d = vault(NOTE);
    let before = shown(&d, 0);
    let h = hash(&d, 0);
    let out = go(
        &d,
        json!({"action": "delete_column", "hash": h, "col": 1, "expected": "qty"}),
    )
    .unwrap();
    let s = out["summary"].as_str().unwrap();
    assert!(s.contains("Deleted column B `qty`"), "{s}");
    assert!(s.contains("#REF!"), "{s}");
    let t = text(&d);
    assert!(t.contains("types=\"text,ugx,ugx\""), "{t}");
    assert!(t.contains("=#REF!*B2"), "{t}");
    assert!(t.contains("=SUM(C2:C4)"), "{t}");
    let after = shown(&d, 0);
    assert_eq!(after[1][..2], without_col(&before, 1)[1][..2]);
    // The calc engine cannot read a #REF! token, so the cell shows an error rather than a number.
    assert!(after[1][2].starts_with('#'), "{}", after[1][2]);
}

#[test]
fn deleting_a_column_nothing_reads_leaves_every_other_value() {
    let d = vault(NOTE);
    let before = shown(&d, 0);
    let h = hash(&d, 0);
    go(&d, json!({"action": "delete_column", "hash": h, "col": 0})).unwrap();
    let after = shown(&d, 0);
    assert_eq!(after, without_col(&before, 0));
    let t = text(&d);
    assert!(t.contains("=A2*B2"), "{t}");
}

#[test]
fn delete_column_is_checked_and_an_agent_sees_a_preview_first() {
    let d = vault(NOTE);
    let h = hash(&d, 0);
    let stale = go(
        &d,
        json!({"action": "delete_column", "hash": h, "col": 1, "expected": "price"}),
    )
    .unwrap_err()
    .to_string();
    assert!(stale.contains("is `qty`, not `price`"), "{stale}");
    let past = go(&d, json!({"action": "delete_column", "hash": h, "col": 7}))
        .unwrap_err()
        .to_string();
    assert!(past.contains("is not one"), "{past}");
    let mut args = json!({"action": "delete_column", "hash": h, "col": 0, "note": REL});
    args["note"] = json!(REL);
    let before = text(&d);
    let out = run(d.path(), &args, Gate::Agent).unwrap();
    assert_eq!(out["preview"], true);
    assert_eq!(out["destructive"], true);
    assert_eq!(text(&d), before);
}

#[test]
fn the_last_column_cannot_be_deleted() {
    let d = vault("```sheet name=S\n| a |\n|---|\n| 1 |\n```\n");
    let h = hash(&d, 0);
    let e = go(&d, json!({"action": "delete_column", "hash": h, "col": 0}))
        .unwrap_err()
        .to_string();
    assert!(e.contains("only column cannot be deleted"), "{e}");
}

#[test]
fn tiles_plots_other_sheets_and_whole_columns_follow_a_column_insert() {
    let note = "# n\n\n```sheet name=Names\n| who | n | m |\n|-----|---|---|\n| Ann | 1 | 2 |\n| Bob | 2 | 3 |\n```\n\n```tile from=\"B2:B3\" fn=sum label=Total\n```\n\n```plot from=\"[[n]]!A1:C3\" kind=bar\n```\n\n```tile from=\"[[Elsewhere]]!B2:B3\" fn=sum label=Far\n```\n\n```sheet name=Other\n| v |\n|---|\n| =SUM(names!B:B) |\n| =SUM(B2:B3) |\n```\n";
    let d = vault(note);
    let h = hash(&d, 0);
    let out = go(
        &d,
        json!({"action": "add_column", "hash": h, "at": 0, "name": "id"}),
    )
    .unwrap();
    let after = text(&d);
    assert!(after.contains("from=\"C2:C3\""), "{after}");
    assert!(after.contains("[[n]]!B1:D3"), "{after}");
    assert!(after.contains("[[Elsewhere]]!B2:B3"), "{after}");
    assert!(after.contains("=SUM(names!C:C)"), "{after}");
    assert!(after.contains("| =SUM(B2:B3) |"), "{after}");
    let s = out["summary"].as_str().unwrap();
    assert!(s.contains("Adjusted 1 formula, 1 tile and 1 chart"), "{s}");
}

#[test]
fn deleting_every_column_a_tile_reads_is_refused_in_one_plain_sentence() {
    let note = "```sheet name=Names\n| who | n |\n|-----|---|\n| Ann | 1 |\n```\n\n```tile from=\"B2:B2\" fn=sum label=Total\n```\n";
    let d = vault(note);
    let h = hash(&d, 0);
    let before = text(&d);
    let e = go(&d, json!({"action": "delete_column", "hash": h, "col": 1}))
        .unwrap_err()
        .to_string();
    assert!(
        e.contains("a tile block reads only the columns you are deleting"),
        "{e}"
    );
    assert_eq!(text(&d), before);
}

#[test]
fn inserting_then_deleting_a_column_gives_the_note_back() {
    let note = "# n\n\n```sheet name=S types=\"int,int\"\n| a | b |\n|---|---|\n| 1 | =A2+1 |\n| 2 | =SUM(A2:B3) |\n```\n\n```tile from=\"A2:B3\" fn=sum label=T\n```\n";
    let d = vault(note);
    let h = hash(&d, 0);
    go(
        &d,
        json!({"action": "add_column", "hash": h, "at": 1, "name": "x", "type": "num"}),
    )
    .unwrap();
    assert_ne!(text(&d), note);
    let h = hash(&d, 0);
    go(
        &d,
        json!({"action": "delete_column", "hash": h, "col": 1, "expected": "x"}),
    )
    .unwrap();
    assert_eq!(text(&d), note);
}

#[test]
fn deleting_a_column_a_plot_names_is_refused_but_one_it_does_not_name_goes() {
    let note =
        format!("{NOTE}\n```plot type=bar from=\"A1:D4\" x=item y=qty where=\"price>1000\"\n```\n");
    let d = vault(&note);
    let h = hash(&d, 0);
    for col in [0, 1, 2] {
        let e = go(
            &d,
            json!({"action": "delete_column", "hash": h, "col": col}),
        )
        .unwrap_err()
        .to_string();
        assert!(
            e.contains("uses the column") && e.contains("by name"),
            "{col}: {e}"
        );
    }
    assert_eq!(text(&d), note);
}
