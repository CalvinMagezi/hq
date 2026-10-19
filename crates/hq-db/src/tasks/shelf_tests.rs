use super::tests::{make, setup};
use super::*;
use crate::pool::Database;

const TTL: i64 = 900;

fn ids(db: &Database, shelf: Shelf) -> Vec<String> {
    let filter = TaskFilter {
        shelf,
        ..Default::default()
    };
    let mut found: Vec<String> = db
        .with_conn(|c| list_tasks(c, &filter))
        .unwrap()
        .into_iter()
        .map(|t| t.id)
        .collect();
    found.sort();
    found
}

fn refs(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|s| s.to_string()).collect()
}

fn complete(db: &Database, id: &str, days_ago: i64) {
    let patch = TaskPatch {
        status: Some(STATUS_COMPLETE.to_string()),
        ..Default::default()
    };
    db.with_conn(|c| update_task(c, id, &patch, None)).unwrap();
    db.with_conn(|c| {
        c.execute(
            "UPDATE tasks SET completed_at = datetime('now', ?1), updated_at = datetime('now', ?1) WHERE id = ?2",
            params![format!("-{days_ago} days"), id],
        )?;
        Ok(())
    })
    .unwrap();
}

fn set_status(db: &Database, id: &str, status: &str) {
    let patch = TaskPatch {
        status: Some(status.to_string()),
        ..Default::default()
    };
    db.with_conn(|c| update_task(c, id, &patch, None)).unwrap();
}

fn shelve(db: &Database, id: &str) -> ShelveOutcome {
    db.with_conn(|c| shelve_tasks(c, &refs(&[id]))).unwrap()
}

fn shelved_at(db: &Database, id: &str) -> Option<String> {
    db.with_conn(|c| get_task(c, id))
        .unwrap()
        .unwrap()
        .shelved_at
}

#[test]
fn the_default_list_keeps_shelved_tasks_and_the_filter_splits_them() {
    let (db, init) = setup();
    make(&db, "a", &init, None).unwrap();
    make(&db, "b", &init, None).unwrap();
    assert_eq!(shelve(&db, "a").changed, vec!["a"]);

    assert_eq!(ids(&db, Shelf::Include), vec!["a", "b"]);
    assert_eq!(ids(&db, Shelf::Exclude), vec!["b"]);
    assert_eq!(ids(&db, Shelf::Only), vec!["a"]);
    let filter = TaskFilter {
        shelf: Shelf::Exclude,
        ..Default::default()
    };
    assert_eq!(db.with_conn(|c| count_tasks(c, &filter)).unwrap(), 1);
}

#[test]
fn shelving_leaves_the_clock_the_events_and_the_status_alone() {
    let (db, init) = setup();
    make(&db, "a", &init, None).unwrap();
    complete(&db, "a", 40);
    let before = db.with_conn(|c| get_task(c, "a")).unwrap().unwrap();
    let events_before = db.with_conn(|c| list_task_events(c, "a")).unwrap().len();

    shelve(&db, "a");

    let after = db.with_conn(|c| get_task(c, "a")).unwrap().unwrap();
    assert_eq!(after.updated_at, before.updated_at);
    assert_eq!(after.status, before.status);
    assert_eq!(after.completed_at, before.completed_at);
    assert_eq!(
        db.with_conn(|c| list_task_events(c, "a")).unwrap().len(),
        events_before
    );
    assert!(
        db.with_conn(|c| list_task_audit(c, None, 10))
            .unwrap()
            .is_empty(),
        "shelving is not a removal"
    );
}

#[test]
fn a_shelved_task_stays_valid_for_the_queue_the_dependencies_and_the_links() {
    let (db, init) = setup();
    make(&db, "blocker", &init, None).unwrap();
    make(&db, "waiting", &init, None).unwrap();
    db.with_conn(|c| add_dependency(c, "waiting", "blocker", "test"))
        .unwrap();
    shelve(&db, "blocker");

    let waiting = db.with_conn(|c| get_task(c, "waiting")).unwrap().unwrap();
    assert_eq!(
        waiting.blocked_by.len(),
        1,
        "a shelved blocker still blocks"
    );

    set_status(&db, "blocker", STATUS_COMPLETE);
    let waiting = db.with_conn(|c| get_task(c, "waiting")).unwrap().unwrap();
    assert!(waiting.blocked_by.is_empty());

    shelve(&db, "waiting");
    let query = NextTaskQuery {
        assignee: "alice",
        include_unassigned: true,
        initiative_id: None,
        tag: None,
        no_code_work: false,
    };
    let next = db.with_conn(|c| next_task(c, &query)).unwrap();
    assert_eq!(next.map(|t| t.id), Some("waiting".to_string()));
}

#[test]
fn claiming_or_reopening_a_shelved_task_brings_it_and_its_list_back() {
    let (db, init) = setup();
    make(&db, "claimed", &init, None).unwrap();
    make(&db, "reopened", &init, None).unwrap();
    complete(&db, "reopened", 40);
    shelve(&db, "claimed");
    shelve(&db, "reopened");
    db.with_conn(|c| set_initiative_shelved(c, &init, true))
        .unwrap();

    let who = LeaseIdentity {
        actor: "alice",
        harness: "claude-code",
        external_session_ref: "sess-1",
        ..Default::default()
    };
    db.with_conn(|c| claim(c, "claimed", &who, TTL, false))
        .unwrap();
    assert!(shelved_at(&db, "claimed").is_none());
    let list = db.with_conn(|c| get_initiative(c, &init)).unwrap().unwrap();
    assert!(list.shelved_at.is_none(), "live work un-hides its list");

    assert!(shelved_at(&db, "reopened").is_some());
    set_status(&db, "reopened", STATUS_TO_DO);
    assert!(shelved_at(&db, "reopened").is_none());
}

#[test]
fn editing_a_shelved_complete_task_keeps_it_shelved() {
    let (db, init) = setup();
    make(&db, "a", &init, None).unwrap();
    complete(&db, "a", 40);
    shelve(&db, "a");
    let patch = TaskPatch {
        title: Some("Renamed".to_string()),
        ..Default::default()
    };
    db.with_conn(|c| update_task(c, "a", &patch, None)).unwrap();
    assert!(shelved_at(&db, "a").is_some());
}

#[test]
fn shelving_a_list_hides_its_tasks_and_unshelving_restores_them_untouched() {
    let (db, init) = setup();
    make(&db, "a", &init, None).unwrap();
    make(&db, "b", &init, None).unwrap();
    shelve(&db, "a");

    let (list, open) = db
        .with_conn(|c| set_initiative_shelved(c, &init, true))
        .unwrap();
    assert!(list.shelved_at.is_some());
    assert_eq!(open, 2);
    assert!(ids(&db, Shelf::Exclude).is_empty());
    assert_eq!(ids(&db, Shelf::Only), vec!["a", "b"]);
    let shelved = db
        .with_conn(|c| list_initiatives_on_shelf(c, None, None, Shelf::Only))
        .unwrap();
    assert_eq!(shelved.len(), 1);

    db.with_conn(|c| set_initiative_shelved(c, &init, false))
        .unwrap();
    assert_eq!(
        ids(&db, Shelf::Exclude),
        vec!["b"],
        "a stays shelved by itself"
    );
    let shown = db
        .with_conn(|c| list_initiatives_on_shelf(c, None, None, Shelf::Exclude))
        .unwrap();
    assert_eq!(shown.len(), 1);
}

#[test]
fn a_task_added_to_a_shelved_list_or_parent_brings_it_back() {
    let (db, init) = setup();
    make(&db, "parent", &init, None).unwrap();
    complete(&db, "parent", 40);
    shelve(&db, "parent");
    db.with_conn(|c| set_initiative_shelved(c, &init, true))
        .unwrap();

    make(&db, "child", &init, Some("parent")).unwrap();

    assert!(shelved_at(&db, "parent").is_none());
    let list = db.with_conn(|c| get_initiative(c, &init)).unwrap().unwrap();
    assert!(list.shelved_at.is_none());
}

#[test]
fn a_shelved_parent_hides_its_subtasks() {
    let (db, init) = setup();
    make(&db, "parent", &init, None).unwrap();
    make(&db, "child", &init, Some("parent")).unwrap();
    shelve(&db, "parent");
    assert!(ids(&db, Shelf::Exclude).is_empty());
    assert_eq!(ids(&db, Shelf::Only), vec!["child", "parent"]);
}

#[test]
fn shelving_reports_what_it_skipped() {
    let (db, init) = setup();
    for id in ["held", "gone", "twice"] {
        make(&db, id, &init, None).unwrap();
    }
    let who = LeaseIdentity {
        actor: "alice",
        harness: "claude-code",
        external_session_ref: "sess-1",
        ..Default::default()
    };
    db.with_conn(|c| claim(c, "held", &who, TTL, false))
        .unwrap();
    db.with_conn(|c| archive_task(c, "gone", false, "test"))
        .unwrap();
    shelve(&db, "twice");

    let out = db
        .with_conn(|c| shelve_tasks(c, &refs(&["held", "gone", "twice", "missing"])))
        .unwrap();
    assert!(out.changed.is_empty());
    let reasons: Vec<&str> = out.skipped.iter().map(|s| s.reason.as_str()).collect();
    assert_eq!(
        reasons,
        [
            "alice is working on it",
            "deleted; restore it first",
            "already archived",
            "not found"
        ]
    );
}

#[test]
fn the_sweep_shelves_only_old_finished_work_and_can_be_undone() {
    let (db, init) = setup();
    for id in [
        "old",
        "legacy",
        "recent",
        "open",
        "parent_old",
        "open_child",
    ] {
        make(&db, id, &init, None).unwrap();
    }
    make(&db, "kid", &init, Some("parent_old")).unwrap();
    make(&db, "kid2", &init, Some("open_child")).unwrap();
    complete(&db, "old", 90);
    complete(&db, "recent", 2);
    complete(&db, "legacy", 90);
    db.with_conn(|c| {
        c.execute(
            "UPDATE tasks SET completed_at = NULL WHERE id = 'legacy'",
            [],
        )?;
        Ok(())
    })
    .unwrap();
    complete(&db, "parent_old", 90);
    complete(&db, "kid", 90);
    complete(&db, "open_child", 90);

    let scope = SweepScope::default();
    let dry = db
        .with_conn(|c| shelve_completed(c, 30, &scope, true))
        .unwrap();
    assert_eq!((dry.matched, dry.changed.len()), (4, 0));
    assert!(
        ids(&db, Shelf::Only).is_empty(),
        "a dry run changes nothing"
    );

    let done = db
        .with_conn(|c| shelve_completed(c, 30, &scope, false))
        .unwrap();
    let mut changed = done.changed.clone();
    changed.sort();
    assert_eq!(changed, vec!["kid", "legacy", "old", "parent_old"]);
    for stays in ["recent", "open", "open_child", "kid2"] {
        assert!(shelved_at(&db, stays).is_none(), "{stays} is not swept");
    }
    let again = db
        .with_conn(|c| shelve_completed(c, 30, &scope, false))
        .unwrap();
    assert_eq!(again.matched, 0);

    let undo = db.with_conn(|c| unshelve_tasks(c, &done.changed)).unwrap();
    assert_eq!(undo.changed.len(), 4);
    assert!(ids(&db, Shelf::Only).is_empty());
}

#[test]
fn the_sweep_respects_its_scope_and_age_limits() {
    let (db, init) = setup();
    db.with_conn(|c| create_initiative(c, "in-2", "personal", None, "Other", "other", "OTHER"))
        .unwrap();
    make(&db, "a", &init, None).unwrap();
    make(&db, "b", "in-2", None).unwrap();
    complete(&db, "a", 90);
    complete(&db, "b", 90);

    let scope = SweepScope {
        initiative_id: Some("in-2".to_string()),
        ..Default::default()
    };
    let out = db
        .with_conn(|c| shelve_completed(c, 30, &scope, false))
        .unwrap();
    assert_eq!(out.changed, vec!["b"]);
    assert!(
        db.with_conn(|c| shelve_completed(c, 0, &SweepScope::default(), true))
            .is_err()
    );
}

fn alice() -> LeaseIdentity<'static> {
    LeaseIdentity {
        actor: "alice",
        harness: "claude-code",
        external_session_ref: "sess-1",
        ..Default::default()
    }
}

#[test]
fn a_session_started_on_a_shelved_task_brings_it_and_its_list_back() {
    let (db, init) = setup();
    make(&db, "a", &init, None).unwrap();
    set_status(&db, "a", STATUS_IN_PROGRESS);
    shelve(&db, "a");
    db.with_conn(|c| set_initiative_shelved(c, &init, true))
        .unwrap();

    db.with_conn(|c| open_for_session(c, "a", "hs-1", "alice", "claude-code", "mac", "/work"))
        .unwrap();

    assert!(shelved_at(&db, "a").is_none());
    let list = db.with_conn(|c| get_initiative(c, &init)).unwrap().unwrap();
    assert!(list.shelved_at.is_none());
}

#[test]
fn claiming_a_task_that_is_already_in_progress_still_surfaces_it() {
    let (db, init) = setup();
    make(&db, "a", &init, None).unwrap();
    set_status(&db, "a", STATUS_IN_PROGRESS);
    shelve(&db, "a");
    assert!(shelved_at(&db, "a").is_some());

    db.with_conn(|c| claim(c, "a", &alice(), TTL, false))
        .unwrap();

    assert!(
        shelved_at(&db, "a").is_none(),
        "no status move happened, so the claim itself must surface it"
    );
}

#[test]
fn filing_open_work_again_by_its_external_id_surfaces_it() {
    let (db, init) = setup();
    let new = NewTask {
        title: "Nightly sync",
        created_by: "test",
        external_id: Some("nightly"),
        ..Default::default()
    };
    let (first, created) = db
        .with_conn(|c| create_task_dedup(c, "a", &init, &new))
        .unwrap();
    assert!(created);
    shelve(&db, &first.id);

    let (again, created) = db
        .with_conn(|c| create_task_dedup(c, "b", &init, &new))
        .unwrap();

    assert!(!created);
    assert_eq!(again.id, first.id);
    assert!(again.shelved_at.is_none());
    assert!(shelved_at(&db, "a").is_none());
}

#[test]
fn a_parent_is_not_shelved_over_a_sub_task_an_agent_holds_and_a_list_is_not_either() {
    let (db, init) = setup();
    make(&db, "parent", &init, None).unwrap();
    make(&db, "child", &init, Some("parent")).unwrap();
    db.with_conn(|c| claim(c, "child", &alice(), TTL, false))
        .unwrap();

    let out = shelve(&db, "parent");
    assert!(out.changed.is_empty());
    assert_eq!(
        out.skipped[0].reason,
        "alice is working on one of its sub-tasks"
    );

    let err = db
        .with_conn(|c| set_initiative_shelved(c, &init, true))
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("alice is working on a task in this list"),
        "{err}"
    );
    let list = db.with_conn(|c| get_initiative(c, &init)).unwrap().unwrap();
    assert!(list.shelved_at.is_none());
}

#[test]
fn unshelving_a_task_hidden_by_its_list_or_parent_says_where_to_look() {
    let (db, init) = setup();
    make(&db, "parent", &init, None).unwrap();
    make(&db, "child", &init, Some("parent")).unwrap();
    shelve(&db, "parent");
    let by_parent = db
        .with_conn(|c| unshelve_tasks(c, &refs(&["child"])))
        .unwrap();
    assert!(
        by_parent.skipped[0].reason.contains("archived parent"),
        "{:?}",
        by_parent.skipped
    );

    db.with_conn(|c| unshelve_tasks(c, &refs(&["parent"])))
        .unwrap();
    db.with_conn(|c| set_initiative_shelved(c, &init, true))
        .unwrap();
    let by_list = db
        .with_conn(|c| unshelve_tasks(c, &refs(&["child"])))
        .unwrap();
    assert!(
        by_list.skipped[0].reason.contains("archived list"),
        "{:?}",
        by_list.skipped
    );

    let plain = db
        .with_conn(|c| unshelve_tasks(c, &refs(&["missing"])))
        .unwrap();
    assert_eq!(plain.skipped[0].reason, "not found");
}

#[test]
fn the_sweep_leaves_a_task_an_agent_holds_and_honours_a_space_scope() {
    let (db, init) = setup();
    db.with_conn(|c| create_initiative(c, "in-2", "professional", None, "Other", "other", "OTHER"))
        .unwrap();
    make(&db, "held", &init, None).unwrap();
    make(&db, "free", &init, None).unwrap();
    make(&db, "elsewhere", "in-2", None).unwrap();
    for id in ["held", "free", "elsewhere"] {
        complete(&db, id, 90);
    }
    db.with_conn(|c| open_for_session(c, "held", "hs-1", "alice", "claude-code", "mac", "/w"))
        .unwrap();
    // open_for_session surfaces; the sweep is what is under test, so nothing is shelved yet.

    let scope = SweepScope {
        space_id: Some("personal".to_string()),
        ..Default::default()
    };
    let out = db
        .with_conn(|c| shelve_completed(c, 30, &scope, false))
        .unwrap();

    assert_eq!(
        out.changed,
        vec!["free"],
        "held is leased and elsewhere is in another space"
    );
}
