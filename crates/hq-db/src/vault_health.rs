//! Stored results of the vault health scan, and the things the owner told it to ignore.
//!
//! The scan itself is `hq_vault::health`; this only keeps what it found, so a screen can show
//! the last scan without reading the vault again and a trend can be drawn from the counts.
//! The report is kept as JSON text: this crate does not read it, only hands it back.

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;

/// How many scans are kept. A weekly trend needs a few months; older rows are dropped.
pub const KEEP_RUNS: i64 = 200;

/// The numbers of one scan. The same names as `hq_vault::health::Counts`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct RunCounts {
    pub notes: i64,
    pub links: i64,
    pub dead: i64,
    pub ambiguous: i64,
    pub broken_anchors: i64,
    pub loose: i64,
    pub duplicates: i64,
    pub empty: i64,
    pub frontmatter: i64,
    pub unread: i64,
}

impl RunCounts {
    /// Dead and ambiguous links, broken anchors, duplicate groups and frontmatter problems:
    /// what a person could fix. Loose and empty notes are not damage.
    pub fn problems(&self) -> i64 {
        self.dead + self.ambiguous + self.broken_anchors + self.duplicates + self.frontmatter
    }
}

/// A scan as stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Run {
    pub id: i64,
    /// RFC 3339.
    pub ran_at: String,
    pub counts: RunCounts,
    /// The report as JSON text.
    pub report: String,
}

/// One point of the trend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TrendPoint {
    pub ran_at: String,
    pub problems: i64,
    pub links: i64,
    pub dead: i64,
}

/// Store a scan, and drop the oldest beyond [`KEEP_RUNS`]. Returns its id.
pub fn insert_run(
    conn: &Connection,
    ran_at: &str,
    counts: &RunCounts,
    report: &str,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO vault_health_runs
            (ran_at, notes, links, dead, ambiguous, broken_anchors, loose, duplicates, empty,
             frontmatter, unread, report)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            ran_at,
            counts.notes,
            counts.links,
            counts.dead,
            counts.ambiguous,
            counts.broken_anchors,
            counts.loose,
            counts.duplicates,
            counts.empty,
            counts.frontmatter,
            counts.unread,
            report
        ],
    )?;
    let id = conn.last_insert_rowid();
    conn.execute(
        "DELETE FROM vault_health_runs WHERE id NOT IN
            (SELECT id FROM vault_health_runs ORDER BY id DESC LIMIT ?1)",
        [KEEP_RUNS],
    )?;
    Ok(id)
}

fn row_to_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<Run> {
    Ok(Run {
        id: row.get(0)?,
        ran_at: row.get(1)?,
        counts: RunCounts {
            notes: row.get(2)?,
            links: row.get(3)?,
            dead: row.get(4)?,
            ambiguous: row.get(5)?,
            broken_anchors: row.get(6)?,
            loose: row.get(7)?,
            duplicates: row.get(8)?,
            empty: row.get(9)?,
            frontmatter: row.get(10)?,
            unread: row.get(11)?,
        },
        report: row.get(12)?,
    })
}

const RUN_COLUMNS: &str = "id, ran_at, notes, links, dead, ambiguous, broken_anchors, loose, \
                           duplicates, empty, frontmatter, unread, report";

/// The newest scan, if there is one.
pub fn latest_run(conn: &Connection) -> Result<Option<Run>> {
    Ok(conn
        .query_row(
            &format!("SELECT {RUN_COLUMNS} FROM vault_health_runs ORDER BY id DESC LIMIT 1"),
            [],
            row_to_run,
        )
        .optional()?)
}

/// The counts of the last `limit` scans, oldest first, for a trend line.
pub fn trend(conn: &Connection, limit: i64) -> Result<Vec<TrendPoint>> {
    let mut stmt = conn.prepare(
        "SELECT ran_at, dead + ambiguous + broken_anchors + duplicates + frontmatter, links, dead
         FROM (SELECT * FROM vault_health_runs ORDER BY id DESC LIMIT ?1) ORDER BY id ASC",
    )?;
    let rows = stmt.query_map([limit], |r| {
        Ok(TrendPoint {
            ran_at: r.get(0)?,
            problems: r.get(1)?,
            links: r.get(2)?,
            dead: r.get(3)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

// ─── Ignores ─────────────────────────────────────────────────────────────────

/// Leave one item out of the counts from now on. Ignoring it twice is fine.
pub fn add_ignore(conn: &Connection, kind: &str, key: &str, at: &str) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO vault_health_ignores (kind, key, ignored_at) VALUES (?1, ?2, ?3)",
        params![kind, key, at],
    )?;
    Ok(())
}

/// Count an ignored item again. Returns whether it had been ignored.
pub fn remove_ignore(conn: &Connection, kind: &str, key: &str) -> Result<bool> {
    let n = conn.execute(
        "DELETE FROM vault_health_ignores WHERE kind = ?1 AND key = ?2",
        params![kind, key],
    )?;
    Ok(n > 0)
}

/// Everything ignored, as `(kind, key)`.
pub fn ignores(conn: &Connection) -> Result<Vec<(String, String)>> {
    let mut stmt = conn.prepare("SELECT kind, key FROM vault_health_ignores ORDER BY kind, key")?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conn() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        crate::migrations::run(&c).unwrap();
        c
    }

    fn counts(dead: i64) -> RunCounts {
        RunCounts {
            notes: 10,
            links: 40,
            dead,
            ..RunCounts::default()
        }
    }

    #[test]
    fn the_newest_scan_comes_back_with_its_report() {
        let c = conn();
        assert!(latest_run(&c).unwrap().is_none());
        insert_run(&c, "2026-11-01T00:00:00Z", &counts(3), "{\"a\":1}").unwrap();
        let id = insert_run(&c, "2026-11-08T00:00:00Z", &counts(1), "{\"a\":2}").unwrap();
        let run = latest_run(&c).unwrap().unwrap();
        assert_eq!(run.id, id);
        assert_eq!(run.counts.dead, 1);
        assert_eq!(run.counts.problems(), 1);
        assert_eq!(run.report, "{\"a\":2}");
    }

    #[test]
    fn the_trend_is_oldest_first_and_limited() {
        let c = conn();
        for (i, dead) in [5, 4, 2, 0].iter().enumerate() {
            insert_run(
                &c,
                &format!("2026-11-0{}T00:00:00Z", i + 1),
                &counts(*dead),
                "{}",
            )
            .unwrap();
        }
        let t = trend(&c, 3).unwrap();
        assert_eq!(t.iter().map(|p| p.problems).collect::<Vec<_>>(), [4, 2, 0]);
        assert_eq!(t[0].ran_at, "2026-11-02T00:00:00Z");
    }

    #[test]
    fn only_the_newest_scans_are_kept() {
        let c = conn();
        for i in 0..(KEEP_RUNS + 5) {
            insert_run(&c, &format!("t{i}"), &counts(i), "{}").unwrap();
        }
        let n: i64 = c
            .query_row("SELECT COUNT(*) FROM vault_health_runs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, KEEP_RUNS);
        assert_eq!(latest_run(&c).unwrap().unwrap().counts.dead, KEEP_RUNS + 4);
    }

    #[test]
    fn ignoring_is_idempotent_and_can_be_undone() {
        let c = conn();
        add_ignore(&c, "dead_link", "Notebooks/a.md|[[X]]", "t1").unwrap();
        add_ignore(&c, "dead_link", "Notebooks/a.md|[[X]]", "t2").unwrap();
        add_ignore(&c, "loose", "Notebooks/b.md", "t3").unwrap();
        assert_eq!(ignores(&c).unwrap().len(), 2);
        assert!(remove_ignore(&c, "loose", "Notebooks/b.md").unwrap());
        assert!(!remove_ignore(&c, "loose", "Notebooks/b.md").unwrap());
        assert_eq!(
            ignores(&c).unwrap(),
            [("dead_link".to_string(), "Notebooks/a.md|[[X]]".to_string())]
        );
    }
}
