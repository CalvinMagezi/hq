use crate::sheets::{plot, tile};

const STOCK: &str = "---\nsuite_kind: sheet\n---\n```sheet name=Stock types=\"text,text,int,ugx\"\n| item  | region | qty | price |\n|-------|--------|-----|-------|\n| Sugar | North  | 40  | 4200  |\n| Salt  | South  | 25  | 1800  |\n| Rice  | North  | 10  | 3000  |\n```\n\n```sheet name=Points types=\"num,num\"\n| x | y |\n|---|---|\n| 1.5 | 3 |\n| 2 | 5.5 |\n| 4 | 4 |\n```\n";

fn vault(dash: &str) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    for (rel, text) in [("Notebooks/Stock.md", STOCK), ("Notebooks/dash.md", dash)] {
        let p = d.path().join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    d
}

fn err(r: anyhow::Result<impl std::fmt::Debug>) -> String {
    r.unwrap_err().to_string()
}

#[test]
fn a_tile_reads_a_range_of_another_note_and_keeps_where_and_format() {
    let d = vault(
        "```tile from=\"[[Stock]]!D2:D4\" fn=sum format=ugx label=Value\n```\n\n```tile from=\"[[Stock#Stock]]!A1:D4\" fn=sum where=\"region=North\" format=ugx\n```\n\n```tile from=\"[[Notebooks/Stock]]!C2:C4\" fn=count\n```\n",
    );
    let v = tile(d.path(), "Notebooks/dash.md", 0).unwrap();
    assert_eq!((v.label.as_str(), v.cells), ("Value", 3));
    assert!(v.text.contains("9,000"), "{}", v.text);
    let north = tile(d.path(), "Notebooks/dash.md", 1).unwrap();
    assert!(north.text.contains("7,200"), "{}", north.text);
    assert_eq!(tile(d.path(), "Notebooks/dash.md", 2).unwrap().cells, 3);
}

#[test]
fn a_plot_reads_another_note_in_every_kind_and_a_scatter_keeps_its_x() {
    let d = vault(
        "```plot type=area from=\"[[Stock]]!A1:D4\" x=item y=qty\n```\n\n```plot type=stacked from=\"[[Stock]]!A1:D4\" x=item y=qty group=region\n```\n\n```plot type=scatter from=\"[[Stock#Points]]!A1:B4\" x=x y=y title=Spread\n```\n\n```plot type=scatter from=\"[[Stock]]!A1:D4\" x=item y=qty\n```\n\n```plot type=scatter from=\"[[Stock#Points]]!A1:B4\" x=x y=y group=x\n```\n",
    );
    let at = |i| -> serde_json::Value {
        serde_json::from_str(&plot(d.path(), "Notebooks/dash.md", i).unwrap()).unwrap()
    };
    let area = at(0);
    assert_eq!(area["type"], "area");
    assert_eq!(area["labels"], serde_json::json!(["Sugar", "Salt", "Rice"]));
    let stacked = at(1);
    assert_eq!(stacked["series"].as_array().unwrap().len(), 2, "{stacked}");
    let xy = at(2);
    assert_eq!(xy["type"], "scatter");
    assert_eq!(xy["xValues"], serde_json::json!([1.5, 2.0, 4.0]));
    assert_eq!(
        xy["series"][0]["values"],
        serde_json::json!([3.0, 5.5, 4.0])
    );
    assert_eq!(xy["title"], "Spread");
    assert!(err(plot(d.path(), "Notebooks/dash.md", 3)).contains("not a number"));
    assert!(err(plot(d.path(), "Notebooks/dash.md", 4)).contains("no group"));
}

#[test]
fn a_missing_note_a_missing_sheet_and_a_traversal_are_refused_in_plain_words() {
    let d = vault(
        "```tile from=\"[[Nothing]]!A1:A2\"\n```\n\n```tile from=\"[[Stock#Ghost]]!A1:A2\"\n```\n\n```tile from=\"[[../secret]]!A1:A2\"\n```\n\n```tile from=\"[[/etc/passwd]]!A1:A2\"\n```\n\n```tile from=\"[[Notebooks/../../x]]!A1:A2\"\n```\n\n```tile from=\"[[..]]!A1:A2\"\n```\n",
    );
    let say = |i| err(tile(d.path(), "Notebooks/dash.md", i));
    assert!(say(0).contains("no note called `Nothing`"), "{}", say(0));
    assert!(say(1).contains("no sheet called `Ghost`"), "{}", say(1));
    for i in 2..=5 {
        let s = say(i);
        assert!(
            s.contains("is not a note in this vault")
                || s.contains("not a note name")
                || s.contains("not a note reference"),
            "{i}: {s}"
        );
    }
}

#[test]
fn a_name_two_notes_share_is_refused_until_the_folder_is_written() {
    let d = vault(
        "```tile from=\"[[Stock]]!C2:C4\"\n```\n\n```tile from=\"[[Archive/Stock]]!C2:C4\"\n```\n",
    );
    std::fs::create_dir_all(d.path().join("Archive")).unwrap();
    std::fs::write(d.path().join("Archive/Stock.md"), STOCK).unwrap();
    let e = err(tile(d.path(), "Notebooks/dash.md", 0));
    assert!(
        e.contains("2 notes are called `Stock`") && e.contains("Archive/Stock.md"),
        "{e}"
    );
    assert!(tile(d.path(), "Notebooks/dash.md", 1).is_ok());
}

#[test]
fn the_vaults_own_folders_and_links_are_not_searched() {
    let d = vault("```tile from=\"[[Hidden]]!C2:C4\"\n```\n");
    for dir in ["_data", ".git", "_trash"] {
        std::fs::create_dir_all(d.path().join(dir)).unwrap();
        std::fs::write(d.path().join(dir).join("Hidden.md"), STOCK).unwrap();
    }
    assert!(err(tile(d.path(), "Notebooks/dash.md", 0)).contains("no note called"));
    #[cfg(unix)]
    {
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("Hidden.md"), STOCK).unwrap();
        std::os::unix::fs::symlink(outside.path(), d.path().join("Linked")).unwrap();
        assert!(err(tile(d.path(), "Notebooks/dash.md", 0)).contains("no note called"));
        let e = err(crate::sheets::tile(d.path(), "Notebooks/dash.md", 0));
        assert!(!e.contains(outside.path().to_str().unwrap()));
    }
}

#[test]
fn the_other_notes_own_tiles_are_never_followed_and_a_big_note_is_refused() {
    let d = vault("```tile from=\"[[Chain]]!A1:A2\"\n```\n\n```tile from=\"[[Big]]!A1:A2\"\n```\n");
    // `Chain` holds only a tile that points elsewhere: it has no sheet to read.
    std::fs::write(
        d.path().join("Notebooks/Chain.md"),
        "```tile from=\"[[Stock]]!C2:C4\"\n```\n",
    )
    .unwrap();
    assert!(err(tile(d.path(), "Notebooks/dash.md", 0)).contains("has no sheet"));
    std::fs::write(
        d.path().join("Notebooks/Big.md"),
        "x".repeat(3 * 1024 * 1024),
    )
    .unwrap();
    assert!(err(tile(d.path(), "Notebooks/dash.md", 1)).contains("too big"));
}

#[test]
fn a_dashboard_tile_added_by_the_tool_reads_another_note_too() {
    let d = vault("# Dash\n");
    let out = crate::edit::run(
        d.path(),
        &serde_json::json!({"action": "add_tile", "note": "Notebooks/dash.md", "from": "[[Stock]]!C2:C4", "fn": "sum", "label": "Units"}),
        crate::edit::Gate::Owner,
    )
    .unwrap();
    assert!(out.to_string().contains("75"), "{out}");
    let text = std::fs::read_to_string(d.path().join("Notebooks/dash.md")).unwrap();
    assert!(text.contains("[[Stock]]!C2:C4"));
}
