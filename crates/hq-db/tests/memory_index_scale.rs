//! Scale check for the memory index: how long the one-time backfill takes on a large database, and
//! how long a search takes once it is there. A diagnostic, not a regression gate (timings depend on
//! the machine), so it is ignored.
//!
//! Run: `MEMORY_SCALE_ROWS=50000 cargo test -p hq-db --test memory_index_scale --release -- --ignored --nocapture`

use hq_db::memories_search::{View, search_keyword};
use std::time::Instant;

const WORDS: [&str; 24] = [
    "delivery",
    "supplier",
    "invoice",
    "payment",
    "schedule",
    "staff",
    "customer",
    "order",
    "price",
    "feed",
    "market",
    "loan",
    "tax",
    "school",
    "transport",
    "storage",
    "meeting",
    "goal",
    "budget",
    "quality",
    "tray",
    "counter",
    "account",
    "bundle",
];

fn summary(i: usize) -> String {
    let w = |k: usize| WORDS[(i * 7 + k * 11) % WORDS.len()];
    format!(
        "{} for {} is set by {} and the {} follows the {} (item {i})",
        w(0),
        w(1),
        w(2),
        w(3),
        w(4)
    )
}

#[test]
#[ignore = "timing diagnostic: set MEMORY_SCALE_ROWS and run with --ignored --nocapture"]
fn backfill_and_search_scale() {
    let n: usize = std::env::var("MEMORY_SCALE_ROWS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(5000);
    let dir = tempfile::tempdir().unwrap();
    let db = hq_db::Database::open(&dir.path().join("vault.db")).unwrap();

    let start = Instant::now();
    db.with_conn(|c| {
        c.execute_batch("BEGIN")?;
        for i in 0..n {
            c.execute(
                "INSERT INTO memories (source, harness, summary, content, entities, topics, importance, created_at)
                 VALUES ('chat-turn', 'web', ?1, ?1, ?2, '[\"t\"]', 0.5, '2026-01-01T00:00:00+00:00')",
                rusqlite::params![summary(i), format!("[\"Person{}\"]", i % 400)],
            )?;
        }
        c.execute_batch("COMMIT")?;
        Ok(())
    })
    .unwrap();
    println!(
        "insert {n} rows with the index kept by triggers: {:?}",
        start.elapsed()
    );

    // The one-time cost a database with existing memories pays when migration 154 runs: the same
    // statement the migration ends with, over every existing row.
    db.with_conn(|c| {
        c.execute_batch("DELETE FROM memories_fts")?;
        let start = Instant::now();
        c.execute_batch(
            "INSERT INTO memories_fts (rowid, summary, entities, topics)
             SELECT id, COALESCE(NULLIF(summary, ''), content, ''), entities, topics FROM memories;",
        )?;
        println!("backfill of {n} rows: {:?}", start.elapsed());
        Ok(())
    })
    .unwrap();

    db.with_conn(|c| {
        for q in [
            "delivery supplier",
            "Person12 payment",
            "the schedule follows the budget",
        ] {
            let mut times = Vec::new();
            let mut hits = 0;
            for _ in 0..50 {
                let start = Instant::now();
                hits = search_keyword(c, q, View::Recall, 20)?.len();
                times.push(start.elapsed());
            }
            times.sort();
            println!(
                "search {q:?}: {hits} hits, p50 {:?}, p95 {:?}",
                times[times.len() / 2],
                times[times.len() * 95 / 100]
            );
        }
        Ok(())
    })
    .unwrap();
}
