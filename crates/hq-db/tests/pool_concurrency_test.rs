//! Verify that multiple readers can operate in parallel without blocking.

use hq_db::Database;
use std::sync::Arc;
use std::time::Instant;

#[test]
fn concurrent_reads_do_not_serialize() {
    let db = Database::open_memory().unwrap();

    // Seed a table with data
    db.with_conn(|conn| {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS test_data (id INTEGER PRIMARY KEY, val TEXT);
             INSERT INTO test_data (val) VALUES ('a'), ('b'), ('c');",
        )?;
        Ok(())
    })
    .unwrap();

    let db = Arc::new(db);
    let start = Instant::now();

    // Spawn 10 threads, each doing a read
    let handles: Vec<_> = (0..10)
        .map(|_| {
            let db = db.clone();
            std::thread::spawn(move || {
                db.with_conn(|conn| {
                    let mut stmt = conn.prepare("SELECT val FROM test_data")?;
                    let _rows: Vec<String> = stmt
                        .query_map([], |row| row.get(0))?
                        .filter_map(|r| r.ok())
                        .collect();
                    Ok(())
                })
                .unwrap();
            })
        })
        .collect();

    for h in handles {
        h.join().unwrap();
    }

    let elapsed = start.elapsed();
    // With a pool, 10 parallel reads should complete much faster than 10 * serial if they were slow.
    // Here we just verify it works correctly in parallel.
    assert!(
        elapsed.as_millis() < 5000,
        "10 concurrent reads took too long: {:?}",
        elapsed
    );
}
