use super::*;
use std::sync::{Arc, Mutex};

fn note(vault: &std::path::Path) {
    let dir = vault.join("Notebooks");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("plan.md"),
        "---\nsuite_kind: doc\n---\n# Plan\n\nFirst.\n\nSecond.\n",
    )
    .unwrap();
}

fn tool(vault: &std::path::Path) -> OfficeEditTool {
    OfficeEditTool {
        vault: vault.to_path_buf(),
        seam: None,
    }
}

/// A seam that only counts the writes it is asked to wrap.
#[derive(Default)]
struct CountingSeam(Mutex<Vec<String>>);

impl hq_tool_api::office::NoteWriteSeam for CountingSeam {
    fn write(
        &self,
        rel: &str,
        _args: &Value,
        write: &mut dyn FnMut() -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        self.0.lock().unwrap().push(rel.to_string());
        write()
    }

    fn created(&self, _rel: &str, _args: &Value) {}
}

#[tokio::test]
async fn a_block_is_read_then_replaced_and_a_stale_hash_writes_nothing() {
    let d = tempfile::tempdir().unwrap();
    note(d.path());
    let tool = tool(d.path());
    let read = tool
        .execute(json!({"action": "blocks", "note": "Notebooks/plan.md"}))
        .await
        .unwrap();
    let first = read["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["src"].as_str().unwrap().contains("First"))
        .unwrap()
        .clone();
    let edit = |hash: String, md: &str| {
        json!({"action": "replace_block", "note": "Notebooks/plan.md",
               "index": first["index"], "hash": hash, "markdown": md})
    };
    tool.execute(edit(first["hash"].as_str().unwrap().into(), "Changed."))
        .await
        .unwrap();
    let after = std::fs::read_to_string(d.path().join("Notebooks/plan.md")).unwrap();
    assert!(after.contains("Changed.") && after.contains("Second."));
    // The old hash no longer matches: refused, file unchanged.
    let stale = tool
        .execute(edit(first["hash"].as_str().unwrap().into(), "Again."))
        .await;
    assert!(stale.is_err());
    assert_eq!(
        std::fs::read_to_string(d.path().join("Notebooks/plan.md")).unwrap(),
        after
    );
}

#[tokio::test]
async fn an_unknown_action_or_a_missing_argument_says_what_is_wrong() {
    let d = tempfile::tempdir().unwrap();
    note(d.path());
    let tool = tool(d.path());
    let e = tool
        .execute(json!({"action": "explode", "note": "Notebooks/plan.md"}))
        .await
        .unwrap_err();
    assert!(e.to_string().contains("unknown action"));
    let e = tool
        .execute(json!({"action": "delete_block", "note": "Notebooks/plan.md"}))
        .await
        .unwrap_err();
    assert!(e.to_string().contains("index"));
}

#[tokio::test]
async fn a_delete_is_previewed_through_the_tool_and_only_the_applied_call_takes_the_seam() {
    let d = tempfile::tempdir().unwrap();
    note(d.path());
    let seam = Arc::new(CountingSeam::default());
    let tool = OfficeEditTool {
        vault: d.path().to_path_buf(),
        seam: Some(seam.clone()),
    };
    let read = tool
        .execute(json!({"action": "blocks", "note": "Notebooks/plan.md"}))
        .await
        .unwrap();
    let second = read["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["src"].as_str().unwrap().contains("Second"))
        .unwrap()
        .clone();
    let call = json!({"action": "delete_block", "note": "Notebooks/plan.md",
        "index": second["index"], "hash": second["hash"],
        "focus": {"note": "Notebooks/plan.md", "block": second["index"]}});
    let before = std::fs::read_to_string(d.path().join("Notebooks/plan.md")).unwrap();
    let preview = tool.execute(call.clone()).await.unwrap();
    assert_eq!(preview["preview"], true);
    assert_eq!(
        preview["focus"]["block"], second["index"],
        "the focus is echoed"
    );
    assert_eq!(
        std::fs::read_to_string(d.path().join("Notebooks/plan.md")).unwrap(),
        before
    );
    assert!(
        seam.0.lock().unwrap().is_empty(),
        "a preview is a read: no note lock, no record"
    );

    let mut apply = call;
    apply["token"] = preview["token"].clone();
    // Identity keys the runtime adds are not part of the request the token fits.
    apply["_hq_native_session"] = json!("s1");
    let done = tool.execute(apply).await.unwrap();
    assert_eq!(done["changed"], true);
    assert_eq!(
        *seam.0.lock().unwrap(),
        vec!["Notebooks/plan.md".to_string()]
    );
    let after = std::fs::read_to_string(d.path().join("Notebooks/plan.md")).unwrap();
    assert!(!after.contains("Second.") && after.contains("First."));
}

#[tokio::test]
async fn a_written_note_is_announced_with_its_new_revision() {
    let d = tempfile::tempdir().unwrap();
    note(d.path());
    let heard: Arc<Mutex<Vec<(String, String)>>> = Arc::default();
    let sink = heard.clone();
    hq_tool_api::office::set_note_listener(Some(Box::new(move |n, r| {
        sink.lock().unwrap().push((n.to_string(), r.to_string()));
    })));
    let tool = tool(d.path());
    let read = tool
        .execute(json!({"action": "blocks", "note": "Notebooks/plan.md"}))
        .await
        .unwrap();
    let first = read["blocks"][1].clone();
    let out = tool
        .execute(
            json!({"action": "replace_block", "note": "Notebooks/plan.md",
            "index": first["index"], "hash": first["hash"], "markdown": "Edited."}),
        )
        .await
        .unwrap();
    hq_tool_api::office::set_note_listener(None);
    let heard = heard.lock().unwrap();
    assert!(
        heard.contains(&(
            "Notebooks/plan.md".to_string(),
            out["revision"].as_str().unwrap().to_string()
        )),
        "{heard:?}"
    );
}

#[test]
fn the_schema_names_every_action_and_every_argument_the_actions_read() {
    let d = tempfile::tempdir().unwrap();
    let t = tool(d.path());
    let schema = t.parameters();
    let actions: Vec<&str> = schema["properties"]["action"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a.as_str().unwrap())
        .collect();
    for a in hq_suite_adapter::edit::ACTIONS {
        assert!(actions.contains(a), "{a} is in the schema");
        assert!(t.description().contains(a), "{a} is in the description");
    }
    for key in [
        "values",
        "expected",
        "rows",
        "at",
        "count",
        "columns",
        "types",
        "default",
        "card",
        "after_card",
        "to",
        "transition",
        "background",
        "color",
        "layout",
        "from",
        "fn",
        "where",
        "group",
        "x",
        "y",
        "format",
        "token",
        "focus",
        "title",
        "text",
        "name",
        "type",
    ] {
        assert!(
            schema["properties"][key].is_object(),
            "{key} is in the schema"
        );
    }
}
