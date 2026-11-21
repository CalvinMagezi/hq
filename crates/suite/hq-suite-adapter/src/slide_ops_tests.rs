use super::*;
use crate::edit::{Gate, run};

const DECK: &str = "---\nsuite_kind: slides\n---\n# Board review\n\nPrepared for the board.\n\n## Results\n<!-- transition: fade -->\n\n- Revenue **up** 12%\n  - Uganda strongest\n\n## Plan\n<!-- layout: two-column -->\n<!-- notes: Say this first. -->\n\n1. Hire\n2. Train\n\n---\n\n| item | qty |\n|---|---|\n| Sugar | 40 |\n\n```text\n## not a card\n```\n\n## Close\n\nThanks.\n";

fn vault(note: &str) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(d.path().join("Notebooks")).unwrap();
    std::fs::write(d.path().join("Notebooks/d.md"), note).unwrap();
    d
}

fn text(d: &tempfile::TempDir) -> String {
    std::fs::read_to_string(d.path().join("Notebooks/d.md")).unwrap()
}

fn go(d: &tempfile::TempDir, mut args: Value) -> Result<Value> {
    args["note"] = json!("Notebooks/d.md");
    run(d.path(), &args, Gate::Owner)
}

fn err(d: &tempfile::TempDir, args: Value) -> String {
    let before = text(d);
    let e = go(d, args).unwrap_err().to_string();
    assert_eq!(text(d), before, "a refused edit writes nothing");
    e
}

fn cards(d: &tempfile::TempDir) -> Vec<Value> {
    go(d, json!({"action": "cards"})).unwrap()["cards"]
        .as_array()
        .unwrap()
        .clone()
}

fn hash(d: &tempfile::TempDir, i: usize) -> String {
    cards(d)[i]["hash"].as_str().unwrap().to_string()
}

#[test]
fn the_cards_are_listed_with_their_hash_look_and_notes() {
    let d = vault(DECK);
    let c = cards(&d);
    let titles: Vec<&str> = c.iter().map(|c| c["title"].as_str().unwrap()).collect();
    assert_eq!(
        titles,
        ["Results", "Plan", "Close"],
        "a heading inside a code fence is not a card"
    );
    assert_eq!(c[0]["transition"], "fade");
    assert_eq!(c[1]["layout"], "two-column");
    assert_eq!(c[1]["notes"], json!(["Say this first."]));
    assert_eq!(c[2]["transition"], Value::Null);
    assert_ne!(c[0]["hash"], c[1]["hash"]);
}

#[test]
fn a_card_is_added_at_the_end_or_after_another_and_the_rest_is_untouched() {
    let d = vault(DECK);
    let out = go(
        &d,
        json!({"action": "add_card", "title": "Risks", "markdown": "- Currency\n- Supply"}),
    )
    .unwrap();
    assert_eq!(out["card"]["index"], 3);
    assert_eq!(
        text(&d),
        format!("{DECK}\n## Risks\n\n- Currency\n- Supply\n")
    );
    // After the first card, quoting its hash.
    let d = vault(DECK);
    let h = hash(&d, 0);
    go(
        &d,
        json!({"action": "add_card", "title": "Agenda", "after_card": 0, "hash": h}),
    )
    .unwrap();
    let t = text(&d);
    assert!(
        t.contains("  - Uganda strongest\n\n## Agenda\n\n## Plan\n<!-- layout: two-column -->"),
        "{t}"
    );
    let titles: Vec<String> = cards(&d)
        .iter()
        .map(|c| c["title"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(titles, ["Results", "Agenda", "Plan", "Close"]);
}

#[test]
fn add_card_refuses_a_stale_hash_a_bad_title_and_text_that_would_make_two_cards() {
    let d = vault(DECK);
    assert!(
        err(
            &d,
            json!({"action": "add_card", "title": "X", "after_card": 0, "hash": "stale"})
        )
        .contains("changed since you read")
    );
    assert!(err(&d, json!({"action": "add_card", "title": "  "})).contains("one line"));
    assert!(err(&d, json!({"action": "add_card", "title": "a\nb"})).contains("one line"));
    assert!(
        err(
            &d,
            json!({"action": "add_card", "title": "X", "markdown": "text\n\n## Another"})
        )
        .contains("one card per call")
    );
    assert!(
        err(
            &d,
            json!({"action": "add_card", "title": "X", "after_card": 9, "hash": "h"})
        )
        .contains("not one")
    );
}

#[test]
fn a_card_is_replaced_whole_and_the_other_cards_stay_byte_for_byte() {
    let d = vault(DECK);
    let h = hash(&d, 1);
    let out = go(&d, json!({"action": "replace_card", "card": 1, "hash": h, "title": "Roadmap", "markdown": "- Q1\n- Q2"})).unwrap();
    assert_eq!(out["destructive"], Value::Null);
    let want = DECK.replace(
        "## Plan\n<!-- layout: two-column -->\n<!-- notes: Say this first. -->\n\n1. Hire\n2. Train\n\n---\n\n| item | qty |\n|---|---|\n| Sugar | 40 |\n\n```text\n## not a card\n```\n",
        "## Roadmap\n\n- Q1\n- Q2\n",
    );
    assert_eq!(text(&d), want);
    assert!(
        err(
            &d,
            json!({"action": "replace_card", "card": 1, "hash": h, "markdown": "x"})
        )
        .contains("changed since you read")
    );
}

#[test]
fn a_card_is_moved_and_its_blocks_travel_with_it() {
    let d = vault(DECK);
    let h = hash(&d, 2);
    go(
        &d,
        json!({"action": "move_card", "card": 2, "hash": h, "to": 0}),
    )
    .unwrap();
    let titles: Vec<String> = cards(&d)
        .iter()
        .map(|c| c["title"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(titles, ["Close", "Results", "Plan"]);
    let t = text(&d);
    assert!(t.starts_with("---\nsuite_kind: slides\n---\n# Board review\n\nPrepared for the board.\n\n## Close\n\nThanks.\n\n## Results\n<!-- transition: fade -->\n\n"), "{t}");
    assert!(t.contains("```text\n## not a card\n```\n"));
    // The first card moves to the end, and the plan card keeps its comments and table.
    let h = hash(&d, 0);
    go(
        &d,
        json!({"action": "move_card", "card": 0, "hash": h, "to": 2}),
    )
    .unwrap();
    let titles: Vec<String> = cards(&d)
        .iter()
        .map(|c| c["title"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(titles, ["Results", "Plan", "Close"]);
    assert_eq!(
        text(&d),
        DECK,
        "moving a card away and back leaves the deck as it was"
    );
    let h = hash(&d, 0);
    assert!(
        err(
            &d,
            json!({"action": "move_card", "card": 0, "hash": h, "to": 0})
        )
        .contains("already")
    );
    assert!(
        err(
            &d,
            json!({"action": "move_card", "card": 0, "hash": h, "to": 3})
        )
        .contains("`to` must be")
    );
    assert!(
        err(
            &d,
            json!({"action": "move_card", "card": 0, "hash": "stale", "to": 1})
        )
        .contains("changed since you read")
    );
}

#[test]
fn a_card_is_deleted_with_everything_in_it() {
    let d = vault(DECK);
    let h = hash(&d, 1);
    go(&d, json!({"action": "delete_card", "card": 1, "hash": h})).unwrap();
    assert_eq!(
        text(&d),
        DECK.replace(
            "## Plan\n<!-- layout: two-column -->\n<!-- notes: Say this first. -->\n\n1. Hire\n2. Train\n\n---\n\n| item | qty |\n|---|---|\n| Sugar | 40 |\n\n```text\n## not a card\n```\n\n",
            ""
        )
    );
    assert!(
        err(&d, json!({"action": "delete_card", "card": 1, "hash": h}))
            .contains("changed since you read"),
        "the hash is of the old card 1"
    );
}

#[test]
fn speaker_notes_are_replaced_added_and_cleared() {
    let d = vault(DECK);
    // A card that has notes: its comment is replaced in place.
    let h = hash(&d, 1);
    go(
        &d,
        json!({"action": "set_notes", "card": 1, "hash": h, "text": "Open with the plan."}),
    )
    .unwrap();
    assert_eq!(
        text(&d),
        DECK.replace(
            "<!-- notes: Say this first. -->",
            "<!-- notes: Open with the plan. -->"
        )
    );
    // A card with none gets one right under its heading.
    let h = hash(&d, 2);
    go(&d, json!({"action": "set_notes", "card": 2, "hash": h, "text": "Thank them.\n\nTake questions."})).unwrap();
    assert!(
        text(&d)
            .contains("## Close\n\n<!-- notes:\nThank them.\n\nTake questions.\n-->\n\nThanks.\n"),
        "{}",
        text(&d)
    );
    assert_eq!(
        cards(&d)[2]["notes"],
        json!(["Thank them.", "Take questions."])
    );
    // Clearing removes the comment.
    let h = hash(&d, 1);
    go(
        &d,
        json!({"action": "set_notes", "card": 1, "hash": h, "text": ""}),
    )
    .unwrap();
    assert_eq!(cards(&d)[1]["notes"], json!([]));
    assert!(!text(&d).contains("Open with the plan."));
    let h = hash(&d, 1);
    assert!(
        err(
            &d,
            json!({"action": "set_notes", "card": 1, "hash": h, "text": "bad --> text"})
        )
        .contains("cannot contain")
    );
    assert!(
        err(
            &d,
            json!({"action": "set_notes", "card": 1, "hash": "stale", "text": "x"})
        )
        .contains("changed since you read")
    );
}

#[test]
fn the_look_of_a_card_is_set_with_the_writers_own_checks() {
    let d = vault(DECK);
    // An existing transition comment is replaced in place; new ones go under the heading.
    let h = hash(&d, 0);
    let out = go(&d, json!({"action": "set_card_style", "card": 0, "hash": h,
        "transition": "Push  SLOW", "background": "#1f3a5f", "color": "fff", "layout": "two-column"})).unwrap();
    let t = text(&d);
    assert!(t.contains("## Results\n<!-- transition: push slow -->\n\n<!-- background: #1F3A5F -->\n<!-- color: #FFFFFF -->\n<!-- layout: two-column -->\n\n- Revenue"), "{t}");
    assert_eq!(out["card"]["transition"], "push slow");
    assert_eq!(out["card"]["background"], "#1F3A5F");
    assert_eq!(out["card"]["layout"], "two-column");
    // A look that is hard to read still goes in, with a warning.
    let h = hash(&d, 2);
    let out = go(&d, json!({"action": "set_card_style", "card": 2, "hash": h, "background": "#FFFFFF", "color": "#FEFEFE"})).unwrap();
    assert!(
        out["warnings"][0]
            .as_str()
            .unwrap()
            .contains("hard to read"),
        "{out}"
    );
    // Null clears one.
    let h = hash(&d, 1);
    go(
        &d,
        json!({"action": "set_card_style", "card": 1, "hash": h, "layout": null}),
    )
    .unwrap();
    assert!(!text(&d).contains("layout: two-column -->\n<!-- notes"));
}

#[test]
fn a_look_the_writer_could_not_use_is_refused_by_name() {
    let d = vault(DECK);
    let h = hash(&d, 0);
    let call = |extra: Value| {
        let mut a = json!({"action": "set_card_style", "card": 0, "hash": h});
        a.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        a
    };
    assert!(
        err(&d, call(json!({"transition": "spin"})))
            .contains("fade, push, wipe, split, cover, dissolve, none")
    );
    assert!(err(&d, call(json!({"transition": "fade quickly"}))).contains("not a transition"));
    assert!(err(&d, call(json!({"background": "blue"}))).contains("#RRGGBB"));
    assert!(err(&d, call(json!({"color": "#12"}))).contains("not a colour"));
    assert!(err(&d, call(json!({"layout": "carousel"}))).contains("content or two-column"));
    assert!(err(&d, call(json!({}))).contains("at least one"));
    assert!(err(&d, call(json!({"layout": 3}))).contains("text, or null"));
}

#[test]
fn a_note_whose_headings_the_editor_cannot_follow_is_refused() {
    let odd = "# T\n\n<div>\n## inside html\n</div>\n\n## Real\n\ntext\n";
    let d = vault(odd);
    let e = err(&d, json!({"action": "add_card", "title": "X"}));
    assert!(e.contains("cannot follow"), "{e}");
}
