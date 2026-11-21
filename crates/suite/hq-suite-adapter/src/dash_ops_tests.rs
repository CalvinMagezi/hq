use super::*;
use crate::edit::{Gate, run};
use crate::sheets;

const NOTE: &str = "---\nsuite_kind: sheet\n---\n# Stock\n\n```sheet name=\"Stock\" types=\"text,text,int,ugx\"\n| item  | region | qty | price |\n|-------|--------|-----|-------|\n| Sugar | North  | 40  | 4200  |\n| Salt  | South  | 25  | 1800  |\n| Rice  | North  | 10  | 3000  |\n```\n\nNotes.\n";

fn vault(note: &str) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(d.path().join("Notebooks")).unwrap();
    std::fs::write(d.path().join("Notebooks/dash.md"), note).unwrap();
    d
}

fn text(d: &tempfile::TempDir) -> String {
    std::fs::read_to_string(d.path().join("Notebooks/dash.md")).unwrap()
}

fn go(d: &tempfile::TempDir, mut args: Value) -> Result<Value> {
    args["note"] = json!("Notebooks/dash.md");
    run(d.path(), &args, Gate::Owner)
}

fn err(d: &tempfile::TempDir, args: Value) -> String {
    let before = text(d);
    let e = go(d, args).unwrap_err().to_string();
    assert_eq!(
        text(d),
        before,
        "a block that would not draw is never written"
    );
    e
}

#[test]
fn a_tile_is_built_from_arguments_evaluated_and_added_at_the_end() {
    let d = vault(NOTE);
    let out = go(
        &d,
        json!({"action": "add_tile", "from": "C2:C4", "fn": "sum", "label": "Units", "format": "int"}),
    )
    .unwrap();
    assert_eq!(out["tile"]["text"], "75");
    assert_eq!(out["tile"]["label"], "Units");
    assert_eq!(
        text(&d),
        format!("{NOTE}\n```tile from=\"C2:C4\" fn=sum label=Units format=int\n```\n")
    );
    // The written block reads back as the tile that was evaluated.
    let v = sheets::tile(d.path(), "Notebooks/dash.md", 0).unwrap();
    assert_eq!(v.text, "75");
}

#[test]
fn a_tile_with_a_where_reads_the_last_column_of_the_matching_rows() {
    let d = vault(NOTE);
    let out = go(
        &d,
        json!({"action": "add_tile", "from": "A1:D4", "fn": "sum", "where": "region=North", "label": "North value", "format": "ugx"}),
    )
    .unwrap();
    assert_eq!(out["tile"]["text"], "UGX 7,200");
    assert!(text(&d).contains("where=\"region=North\""));
}

#[test]
fn a_tile_that_would_not_draw_is_refused_with_the_evaluators_own_sentence() {
    let d = vault(NOTE);
    assert!(err(&d, json!({"action": "add_tile", "from": "A2:A4"})).contains("no numbers"));
    assert!(
        err(
            &d,
            json!({"action": "add_tile", "from": "A1:D4", "where": "colour=red"})
        )
        .contains("no column called `colour`")
    );
    let e = err(
        &d,
        json!({"action": "add_tile", "from": "C2:C4", "fn": "total"}),
    );
    assert!(
        e.contains("not one of sum, count, avg, min, max, median"),
        "{e}"
    );
    assert!(
        err(
            &d,
            json!({"action": "add_tile", "from": "C2:C4", "group": "region"})
        )
        .contains("one number")
    );
    assert!(err(&d, json!({"action": "add_tile", "from": "not a range"})).contains("from"));
    assert!(
        err(&d, json!({"action": "add_tile", "from": "[[other]]!C2:C4"}))
            .contains("no note called")
    );
    assert!(err(&d, json!({"action": "add_tile"})).contains("`from`"));
    assert!(
        err(
            &d,
            json!({"action": "add_tile", "from": "C2:C4", "label": "a\nb"})
        )
        .contains("one line")
    );
}

#[test]
fn a_plot_is_built_evaluated_and_placed_after_a_block_when_asked() {
    let d = vault(NOTE);
    let doc = Document::parse(&text(&d));
    let head = doc.node(0).unwrap().hash();
    let out = go(
        &d,
        json!({"action": "add_plot", "type": "bar", "from": "A1:D4", "x": "item", "y": "qty", "label": "Stock by item",
               "after_index": 0, "hash": head}),
    )
    .unwrap();
    assert_eq!(out["chart"]["labels"], 3);
    assert_eq!(out["block_index"], 1);
    assert_eq!(
        text(&d),
        NOTE.replace(
            "# Stock\n\n",
            "# Stock\n\n```plot type=bar from=\"A1:D4\" x=item y=qty title=\"Stock by item\"\n```\n\n"
        )
    );
    let chart = sheets::plot(d.path(), "Notebooks/dash.md", 0).unwrap();
    assert!(
        chart.contains("\"labels\":[\"Sugar\",\"Salt\",\"Rice\"]"),
        "{chart}"
    );
}

#[test]
fn a_plot_with_a_group_draws_one_series_per_value() {
    let d = vault(NOTE);
    let out = go(
        &d,
        json!({"action": "add_plot", "type": "line", "from": "A1:D4", "x": "item", "y": "qty", "group": "region"}),
    )
    .unwrap();
    assert_eq!(out["chart"]["series"], 2);
}

#[test]
fn a_plot_that_would_not_draw_is_refused_and_nothing_is_written() {
    let d = vault(NOTE);
    let base = |extra: Value| {
        let mut a =
            json!({"action": "add_plot", "type": "bar", "from": "A1:D4", "x": "item", "y": "qty"});
        a.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        a
    };
    let e = err(&d, base(json!({"type": "radar"})));
    assert!(
        e.contains("not one of bar, line, pie, donut, area, stacked, scatter"),
        "{e}"
    );
    assert!(err(&d, base(json!({"y": "region"}))).contains("not a number"));
    assert!(err(&d, base(json!({"y": "weight"}))).contains("no column called `weight`"));
    assert!(err(&d, base(json!({"type": "pie", "group": "region"}))).contains("no group"));
    assert!(err(&d, base(json!({"from": "A1:D1"}))).contains("no rows"));
    assert!(
        err(&d, base(json!({"after_index": 0, "hash": "stale"})))
            .contains("changed since you read")
    );
    assert!(err(&d, json!({"action": "add_plot", "type": "bar"})).contains("`from`"));
}
