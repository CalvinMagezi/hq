use super::from_note::{
    PlacementCandidate, choose_placement_via_jev, heuristic_placement, top_level_dir_name,
};
use super::placement::prefix_slug;
use super::*;
use crate::util::generate_id;
use hq_core::config::{DecisionMode, SITE_TASK_PLACEMENT};
use hq_core::mailbox;
use hq_db::tasks as t;
use hq_llm::decision::Decisions;
use serde_json::json;
use std::collections::BTreeMap;

fn tools() -> Vec<Box<dyn HqTool>> {
    let vault_path = PathBuf::from("/tmp/test-vault");
    let vault = Arc::new(VaultClient::new(vault_path.clone()).unwrap());
    create_task_tools(
        vault_path,
        vault,
        Arc::new(Database::open_memory().unwrap()),
    )
}

#[test]
fn factory_returns_sixteen_tools() {
    assert_eq!(tools().len(), 16);
}

#[test]
fn all_tools_have_tasks_category() {
    for tool in tools() {
        assert_eq!(
            tool.category(),
            "tasks",
            "{} has wrong category",
            tool.name()
        );
    }
}

#[test]
fn tool_names_are_distinct() {
    let tool_list = tools();
    let names: Vec<&str> = tool_list.iter().map(|t| t.name()).collect();
    let mut sorted = names.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(names.len(), sorted.len());
}

#[test]
fn parameters_are_valid_json_objects() {
    for tool in tools() {
        let params = tool.parameters();
        assert!(
            params.is_object(),
            "{}'s parameters() is not a JSON object",
            tool.name()
        );
        assert_eq!(params["type"], "object");
    }
}

#[tokio::test]
async fn create_then_list_then_claim_safe_update() {
    let db = Arc::new(Database::open_memory().unwrap());
    let vault_dir = tempfile::tempdir().unwrap();
    let vault_path = vault_dir.path().to_path_buf();
    let vault = Arc::new(VaultClient::new(vault_path.clone()).unwrap());
    let tools = create_task_tools(vault_path, vault, db.clone());
    let create = tools.iter().find(|t| t.name() == "task_create").unwrap();

    let created = create
        .execute(json!({ "title": "Ship the thing", "tags": ["hq"], "description": "long body" }))
        .await
        .unwrap();
    let display_id = created["display_id"].as_str().unwrap().to_string();
    assert!(display_id.ends_with("-001"));

    let list = tools.iter().find(|t| t.name() == "task_list").unwrap();
    let listed = list.execute(json!({ "tag": "hq" })).await.unwrap();
    assert_eq!(listed["count"], 1);
    assert!(listed["tasks"][0].get("description").is_none());
    let full = list
        .execute(json!({ "tag": "hq", "include_description": true }))
        .await
        .unwrap();
    assert_eq!(full["tasks"][0]["description"], "long body");

    let update = tools.iter().find(|t| t.name() == "task_update").unwrap();
    let first = update
        .execute(json!({ "id": display_id, "status": "in_progress", "expected_status": "to_do" }))
        .await;
    assert!(first.is_ok());

    let second = update
        .execute(json!({ "id": display_id, "status": "in_progress", "expected_status": "to_do" }))
        .await;
    assert!(
        second.is_err(),
        "second claim on an already-claimed task should fail"
    );
}

#[tokio::test]
async fn task_create_with_an_external_id_is_idempotent_per_space() {
    let db = Arc::new(Database::open_memory().unwrap());
    let vault_dir = tempfile::tempdir().unwrap();
    let vault_path = vault_dir.path().to_path_buf();
    let vault = Arc::new(VaultClient::new(vault_path.clone()).unwrap());
    let tools = create_task_tools(vault_path, vault, db.clone());
    let create = tools.iter().find(|t| t.name() == "task_create").unwrap();

    let first = create
        .execute(json!({ "title": "Import", "external_id": "req-1", "tags": ["hq"] }))
        .await
        .unwrap();
    assert!(first.get("deduplicated").is_none());
    assert_eq!(first["external_id"], "req-1");

    let again = create
        .execute(json!({ "title": "Import (retry)", "external_id": "req-1" }))
        .await
        .unwrap();
    assert_eq!(again["deduplicated"], true);
    assert_eq!(again["id"], first["id"]);
    assert_eq!(again["title"], "Import", "the existing task is returned untouched");

    db.with_conn(|c| t::create_space(c, "sp-2", "Other", "other")).unwrap();
    let other_space = create
        .execute(json!({ "title": "Import", "external_id": "req-1", "space_id": "other" }))
        .await
        .unwrap();
    assert!(other_space.get("deduplicated").is_none(), "another space has its own key space");
    assert_ne!(other_space["id"], first["id"]);

    let plain = create.execute(json!({ "title": "No key" })).await.unwrap();
    let plain_again = create.execute(json!({ "title": "No key" })).await.unwrap();
    assert_ne!(plain["id"], plain_again["id"], "no external_id means no dedup");
}

#[tokio::test]
async fn subtasks_dependencies_and_unblock_notifications() {
    let db = Arc::new(Database::open_memory().unwrap());
    let vault_dir = tempfile::tempdir().unwrap();
    let vault_path = vault_dir.path().to_path_buf();
    std::fs::create_dir_all(vault_path.join(mailbox::MAILBOX_DIR).join("reviewer")).unwrap();
    let vault = Arc::new(VaultClient::new(vault_path.clone()).unwrap());
    let tools = create_task_tools(vault_path.clone(), vault, db.clone());
    let tool = |name: &str| tools.iter().find(|t| t.name() == name).unwrap();

    let parent = tool("task_create")
        .execute(json!({ "title": "Launch" }))
        .await
        .unwrap();
    let parent_id = parent["display_id"].as_str().unwrap().to_string();
    let child = tool("task_create")
        .execute(json!({ "title": "Write copy", "parent_id": parent_id, "due_date": "2026-10-02" }))
        .await
        .unwrap();
    let child_id = child["display_id"].as_str().unwrap().to_string();
    assert_eq!(child["parent_task_id"], parent["id"]);
    assert_eq!(child["initiative_id"], parent["initiative_id"]);

    let blocked = tool("task_create")
        .execute(json!({ "title": "Publish", "tags": ["reviewer"], "depends_on": [child_id] }))
        .await
        .unwrap();
    let blocked_id = blocked["display_id"].as_str().unwrap().to_string();
    assert_eq!(blocked["blocked_by"], json!([child_id]));

    let cycle = tool("task_update")
        .execute(json!({ "id": child_id, "add_depends_on": [blocked_id] }))
        .await;
    assert!(cycle.is_err());

    let started = tool("task_update")
        .execute(json!({ "id": blocked_id, "status": "in_progress" }))
        .await
        .unwrap();
    assert!(
        started["warnings"].is_array(),
        "starting a blocked task should warn, not fail"
    );

    let inbox = vault_path.join(mailbox::MAILBOX_DIR).join("reviewer");
    let count_messages = || walk_files(&inbox);
    let before = count_messages();
    tool("task_update")
        .execute(json!({ "id": child_id, "status": "complete" }))
        .await
        .unwrap();
    assert_eq!(
        count_messages(),
        before + 1,
        "completing the blocker notifies the dependent's tags"
    );

    let got = tool("task_get")
        .execute(json!({ "id": parent_id }))
        .await
        .unwrap();
    assert_eq!(got["subtask_count"], 1);
    assert!(
        got["work_started_at"].is_null(),
        "unstarted task has unknown work_started_at"
    );
    assert_eq!(got["lifecycle_events"], json!([]));
    assert_eq!(got["subtasks"][0]["display_id"], child_id);

    let top = tool("task_list")
        .execute(json!({ "top_level_only": true }))
        .await
        .unwrap();
    assert_eq!(top["count"], 2);

    assert!(
        tool("task_delete")
            .execute(json!({ "id": parent_id }))
            .await
            .is_err()
    );
    let deleted = tool("task_delete")
        .execute(json!({ "id": parent_id, "cascade": true }))
        .await
        .unwrap();
    assert_eq!(deleted["deleted_ids"].as_array().unwrap().len(), 2);
}

fn walk_files(dir: &std::path::Path) -> usize {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .map(|e| {
                    if e.path().is_dir() {
                        walk_files(&e.path())
                    } else {
                        1
                    }
                })
                .sum()
        })
        .unwrap_or(0)
}

/// The value item fires only on the actual transition into
/// `ready_for_review`, not on every update once already there.
#[tokio::test]
async fn update_to_ready_for_review_emits_a_value_item_once() {
    let db = Arc::new(Database::open_memory().unwrap());
    let vault_dir = tempfile::tempdir().unwrap();
    let vault_path = vault_dir.path().to_path_buf();
    let vault = Arc::new(VaultClient::new(vault_path.clone()).unwrap());
    let tools = create_task_tools(vault_path.clone(), vault, db.clone());

    let create = tools.iter().find(|t| t.name() == "task_create").unwrap();
    let created = create
        .execute(json!({ "title": "Review my PR" }))
        .await
        .unwrap();
    let display_id = created["display_id"].as_str().unwrap().to_string();

    let value_db = hq_db::Database::open(&vault_path.join("_data").join("vault.db")).unwrap();
    let count_action_needed = || {
        hq_db::value_items::list_by_state(&value_db, hq_core::types::ValueState::Pending)
            .unwrap()
            .len()
    };
    assert_eq!(count_action_needed(), 0);

    let update = tools.iter().find(|t| t.name() == "task_update").unwrap();
    update
        .execute(json!({ "id": display_id, "status": "ready_for_review" }))
        .await
        .unwrap();
    assert_eq!(
        count_action_needed(),
        1,
        "transition into ready_for_review should emit one value item"
    );

    // Already ready_for_review: touching an unrelated field must not re-fire.
    update
        .execute(json!({ "id": display_id, "priority": "high" }))
        .await
        .unwrap();
    assert_eq!(
        count_action_needed(),
        1,
        "no transition occurred, so no second value item"
    );

    // Leaving and re-entering ready_for_review is a genuine second transition
    // in this crate's own logic, but `hq_db::value_items::emit`'s dedup_key
    // collapses it anyway while the first notification is still pending/
    // unactioned — the right behavior: don't re-notify about a task that's
    // already sitting in an outstanding review request.
    update
        .execute(json!({ "id": display_id, "status": "in_progress" }))
        .await
        .unwrap();
    update
        .execute(json!({ "id": display_id, "status": "ready_for_review" }))
        .await
        .unwrap();
    assert_eq!(
        count_action_needed(),
        1,
        "still deduped while the first notification is unresolved"
    );
}

fn make_note(path: &str, tags: Vec<String>, content: &str) -> hq_core::types::Note {
    // `notes::read_note` derives `tags` purely from the file's own YAML
    // frontmatter, not from `Note.tags` at write time (`write_note` only
    // ever serializes the `frontmatter` map) — so a round-trippable fixture
    // has to put them there itself, same as a hand-written vault note would.
    let mut frontmatter = std::collections::HashMap::new();
    if !tags.is_empty() {
        frontmatter.insert(
            "tags".to_string(),
            serde_yaml::Value::Sequence(
                tags.iter()
                    .map(|t| serde_yaml::Value::String(t.clone()))
                    .collect(),
            ),
        );
    }
    hq_core::types::Note {
        title: path
            .rsplit('/')
            .next()
            .unwrap_or(path)
            .trim_end_matches(".md")
            .to_string(),
        content: content.to_string(),
        path: path.to_string(),
        frontmatter,
        note_type: None,
        tags,
        pinned: false,
        source: None,
        embedding_status: None,
        created_at: None,
        updated_at: None,
        modified_at: chrono::Utc::now(),
    }
}

fn candidate(
    space_id: &str,
    space_name: &str,
    folder_name: Option<&str>,
    initiative_name: &str,
) -> PlacementCandidate {
    let initiative = t::Initiative {
        id: generate_id("in"),
        space_id: space_id.to_string(),
        folder_id: folder_name.map(|_| "fo-1".to_string()),
        name: initiative_name.to_string(),
        slug: slug(initiative_name),
        id_prefix: prefix_slug(initiative_name),
        next_sequence: 1,
        created_at: String::new(),
    };
    let label = match folder_name {
        Some(f) => format!("{space_name} > {f} > {initiative_name}"),
        None => format!("{space_name} > {initiative_name}"),
    };
    PlacementCandidate {
        initiative,
        label,
        folder_name: folder_name.map(String::from),
    }
}

#[test]
fn heuristic_placement_matches_note_folder_to_initiative_name() {
    let candidates = vec![
        candidate(
            "professional",
            "Professional",
            Some("Clients & Retainers"),
            "AcmeCorp — Platform & Infra",
        ),
        candidate("personal", "Personal", None, "Home"),
    ];
    let found = heuristic_placement(
        "notes",
        &[],
        "Notebooks/Projects/AcmeCorp — Platform & Infra/notes.md",
        &candidates,
    );
    assert_eq!(
        found.unwrap().initiative.name,
        "AcmeCorp — Platform & Infra"
    );
}

#[test]
fn heuristic_placement_matches_by_tag() {
    let candidates = vec![candidate("professional", "Professional", None, "agent-hq")];
    let found = heuristic_placement(
        "notes",
        &["agent-hq".to_string()],
        "Notebooks/Daily/2026-09-22.md",
        &candidates,
    );
    assert_eq!(found.unwrap().initiative.name, "agent-hq");
}

/// Reproduces the motivating real-world case: a note whose folder and tags
/// don't match anything, but whose own heading strongly overlaps an
/// existing List's name ("AcmeCorp Platform & Infrastructure Research
/// Findings" vs. the List "AcmeCorp — Platform & Infra").
#[test]
fn heuristic_placement_matches_by_title_when_folder_and_tags_dont() {
    let candidates = vec![
        candidate(
            "professional",
            "Professional",
            Some("Clients & Retainers"),
            "AcmeCorp — Platform & Infra",
        ),
        candidate("personal", "Personal", None, "Home"),
    ];
    let found = heuristic_placement(
        "AcmeCorp Platform & Infrastructure Research Findings",
        &[],
        "Notebooks/Daily/research-findings-2026-09-18.md",
        &candidates,
    );
    assert_eq!(
        found.unwrap().initiative.name,
        "AcmeCorp — Platform & Infra"
    );
}

#[test]
fn heuristic_placement_returns_none_when_ambiguous_or_no_match() {
    let ambiguous = vec![
        candidate("p", "P", None, "agent-hq"),
        candidate("p", "P", None, "agent-hq-api"),
    ];
    assert!(
        heuristic_placement("notes", &["agent-hq".to_string()], "notes.md", &ambiguous).is_none()
    );

    let no_match = vec![candidate("p", "P", None, "totally-unrelated")];
    assert!(heuristic_placement("notes", &[], "Notebooks/Random/notes.md", &no_match).is_none());
}

fn fake_decisions(
    answers: BTreeMap<String, hq_llm::decision::Answer>,
    site_mode: DecisionMode,
) -> Arc<Decisions> {
    let fake = hq_llm::decision::FakeDecisionProvider {
        answers,
        fail: false,
    };
    let mut sites = BTreeMap::new();
    sites.insert(
        SITE_TASK_PLACEMENT.to_string(),
        hq_core::config::DecisionSite {
            mode: site_mode,
            threshold: None,
        },
    );
    let config = hq_core::config::DecisionsConfig {
        enabled: true,
        sites,
        ..Default::default()
    };
    let dir = tempfile::tempdir().unwrap();
    Arc::new(Decisions::new(Arc::new(fake), config, dir.path()))
}

#[tokio::test]
async fn jev_pick_overrides_when_enforced_and_confident() {
    let candidates = vec![candidate("professional", "Professional", None, "agent-hq")];
    let target_id = candidates[0].initiative.id.clone();
    let answers = BTreeMap::from([(
        "task_placement".to_string(),
        hq_llm::decision::Answer::Choice {
            choice: target_id.clone(),
            probabilities: BTreeMap::new(),
            confidence: 0.9,
        },
    )]);
    let decisions = fake_decisions(answers, DecisionMode::Enforce);
    let picked =
        choose_placement_via_jev(Some(&decisions), "Some note", "body", &[], &candidates).await;
    assert_eq!(picked, Some(target_id));
}

#[tokio::test]
async fn jev_pick_is_none_when_site_mode_is_off() {
    let candidates = vec![candidate("professional", "Professional", None, "agent-hq")];
    let answers = BTreeMap::from([(
        "task_placement".to_string(),
        hq_llm::decision::Answer::Choice {
            choice: candidates[0].initiative.id.clone(),
            probabilities: BTreeMap::new(),
            confidence: 0.9,
        },
    )]);
    // Off is the built-in default for a site the config never mentions.
    let decisions = fake_decisions(answers, DecisionMode::Off);
    let picked =
        choose_placement_via_jev(Some(&decisions), "Some note", "body", &[], &candidates).await;
    assert_eq!(picked, None);
}

#[tokio::test]
async fn jev_pick_is_none_without_a_decisions_handle() {
    let candidates = vec![candidate("professional", "Professional", None, "agent-hq")];
    let picked = choose_placement_via_jev(None, "Some note", "body", &[], &candidates).await;
    assert_eq!(picked, None);
}

/// `044_tasks.sql` seeds `personal`/`professional` (id == slug) into every
/// fresh `Database::open_memory()` via `INSERT OR IGNORE` — real code must
/// never re-create them, and tests build on top of them rather than
/// re-creating spaces with those slugs (which would collide).
#[tokio::test]
async fn create_from_note_places_into_existing_matching_initiative() {
    let db = Arc::new(Database::open_memory().unwrap());
    let vault_dir = tempfile::tempdir().unwrap();
    let vault_path = vault_dir.path().to_path_buf();
    let vault = Arc::new(VaultClient::new(vault_path.clone()).unwrap());

    db.with_conn(|c| find_or_create_initiative(c, "professional", None, "agent-hq"))
        .unwrap();

    let note = make_note(
        "Notebooks/Projects/notes.md",
        vec!["agent-hq".to_string()],
        "Findings about agent-hq.",
    );
    vault.write_note(&note.path, &note).unwrap();

    let tools = create_task_tools(vault_path, vault, db.clone());
    let tool = tools
        .iter()
        .find(|t| t.name() == "task_create_from_note")
        .unwrap();
    let result = tool
        .execute(json!({ "note_path": note.path }))
        .await
        .unwrap();

    assert_eq!(result["created_new_destination"], false);
    let initiatives = db
        .with_conn(|c| t::list_initiatives(c, None, None))
        .unwrap();
    assert_eq!(
        initiatives.len(),
        1,
        "should reuse the existing initiative, not create a second one"
    );
    assert_eq!(result["task"]["title"], "notes");
}

/// The motivating real-world case end to end: a research note filed under
/// a folder ("Daily") that matches no List, with no tags, but whose own
/// heading overlaps an existing List's name closely enough that it should
/// still land there instead of spawning a duplicate List.
#[tokio::test]
async fn create_from_note_places_by_title_overlap_alone() {
    let db = Arc::new(Database::open_memory().unwrap());
    let vault_dir = tempfile::tempdir().unwrap();
    let vault_path = vault_dir.path().to_path_buf();
    let vault = Arc::new(VaultClient::new(vault_path.clone()).unwrap());

    db.with_conn(|c| {
        find_or_create_initiative(
            c,
            "professional",
            Some("Clients & Retainers"),
            "AcmeCorp — Platform & Infra",
        )
    })
    .unwrap();

    let note = make_note(
        "Notebooks/Daily/research-findings-2026-09-18.md",
        vec![],
        "# AcmeCorp Platform & Infrastructure Research Findings\n\nAudit findings below.",
    );
    vault.write_note(&note.path, &note).unwrap();

    let tools = create_task_tools(vault_path, vault, db.clone());
    let tool = tools
        .iter()
        .find(|t| t.name() == "task_create_from_note")
        .unwrap();
    let result = tool
        .execute(json!({ "note_path": note.path }))
        .await
        .unwrap();

    assert_eq!(
        result["created_new_destination"], false,
        "a title-overlap match should reuse the existing List, not spawn a new one: {result}"
    );
    let initiatives = db
        .with_conn(|c| t::list_initiatives(c, None, None))
        .unwrap();
    assert_eq!(initiatives.len(), 1);
    assert_eq!(initiatives[0].name, "AcmeCorp — Platform & Infra");
}

#[tokio::test]
async fn create_from_note_creates_new_list_when_nothing_fits() {
    let db = Arc::new(Database::open_memory().unwrap());
    let vault_dir = tempfile::tempdir().unwrap();
    let vault_path = vault_dir.path().to_path_buf();
    let vault = Arc::new(VaultClient::new(vault_path.clone()).unwrap());

    let note = make_note(
        "Notebooks/ClientZ/research.md",
        vec![],
        "Nothing here matches an existing list.",
    );
    vault.write_note(&note.path, &note).unwrap();

    let tools = create_task_tools(vault_path, vault, db.clone());
    let tool = tools
        .iter()
        .find(|t| t.name() == "task_create_from_note")
        .unwrap();
    let result = tool
        .execute(json!({ "note_path": note.path }))
        .await
        .unwrap();

    assert_eq!(result["created_new_destination"], true);
    let initiatives = db
        .with_conn(|c| t::list_initiatives(c, None, None))
        .unwrap();
    assert_eq!(initiatives.len(), 1);
    assert_eq!(initiatives[0].name, "ClientZ");
    // Falls back to one of the existing (default-seeded) Spaces rather than inventing a new one.
    let spaces = db.with_conn(t::list_spaces).unwrap();
    assert_eq!(
        spaces.len(),
        2,
        "no new Space should be created when some already exist"
    );
    assert!(spaces.iter().any(|s| s.id == initiatives[0].space_id));
}

#[tokio::test]
async fn create_from_note_creates_new_space_when_none_exist() {
    let db = Arc::new(Database::open_memory().unwrap());
    // Simulate a vault whose default Spaces were removed: the "no Space
    // exists at all" branch is otherwise unreachable, since migrations
    // always seed personal/professional.
    db.with_conn(|c| {
        c.execute("DELETE FROM spaces", [])
            .map_err(anyhow::Error::from)
    })
    .unwrap();

    let vault_dir = tempfile::tempdir().unwrap();
    let vault_path = vault_dir.path().to_path_buf();
    let vault = Arc::new(VaultClient::new(vault_path.clone()).unwrap());

    let note = make_note(
        "Professional/ClientZ/research.md",
        vec![],
        "A brand new domain.",
    );
    vault.write_note(&note.path, &note).unwrap();

    let tools = create_task_tools(vault_path, vault, db.clone());
    let tool = tools
        .iter()
        .find(|t| t.name() == "task_create_from_note")
        .unwrap();
    let result = tool
        .execute(json!({ "note_path": note.path }))
        .await
        .unwrap();

    assert_eq!(result["created_new_destination"], true);
    let spaces = db.with_conn(t::list_spaces).unwrap();
    assert_eq!(spaces.len(), 1);
    assert_eq!(spaces[0].name, "Professional");
}

#[test]
fn top_level_dir_name_is_none_for_a_root_level_note() {
    assert_eq!(top_level_dir_name("todo.md"), None);
    assert_eq!(
        top_level_dir_name("Notebooks/todo.md"),
        Some("Notebooks".to_string())
    );
}

#[tokio::test]
async fn create_from_note_falls_back_to_inbox_for_a_root_level_note_with_no_spaces() {
    let db = Arc::new(Database::open_memory().unwrap());
    db.with_conn(|c| {
        c.execute("DELETE FROM spaces", [])
            .map_err(anyhow::Error::from)
    })
    .unwrap();

    let vault_dir = tempfile::tempdir().unwrap();
    let vault_path = vault_dir.path().to_path_buf();
    let vault = Arc::new(VaultClient::new(vault_path.clone()).unwrap());

    // A root-level note has no containing directory at all — the fallback
    // space name must not become the note's own filename ("todo.md").
    let note = make_note("todo.md", vec![], "A note with nowhere to file it.");
    vault.write_note(&note.path, &note).unwrap();

    let tools = create_task_tools(vault_path, vault, db.clone());
    let tool = tools
        .iter()
        .find(|t| t.name() == "task_create_from_note")
        .unwrap();
    let result = tool
        .execute(json!({ "note_path": note.path }))
        .await
        .unwrap();

    assert_eq!(result["created_new_destination"], true);
    let spaces = db.with_conn(t::list_spaces).unwrap();
    assert_eq!(spaces.len(), 1);
    assert_eq!(spaces[0].name, "Inbox");
}

#[tokio::test]
async fn create_from_note_honors_explicit_space_override() {
    let db = Arc::new(Database::open_memory().unwrap());
    let vault_dir = tempfile::tempdir().unwrap();
    let vault_path = vault_dir.path().to_path_buf();
    let vault = Arc::new(VaultClient::new(vault_path.clone()).unwrap());

    let note = make_note("Notebooks/notes.md", vec![], "Body");
    vault.write_note(&note.path, &note).unwrap();

    let tools = create_task_tools(vault_path, vault, db.clone());
    let tool = tools
        .iter()
        .find(|t| t.name() == "task_create_from_note")
        .unwrap();
    let result = tool
        .execute(json!({ "note_path": note.path, "space_id": "professional", "initiative": "Forced List" }))
        .await
        .unwrap();

    assert_eq!(result["created_new_destination"], false);
    let initiatives = db
        .with_conn(|c| t::list_initiatives(c, None, None))
        .unwrap();
    assert_eq!(initiatives.len(), 1);
    assert_eq!(initiatives[0].name, "Forced List");
}

/// `personal`/`professional` are the only two Spaces where `id == slug`
/// (seeded that way by migrations) — every other Space, including any a
/// user creates via `space_create`, gets a generated id distinct from its
/// slug. An explicit `space_id` override (documented, like every other
/// tool in this file, as accepting a slug) must resolve correctly against
/// one of those too, not just the two coincidentally-matching defaults.
#[tokio::test]
async fn create_from_note_resolves_explicit_space_override_by_slug_not_just_id() {
    let db = Arc::new(Database::open_memory().unwrap());
    let vault_dir = tempfile::tempdir().unwrap();
    let vault_path = vault_dir.path().to_path_buf();
    let vault = Arc::new(VaultClient::new(vault_path.clone()).unwrap());
    let tools = create_task_tools(vault_path, vault.clone(), db.clone());

    let space_create = tools.iter().find(|t| t.name() == "space_create").unwrap();
    let space = space_create
        .execute(json!({ "name": "Acme Corp" }))
        .await
        .unwrap();
    let space_id = space["id"].as_str().unwrap().to_string();
    let space_slug = space["slug"].as_str().unwrap().to_string();
    assert_ne!(
        space_id, space_slug,
        "a user-created Space's id must not equal its slug"
    );

    let note = make_note("Notebooks/notes.md", vec![], "Body");
    vault.write_note(&note.path, &note).unwrap();

    let result = tools
        .iter()
        .find(|t| t.name() == "task_create_from_note")
        .unwrap()
        .execute(json!({ "note_path": note.path, "space_id": space_slug, "initiative": "Kickoff" }))
        .await
        .unwrap();

    assert_eq!(result["created_new_destination"], false);
    let initiatives = db
        .with_conn(|c| t::list_initiatives(c, None, None))
        .unwrap();
    assert_eq!(initiatives.len(), 1);
    assert_eq!(
        initiatives[0].space_id, space_id,
        "should resolve the slug to the real space id"
    );
}

#[tokio::test]
async fn task_related_separates_explicit_from_inferred_and_falls_back() {
    let db = Arc::new(Database::open_memory().unwrap());
    db.with_conn(|c| {
        t::create_initiative(c, "in-1", "personal", None, "HQ", "hq", "HQ")?;
        for (id, title) in [
            ("k1", "Stabilize chat streaming rendering"),
            ("k2", "Recover chat streaming after disconnect"),
            ("k3", "Unrelated llama grooming"),
        ] {
            t::create_task(
                c,
                id,
                "in-1",
                &t::NewTask {
                    title,
                    created_by: "test",
                    ..Default::default()
                },
            )?;
        }
        t::add_dependency(c, "k2", "k1", "test")
    })
    .unwrap();
    let tool = tools_graph::TaskRelatedTool { db };
    let out = tool.execute(json!({ "id": "k2" })).await.unwrap();
    assert_eq!(out["explicit"][0]["kind"], "depends_on");
    assert_eq!(out["inferred"][0]["class"], "inferred");
    assert_eq!(out["inferred"][0]["task"]["id"], "k1");
    assert!(out["inferred"][0]["evidence"]["shared_terms"].is_array());
    assert!(out.get("fallback").is_none());

    let lonely = tool.execute(json!({ "id": "k3" })).await.unwrap();
    assert_eq!(lonely["inferred"].as_array().unwrap().len(), 0);
    assert_eq!(lonely["fallback"]["kind"], "same_initiative_listing");
    assert!(tool.execute(json!({ "id": "nope" })).await.is_err());
}

/// A db shared by a create tool and a comment tool.
async fn comment_as(attested: Option<&str>, supplied_author: Option<&str>) -> String {
    let vault_path = PathBuf::from("/tmp/test-vault");
    let vault = Arc::new(VaultClient::new(vault_path.clone()).unwrap());
    let db = Arc::new(Database::open_memory().unwrap());
    let all = create_task_tools(vault_path, vault, db.clone());
    let tool = |name: &str| all.iter().find(|t| t.name() == name).unwrap();

    let made = tool("task_create")
        .execute(json!({"title": "thread", "created_by": "hq"}))
        .await
        .unwrap();
    let mut args = json!({"task_id": made["id"], "body": "hello"});
    if let Some(author) = supplied_author {
        args["author"] = author.into();
    }
    if let Some(id) = attested {
        use hq_db::harness_sessions_registry::{self as registry, NewSession, Placement};
        db.with_conn(|c| {
            registry::insert(
                c,
                &NewSession {
                    id,
                    harness: "claude-code",
                    label: "t",
                    cwd: "/t",
                    mission_id: made["id"].as_str(),
                    placement: Placement { host: "native", agent_name: id, workspace_id: "w", pane_id: "p" },
                },
            )
        })
        .unwrap();
        args[hq_tools_arg()] = id.into();
    }
    tool("task_comment_add").execute(args).await.unwrap();
    let listed = tool("task_comment_list")
        .execute(json!({"task_id": made["id"]}))
        .await
        .unwrap();
    listed["comments"][0]["author"].as_str().unwrap().to_string()
}

fn hq_tools_arg() -> &'static str {
    crate::harness_session::CALLER_SESSION_ARG
}

#[tokio::test]
async fn a_comment_from_a_launched_agent_is_authored_by_its_attested_session() {
    assert_eq!(
        comment_as(Some("hs-claude-code-1"), Some("someone-else")).await,
        "hs-claude-code-1",
        "a name the agent supplies must not replace the session it proved"
    );
}

#[tokio::test]
async fn other_callers_keep_the_author_they_name() {
    assert_eq!(comment_as(None, Some("calvin")).await, "calvin");
    assert_eq!(comment_as(None, None).await, "unknown");
}

#[tokio::test]
async fn a_launched_agent_reaches_only_its_own_task_through_the_task_tools() {
    use hq_db::harness_sessions_registry::{self as registry, NewSession, Placement};
    let vault_path = PathBuf::from("/tmp/test-vault");
    let vault = Arc::new(VaultClient::new(vault_path.clone()).unwrap());
    let db = Arc::new(Database::open_memory().unwrap());
    let all = create_task_tools(vault_path, vault, db.clone());
    let tool = |name: &str| all.iter().find(|t| t.name() == name).unwrap();
    let make = |title: &str| {
        let t = tool("task_create");
        let title = title.to_string();
        async move { t.execute(json!({"title": title, "created_by": "calvin"})).await.unwrap() }
    };
    let (mine, theirs) = (make("mine").await, make("theirs").await);
    db.with_conn(|c| {
        registry::insert(
            c,
            &NewSession {
                id: "hs-me",
                harness: "claude-code",
                label: "t",
                cwd: "/t",
                mission_id: mine["id"].as_str(),
                placement: Placement { host: "native", agent_name: "hs-me", workspace_id: "w", pane_id: "p" },
            },
        )
    })
    .unwrap();
    let as_me = |mut args: serde_json::Value| {
        args[crate::harness_session::CALLER_SESSION_ARG] = "hs-me".into();
        args
    };

    for name in ["task_get"] {
        assert!(tool(name).execute(as_me(json!({"id": mine["id"]}))).await.is_ok());
        assert!(tool(name).execute(as_me(json!({"id": theirs["id"]}))).await.is_err());
    }
    for name in ["task_comment_list"] {
        assert!(tool(name).execute(as_me(json!({"task_id": mine["id"]}))).await.is_ok());
        assert!(tool(name).execute(as_me(json!({"task_id": theirs["id"]}))).await.is_err());
    }
    let add = |task: &serde_json::Value| as_me(json!({"task_id": task["id"], "body": "note"}));
    assert!(tool("task_comment_add").execute(add(&mine)).await.is_ok());
    assert!(tool("task_comment_add").execute(add(&theirs)).await.is_err(), "no planting text on other tasks");

    let listed = tool("task_list").execute(as_me(json!({}))).await.unwrap();
    assert_eq!(listed["count"], 1, "{listed}");
    let unscoped = tool("task_list").execute(json!({})).await.unwrap();
    assert_eq!(unscoped["count"], 2, "callers without a session are not scoped");
}
