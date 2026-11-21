use super::*;
use axum::http::StatusCode;

fn vault() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("Notebooks/Inbox")).unwrap();
    dir
}

#[tokio::test]
async fn web_note_writes_are_searchable() {
    let v = vault();
    let state = Arc::new(WsState::new(v.path().to_path_buf(), None));
    let body = json!({"folder": "Notebooks/Inbox", "title": "Plan", "content": "zebra"});
    assert_eq!(
        note_create_handler(State(state.clone()), Json(body))
            .await
            .status(),
        StatusCode::OK
    );
    let edit = json!({"path": "Notebooks/Inbox/Plan.md", "content": "quokka"});
    assert_eq!(
        note_update_handler(State(state.clone()), Json(edit))
            .await
            .status(),
        StatusCode::OK
    );
    let hits = |q: &'static str| {
        state
            .db
            .with_conn(move |c| hq_db::search::keyword_search(c, q, 5))
            .unwrap()
    };
    assert_eq!(hits("quokka")[0].note_path, "Notebooks/Inbox/Plan.md");
    assert!(hits("zebra").is_empty());
}

#[tokio::test]
async fn the_owner_reads_a_notes_versions_and_reviews_the_current_one() {
    let v = vault();
    let state = Arc::new(WsState::new(v.path().to_path_buf(), None));
    let body = json!({"folder": "Notebooks/Inbox", "title": "Plan", "content": "zebra"});
    note_create_handler(State(state.clone()), Json(body)).await;
    let path = "Notebooks/Inbox/Plan.md";
    let q = std::collections::HashMap::from([("path".to_string(), path.to_string())]);
    let res = note_versions_handler(State(state.clone()), axum::extract::Query(q)).await;
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let listed: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(listed.to_string().contains("content_hash"), "{listed}");

    let no_reason = json!({"path": path, "verdict": "disputed"});
    let res = note_review_handler(State(state.clone()), Json(no_reason)).await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let odd = json!({"path": path, "verdict": "superseded", "reason": "x"});
    let res = note_review_handler(State(state.clone()), Json(odd)).await;
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    let wrong = json!({"path": path, "verdict": "disputed", "reason": "the date is wrong"});
    let res = note_review_handler(State(state.clone()), Json(wrong)).await;
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let reviewed: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(reviewed["review_state"], "disputed");
}

#[tokio::test]
async fn a_note_moves_with_its_links_and_a_taken_path_is_refused() {
    let v = vault();
    let state = Arc::new(WsState::new(v.path().to_path_buf(), None));
    let root = v.path().join("Notebooks/Inbox");
    std::fs::write(root.join("Idea.md"), "# Idea\n").unwrap();
    std::fs::write(root.join("Other.md"), "See [[Idea]] later.\n").unwrap();
    let ask = |from: &str, to: &str, dry: bool| json!({ "from": from, "to": to, "dry_run": dry });

    let dry = note_move_handler(
        State(state.clone()),
        Json(ask(
            "Notebooks/Inbox/Idea.md",
            "Notebooks/Projects/Plan.md",
            true,
        )),
    )
    .await;
    assert_eq!(dry.status(), StatusCode::OK);
    assert!(root.join("Idea.md").exists(), "a dry run moves nothing");

    let moved = note_move_handler(
        State(state.clone()),
        Json(ask(
            "Notebooks/Inbox/Idea.md",
            "Notebooks/Projects/Plan.md",
            false,
        )),
    )
    .await;
    assert_eq!(moved.status(), StatusCode::OK);
    assert!(!root.join("Idea.md").exists());
    assert!(v.path().join("Notebooks/Projects/Plan.md").exists());
    let other = std::fs::read_to_string(root.join("Other.md")).unwrap();
    assert!(
        other.contains("[[Plan]]"),
        "the link follows the note: {other}"
    );

    // Moving it back undoes it; a path that is taken is refused.
    let taken = note_move_handler(
        State(state.clone()),
        Json(ask(
            "Notebooks/Projects/Plan.md",
            "Notebooks/Inbox/Other.md",
            false,
        )),
    )
    .await;
    assert_eq!(taken.status(), StatusCode::BAD_REQUEST);
    let empty = note_move_handler(State(state.clone()), Json(json!({ "from": "x" }))).await;
    assert_eq!(empty.status(), StatusCode::BAD_REQUEST);
    let back = note_move_handler(
        State(state),
        Json(ask(
            "Notebooks/Projects/Plan.md",
            "Notebooks/Inbox/Idea.md",
            false,
        )),
    )
    .await;
    assert_eq!(back.status(), StatusCode::OK);
    assert!(root.join("Idea.md").exists());
}

#[tokio::test]
async fn a_folder_moves_with_its_notes_and_never_into_itself() {
    let v = vault();
    let state = Arc::new(WsState::new(v.path().to_path_buf(), None));
    let nb = v.path().join("Notebooks");
    std::fs::create_dir_all(nb.join("Inbox/Old/Deep")).unwrap();
    std::fs::write(nb.join("Inbox/Old/A.md"), "# A\n").unwrap();
    std::fs::write(nb.join("Inbox/Old/Deep/B.md"), "See [[A]].\n").unwrap();
    let ask = |from: &str, to: &str, dry: bool| json!({ "from": from, "to": to, "dry_run": dry });
    let call = |b: Value| {
        let state = state.clone();
        async move { folder_move_handler(State(state), Json(b)).await }
    };

    let dry = call(ask("Notebooks/Inbox/Old", "Notebooks/Projects/Old", true)).await;
    assert_eq!(dry.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(dry.into_body(), usize::MAX)
        .await
        .unwrap();
    let report: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(report["notes"].as_array().unwrap().len(), 2);
    assert!(
        nb.join("Inbox/Old/A.md").exists(),
        "a dry run moves nothing"
    );

    let moved = call(ask("Notebooks/Inbox/Old", "Notebooks/Projects/Old", false)).await;
    assert_eq!(moved.status(), StatusCode::OK);
    assert!(nb.join("Projects/Old/Deep/B.md").exists());
    assert!(!nb.join("Inbox/Old").exists());

    let into_self = call(ask(
        "Notebooks/Projects/Old",
        "Notebooks/Projects/Old/Deep/Old",
        false,
    ))
    .await;
    assert_eq!(into_self.status(), StatusCode::BAD_REQUEST);
    let outside = call(ask("Notebooks/Projects/Old", "Elsewhere/Old", false)).await;
    assert_eq!(outside.status(), StatusCode::BAD_REQUEST);
    std::fs::create_dir_all(nb.join("Inbox/Taken")).unwrap();
    let taken = call(ask(
        "Notebooks/Projects/Old",
        "Notebooks/Inbox/Taken",
        false,
    ))
    .await;
    assert_eq!(taken.status(), StatusCode::BAD_REQUEST);
    let back = call(ask("Notebooks/Projects/Old", "Notebooks/Inbox/Old", false)).await;
    assert_eq!(back.status(), StatusCode::OK);
    assert!(nb.join("Inbox/Old/A.md").exists());
}

#[tokio::test]
async fn a_save_from_an_older_version_is_refused_and_writes_nothing() {
    let v = vault();
    let state = Arc::new(WsState::new(v.path().to_path_buf(), None));
    let path = v.path().join("Notebooks/Inbox/Plan.md");
    std::fs::write(&path, "first").unwrap();
    let opened = hq_db::search::content_hash("first");
    std::fs::write(&path, "changed on the phone").unwrap();
    let stale = json!({"path": "Notebooks/Inbox/Plan.md", "content": "mine", "base": opened});
    let res = note_update_handler(State(state.clone()), Json(stale)).await;
    assert_eq!(res.status(), StatusCode::CONFLICT);
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["changed"], true);
    assert_eq!(
        body["theirs_hash"],
        hq_db::search::content_hash("changed on the phone")
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "changed on the phone"
    );

    let fresh = hq_db::search::content_hash("changed on the phone");
    let ok = json!({"path": "Notebooks/Inbox/Plan.md", "content": "mine", "base": fresh});
    let res = note_update_handler(State(state.clone()), Json(ok)).await;
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn web_note_writes_are_recorded_as_the_owners() {
    let v = vault();
    let state = Arc::new(WsState::new(v.path().to_path_buf(), None));
    let body = json!({"folder": "Notebooks/Inbox", "title": "Plan", "content": "zebra"});
    note_create_handler(State(state.clone()), Json(body)).await;
    let edit = json!({"path": "Notebooks/Inbox/Plan.md", "content": "quokka"});
    note_update_handler(State(state.clone()), Json(edit)).await;

    let rel = "Notebooks/Inbox/Plan.md";
    let history = state
        .db
        .with_conn(|c| hq_db::provenance::history(c, rel, 10))
        .unwrap();
    assert_eq!(history.len(), 2, "the create and the edit are two versions");
    assert!(history.iter().all(|p| p.actor_id == "owner"));
    // The latest record is for what is on disk now.
    let raw = std::fs::read_to_string(v.path().join(rel)).unwrap();
    let hash = hq_db::search::content_hash(&raw);
    assert_eq!(history[0].content_hash, hash);
}

/// A note the index has not seen, found by a partial word through the walk.
#[tokio::test]
async fn search_falls_back_to_a_substring_walk() {
    let v = vault();
    std::fs::write(
        v.path().join("Notebooks/Inbox/Trip.md"),
        "packing for london",
    )
    .unwrap();
    let state = Arc::new(WsState::new(v.path().to_path_buf(), None));
    let params = std::collections::HashMap::from([("q".to_string(), "lond".to_string())]);
    let res = crate::api::search_handler(State(state), axum::extract::Query(params)).await;
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["results"][0]["note_path"], "Notebooks/Inbox/Trip.md");
}

#[test]
fn resolve_rejects_traversal_and_absolute_paths() {
    let v = vault();
    assert!(resolve_in_vault(v.path(), "../etc/passwd").is_none());
    assert!(resolve_in_vault(v.path(), "Notebooks/../../x").is_none());
    assert!(resolve_in_vault(v.path(), "/etc/passwd").is_none());
    assert!(resolve_in_vault(v.path(), "Notebooks/New/Deep.md").is_some());
}

#[test]
fn resolve_note_path_handles_exact_extensionless_and_notebooks() {
    let v = vault();
    let plan_path = v.path().join("Notebooks/Inbox/Plan.md");
    std::fs::write(&plan_path, "Plan body").unwrap();
    let root_note = v.path().join("Readme.md");
    std::fs::write(&root_note, "Readme body").unwrap();

    // 1. Exact path
    let (abs, rel) = resolve_note_path(v.path(), "Notebooks/Inbox/Plan.md").expect("exact path");
    assert_eq!(rel, "Notebooks/Inbox/Plan.md");
    assert_eq!(abs, plan_path);

    // 2. Extensionless path
    let (abs, rel) =
        resolve_note_path(v.path(), "Notebooks/Inbox/Plan").expect("extensionless path");
    assert_eq!(rel, "Notebooks/Inbox/Plan.md");
    assert_eq!(abs, plan_path);

    // 3. Root note extensionless
    let (abs, rel) = resolve_note_path(v.path(), "Readme").expect("root extensionless");
    assert_eq!(rel, "Readme.md");
    assert_eq!(abs, root_note);

    // 4. Bare note name in Notebooks subfolder
    let (abs, rel) = resolve_note_path(v.path(), "Plan").expect("bare note in Notebooks");
    assert_eq!(rel, "Notebooks/Inbox/Plan.md");
    assert_eq!(abs, plan_path);

    // 5. Traversal and absolute paths rejected
    assert!(resolve_note_path(v.path(), "../etc/passwd").is_none());
    assert!(resolve_note_path(v.path(), "/etc/passwd").is_none());

    // 6. Missing note returns None
    assert!(resolve_note_path(v.path(), "Nonexistent").is_none());
}

#[test]
fn a_folder_resolves_for_its_listing_but_never_the_root_or_outside() {
    let v = vault();
    std::fs::write(v.path().join("Notebooks/Inbox/Plan.md"), "Plan").unwrap();
    let (abs, rel) = resolve_folder(v.path(), "Notebooks/Inbox/").expect("a folder");
    assert_eq!(rel, "Notebooks/Inbox");
    assert_eq!(abs, v.path().join("Notebooks/Inbox"));
    assert!(
        resolve_folder(v.path(), "Notebooks/Inbox/Plan.md").is_none(),
        "a file"
    );
    assert!(resolve_folder(v.path(), "").is_none(), "the root");
    assert!(resolve_folder(v.path(), "Notebooks/../..").is_none());
    assert!(resolve_folder(v.path(), "Notebooks/Missing").is_none());
}

/// FR-057: a symlink planted inside `Notebooks/` that points outside the
/// vault must not let the bare-name search escape it. Before the fix,
/// `find_note_by_stem` followed `path.is_dir()` through the symlink and
/// returned the outside file directly, bypassing `resolve_in_vault`'s
/// canonicalize-and-check entirely.
#[cfg(unix)]
#[test]
fn resolve_note_path_bare_name_does_not_follow_symlink_out_of_vault() {
    let v = vault();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("secret.md"), "top secret").unwrap();
    std::os::unix::fs::symlink(outside.path(), v.path().join("Notebooks/escape")).unwrap();

    assert!(resolve_note_path(v.path(), "secret").is_none());
}

/// FR-057: `note_read_handler` distinguishes a malformed/malicious
/// reference (400) from a well-formed one that just doesn't resolve to a
/// note (404) using this predicate — it must reject exactly the inputs
/// `resolve_note_path` itself refuses, and nothing else.
#[test]
fn is_rejected_note_ref_flags_traversal_not_missing_notes() {
    assert!(is_rejected_note_ref("../etc/passwd"));
    assert!(is_rejected_note_ref("/etc/passwd"));
    assert!(is_rejected_note_ref("Notebooks/../../x"));
    assert!(is_rejected_note_ref(""));
    assert!(!is_rejected_note_ref("Nonexistent"));
    assert!(!is_rejected_note_ref("Notebooks/Inbox/Plan.md"));
}

#[test]
fn create_unique_dedupes_and_never_overwrites() {
    let v = vault();
    let dir = v.path().join("Notebooks/Inbox");
    let a = create_unique(&dir, "My: Note!", "one").unwrap();
    let b = create_unique(&dir, "My: Note!", "two").unwrap();
    assert_eq!(a.file_name().unwrap(), "My Note.md");
    assert_eq!(b.file_name().unwrap(), "My Note 1.md");
    assert_eq!(std::fs::read_to_string(a).unwrap(), "one");
    assert_eq!(
        create_unique(&dir, "???", "").unwrap().file_name().unwrap(),
        "Untitled.md"
    );
}

#[test]
fn update_keeps_frontmatter_and_stamps_modified() {
    let existing = "---\ntitle: Plan\nmodified: old\ntags: [a]\n---\nold body\n";
    let out = update_preserving_frontmatter(existing, "new body\n", "2026-09-24T00:00:00Z");
    assert!(out.starts_with(
        "---\ntitle: Plan\ntags: [a]\nmodified: '2026-09-24T00:00:00Z'\n---\nnew body"
    ));
    assert!(!out.contains("old body"));
    let bare = update_preserving_frontmatter("no fm", "body", "t");
    assert_eq!(bare, "---\nmodified: 't'\n---\nbody");
}

#[test]
fn update_with_a_body_that_carries_its_own_frontmatter_keeps_one_block() {
    let existing = "---\ntitle: Deck\nsuite_kind: slides\n---\nold\n";
    // What a viewer holding the whole note sends back: its frontmatter, then the edited text.
    let whole = "---\ntitle: Deck\nsuite_kind: slides\n---\n# Deck\n\n## One\n";
    let out = update_preserving_frontmatter(existing, whole, "t");
    assert_eq!(out.matches("suite_kind: slides").count(), 1, "{out}");
    assert_eq!(out.matches("---\n").count(), 2, "{out}");
    assert!(out.ends_with("---\n# Deck\n\n## One\n"), "{out}");
    // A body that only starts with a horizontal rule (no closing fence) is left as written.
    let rule = update_preserving_frontmatter(existing, "---\nnot frontmatter", "t");
    assert!(rule.ends_with("---\nnot frontmatter"), "{rule}");
}

#[test]
fn split_frontmatter_handles_missing_and_present() {
    assert_eq!(
        split_frontmatter("---\na: 1\n---\nbody"),
        (Some("a: 1\n"), "body")
    );
    assert_eq!(split_frontmatter("plain"), (None, "plain"));
    assert_eq!(
        split_frontmatter("---\nunterminated"),
        (None, "---\nunterminated")
    );
}

#[test]
fn set_pinned_round_trips_without_blank_lines() {
    let note = "---\ntitle: A\n---\nbody\n";
    let pinned = set_pinned(note, true);
    assert_eq!(pinned, "---\ntitle: A\npinned: true\n---\nbody\n");
    assert_eq!(set_pinned(&pinned, false), note);
    assert_eq!(set_pinned("plain", true), "---\npinned: true\n---\nplain");
    assert_eq!(
        set_pinned("---\n\ntitle: A\npinned: true\n---\nb", false),
        "---\ntitle: A\n---\nb"
    );
}

#[test]
fn preview_drops_headings_code_and_wikilinks() {
    let body = "# Title\nSee [[Other Note]] now.\n```\ncode\n```\nEnd.";
    assert_eq!(preview_text(body), "See  now.\nEnd.");
}

#[test]
fn signals_classify_work_review_and_done() {
    let v = vault();
    let nb = v.path().join("Notebooks");
    std::fs::write(nb.join("task.md"), "---\ntype: task\nstatus: open\n---\nx").unwrap();
    std::fs::write(nb.join("done.md"), "---\ntype: task\nstatus: Done\n---\nx").unwrap();
    std::fs::write(nb.join("rev.md"), "---\ntype: task\nstatus: review\n---\nx").unwrap();
    std::fs::write(nb.join("dec.md"), "---\ndecision_needed: true\n---\nx").unwrap();
    std::fs::create_dir_all(v.path().join("_logs")).unwrap();
    std::fs::write(v.path().join("_logs/log.md"), "entry").unwrap();
    let s = vault_signals(v.path(), false);
    let paths = |k: &str| -> Vec<String> {
        let mut p: Vec<String> = s[k]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n["path"].as_str().unwrap().to_string())
            .collect();
        p.sort();
        p
    };
    assert_eq!(paths("work"), vec!["Notebooks/task.md"]);
    assert_eq!(
        paths("review"),
        vec!["Notebooks/dec.md", "Notebooks/rev.md"]
    );
    assert_eq!(paths("recent").len(), 4);
    assert_eq!(paths("activity"), vec!["_logs/log.md"]);
}

#[test]
fn pinned_reads_only_the_frontmatter() {
    let v = vault();
    let nb = v.path().join("Notebooks");
    std::fs::write(
        nb.join("yes.md"),
        "---\r\ntitle: Kept\r\npinned: true\r\ntags:\r\n  - a\r\n---\r\nbody",
    )
    .unwrap();
    std::fs::write(
        nb.join("hr.md"),
        "---\ntitle: No\n---\ntext\n---\npinned: true\n",
    )
    .unwrap();
    std::fs::write(
        nb.join("dash.md"),
        "---\ntitle: a---b\npinned: true\n---\nx",
    )
    .unwrap();
    let db = hq_db::Database::open_memory().unwrap();
    // An agent wrote the current version of dash.md, so its pin is only a suggestion.
    let raw = std::fs::read_to_string(nb.join("dash.md")).unwrap();
    db.with_conn(|c| {
        hq_db::provenance::record(
            c,
            "Notebooks/dash.md",
            &hq_db::search::content_hash(&raw),
            &hq_db::provenance::Attribution {
                actor_id: Some("hs-claude-1"),
                source_kind: Some(hq_db::provenance::SourceKind::Agent),
                ..Default::default()
            },
        )
    })
    .unwrap();
    let pinned = pinned_notes(v.path(), false, &db);
    let mut titles: Vec<&str> = pinned
        .iter()
        .map(|n| n["title"].as_str().unwrap())
        .collect();
    titles.sort();
    assert_eq!(titles, vec!["Kept", "a---b"]);
    let kept = pinned.iter().find(|n| n["title"] == "Kept").unwrap();
    assert_eq!(kept["tags"], json!(["a"]));
    assert!(
        kept["suggestedBy"].is_null(),
        "nobody recorded an agent writing it"
    );
    let suggested = pinned.iter().find(|n| n["title"] == "a---b").unwrap();
    assert_eq!(suggested["suggestedBy"], "hs-claude-1");
}

#[test]
fn tree_nests_dirs_first_and_skips_dotfiles() {
    let v = vault();
    let nb = v.path().join("Notebooks");
    std::fs::write(nb.join("a.md"), "").unwrap();
    std::fs::write(nb.join(".hidden.md"), "").unwrap();
    std::fs::write(nb.join("Inbox/b.md"), "").unwrap();
    let tree = build_tree(v.path(), &nb, "Notebooks".into(), false);
    let kids = tree["children"].as_array().unwrap();
    assert_eq!(kids.len(), 2);
    assert_eq!(kids[0]["name"], "Inbox");
    assert_eq!(kids[0]["children"][0]["path"], "Notebooks/Inbox/b.md");
    assert_eq!(kids[1]["type"], "file");
}
