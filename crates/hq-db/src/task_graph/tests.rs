use super::*;
use crate::pool::Database;
use crate::tasks::{self, NewTask, TaskPatch};

const INIT: &str = "in-1";

fn db() -> Database {
    let db = Database::open_memory().unwrap();
    db.with_conn(|c| {
        tasks::create_initiative(c, INIT, "personal", None, "Agent HQ", "agent-hq", "AHQ")?;
        Ok(())
    })
    .unwrap();
    db
}

fn mk(db: &Database, id: &str, title: &str, description: &str, tags: &[&str]) {
    let tags: Vec<String> = tags.iter().map(|t| t.to_string()).collect();
    db.with_conn(|c| {
        tasks::create_task(
            c,
            id,
            INIT,
            &NewTask {
                title,
                description,
                tags: &tags,
                created_by: "test",
                ..Default::default()
            },
        )
    })
    .unwrap();
}

/// Clusters of genuinely related tasks plus unrelated distractors.
const FIXTURE: &[(&str, &str, &str, &[&str])] = &[
    (
        "a1",
        "Stabilize web app streaming so responses cannot crash",
        "chat streaming render crash error boundary",
        &["stability", "streaming"],
    ),
    (
        "a2",
        "Recover chat stream after websocket disconnect",
        "streaming reconnect websocket resume",
        &["streaming"],
    ),
    (
        "a3",
        "Add error boundary around streamed markdown rendering",
        "render crash boundary markdown streaming",
        &["stability", "streaming"],
    ),
    (
        "b1",
        "Record task work-start timestamps",
        "lifecycle events in_progress timestamps",
        &["lifecycle", "timestamps"],
    ),
    (
        "b2",
        "Show task lifecycle history on detail page",
        "lifecycle events timeline",
        &["lifecycle"],
    ),
    (
        "b3",
        "Report average time from work start to review",
        "durations from lifecycle timestamps",
        &["timestamps"],
    ),
    (
        "c1",
        "Monthly retainer billing invoice for pharmacy client",
        "invoice and receipt",
        &[],
    ),
    (
        "c2",
        "Send invoice receipt for retainer payment",
        "receipt after payment",
        &[],
    ),
    (
        "c3",
        "Follow up overdue invoice payment",
        "client invoice payment reminder",
        &[],
    ),
    ("d1", "Call plumber about bathroom door", "", &["home"]),
    (
        "d2",
        "Update Rust toolchain on the CI runner",
        "compiler version",
        &["ci"],
    ),
    (
        "d3",
        "Plan family trip itinerary",
        "flights and hotel",
        &["travel"],
    ),
];

const RELATED_PAIRS: &[(&str, &str)] = &[
    ("a1", "a2"),
    ("a1", "a3"),
    ("a2", "a3"),
    ("b1", "b2"),
    ("b1", "b3"),
    ("b2", "b3"),
    ("c1", "c2"),
    ("c1", "c3"),
    ("c2", "c3"),
];

fn seed_fixture(db: &Database) {
    for (id, title, desc, tags) in FIXTURE {
        mk(db, id, title, desc, tags);
    }
}

fn sync_all(db: &Database) -> SyncReport {
    db.with_conn(|c| sync(c, DEFAULT_SYNC_BUDGET)).unwrap()
}

fn top_ids(db: &Database, id: &str, k: usize) -> Vec<String> {
    db.with_conn(|c| {
        Ok(inferred_links(c, id, k)?
            .into_iter()
            .map(|l| l.task.id)
            .collect())
    })
    .unwrap()
}

#[test]
fn retrieval_quality_on_labeled_fixture() {
    const K: usize = 2;
    const MIN_PRECISION: f64 = 0.8;
    const MIN_RECALL: f64 = 0.8;
    let db = db();
    seed_fixture(&db);
    sync_all(&db);
    let related = |a: &str, b: &str| {
        RELATED_PAIRS
            .iter()
            .any(|(x, y)| (*x == a && *y == b) || (*x == b && *y == a))
    };
    let (mut hits, mut returned, mut expected) = (0usize, 0usize, 0usize);
    for (id, ..) in FIXTURE {
        expected += FIXTURE.iter().filter(|(o, ..)| related(id, o)).count();
        for other in top_ids(&db, id, K) {
            returned += 1;
            hits += usize::from(related(id, &other));
        }
    }
    let precision = hits as f64 / returned as f64;
    let recall = hits as f64 / expected as f64;
    println!("task graph fixture: precision@{K}={precision:.3} recall@{K}={recall:.3}");
    assert!(precision >= MIN_PRECISION, "precision@{K} {precision}");
    assert!(recall >= MIN_RECALL, "recall@{K} {recall}");
}

#[test]
fn inferred_links_carry_evidence_and_explicit_links_stay_separate() {
    let db = db();
    seed_fixture(&db);
    db.with_conn(|c| tasks::add_dependency(c, "a2", "a1", "test"))
        .unwrap();
    sync_all(&db);
    let (explicit, inferred) = db
        .with_conn(|c| Ok((explicit_links(c, "a2")?, inferred_links(c, "a2", 5)?)))
        .unwrap();
    assert_eq!(explicit.len(), 1);
    assert_eq!(explicit[0].kind, KIND_DEPENDS_ON);
    assert_eq!(explicit[0].task.id, "a1");
    let link = inferred
        .iter()
        .find(|l| l.task.id == "a1")
        .expect("a1 also similar");
    assert!(
        link.evidence["shared_terms"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t == "streaming")
    );
    assert!(
        link.evidence["shared_tags"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t == "streaming")
    );
}

#[test]
fn edits_make_a_task_stale_and_sync_is_incremental() {
    let db = db();
    seed_fixture(&db);
    let first = sync_all(&db);
    assert_eq!(
        (first.tasks, first.reindexed),
        (FIXTURE.len(), FIXTURE.len())
    );
    assert_eq!(sync_all(&db).reindexed, 0);

    db.with_conn(|c| {
        tasks::update_task(
            c,
            "d1",
            &TaskPatch {
                title: Some("Fix invoice payment reminder emails".into()),
                ..Default::default()
            },
            None,
        )
    })
    .unwrap();
    let second = sync_all(&db);
    assert_eq!(second.reindexed, 1);
    assert!(top_ids(&db, "d1", 3).contains(&"c3".to_string()));
}

#[test]
fn sync_to_current_finishes_what_one_budgeted_pass_leaves() {
    let db = db();
    seed_fixture(&db);
    let report = db.with_conn(|c| sync_to_current(c, 4)).unwrap();
    assert_eq!(report.stale_remaining, 0);
    assert_eq!(report.reindexed, FIXTURE.len());
    let again = db.with_conn(|c| sync_to_current(c, 4)).unwrap();
    assert_eq!((again.reindexed, again.stale_remaining), (0, 0));
}

#[test]
fn deleted_tasks_are_pruned_and_budget_leaves_stale_remaining() {
    let db = db();
    seed_fixture(&db);
    let partial = db.with_conn(|c| sync(c, 4)).unwrap();
    assert_eq!(
        (partial.reindexed, partial.stale_remaining),
        (4, FIXTURE.len() - 4)
    );
    sync_all(&db);

    db.with_conn(|c| tasks::delete_task(c, "a2", false))
        .unwrap();
    let report = sync_all(&db);
    assert_eq!(report.removed, 1);
    assert!(!top_ids(&db, "a1", 10).contains(&"a2".to_string()));
    let orphans: i64 = db
        .with_conn(|c| {
            Ok(c.query_row(
                "SELECT COUNT(*) FROM task_graph_edges WHERE task_a = 'a2' OR task_b = 'a2'",
                [],
                |r| r.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(orphans, 0);
}

#[test]
fn rebuild_recovers_from_a_wiped_index_and_results_are_bounded() {
    let db = db();
    seed_fixture(&db);
    sync_all(&db);
    db.with_conn(|c| Ok(c.execute_batch("DELETE FROM task_graph_edges")?))
        .unwrap();
    assert!(top_ids(&db, "a1", 5).is_empty());
    db.with_conn(rebuild).unwrap();
    assert!(!top_ids(&db, "a1", 5).is_empty());
    assert_eq!(top_ids(&db, "a1", 1).len(), 1);
}

#[test]
fn empty_graph_and_untagged_lonely_task_fall_back_to_listing() {
    let db = db();
    mk(&db, "x1", "Quantum llama", "", &[]);
    mk(&db, "x2", "Sourdough starter", "", &[]);
    sync_all(&db);
    assert!(top_ids(&db, "x1", 5).is_empty());
    let listing = db.with_conn(|c| fallback_listing(c, "x1", 5)).unwrap();
    assert_eq!(listing.len(), 1);
    assert_eq!(listing[0].id, "x2");
}

#[test]
fn universal_tags_and_bare_numbers_do_not_create_links() {
    let db = db();
    mk(&db, "u1", "Budget 2026 limit 1,683,862", "", &["hq"]);
    mk(&db, "u2", "Fix gutter 2026", "", &["hq"]);
    mk(&db, "u3", "Order flowers", "", &["hq"]);
    sync_all(&db);
    assert!(top_ids(&db, "u1", 5).is_empty());
}

fn load_fixture(db: &Database) {
    for (id, title, description, tags) in FIXTURE {
        mk(db, id, title, description, tags);
    }
}

#[test]
fn a_near_copy_of_an_existing_task_is_called_a_likely_duplicate() {
    let db = db();
    load_fixture(&db);
    let found = db
        .with_conn(|c| {
            similar_to_text(
                c,
                None,
                INIT,
                "Stabilize the web app streaming so responses cannot crash",
                "chat streaming render crash error boundary",
                &["streaming".to_string()],
            )
        })
        .unwrap();
    assert_eq!(found.first().map(|s| s.task.id.as_str()), Some("a1"), "{found:?}");
    assert!(found[0].score >= 0.45);
    assert!(found[0].evidence["shared_terms"].as_array().is_some_and(|t| !t.is_empty()));
}

#[test]
fn unrelated_text_matches_nothing_and_a_cluster_never_reaches_across() {
    let db = db();
    load_fixture(&db);
    let none = db
        .with_conn(|c| similar_to_text(c, None, INIT, "Order new running shoes", "size ten, wide fit", &[]))
        .unwrap();
    assert!(none.is_empty(), "{none:?}");
    for (id, title, description, tags) in FIXTURE {
        let cluster = &id[..1];
        let tags: Vec<String> = tags.iter().map(|t| t.to_string()).collect();
        let found = db
            .with_conn(|c| similar_to_text(c, Some(id), INIT, title, description, &tags))
            .unwrap();
        for s in &found {
            assert_eq!(&s.task.id[..1], cluster, "{id} matched {} from another cluster", s.task.id);
            assert_ne!(s.task.id, *id, "the task itself is excluded");
        }
    }
}

fn finish_with_lease(db: &Database, id: &str, minutes: i64) {
    db.with_conn(|c| {
        tasks::update_task(c, id, &TaskPatch { status: Some("complete".into()), ..Default::default() }, None)?;
        c.execute(
            "INSERT INTO task_work_sessions (id, task_id, actor, token_hash, started_at, ended_at, end_reason) \
             VALUES (?1, ?2, 'a', ?1, datetime('now', '-1 day'), datetime('now', '-1 day', ?3), 'released')",
            params![format!("ws-{id}"), id, format!("+{minutes} minutes")],
        )?;
        Ok(())
    })
    .unwrap();
}

#[test]
fn an_estimate_is_suggested_only_from_enough_real_samples_and_rounded() {
    let db = db();
    mk(&db, "s1", "Write the invoice receipt email template", "receipt invoice email", &[]);
    mk(&db, "s2", "Write the invoice receipt PDF template", "receipt invoice pdf", &[]);
    mk(&db, "s3", "Write the invoice receipt SMS template", "receipt invoice sms", &[]);
    let similar = |db: &Database| {
        db.with_conn(|c| similar_to_text(c, None, INIT, "Write the invoice receipt template", "receipt invoice", &[])).unwrap()
    };
    let suggest = |db: &Database| db.with_conn(|c| suggest_estimate(c, &similar(db))).unwrap();

    assert!(suggest(&db).is_none(), "nothing has been finished yet");
    finish_with_lease(&db, "s1", 40);
    assert!(suggest(&db).is_none(), "one sample is not a pattern");
    finish_with_lease(&db, "s2", 50);
    finish_with_lease(&db, "s3", 62);
    let got = suggest(&db).unwrap();
    assert_eq!(got.minutes, 50, "the median of 40, 50 and 62, to the nearest five");
    assert_eq!(got.based_on.len(), 3);
}

#[test]
fn many_completed_lookalikes_do_not_hide_an_open_duplicate() {
    let db = db();
    mk(&db, "open", "Write the invoice receipt template for email", "invoice receipt email template", &[]);
    for n in 0..8 {
        mk(&db, &format!("done-{n}"), "Write the invoice receipt template for email now", "invoice receipt email template now", &[]);
        db.with_conn(|c| {
            tasks::update_task(c, &format!("done-{n}"), &TaskPatch { status: Some("complete".into()), ..Default::default() }, None).map(|_| ())
        })
        .unwrap();
    }
    let found = db
        .with_conn(|c| similar_to_text(c, None, INIT, "Write the invoice receipt template for email", "invoice receipt email template", &[]))
        .unwrap();
    assert!(found.iter().any(|s| s.task.id == "open"), "{found:?}");
    assert!(found.iter().filter(|s| s.task.status == "complete").count() <= MAX_SIMILAR);
}

#[test]
fn an_even_number_of_samples_takes_the_middle_of_the_two() {
    let db = db();
    mk(&db, "s1", "Write the invoice receipt email template", "receipt invoice email", &[]);
    mk(&db, "s2", "Write the invoice receipt PDF template", "receipt invoice pdf", &[]);
    finish_with_lease(&db, "s1", 30);
    finish_with_lease(&db, "s2", 60);
    let similar = db
        .with_conn(|c| similar_to_text(c, None, INIT, "Write the invoice receipt template", "receipt invoice", &[]))
        .unwrap();
    assert_eq!(db.with_conn(|c| suggest_estimate(c, &similar)).unwrap().unwrap().minutes, 45);
}

#[test]
fn leases_shorter_than_a_minute_are_not_samples() {
    let db = db();
    mk(&db, "s1", "Write the invoice receipt email template", "receipt invoice email", &[]);
    mk(&db, "s2", "Write the invoice receipt PDF template", "receipt invoice pdf", &[]);
    for id in ["s1", "s2"] {
        db.with_conn(|c| {
            tasks::update_task(c, id, &TaskPatch { status: Some("complete".into()), ..Default::default() }, None)?;
            c.execute(
                "INSERT INTO task_work_sessions (id, task_id, actor, token_hash, started_at, ended_at, end_reason) \
                 VALUES (?1, ?2, 'a', ?1, datetime('now', '-1 hour'), datetime('now', '-1 hour', '+20 seconds'), 'released')",
                params![format!("ws-{id}"), id],
            )?;
            Ok(())
        })
        .unwrap();
    }
    let similar = db
        .with_conn(|c| similar_to_text(c, None, INIT, "Write the invoice receipt template", "receipt invoice", &[]))
        .unwrap();
    assert!(db.with_conn(|c| suggest_estimate(c, &similar)).unwrap().is_none());
}
