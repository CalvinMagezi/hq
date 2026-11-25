//! Scale check for the full-text index: how long indexing and path lookups take as the vault grows.
//! A diagnostic, not a regression gate (timings depend on the machine), so it is ignored.
//!
//! Run: `FTS_SCALE_NOTES=5000 cargo test -p hq-db --test fts_scale --release -- --ignored --nocapture`

use std::time::Instant;

fn body(i: usize) -> String {
    let mut s = String::new();
    for w in 0..250 {
        s.push_str(&format!(
            "word{} topic{} ",
            (i * 7 + w) % 997,
            (i + w * 3) % 211
        ));
    }
    s
}

#[test]
#[ignore = "timing diagnostic: set FTS_SCALE_NOTES and run with --ignored --nocapture"]
fn indexing_and_path_lookups_scale() {
    let n: usize = std::env::var("FTS_SCALE_NOTES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2000);
    let dir = tempfile::tempdir().unwrap();
    let db = hq_db::Database::open(&dir.path().join("vault.db")).unwrap();

    let start = Instant::now();
    db.with_conn(|c| {
        c.execute_batch("BEGIN")?;
        for i in 0..n {
            let path = format!("Notebooks/n{i}.md");
            hq_db::search::index_note(c, &path, &format!("Note {i}"), &body(i), "a b")?;
        }
        c.execute_batch("COMMIT")?;
        Ok(())
    })
    .unwrap();
    let initial = start.elapsed();

    // Re-index every 10th note: the edit path (delete by path, then insert).
    let start = Instant::now();
    let edits = n / 10;
    db.with_conn(|c| {
        for i in (0..n).step_by(10) {
            let path = format!("Notebooks/n{i}.md");
            hq_db::search::index_note(c, &path, &format!("Note {i}"), &body(i + 1), "a b")?;
        }
        Ok(())
    })
    .unwrap();
    let edit = start.elapsed();

    // Title and tags by path, the lookup a semantic hit does once per result.
    let start = Instant::now();
    let lookups = 200.min(n);
    db.with_conn(|c| {
        for i in 0..lookups {
            let path = format!("Notebooks/n{}.md", i * (n / lookups));
            c.prepare(
                "SELECT title, tags FROM notes_fts
                  WHERE rowid = (SELECT fts_rowid FROM notes_fts_rows WHERE path = ?1)",
            )?
            .query_row([path.as_str()], |r| r.get::<_, String>(0))?;
        }
        Ok(())
    })
    .unwrap();
    let lookup = start.elapsed();

    println!(
        "FTS_SCALE notes={n} initial_index={initial:?} edits({edits})={edit:?} path_lookups({lookups})={lookup:?}"
    );
}
