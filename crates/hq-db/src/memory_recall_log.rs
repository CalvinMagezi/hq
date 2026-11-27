//! What recall worked out for each chat message, as counts and ids only (migration 157).
//!
//! Written by the chat path, read by `hq memory recall-report`. Nothing here holds a message or a
//! memory's text, and nothing here leaves the device.

use anyhow::Result;
use rusqlite::{Connection, params};

/// How long a row is kept.
pub const KEEP_DAYS: i64 = 30;
/// A row older than this is deleted on every this-many-th write, so trimming costs nothing per turn.
const TRIM_EVERY: i64 = 50;

/// Reasons recall said nothing because something went wrong, as opposed to nothing fitting.
pub const FAILURE_REASONS: [&str; 2] = ["timed out", "error"];

/// One recall.
#[derive(Debug, Clone)]
pub struct Entry<'a> {
    pub session_id: &'a str,
    /// `shadow` or `query`.
    pub mode: &'a str,
    pub memory_ids: &'a [i64],
    /// The size of the block that was (or, in shadow mode, would have been) injected.
    pub tokens: usize,
    /// Why nothing was brought up, when nothing was.
    pub silent_because: Option<&'a str>,
    pub latency_ms: u64,
}

/// Writes one row, and now and then trims rows past [`KEEP_DAYS`].
pub fn record(conn: &Connection, entry: &Entry<'_>) -> Result<()> {
    let now = chrono::Utc::now();
    conn.execute(
        "INSERT INTO memory_recall_log
            (at, session_id, mode, memory_ids, items, tokens, silent_because, latency_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            now.to_rfc3339(),
            entry.session_id,
            entry.mode,
            serde_json::to_string(entry.memory_ids)?,
            entry.memory_ids.len() as i64,
            entry.tokens as i64,
            entry.silent_because,
            entry.latency_ms as i64
        ],
    )?;
    if conn.last_insert_rowid() % TRIM_EVERY == 0 {
        let cutoff = (now - chrono::Duration::days(KEEP_DAYS)).to_rfc3339();
        conn.execute("DELETE FROM memory_recall_log WHERE at < ?1", [cutoff])?;
    }
    Ok(())
}

/// A summary of recent recalls.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Report {
    pub days: i64,
    pub turns: i64,
    /// Turns that brought up at least one memory.
    pub with_memories: i64,
    /// Turns that brought up nothing, by reason, most common first.
    pub silent: Vec<(String, i64)>,
    /// Turns where recall timed out or failed. A subset of `silent`.
    pub failures: i64,
    pub mean_items: f64,
    pub latency_ms: Percentiles,
    pub tokens: Percentiles,
}

/// The middle and the slow end of a set of numbers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Percentiles {
    pub p50: u64,
    pub p95: u64,
    pub max: u64,
}

fn percentiles(mut values: Vec<u64>) -> Percentiles {
    if values.is_empty() {
        return Percentiles::default();
    }
    values.sort_unstable();
    let at = |p: f64| values[((values.len() as f64 - 1.0) * p).round() as usize];
    Percentiles {
        p50: at(0.5),
        p95: at(0.95),
        max: *values.last().unwrap_or(&0),
    }
}

/// What recall did over the last `days` days.
pub fn report(conn: &Connection, days: i64) -> Result<Report> {
    let since = (chrono::Utc::now() - chrono::Duration::days(days.max(1))).to_rfc3339();
    let mut stmt = conn.prepare(
        "SELECT items, tokens, silent_because, latency_ms FROM memory_recall_log WHERE at >= ?1",
    )?;
    let rows = stmt.query_map([since], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, Option<String>>(2)?,
            r.get::<_, i64>(3)?,
        ))
    })?;

    let mut report = Report {
        days: days.max(1),
        ..Report::default()
    };
    let (mut latencies, mut sizes, mut items_total) = (Vec::new(), Vec::new(), 0i64);
    let mut reasons: std::collections::BTreeMap<String, i64> = std::collections::BTreeMap::new();
    for row in rows {
        let (items, tokens, reason, latency) = row?;
        report.turns += 1;
        items_total += items;
        latencies.push(latency.max(0) as u64);
        if items > 0 {
            report.with_memories += 1;
            sizes.push(tokens.max(0) as u64);
        }
        if let Some(reason) = reason {
            if FAILURE_REASONS.contains(&reason.as_str()) {
                report.failures += 1;
            }
            *reasons.entry(reason).or_default() += 1;
        }
    }
    if report.turns > 0 {
        report.mean_items = items_total as f64 / report.turns as f64;
    }
    report.latency_ms = percentiles(latencies);
    report.tokens = percentiles(sizes);
    let mut silent: Vec<(String, i64)> = reasons.into_iter().collect();
    silent.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    report.silent = silent;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::migrations::run(&conn).unwrap();
        conn
    }

    fn entry<'a>(ids: &'a [i64], reason: Option<&'a str>, ms: u64, tokens: usize) -> Entry<'a> {
        Entry {
            session_id: "s1",
            mode: "shadow",
            memory_ids: ids,
            tokens,
            silent_because: reason,
            latency_ms: ms,
        }
    }

    #[test]
    fn an_empty_log_reports_nothing_rather_than_failing() {
        let r = report(&db(), 7).unwrap();
        assert_eq!(r.turns, 0);
        assert_eq!(r.latency_ms, Percentiles::default());
        assert!(r.silent.is_empty());
    }

    #[test]
    fn the_report_counts_turns_reasons_failures_and_sizes() {
        let conn = db();
        record(&conn, &entry(&[1, 2], None, 4, 80)).unwrap();
        record(&conn, &entry(&[3], None, 8, 40)).unwrap();
        record(&conn, &entry(&[], Some("nothing fits"), 2, 0)).unwrap();
        record(&conn, &entry(&[], Some("nothing fits"), 3, 0)).unwrap();
        record(&conn, &entry(&[], Some("asks about nothing"), 1, 0)).unwrap();
        record(&conn, &entry(&[], Some("timed out"), 150, 0)).unwrap();

        let r = report(&conn, 7).unwrap();

        assert_eq!(r.turns, 6);
        assert_eq!(r.with_memories, 2);
        assert_eq!(r.failures, 1, "only a timeout or an error is a failure");
        assert_eq!(
            r.silent,
            vec![
                ("nothing fits".to_string(), 2),
                ("asks about nothing".to_string(), 1),
                ("timed out".to_string(), 1),
            ]
        );
        assert!((r.mean_items - 0.5).abs() < 1e-9, "3 memories over 6 turns");
        assert_eq!(r.latency_ms.max, 150);
        assert_eq!(
            r.tokens.max, 80,
            "sizes count only turns that brought something up"
        );
    }

    #[test]
    fn a_row_holds_ids_and_counts_and_no_text() {
        let conn = db();
        record(&conn, &entry(&[7, 9], None, 5, 33)).unwrap();
        let (ids, items, tokens): (String, i64, i64) = conn
            .query_row(
                "SELECT memory_ids, items, tokens FROM memory_recall_log",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!((ids.as_str(), items, tokens), ("[7,9]", 2, 33));
        // The table has no column that could hold a message or a memory's words.
        let text_columns: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('memory_recall_log')
                  WHERE name IN ('message', 'text', 'summary', 'content', 'prompt')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(text_columns, 0);
    }

    #[test]
    fn old_rows_are_trimmed_as_new_ones_are_written() {
        let conn = db();
        conn.execute(
            "INSERT INTO memory_recall_log (at, mode) VALUES (?1, 'shadow')",
            [(chrono::Utc::now() - chrono::Duration::days(KEEP_DAYS + 5)).to_rfc3339()],
        )
        .unwrap();
        // Rows are trimmed on every TRIM_EVERY-th id.
        for _ in 0..TRIM_EVERY {
            record(&conn, &entry(&[1], None, 1, 10)).unwrap();
        }
        let old: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memory_recall_log WHERE at < ?1",
                [(chrono::Utc::now() - chrono::Duration::days(KEEP_DAYS)).to_rfc3339()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(old, 0);
    }

    #[test]
    fn percentiles_are_the_middle_and_the_slow_end() {
        let p = percentiles((1..=100).collect());
        assert_eq!((p.p50, p.p95, p.max), (51, 95, 100));
        assert_eq!(percentiles(vec![]), Percentiles::default());
    }
}
