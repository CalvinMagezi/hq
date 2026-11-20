use super::*;

fn big_sheet(rows: usize) -> String {
    let mut s = String::from("# Log\n\nIntro.\n\n```sheet name=Log\n| n | v |\n|---|---|\n");
    for i in 0..rows {
        s.push_str(&format!("| {i} | x{i} |\n"));
    }
    s.push_str("```\n\nEnd.\n");
    s
}

fn vault(note: &str) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(d.path().join("Notebooks")).unwrap();
    std::fs::write(d.path().join("Notebooks/n.md"), note).unwrap();
    d
}

fn text(d: &tempfile::TempDir) -> String {
    std::fs::read_to_string(d.path().join("Notebooks/n.md")).unwrap()
}

fn agent(d: &tempfile::TempDir, mut args: Value) -> Result<Value> {
    args["note"] = json!("Notebooks/n.md");
    run(d.path(), &args, Gate::Agent)
}

fn sheet_hash(d: &tempfile::TempDir) -> String {
    let doc = Document::parse(&text(d));
    doc.nodes()
        .iter()
        .find(|n| n.src().starts_with("```sheet"))
        .unwrap()
        .hash()
}

fn grid(rows: usize, cols: usize, f: impl Fn(usize, usize) -> String) -> Value {
    json!(
        (0..rows)
            .map(|r| (0..cols).map(|c| f(r, c)).collect::<Vec<_>>())
            .collect::<Vec<_>>()
    )
}

#[test]
fn a_small_cell_edit_is_one_step_and_a_large_one_is_previewed_first() {
    let d = vault(&big_sheet(60));
    let small = agent(
        &d,
        json!({"action": "set_cells", "row": 1, "col": 1, "values": grid(50, 1, |r, _| format!("s{r}")),
               "expected": grid(50, 1, |r, _| format!("x{r}"))}),
    )
    .unwrap();
    assert_eq!(
        small["changed"], true,
        "50 cells is within the one-step limit: {small}"
    );
    assert!(text(&d).contains("| 0 | s0 |"));

    let before = text(&d);
    let big = json!({"action": "set_cells", "row": 1, "col": 1, "values": grid(51, 1, |r, _| format!("b{r}")),
        "expected": grid(51, 1, |r, _| if r < 50 { format!("s{r}") } else { format!("x{r}") })});
    let preview = agent(&d, big.clone()).unwrap();
    assert_eq!(preview["preview"], true);
    assert_eq!(preview["applied"], false);
    assert_eq!(preview["touched"], 51);
    assert!(preview["summary"].as_str().unwrap().contains("51 cells"));
    assert!(preview["token"].as_str().unwrap().len() >= 16);
    assert_eq!(text(&d), before, "a preview writes nothing");

    // The same request with the token applies it.
    let mut again = big.clone();
    again["token"] = preview["token"].clone();
    let done = agent(&d, again).unwrap();
    assert_eq!(done["changed"], true);
    assert!(text(&d).contains("| 0 | b0 |") && text(&d).contains("| 50 | b50 |"));
}

#[test]
fn a_token_fits_only_the_note_and_the_request_it_was_made_for() {
    let d = vault(&big_sheet(60));
    let hash = sheet_hash(&d);
    let call = json!({"action": "delete_rows", "hash": hash, "row": 1, "count": 2});
    let preview = agent(&d, call.clone()).unwrap();
    assert_eq!(preview["destructive"], true);
    let token = preview["token"].clone();

    // Another request with this token is refused.
    let mut other = json!({"action": "delete_rows", "hash": hash, "row": 5, "count": 2});
    other["token"] = token.clone();
    assert!(
        agent(&d, other)
            .unwrap_err()
            .to_string()
            .contains("token does not fit")
    );

    // No token: only a preview, nothing deleted. A wrong token is refused.
    let mut wrong = call.clone();
    wrong["token"] = json!("0000000000000000");
    let before = text(&d);
    assert!(agent(&d, wrong).is_err());
    assert_eq!(text(&d), before);

    // The note moves on (the owner edits a cell elsewhere): the old token is dead.
    std::fs::write(
        d.path().join("Notebooks/n.md"),
        before.replace("Intro.", "Intro, edited."),
    )
    .unwrap();
    let mut late = json!({"action": "delete_rows", "hash": sheet_hash(&d), "row": 1, "count": 2});
    late["token"] = token.clone();
    assert!(
        agent(&d, late)
            .unwrap_err()
            .to_string()
            .contains("token does not fit")
    );

    // A fresh preview and its token apply.
    let fresh = agent(
        &d,
        json!({"action": "delete_rows", "hash": sheet_hash(&d), "row": 1, "count": 2}),
    )
    .unwrap();
    let mut apply = json!({"action": "delete_rows", "hash": sheet_hash(&d), "row": 1, "count": 2});
    apply["token"] = fresh["token"].clone();
    assert_eq!(agent(&d, apply).unwrap()["changed"], true);
    assert!(
        !text(&d).contains("| 0 | x0 |")
            && !text(&d).contains("| 1 | x1 |")
            && text(&d).contains("| 2 | x2 |")
    );
}

#[test]
fn anything_that_deletes_is_previewed_and_single_edits_stay_one_step() {
    let d = vault(&big_sheet(5));
    let before = text(&d);
    let doc = Document::parse(&before);
    let intro = doc
        .nodes()
        .iter()
        .position(|n| n.src() == "Intro.")
        .unwrap();
    let h = doc.node(intro).unwrap().hash();
    let p = agent(
        &d,
        json!({"action": "delete_block", "index": intro, "hash": h}),
    )
    .unwrap();
    assert_eq!(p["preview"], true);
    assert_eq!(text(&d), before);
    // One block replaced, and one cell set, are one step.
    assert_eq!(
        agent(
            &d,
            json!({"action": "replace_block", "index": intro, "hash": h, "markdown": "Hello."})
        )
        .unwrap()["changed"],
        true
    );
    let e = agent(
        &d,
        json!({"action": "set_cell", "row": 1, "col": 1, "expected": "x0", "text": "y0"}),
    )
    .unwrap();
    assert_eq!(e["changed"], true);
    assert!(text(&d).contains("| 0 | y0 |") && text(&d).contains("Hello."));
    // Adding more than 50 cells of rows is previewed too.
    let hash = sheet_hash(&d);
    let p = agent(
        &d,
        json!({"action": "add_rows", "hash": hash, "rows": grid(26, 2, |r, c| format!("{r}{c}"))}),
    )
    .unwrap();
    assert_eq!(p["preview"], true);
    assert_eq!(p["touched"], 52);
}

#[test]
fn the_owner_edits_in_one_step_and_can_ask_for_a_dry_run() {
    let d = vault(&big_sheet(5));
    let hash = sheet_hash(&d);
    let call = json!({"action": "delete_rows", "note": "Notebooks/n.md", "hash": hash, "row": 1});
    let mut dry = call.clone();
    dry["dry_run"] = json!(true);
    let before = text(&d);
    let p = run(d.path(), &dry, Gate::Owner).unwrap();
    assert_eq!(p["preview"], true);
    assert!(p.get("token").is_none(), "the owner needs no token");
    assert_eq!(text(&d), before);
    assert_eq!(run(d.path(), &call, Gate::Owner).unwrap()["changed"], true);
    assert!(!text(&d).contains("| 0 | x0 |"));
}

#[test]
fn a_deck_delete_and_a_replace_are_previewed_and_the_result_names_the_new_revision() {
    let deck = "# T\n\n## A\n\nOne.\n\n## B\n\nTwo.\n";
    let d = vault(deck);
    let cards = agent(&d, json!({"action": "cards"})).unwrap();
    let hash = cards["cards"][1]["hash"].clone();
    let p = agent(
        &d,
        json!({"action": "delete_card", "card": 1, "hash": hash}),
    )
    .unwrap();
    assert_eq!(p["preview"], true);
    let mut go = json!({"action": "delete_card", "card": 1, "hash": hash});
    go["token"] = p["token"].clone();
    let out = agent(&d, go).unwrap();
    assert_eq!(text(&d), "# T\n\n## A\n\nOne.\n");
    assert_eq!(
        out["revision"].as_str().unwrap(),
        note_revision(d.path(), "Notebooks/n.md").unwrap()
    );
}

#[test]
fn a_focus_is_checked_against_the_note_and_echoed_on_every_reply() {
    let d = vault(&big_sheet(3));
    let read = agent(&d, json!({"action": "blocks", "focus": {"note": "Notebooks/n.md", "block": 2, "row": 1, "col": 0}})).unwrap();
    assert_eq!(read["focus"]["block"], 2);
    assert_eq!(read["focus"]["row"], 1);
    assert!(read["focus"]["block_hash"].is_string());
    let hash = sheet_hash(&d);
    let write = agent(
        &d,
        json!({"action": "add_rows", "hash": hash, "rows": [["9", "z"]], "focus": {"note": "Notebooks/n.md", "block": 2}}),
    )
    .unwrap();
    assert_eq!(write["focus"]["note"], "Notebooks/n.md");
    // A focus that names a note or block that is not there is refused before anything is written.
    let before = text(&d);
    let hash = sheet_hash(&d);
    for focus in [
        json!({"note": "Notebooks/gone.md"}),
        json!({"note": "Notebooks/n.md", "block": 99}),
        json!({"block": 1}),
    ] {
        let r = agent(
            &d,
            json!({"action": "add_rows", "hash": hash, "rows": [["9", "z"]], "focus": focus}),
        );
        assert!(r.is_err());
        assert_eq!(text(&d), before);
    }
    // The focus is a pointer, not a target: a focus on one note never moves an edit to it.
    let other = d.path().join("Notebooks/other.md");
    std::fs::write(&other, "# Other\n\nText.\n").unwrap();
    let hash = sheet_hash(&d);
    let r = agent(
        &d,
        json!({"action": "add_rows", "hash": hash, "rows": [["8", "y"]], "focus": {"note": "Notebooks/other.md"}}),
    )
    .unwrap();
    assert_eq!(r["changed"], true);
    assert!(text(&d).contains("| 8 | y |"));
    assert_eq!(
        std::fs::read_to_string(other).unwrap(),
        "# Other\n\nText.\n"
    );
}

#[test]
fn an_unknown_action_lists_the_known_ones_and_blocks_and_cards_never_write() {
    let d = vault("# T\n\n## A\n\nOne.\n");
    let e = agent(&d, json!({"action": "explode"}))
        .unwrap_err()
        .to_string();
    assert!(
        e.contains("unknown action") && e.contains("add_rows") && e.contains("set_card_style"),
        "{e}"
    );
    assert!(!is_write(&json!({"action": "blocks"})));
    assert!(!is_write(&json!({"action": "cards"})));
    assert!(is_write(&json!({"action": "add_card"})));
    let e = agent(&d, json!({"action": "delete_block"}))
        .unwrap_err()
        .to_string();
    assert!(e.contains("index"));
}

#[test]
fn an_agent_comments_on_a_block_and_resolves_it_and_a_stale_anchor_writes_nothing() {
    let d = vault("# Plan\n\nThe revenue figure is final.\n");
    let doc = Document::parse(&text(&d));
    let para = doc
        .nodes()
        .iter()
        .find(|n| n.src().starts_with("The revenue"))
        .unwrap()
        .hash();
    let added = agent(
        &d,
        json!({"action": "add_comment", "on": para, "text": "Is this audited?", "at": "2026-11-20"}),
    )
    .unwrap();
    assert_eq!(added["applied"], true);
    assert!(text(&d).contains("Is this audited?") && text(&d).contains("by=HQ"));
    let listed = agent(&d, json!({"action": "comments"})).unwrap();
    let row = &listed["comments"][0];
    assert_eq!(
        (row["state"].as_str(), row["by"].as_str()),
        (Some("open"), Some("HQ"))
    );
    let before = text(&d);
    agent(
        &d,
        json!({"action": "resolve_comment", "index": row["index"], "hash": row["hash"]}),
    )
    .unwrap();
    assert!(text(&d).contains("state=resolved"));
    // An anchor hash that no block has any more is refused and nothing is written.
    let before_stale = text(&d);
    assert!(
        agent(
            &d,
            json!({"action": "add_comment", "on": "0123456789ab", "text": "x", "at": "2026-11-20"})
        )
        .is_err()
    );
    assert_eq!(text(&d), before_stale);
    assert_ne!(before, before_stale);
}
