#[test]
fn event_for_status_only_names_real_task_columns() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    crate::migrations::run(&conn).unwrap();
    for status in [STATUS_IN_PROGRESS, STATUS_READY_FOR_REVIEW] {
        let (_, column) = event_for_status(status).unwrap();
        let exists: bool = conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM pragma_table_info('tasks') WHERE name = ?1",
                [column],
                |r| r.get(0),
            )
            .unwrap();
        assert!(exists, "{column} is not a tasks column");
    }
}

#[test]
fn event_for_status_rejects_unknown_and_hostile_statuses() {
    for status in [
        "",
        "complete",
        "work_started_at",
        "in_progress'; DROP TABLE tasks; --",
    ] {
        assert_eq!(event_for_status(status), None, "{status}");
    }
}

use super::*;
use crate::pool::Database;

fn setup() -> (Database, String) {
    let db = Database::open_memory().unwrap();
    let initiative_id = db
        .with_conn(|c| {
            create_initiative(
                c, "in-1", "personal", None, "Agent HQ", "agent-hq", "AGENT-HQ",
            )?;
            Ok("in-1".to_string())
        })
        .unwrap();
    (db, initiative_id)
}

fn new_task<'a>(title: &'a str, tags: &'a [String]) -> NewTask<'a> {
    NewTask {
        title,
        tags,
        created_by: "test",
        ..Default::default()
    }
}

fn make(db: &Database, id: &str, initiative_id: &str, parent: Option<&str>) -> Result<Task> {
    db.with_conn(|c| {
        create_task(
            c,
            id,
            initiative_id,
            &NewTask {
                parent_task_id: parent,
                ..new_task("Task", &[])
            },
        )
    })
}

fn set_status(db: &Database, id: &str, status: &str) {
    let patch = TaskPatch {
        status: Some(status.to_string()),
        ..Default::default()
    };
    db.with_conn(|c| update_task(c, id, &patch, None)).unwrap();
}

fn get(db: &Database, id: &str) -> Task {
    db.with_conn(|c| get_task(c, id)).unwrap().unwrap()
}

#[test]
fn display_ids_are_sequential_and_unique() {
    let (db, initiative_id) = setup();
    let ids: Vec<String> = (0..5)
        .map(|i| {
            make(&db, &format!("tk-{i}"), &initiative_id, None)
                .unwrap()
                .display_id
        })
        .collect();
    assert_eq!(
        ids,
        vec![
            "AGENT-HQ-001",
            "AGENT-HQ-002",
            "AGENT-HQ-003",
            "AGENT-HQ-004",
            "AGENT-HQ-005"
        ]
    );
}

#[test]
fn get_task_resolves_by_display_id() {
    let (db, initiative_id) = setup();
    let tags = ["hq".to_string()];
    db.with_conn(|c| create_task(c, "tk-1", &initiative_id, &new_task("Task", &tags)))
        .unwrap();
    let found = get(&db, "AGENT-HQ-001");
    assert_eq!(found.id, "tk-1");
    assert_eq!(found.tags, vec!["hq".to_string()]);
    assert!(found.parent_task_id.is_none() && found.start_date.is_none());
}

#[test]
fn claim_safe_update_rejects_status_conflict() {
    let (db, initiative_id) = setup();
    make(&db, "tk-1", &initiative_id, None).unwrap();

    let patch = TaskPatch {
        status: Some(STATUS_IN_PROGRESS.to_string()),
        ..Default::default()
    };
    db.with_conn(|c| update_task(c, "tk-1", &patch, Some(STATUS_TO_DO)))
        .unwrap();

    // Second claim attempt still expects to_do — must fail, task is now in_progress.
    let result = db.with_conn(|c| update_task(c, "tk-1", &patch, Some(STATUS_TO_DO)));
    assert!(result.is_err());
}

#[test]
fn retag_by_display_id_writes_internal_id() {
    let (db, initiative_id) = setup();
    make(&db, "tk-1", &initiative_id, None).unwrap();
    let patch = TaskPatch {
        tags: Some(vec!["hq".to_string()]),
        ..Default::default()
    };
    db.with_conn(|c| update_task(c, "AGENT-HQ-001", &patch, None))
        .unwrap();
    assert_eq!(get(&db, "tk-1").tags, vec!["hq".to_string()]);
}

#[test]
fn delete_task_removes_children_first() {
    let (db, initiative_id) = setup();
    let tags = ["hq".to_string()];
    db.with_conn(|c| create_task(c, "tk-1", &initiative_id, &new_task("Task", &tags)))
        .unwrap();
    db.with_conn(|c| add_comment(c, "tk-1", "test", "a comment", None))
        .unwrap();

    db.with_conn(|c| delete_task(c, "tk-1", false)).unwrap();

    assert!(db.with_conn(|c| get_task(c, "tk-1")).unwrap().is_none());
}

#[test]
fn subtasks_nest_one_level_and_share_the_initiative() {
    let (db, initiative_id) = setup();
    make(&db, "tk-p", &initiative_id, None).unwrap();
    let child = make(&db, "tk-c", &initiative_id, Some("AGENT-HQ-001")).unwrap();
    assert_eq!(child.parent_task_id.as_deref(), Some("tk-p"));
    assert_eq!(child.display_id, "AGENT-HQ-002");

    assert!(make(&db, "tk-g", &initiative_id, Some("tk-c")).is_err());

    db.with_conn(|c| create_initiative(c, "in-2", "personal", None, "Other", "other", "OTHER"))
        .unwrap();
    assert!(make(&db, "tk-x", "in-2", Some("tk-p")).is_err());

    set_status(&db, "tk-c", STATUS_COMPLETE);
    let parent = get(&db, "tk-p");
    assert_eq!((parent.subtask_count, parent.subtask_done), (1, 1));
}

#[test]
fn reparenting_rejects_a_task_that_has_subtasks() {
    let (db, initiative_id) = setup();
    make(&db, "tk-a", &initiative_id, None).unwrap();
    make(&db, "tk-b", &initiative_id, None).unwrap();
    make(&db, "tk-b1", &initiative_id, Some("tk-b")).unwrap();

    let into_a = TaskPatch {
        parent_task_id: Some(Some("tk-a".into())),
        ..Default::default()
    };
    assert!(
        db.with_conn(|c| update_task(c, "tk-b", &into_a, None))
            .is_err()
    );

    let promote = TaskPatch {
        parent_task_id: Some(None),
        ..Default::default()
    };
    db.with_conn(|c| update_task(c, "tk-b1", &promote, None))
        .unwrap();
    assert!(get(&db, "tk-b1").parent_task_id.is_none());
}

#[test]
fn dates_are_validated_and_ordered() {
    let (db, initiative_id) = setup();
    let bad = NewTask {
        start_date: Some("2026-10-05"),
        due_date: Some("2026-10-01"),
        ..new_task("T", &[])
    };
    assert!(
        db.with_conn(|c| create_task(c, "tk-1", &initiative_id, &bad))
            .is_err()
    );

    let garbage = NewTask {
        due_date: Some("1759276800000"),
        ..new_task("T", &[])
    };
    assert!(
        db.with_conn(|c| create_task(c, "tk-2", &initiative_id, &garbage))
            .is_err()
    );

    make(&db, "tk-3", &initiative_id, None).unwrap();
    let patch = TaskPatch {
        start_date: Some(Some("2026-10-01".into())),
        due_date: Some(Some("2026-10-05".into())),
        ..Default::default()
    };
    let task = db
        .with_conn(|c| update_task(c, "tk-3", &patch, None))
        .unwrap();
    assert_eq!(task.start_date.as_deref(), Some("2026-10-01"));
}

#[test]
fn dependencies_reject_self_and_cycles() {
    let (db, initiative_id) = setup();
    for id in ["tk-a", "tk-b", "tk-c"] {
        make(&db, id, &initiative_id, None).unwrap();
    }
    let add = |task: &str, dep: &str| db.with_conn(|c| add_dependency(c, task, dep, "test"));

    assert!(add("tk-a", "tk-a").is_err());
    add("tk-a", "tk-b").unwrap();
    assert!(add("tk-b", "tk-a").is_err());
    add("tk-b", "tk-c").unwrap();
    assert!(add("tk-c", "tk-a").is_err());
    add("tk-a", "tk-b").unwrap();

    assert_eq!(get(&db, "tk-a").depends_on, vec!["tk-b".to_string()]);
    assert_eq!(
        get(&db, "tk-a").blocked_by,
        vec!["AGENT-HQ-002".to_string()]
    );
}

#[test]
fn completing_the_last_blocker_unblocks_dependents() {
    let (db, initiative_id) = setup();
    for id in ["tk-a", "tk-b", "tk-c", "tk-d"] {
        make(&db, id, &initiative_id, None).unwrap();
    }
    db.with_conn(|c| {
        add_dependency(c, "tk-a", "tk-b", "test")?;
        add_dependency(c, "tk-a", "tk-c", "test")?;
        add_dependency(c, "tk-d", "tk-b", "test")
    })
    .unwrap();

    set_status(&db, "tk-b", STATUS_COMPLETE);
    let unblocked: Vec<String> = db
        .with_conn(|c| newly_unblocked(c, "tk-b"))
        .unwrap()
        .into_iter()
        .map(|t| t.id)
        .collect();
    assert_eq!(unblocked, vec!["tk-d".to_string()]);
    assert_eq!(
        get(&db, "tk-a").blocked_by,
        vec!["AGENT-HQ-003".to_string()]
    );

    db.with_conn(|c| remove_dependency(c, "tk-a", "tk-c"))
        .unwrap();
    assert!(get(&db, "tk-a").blocked_by.is_empty());
}

#[test]
fn delete_with_subtasks_needs_cascade_and_cleans_dependencies() {
    let (db, initiative_id) = setup();
    make(&db, "tk-p", &initiative_id, None).unwrap();
    make(&db, "tk-c", &initiative_id, Some("tk-p")).unwrap();
    make(&db, "tk-o", &initiative_id, None).unwrap();
    db.with_conn(|c| add_dependency(c, "tk-o", "tk-c", "test"))
        .unwrap();

    assert!(db.with_conn(|c| delete_task(c, "tk-p", false)).is_err());
    let deleted = db.with_conn(|c| delete_task(c, "tk-p", true)).unwrap();
    assert_eq!(deleted, vec!["tk-c".to_string(), "tk-p".to_string()]);
    assert!(get(&db, "tk-o").depends_on.is_empty());
}

/// A file-backed database: the in-memory one uses shared-cache locking,
/// which fails fast instead of waiting like real WAL connections do.
fn setup_file_db() -> (Database, String) {
    let path = std::env::temp_dir().join(format!("hq-tasks-{}.db", uuid::Uuid::new_v4()));
    let db = Database::open(&path).unwrap();
    db.with_conn(|c| {
        create_initiative(
            c, "in-1", "personal", None, "Agent HQ", "agent-hq", "AGENT-HQ",
        )?;
        Ok(())
    })
    .unwrap();
    (db, "in-1".to_string())
}

fn events(db: &Database, id: &str) -> Vec<(String, String)> {
    db.with_conn(|c| list_task_events(c, id))
        .unwrap()
        .into_iter()
        .map(|e| (e.event_type, e.occurred_at))
        .collect()
}

fn assert_utc_stamp(stamp: &str) {
    chrono::NaiveDateTime::parse_from_str(stamp, "%Y-%m-%d %H:%M:%S").unwrap();
    let utc_now = chrono::Utc::now().naive_utc();
    let parsed = chrono::NaiveDateTime::parse_from_str(stamp, "%Y-%m-%d %H:%M:%S").unwrap();
    assert!(
        (utc_now - parsed).num_seconds().abs() < 60,
        "{stamp} is not UTC now"
    );
}

#[test]
fn new_and_legacy_tasks_have_unknown_timestamps() {
    let (db, initiative_id) = setup();
    let task = make(&db, "tk-1", &initiative_id, None).unwrap();
    assert!(task.work_started_at.is_none() && task.first_ready_for_review_at.is_none());
    assert!(events(&db, "tk-1").is_empty());

    // A task already in_progress before the feature has no event and stays unknown.
    db.with_conn(|c| {
        c.execute(
            "UPDATE tasks SET status = 'in_progress' WHERE id = 'tk-1'",
            [],
        )?;
        Ok(())
    })
    .unwrap();
    set_status(&db, "tk-1", STATUS_IN_PROGRESS);
    let task = get(&db, "tk-1");
    assert!(task.work_started_at.is_none());
    assert!(events(&db, "tk-1").is_empty());
}

#[test]
fn transitions_record_utc_summaries_and_events() {
    let (db, initiative_id) = setup();
    make(&db, "tk-1", &initiative_id, None).unwrap();
    set_status(&db, "tk-1", STATUS_IN_PROGRESS);
    set_status(&db, "tk-1", STATUS_READY_FOR_REVIEW);

    let task = get(&db, "tk-1");
    assert_utc_stamp(task.work_started_at.as_deref().unwrap());
    assert_utc_stamp(task.first_ready_for_review_at.as_deref().unwrap());
    let kinds: Vec<String> = events(&db, "AGENT-HQ-001")
        .into_iter()
        .map(|e| e.0)
        .collect();
    assert_eq!(
        kinds,
        [EVENT_ENTERED_IN_PROGRESS, EVENT_ENTERED_READY_FOR_REVIEW]
    );
}

#[test]
fn reopen_appends_events_and_keeps_first_timestamps() {
    let (db, initiative_id) = setup();
    make(&db, "tk-1", &initiative_id, None).unwrap();
    set_status(&db, "tk-1", STATUS_IN_PROGRESS);
    set_status(&db, "tk-1", STATUS_READY_FOR_REVIEW);
    let first = get(&db, "tk-1");
    // Age the summaries so an overwrite would be visible.
    db.with_conn(|c| {
        c.execute(
            "UPDATE tasks SET work_started_at = '2020-01-01 00:00:00', \
             first_ready_for_review_at = '2020-01-02 00:00:00'",
            [],
        )?;
        Ok(())
    })
    .unwrap();
    set_status(&db, "tk-1", STATUS_IN_PROGRESS);
    set_status(&db, "tk-1", STATUS_READY_FOR_REVIEW);

    let task = get(&db, "tk-1");
    assert_eq!(task.work_started_at.as_deref(), Some("2020-01-01 00:00:00"));
    assert_eq!(
        task.first_ready_for_review_at.as_deref(),
        Some("2020-01-02 00:00:00")
    );
    assert!(first.work_started_at.is_some());
    assert_eq!(events(&db, "tk-1").len(), 4);
}

#[test]
fn same_status_and_other_edits_record_nothing() {
    let (db, initiative_id) = setup();
    make(&db, "tk-1", &initiative_id, None).unwrap();
    set_status(&db, "tk-1", STATUS_IN_PROGRESS);
    set_status(&db, "tk-1", STATUS_IN_PROGRESS);
    set_status(&db, "tk-1", STATUS_BLOCKED);
    set_status(&db, "tk-1", STATUS_COMPLETE);
    assert_eq!(events(&db, "tk-1").len(), 1);
    assert!(get(&db, "tk-1").first_ready_for_review_at.is_none());
}

#[test]
fn lost_claim_records_no_event() {
    let (db, initiative_id) = setup();
    make(&db, "tk-1", &initiative_id, None).unwrap();
    let patch = TaskPatch {
        status: Some(STATUS_IN_PROGRESS.to_string()),
        ..Default::default()
    };
    db.with_conn(|c| update_task(c, "tk-1", &patch, Some(STATUS_TO_DO)))
        .unwrap();
    assert!(
        db.with_conn(|c| update_task(c, "tk-1", &patch, Some(STATUS_TO_DO)))
            .is_err()
    );
    assert_eq!(events(&db, "tk-1").len(), 1);
}

#[test]
fn concurrent_claims_have_one_winner_and_one_event() {
    let (db, initiative_id) = setup_file_db();
    make(&db, "tk-1", &initiative_id, None).unwrap();
    let db = std::sync::Arc::new(db);
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let db = db.clone();
            std::thread::spawn(move || {
                let patch = TaskPatch {
                    status: Some(STATUS_IN_PROGRESS.to_string()),
                    ..Default::default()
                };
                db.with_conn(|c| update_task(c, "tk-1", &patch, Some(STATUS_TO_DO)))
                    .is_ok()
            })
        })
        .collect();
    let winners = handles
        .into_iter()
        .filter(|_| true)
        .map(|h| h.join().unwrap())
        .filter(|ok| *ok)
        .count();
    assert_eq!(winners, 1);
    assert_eq!(events(&db, "tk-1").len(), 1);
}

#[test]
fn unguarded_concurrent_updates_record_one_transition() {
    let (db, initiative_id) = setup_file_db();
    make(&db, "tk-1", &initiative_id, None).unwrap();
    let db = std::sync::Arc::new(db);
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let db = db.clone();
            std::thread::spawn(move || {
                let patch = TaskPatch {
                    status: Some(STATUS_IN_PROGRESS.to_string()),
                    ..Default::default()
                };
                db.with_conn(|c| update_task(c, "tk-1", &patch, None))
                    .unwrap();
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
    assert_eq!(events(&db, "tk-1").len(), 1);
}

#[test]
fn delete_removes_lifecycle_events() {
    let (db, initiative_id) = setup();
    make(&db, "tk-1", &initiative_id, None).unwrap();
    set_status(&db, "tk-1", STATUS_IN_PROGRESS);
    db.with_conn(|c| delete_task(c, "tk-1", false)).unwrap();
    assert!(events(&db, "tk-1").is_empty());
}
fn keyed<'a>(title: &'a str, ext: &'a str) -> NewTask<'a> {
    NewTask {
        external_id: Some(ext),
        ..new_task(title, &[])
    }
}

#[test]
fn an_external_id_returns_the_existing_task_in_the_same_space() {
    let (db, initiative_id) = setup();
    let (first, created) = db
        .with_conn(|c| create_task_dedup(c, "tk-1", &initiative_id, &keyed("A", "ext-1")))
        .unwrap();
    assert!(created);
    assert_eq!(first.external_id.as_deref(), Some("ext-1"));

    let (again, created) = db
        .with_conn(|c| create_task_dedup(c, "tk-2", &initiative_id, &keyed("B", " ext-1 ")))
        .unwrap();
    assert!(!created, "a retry must not create a second task");
    assert_eq!((again.id.as_str(), again.title.as_str()), ("tk-1", "A"));

    let count: i64 = db
        .with_conn(|c| Ok(c.query_row("SELECT COUNT(*) FROM tasks", [], |r| r.get(0))?))
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn an_external_id_is_scoped_to_its_space() {
    let (db, initiative_id) = setup();
    let other = db
        .with_conn(|c| {
            c.execute(
                "INSERT INTO spaces (id, name, slug) VALUES ('sp-2', 'Other', 'other')",
                [],
            )?;
            create_initiative(c, "in-2", "sp-2", None, "Inbox", "inbox", "OTHER-INBOX")?;
            Ok("in-2".to_string())
        })
        .unwrap();
    db.with_conn(|c| create_task_dedup(c, "tk-1", &initiative_id, &keyed("A", "ext-1")))
        .unwrap();
    let (_, created) = db
        .with_conn(|c| create_task_dedup(c, "tk-2", &other, &keyed("A", "ext-1")))
        .unwrap();
    assert!(created, "the same key in another space is a different task");

    let sibling = db
        .with_conn(|c| {
            create_initiative(
                c,
                "in-3",
                "personal",
                None,
                "Second",
                "second",
                "AGENT-HQ-2",
            )?;
            create_task_dedup(c, "tk-3", "in-3", &keyed("C", "ext-1"))
        })
        .unwrap();
    assert!(
        !sibling.1,
        "another initiative in the same space shares the key"
    );
}

#[test]
fn the_unique_index_rejects_a_duplicate_that_skips_the_lookup() {
    let (db, initiative_id) = setup();
    db.with_conn(|c| create_task(c, "tk-1", &initiative_id, &keyed("A", "ext-1")))
        .unwrap();
    assert!(
        db.with_conn(|c| create_task(c, "tk-2", &initiative_id, &keyed("A", "ext-1")))
            .is_err()
    );
}

#[test]
fn a_blank_external_id_is_no_key_and_an_oversized_one_is_refused() {
    let (db, initiative_id) = setup();
    for (n, blank) in ["", "   "].iter().enumerate() {
        let (task, created) = db
            .with_conn(|c| {
                create_task_dedup(c, &format!("tk-{n}"), &initiative_id, &keyed("A", blank))
            })
            .unwrap();
        assert!(created && task.external_id.is_none());
    }
    let long = "x".repeat(MAX_EXTERNAL_ID_LEN + 1);
    assert!(
        db.with_conn(|c| create_task_dedup(c, "tk-9", &initiative_id, &keyed("A", &long)))
            .is_err()
    );
}
