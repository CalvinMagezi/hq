use super::*;
use crate::edit::{Gate, run};

const NOTE: &str = "---\nsuite_kind: sheet\n---\n# Stock\n\nIntro text.\n\n```sheet name=\"Stock\" types=\"text,int,ugx,ugx\"\n| item  | qty | price | total  |\n|-------|-----|-------|--------|\n| Sugar | 40  | 4200  | =B2*C2 |\n| Salt  | 25  | 1800  | =B3*C3 |\n| Total |     |       | =SUM(D2:D3) |\n```\n\nAfter text.\n\n```rust\nfn keep() {}\n```\n";

/// Rows with no formula, so rows can be added and deleted anywhere.
const PLAIN: &str = "# Names\n\n```sheet name=Names\n| who | n |\n|-----|---|\n| Ann | 1 |\n| Bob | 2 |\n| Cy  | 3 |\n```\n\nEnd.\n";

fn vault(note: &str) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(d.path().join("Notebooks")).unwrap();
    std::fs::write(d.path().join("Notebooks/n.md"), note).unwrap();
    d
}

fn text(d: &tempfile::TempDir) -> String {
    std::fs::read_to_string(d.path().join("Notebooks/n.md")).unwrap()
}

/// The hash of the `n`th sheet block, as `blocks` would list it.
fn sheet_hash(d: &tempfile::TempDir, n: usize) -> String {
    let doc = Document::parse(&text(d));
    let (node, _) = doc
        .typed_blocks()
        .filter_map(|(i, b)| b.ok().map(|b| (i, b)))
        .filter(|(_, b)| b.ty == BlockType::Sheet)
        .nth(n)
        .unwrap();
    doc.node(node).unwrap().hash()
}

fn go(d: &tempfile::TempDir, mut args: Value) -> Result<Value> {
    args["note"] = json!("Notebooks/n.md");
    run(d.path(), &args, Gate::Owner)
}

fn err(d: &tempfile::TempDir, args: Value) -> String {
    let before = text(d);
    let e = go(d, args).unwrap_err().to_string();
    assert_eq!(text(d), before, "a refused edit writes nothing");
    e
}

#[test]
fn set_cells_changes_only_the_lines_it_touches() {
    let d = vault(NOTE);
    let out = go(
        &d,
        json!({"action": "set_cells", "row": 1, "col": 1,
               "values": [[50, "4300"], [30, "1900"]],
               "expected": [["40", "4200"], ["25", "1800"]]}),
    )
    .unwrap();
    assert_eq!(out["changed"], true);
    let want = NOTE
        .replace(
            "| Sugar | 40  | 4200  | =B2*C2 |",
            "| Sugar | 50 | 4300 | =B2*C2 |",
        )
        .replace(
            "| Salt  | 25  | 1800  | =B3*C3 |",
            "| Salt | 30 | 1900 | =B3*C3 |",
        );
    assert_eq!(
        text(&d),
        want,
        "two table lines changed, every other byte is as it was"
    );
    assert!(out["summary"].as_str().unwrap().contains("B2"));
}

#[test]
fn set_cells_is_refused_whole_when_any_cell_went_stale() {
    let d = vault(NOTE);
    let e = err(
        &d,
        json!({"action": "set_cells", "row": 1, "col": 1,
               "values": [[50, 1], [30, 2]], "expected": [["40", "4200"], ["25", "9999"]]}),
    );
    assert!(e.contains("C3 changed since you read it"), "{e}");
}

#[test]
fn set_cells_checks_shape_text_and_formulas() {
    let d = vault(NOTE);
    let base = |values: Value, expected: Value| json!({"action": "set_cells", "row": 1, "col": 1, "values": values, "expected": expected});
    assert!(
        err(
            &d,
            base(json!([[1, 2], [3]]), json!([["40", "4200"], ["25"]]))
        )
        .contains("rectangle")
    );
    assert!(err(&d, base(json!([[1]]), json!([["40", "4200"]]))).contains("same size"));
    assert!(err(&d, base(json!([["a|b"]]), json!([["40"]]))).contains("cannot hold a |"));
    assert!(err(&d, base(json!([["a\nb"]]), json!([["40"]]))).contains("line break"));
    let long = "x".repeat(MAX_CELL_CHARS + 1);
    assert!(err(&d, base(json!([[long]]), json!([["40"]]))).contains("at most"));
    let outside = json!({"action": "set_cells", "row": 4, "col": 3,
        "values": [[1, 2]], "expected": [["", ""]]});
    assert!(err(&d, outside).contains("outside the sheet"));
    // Formulas: unsafe, unknown functions and unreadable ones are refused; a good one is kept.
    for bad in [
        "=HYPERLINK(\"http://example.com\")",
        "=MYMACRO(1)",
        "=SUM(",
        "=",
        "='[x.xlsx]S'!A1",
    ] {
        let e = err(&d, base(json!([[bad]]), json!([["40"]])));
        assert!(
            e.contains("formula") || e.contains("sheet does not allow"),
            "{bad}: {e}"
        );
    }
    go(&d, base(json!([["=C2+1"]]), json!([["40"]]))).unwrap();
    assert!(text(&d).contains("| Sugar | =C2+1 | 4200 | =B2*C2 |"));
}

#[test]
fn set_cells_can_rename_a_header_but_not_into_a_duplicate() {
    let d = vault(NOTE);
    go(
        &d,
        json!({"action": "set_cells", "row": 0, "col": 0, "values": [["name"]], "expected": [["item"]]}),
    )
    .unwrap();
    assert!(text(&d).contains("| name | qty | price | total |"));
    let e = err(
        &d,
        json!({"action": "set_cells", "row": 0, "col": 1, "values": [["name"]], "expected": [["qty"]]}),
    );
    assert!(e.contains("twice") || e.contains("break the sheet"), "{e}");
}

#[test]
fn rows_are_added_at_the_end_even_when_formulas_read_the_rows_above() {
    let d = vault(NOTE);
    let hash = sheet_hash(&d, 0);
    let out = go(
        &d,
        json!({"action": "add_rows", "hash": hash, "rows": [["Rice", 10, 3000, "=B5*C5"], ["Tea", 2]]}),
    )
    .unwrap();
    assert_eq!(out["changed"], true);
    let want = NOTE.replace(
        "| Total |     |       | =SUM(D2:D3) |\n",
        "| Total |     |       | =SUM(D2:D3) |\n| Rice | 10 | 3000 | =B5*C5 |\n| Tea | 2 |  |  |\n",
    );
    assert_eq!(text(&d), want);
}

#[test]
fn add_rows_checks_its_arguments() {
    let d = vault(PLAIN);
    let hash = sheet_hash(&d, 0);
    assert!(err(&d, json!({"action": "add_rows", "rows": [["a", 1]]})).contains("`hash`"));
    assert!(
        err(
            &d,
            json!({"action": "add_rows", "hash": "stale", "rows": [["a", 1]]})
        )
        .contains("changed since you read")
    );
    assert!(
        err(
            &d,
            json!({"action": "add_rows", "hash": hash, "rows": [["a", 1, 2]]})
        )
        .contains("3 cells but the sheet has 2")
    );
    assert!(err(&d, json!({"action": "add_rows", "hash": hash, "rows": []})).contains("between 1"));
    assert!(
        err(
            &d,
            json!({"action": "add_rows", "hash": hash, "at": 0, "rows": [["a", 1]]})
        )
        .contains("`at` must be")
    );
    assert!(
        err(
            &d,
            json!({"action": "add_rows", "hash": hash, "at": 9, "rows": [["a", 1]]})
        )
        .contains("`at` must be")
    );
    assert!(
        err(
            &d,
            json!({"action": "add_rows", "hash": hash, "rows": [["=NOPE(1)", 1]]})
        )
        .contains("formula")
    );
}

#[test]
fn the_cell_limit_of_a_sheet_is_kept() {
    let d = vault(PLAIN);
    let hash = sheet_hash(&d, 0);
    let rows: Vec<Value> = (0..MAX_ROWS_PER_CALL).map(|i| json!([i, i])).collect();
    // Nine calls of 5,000 rows of 2 cells fit; a tenth would pass 100,000 cells.
    go(
        &d,
        json!({"action": "add_rows", "hash": hash, "rows": rows}),
    )
    .unwrap();
    for _ in 0..8 {
        let hash = sheet_hash(&d, 0);
        let rows: Vec<Value> = (0..MAX_ROWS_PER_CALL).map(|i| json!([i, i])).collect();
        go(
            &d,
            json!({"action": "add_rows", "hash": hash, "rows": rows}),
        )
        .unwrap();
    }
    let hash = sheet_hash(&d, 0);
    let rows: Vec<Value> = (0..MAX_ROWS_PER_CALL).map(|i| json!([i, i])).collect();
    let e = err(
        &d,
        json!({"action": "add_rows", "hash": hash, "rows": rows}),
    );
    assert!(e.contains("too many") || e.contains("cells"), "{e}");
}

#[test]
fn rows_are_deleted_and_the_formulas_below_follow() {
    let d = vault(NOTE);
    let hash = sheet_hash(&d, 0);
    let out = go(&d, json!({"action": "delete_rows", "hash": hash, "row": 1})).unwrap();
    assert!(
        out["summary"]
            .as_str()
            .unwrap()
            .contains("Adjusted 2 formulas")
    );
    assert_eq!(
        text(&d),
        NOTE.replace("| Sugar | 40  | 4200  | =B2*C2 |\n", "")
            .replace("=B3*C3", "=B2*C2")
            .replace("=SUM(D2:D3)", "=SUM(D2:D2)")
    );
    let p = vault(PLAIN);
    let hash = sheet_hash(&p, 0);
    let call = |args: Value| {
        let mut a = args;
        a["note"] = json!("Notebooks/n.md");
        run(p.path(), &a, Gate::Owner)
    };
    assert!(
        call(json!({"action": "delete_rows", "hash": hash, "row": 0})).is_err(),
        "header"
    );
    assert!(
        call(json!({"action": "delete_rows", "hash": hash, "row": 3, "count": 2})).is_err(),
        "past the end"
    );
    assert!(
        call(
            json!({"action": "delete_rows", "hash": hash, "row": 2, "expected": [["Wrong", "9"]]})
        )
        .is_err(),
        "stale rows"
    );
    let out =
        call(json!({"action": "delete_rows", "hash": hash, "row": 2, "expected": [["Bob", "2"]]}))
            .unwrap();
    assert_eq!(out["changed"], true);
    assert_eq!(text(&p), PLAIN.replace("| Bob | 2 |\n", ""));
    assert!(out["summary"].as_str().unwrap().contains("Bob"));
}

#[test]
fn a_formula_inside_the_deleted_rows_does_not_block_their_deletion() {
    let note = "```sheet name=S\n| a | b |\n|---|---|\n| 1 | =A2*2 |\n| 2 | 4 |\n```\n";
    let d = vault(note);
    let hash = sheet_hash(&d, 0);
    go(&d, json!({"action": "delete_rows", "hash": hash, "row": 1})).unwrap();
    assert_eq!(
        text(&d),
        "```sheet name=S\n| a | b |\n|---|---|\n| 2 | 4 |\n```\n"
    );
}

#[test]
fn a_column_is_added_on_the_right_with_a_type_and_a_default() {
    let d = vault(NOTE);
    let hash = sheet_hash(&d, 0);
    let out = go(
        &d,
        json!({"action": "add_column", "hash": hash, "name": "note", "type": "TEXT", "default": "ok"}),
    )
    .unwrap();
    assert_eq!(out["changed"], true);
    let want = NOTE
        .replace(
            "types=\"text,int,ugx,ugx\"",
            "types=\"text,int,ugx,ugx,text\"",
        )
        .replace(
            "| item  | qty | price | total  |",
            "| item  | qty | price | total  | note |",
        )
        .replace(
            "|-------|-----|-------|--------|",
            "|-------|-----|-------|--------|---|",
        )
        .replace(
            "| Sugar | 40  | 4200  | =B2*C2 |",
            "| Sugar | 40  | 4200  | =B2*C2 | ok |",
        )
        .replace(
            "| Salt  | 25  | 1800  | =B3*C3 |",
            "| Salt  | 25  | 1800  | =B3*C3 | ok |",
        )
        .replace(
            "| Total |     |       | =SUM(D2:D3) |",
            "| Total |     |       | =SUM(D2:D3) | ok |",
        );
    assert_eq!(text(&d), want);
}

#[test]
fn a_typed_column_on_a_sheet_with_no_types_declares_text_for_the_rest() {
    let d = vault(PLAIN);
    let hash = sheet_hash(&d, 0);
    go(
        &d,
        json!({"action": "add_column", "hash": hash, "name": "paid", "type": "bool"}),
    )
    .unwrap();
    let t = text(&d);
    assert!(
        t.contains("```sheet name=Names types=\"text,text,bool\"\n"),
        "{t}"
    );
    assert!(t.contains("| who | n | paid |"));
    assert!(t.contains("| Ann | 1 |  |"));
    // Without a type, a sheet with no types stays without.
    let p = vault(PLAIN);
    let hash = sheet_hash(&p, 0);
    run(
        p.path(),
        &json!({"action": "add_column", "note": "Notebooks/n.md", "hash": hash, "name": "x"}),
        Gate::Owner,
    )
    .unwrap();
    assert!(!text(&p).contains("types="));
}

#[test]
fn add_column_refuses_a_duplicate_name_a_formula_default_and_a_bad_type() {
    let d = vault(NOTE);
    let hash = sheet_hash(&d, 0);
    assert!(
        err(
            &d,
            json!({"action": "add_column", "hash": hash, "name": "qty"})
        )
        .contains("twice")
    );
    assert!(
        err(
            &d,
            json!({"action": "add_column", "hash": hash, "name": "z", "default": "=A1"})
        )
        .contains("default cannot be a formula")
    );
    assert!(
        err(
            &d,
            json!({"action": "add_column", "hash": hash, "name": "z", "type": "money!"})
        )
        .contains("not a column type")
    );
    assert!(
        err(
            &d,
            json!({"action": "add_column", "hash": hash, "name": " "})
        )
        .contains("needs a name")
    );
    assert!(
        err(
            &d,
            json!({"action": "add_column", "hash": "old", "name": "z"})
        )
        .contains("changed since you read")
    );
}

#[test]
fn a_column_type_changes_the_types_list_and_nothing_else() {
    let d = vault(NOTE);
    let hash = sheet_hash(&d, 0);
    go(
        &d,
        json!({"action": "set_column_type", "hash": hash, "col": 1, "type": "usd"}),
    )
    .unwrap();
    assert_eq!(
        text(&d),
        NOTE.replace("types=\"text,int,ugx,ugx\"", "types=\"text,usd,ugx,ugx\"")
    );
    let hash = sheet_hash(&d, 0);
    assert!(
        err(
            &d,
            json!({"action": "set_column_type", "hash": hash, "col": 9, "type": "int"})
        )
        .contains("not one")
    );
    assert!(
        err(
            &d,
            json!({"action": "set_column_type", "hash": hash, "col": 0, "type": "dollars"})
        )
        .contains("not a column type")
    );
    assert!(
        err(
            &d,
            json!({"action": "set_column_type", "hash": "stale", "col": 0, "type": "int"})
        )
        .contains("changed since you read")
    );
}

#[test]
fn a_sheet_is_added_at_the_end_or_after_a_block_and_names_are_checked() {
    let d = vault(NOTE);
    let out = go(
        &d,
        json!({"action": "add_sheet", "name": "Prices", "columns": ["item", "price"],
               "types": ["text", "ugx"], "rows": [["Sugar", 4200], ["Salt", "=B2/2"]]}),
    )
    .unwrap();
    assert_eq!(out["sheet"], 1);
    let t = text(&d);
    assert!(
        t.starts_with(NOTE.trim_end()),
        "everything before the new block is untouched"
    );
    assert!(t.ends_with("```sheet name=Prices types=\"text,ugx\"\n| item | price |\n|---|---|\n| Sugar | 4200 |\n| Salt | =B2/2 |\n```\n"), "{t}");
    // A formula in the new sheet may read the other sheet by name.
    go(&d, json!({"action": "add_sheet", "name": "Sum", "columns": ["v"], "rows": [["=SUM(Prices!B2:B3)"]]})).unwrap();
    assert!(
        err(
            &d,
            json!({"action": "add_sheet", "name": "prices", "columns": ["a"]})
        )
        .contains("already has a sheet")
    );
    assert!(
        err(
            &d,
            json!({"action": "add_sheet", "name": "a:b", "columns": ["a"]})
        )
        .contains("cannot hold")
    );
    assert!(
        err(
            &d,
            json!({"action": "add_sheet", "name": "x", "columns": []})
        )
        .contains("at least one")
    );
    assert!(
        err(
            &d,
            json!({"action": "add_sheet", "name": "x", "columns": ["a", "a"]})
        )
        .contains("twice")
    );
    assert!(
        err(
            &d,
            json!({"action": "add_sheet", "name": "x", "columns": ["a"], "types": ["int", "int"]})
        )
        .contains("one type per column")
    );
    assert!(
        err(
            &d,
            json!({"action": "add_sheet", "name": "x", "columns": ["a"], "rows": [["=NOPE(1)"]]})
        )
        .contains("formula")
    );
    // After a given block: it needs that block's hash.
    let doc = Document::parse(&text(&d));
    let hash = doc.node(0).unwrap().hash();
    assert!(err(&d, json!({"action": "add_sheet", "name": "y", "columns": ["a"], "after_index": 0, "hash": "stale"})).contains("changed since you read"));
    go(&d, json!({"action": "add_sheet", "name": "y", "columns": ["a"], "after_index": 0, "hash": hash})).unwrap();
    let doc = Document::parse(&text(&d));
    assert_eq!(doc.typed(1).unwrap().unwrap().info.get("name"), Some("y"));
}

#[test]
fn a_sidecar_sheet_and_an_odd_table_are_left_alone() {
    let side = "```sheet name=big data=big.bin rows=9\n| a |\n|---|\n| 1 |\n```\n";
    let d = vault(side);
    let hash = sheet_hash(&d, 0);
    assert!(
        err(
            &d,
            json!({"action": "add_rows", "hash": hash, "rows": [[1]]})
        )
        .contains("sidecar")
    );
    let escaped = "```sheet name=e\n| a |\n|---|\n| x \\| y |\n```\n";
    let d = vault(escaped);
    let hash = sheet_hash(&d, 0);
    assert!(
        err(
            &d,
            json!({"action": "add_rows", "hash": hash, "rows": [[1]]})
        )
        .contains("escaped pipe")
    );
    let d = vault(PLAIN);
    assert!(
        err(
            &d,
            json!({"action": "add_rows", "hash": "x", "sheet": 4, "rows": [[1]]})
        )
        .contains("no sheet number 5")
    );
}

#[test]
fn untouched_blocks_and_other_sheets_come_back_byte_for_byte() {
    let note =
        format!("{NOTE}\n```sheet name=Other\n|  a |   b |\n|----|-----|\n|  1 |   2 |\n```\n");
    let d = vault(&note);
    let hash = sheet_hash(&d, 0);
    go(
        &d,
        json!({"action": "add_rows", "hash": hash, "rows": [["Rice", 1, 2, 3]]}),
    )
    .unwrap();
    let after = text(&d);
    assert!(after.starts_with("---\nsuite_kind: sheet\n---\n# Stock\n\nIntro text.\n\n"));
    assert!(after.contains("\n\nAfter text.\n\n```rust\nfn keep() {}\n```\n"));
    assert!(
        after.ends_with("```sheet name=Other\n|  a |   b |\n|----|-----|\n|  1 |   2 |\n```\n")
    );
}
