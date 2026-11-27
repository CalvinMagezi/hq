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

/// What a scan found to repair, as stored with it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FixSummary {
    pub a: i64,
    pub b: i64,
    /// The fixes as JSON, as the web reads them.
    pub fixes: String,
    /// The notes it had to leave alone, as JSON.
    pub skipped: String,
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
    pub fix_a: i64,
    pub fix_b: i64,
    /// The fixes found, as JSON text.
    pub fixes: String,
    /// The notes left alone, as JSON text.
    pub skipped: String,
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
    fixes: &FixSummary,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO vault_health_runs
            (ran_at, notes, links, dead, ambiguous, broken_anchors, loose, duplicates, empty,
             frontmatter, unread, report, fix_a, fix_b, fixes, skipped)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
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
            report,
            fixes.a,
            fixes.b,
            fixes.fixes,
            fixes.skipped
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
        fix_a: row.get(13)?,
        fix_b: row.get(14)?,
        fixes: row.get(15)?,
        skipped: row.get(16)?,
    })
}

const RUN_COLUMNS: &str = "id, ran_at, notes, links, dead, ambiguous, broken_anchors, loose, \
                           duplicates, empty, frontmatter, unread, report, fix_a, fix_b, fixes, \
                           skipped";

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

// ─── Repairs ─────────────────────────────────────────────────────────────────

/// Who started a repair run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    /// A person: a tap in the app or a command.
    Owner,
    /// The daemon, with the owner's standing permission.
    Auto,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Owner => "owner",
            Source::Auto => "auto",
        }
    }
}

/// One repair run as stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Repair {
    pub run_id: String,
    /// The folder with its saved copies, under `_trash/`.
    pub dir: String,
    pub ran_at: String,
    pub source: String,
    pub fixes: i64,
    pub rewritten: i64,
    pub trashed: i64,
    pub undone_at: Option<String>,
    /// How many notes undo left alone because they were edited since.
    pub edited_since: i64,
}

pub fn insert_repair(conn: &Connection, r: &Repair) -> Result<()> {
    conn.execute(
        "INSERT INTO vault_repairs
            (run_id, dir, ran_at, source, fixes, rewritten, trashed, undone_at, edited_since)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            r.run_id,
            r.dir,
            r.ran_at,
            r.source,
            r.fixes,
            r.rewritten,
            r.trashed,
            r.undone_at,
            r.edited_since
        ],
    )?;
    Ok(())
}

fn row_to_repair(row: &rusqlite::Row<'_>) -> rusqlite::Result<Repair> {
    Ok(Repair {
        run_id: row.get(0)?,
        dir: row.get(1)?,
        ran_at: row.get(2)?,
        source: row.get(3)?,
        fixes: row.get(4)?,
        rewritten: row.get(5)?,
        trashed: row.get(6)?,
        undone_at: row.get(7)?,
        edited_since: row.get(8)?,
    })
}

const REPAIR_COLUMNS: &str =
    "run_id, dir, ran_at, source, fixes, rewritten, trashed, undone_at, edited_since";

pub fn repair(conn: &Connection, run_id: &str) -> Result<Option<Repair>> {
    Ok(conn
        .query_row(
            &format!("SELECT {REPAIR_COLUMNS} FROM vault_repairs WHERE run_id = ?1"),
            [run_id],
            row_to_repair,
        )
        .optional()?)
}

/// The newest runs first.
pub fn repairs(conn: &Connection, limit: i64) -> Result<Vec<Repair>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {REPAIR_COLUMNS} FROM vault_repairs ORDER BY ran_at DESC, run_id DESC LIMIT ?1"
    ))?;
    let rows = stmt.query_map([limit], row_to_repair)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Mark a run taken back. Returns false when it was not found or already undone.
pub fn mark_undone(conn: &Connection, run_id: &str, at: &str, edited_since: i64) -> Result<bool> {
    let n = conn.execute(
        "UPDATE vault_repairs SET undone_at = ?2, edited_since = ?3
         WHERE run_id = ?1 AND undone_at IS NULL",
        params![run_id, at, edited_since],
    )?;
    Ok(n > 0)
}

/// Of the daemon's last `window` runs, how many there were and how many a person took back.
/// A person undoing what HQ did on its own is the signal that it is doing the wrong thing.
pub fn auto_undo_rate(conn: &Connection, window: i64) -> Result<(i64, i64)> {
    let (total, undone) = conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(undone_at IS NOT NULL), 0) FROM
            (SELECT undone_at FROM vault_repairs WHERE source = 'auto' ORDER BY ran_at DESC LIMIT ?1)",
        [window],
        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
    )?;
    Ok((total, undone))
}

// ─── What the last scan saw ──────────────────────────────────────────────────

/// Each note's path and hash as the last scan saw them.
pub fn seen(conn: &Connection) -> Result<Vec<(String, String)>> {
    let mut stmt = conn.prepare("SELECT path, hash FROM vault_health_seen")?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Replace what the scan saw with `now`.
pub fn replace_seen(conn: &Connection, now: &[(String, String)]) -> Result<()> {
    conn.execute_batch("SAVEPOINT replace_seen")?;
    let result = (|| -> Result<()> {
        conn.execute("DELETE FROM vault_health_seen", [])?;
        let mut stmt =
            conn.prepare("INSERT OR REPLACE INTO vault_health_seen (path, hash) VALUES (?1, ?2)")?;
        for (path, hash) in now {
            stmt.execute(params![path, hash])?;
        }
        Ok(())
    })();
    match result {
        Ok(()) => {
            conn.execute_batch("RELEASE replace_seen")?;
            Ok(())
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK TO replace_seen; RELEASE replace_seen");
            Err(e)
        }
    }
}

/// How long a rename is remembered.
pub const RENAME_DAYS: i64 = 30;

pub fn add_renames(conn: &Connection, renames: &[(String, String)], at: &str) -> Result<()> {
    for (old, new) in renames {
        conn.execute(
            "INSERT OR REPLACE INTO vault_health_renames (old_path, new_path, seen_at) VALUES (?1, ?2, ?3)",
            params![old, new, at],
        )?;
    }
    Ok(())
}

/// Renames seen in the last [`RENAME_DAYS`] days, as `(old, new)`. Older ones are dropped.
pub fn renames(
    conn: &Connection,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<Vec<(String, String)>> {
    let cutoff = (now - chrono::Duration::days(RENAME_DAYS)).to_rfc3339();
    conn.execute(
        "DELETE FROM vault_health_renames WHERE seen_at < ?1",
        [&cutoff],
    )?;
    let mut stmt = conn.prepare(
        "SELECT old_path, new_path FROM vault_health_renames ORDER BY seen_at, old_path",
    )?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Every stored embedding made by a model whose name starts with `model_prefix`, as
/// `(note path, vector)`. For comparing notes with each other, on the device.
pub fn embeddings_by_model(
    conn: &Connection,
    model_prefix: &str,
) -> Result<Vec<(String, Vec<f32>)>> {
    let mut stmt = conn.prepare(
        "SELECT e.note_path, e.embedding FROM embeddings e
         JOIN notes_fts_rows r ON r.path = e.note_path
         WHERE e.model LIKE ?1 || '%' ORDER BY e.note_path",
    )?;
    let rows = stmt.query_map([model_prefix], |r| {
        let blob: Vec<u8> = r.get(1)?;
        Ok((
            r.get::<_, String>(0)?,
            crate::search::bytes_to_embedding(&blob),
        ))
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
        insert_run(
            &c,
            "2026-11-01T00:00:00Z",
            &counts(3),
            "{\"a\":1}",
            &FixSummary::default(),
        )
        .unwrap();
        let id = insert_run(
            &c,
            "2026-11-08T00:00:00Z",
            &counts(1),
            "{\"a\":2}",
            &FixSummary {
                a: 2,
                b: 1,
                fixes: "[1]".into(),
                skipped: "[]".into(),
            },
        )
        .unwrap();
        let run = latest_run(&c).unwrap().unwrap();
        assert_eq!(run.id, id);
        assert_eq!(run.counts.dead, 1);
        assert_eq!(run.counts.problems(), 1);
        assert_eq!(run.report, "{\"a\":2}");
        assert_eq!((run.fix_a, run.fix_b, run.fixes.as_str()), (2, 1, "[1]"));
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
                &FixSummary::default(),
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
            insert_run(
                &c,
                &format!("t{i}"),
                &counts(i),
                "{}",
                &FixSummary::default(),
            )
            .unwrap();
        }
        let n: i64 = c
            .query_row("SELECT COUNT(*) FROM vault_health_runs", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, KEEP_RUNS);
        assert_eq!(latest_run(&c).unwrap().unwrap().counts.dead, KEEP_RUNS + 4);
    }

    fn repair_row(id: &str, at: &str, source: &str) -> Repair {
        Repair {
            run_id: id.into(),
            dir: format!("_trash/2026-11-27/link-repair/{id}"),
            ran_at: at.into(),
            source: source.into(),
            fixes: 3,
            rewritten: 2,
            trashed: 0,
            undone_at: None,
            edited_since: 0,
        }
    }

    #[test]
    fn a_repair_is_stored_listed_newest_first_and_undone_once() {
        let c = conn();
        insert_repair(&c, &repair_row("a", "2026-11-01T00:00:00Z", "owner")).unwrap();
        insert_repair(&c, &repair_row("b", "2026-11-02T00:00:00Z", "auto")).unwrap();
        let all = repairs(&c, 10).unwrap();
        assert_eq!(
            all.iter().map(|r| r.run_id.as_str()).collect::<Vec<_>>(),
            ["b", "a"]
        );
        assert!(mark_undone(&c, "a", "2026-11-03T00:00:00Z", 1).unwrap());
        assert!(
            !mark_undone(&c, "a", "2026-11-04T00:00:00Z", 0).unwrap(),
            "only the first undo counts"
        );
        let a = repair(&c, "a").unwrap().unwrap();
        assert_eq!(
            (a.undone_at.as_deref(), a.edited_since),
            (Some("2026-11-03T00:00:00Z"), 1)
        );
        assert!(repair(&c, "zzz").unwrap().is_none());
    }

    #[test]
    fn the_undo_rate_counts_only_what_the_daemon_did() {
        let c = conn();
        for (i, source) in ["owner", "auto", "auto", "auto", "auto"].iter().enumerate() {
            insert_repair(
                &c,
                &repair_row(
                    &format!("r{i}"),
                    &format!("2026-11-0{}T00:00:00Z", i + 1),
                    source,
                ),
            )
            .unwrap();
        }
        mark_undone(&c, "r0", "t", 0).unwrap();
        mark_undone(&c, "r2", "t", 0).unwrap();
        assert_eq!(auto_undo_rate(&c, 10).unwrap(), (4, 1));
        assert_eq!(
            auto_undo_rate(&c, 2).unwrap(),
            (2, 0),
            "the newest two runs"
        );
    }

    #[test]
    fn what_a_scan_saw_is_replaced_whole_and_renames_expire() {
        let c = conn();
        replace_seen(&c, &[("a".into(), "h1".into()), ("b".into(), "h2".into())]).unwrap();
        replace_seen(&c, &[("c".into(), "h3".into())]).unwrap();
        assert_eq!(seen(&c).unwrap(), [("c".to_string(), "h3".to_string())]);

        let now = chrono::Utc::now();
        add_renames(&c, &[("old".into(), "new".into())], &now.to_rfc3339()).unwrap();
        let stale = (now - chrono::Duration::days(RENAME_DAYS + 1)).to_rfc3339();
        add_renames(&c, &[("older".into(), "newer".into())], &stale).unwrap();
        assert_eq!(
            renames(&c, now).unwrap(),
            [("old".to_string(), "new".to_string())]
        );
        assert_eq!(
            renames(&c, now).unwrap().len(),
            1,
            "the expired one is gone for good"
        );
    }

    #[test]
    fn embeddings_are_read_by_model_and_only_for_notes_still_indexed() {
        let c = conn();
        let blob = crate::search::embedding_to_bytes(&[0.5, 0.25]);
        for (path, model) in [
            ("Notebooks/a.md", "model2vec/x@1"),
            ("Notebooks/b.md", "nomic"),
            ("Notebooks/gone.md", "model2vec/x@1"),
        ] {
            c.execute(
                "INSERT INTO embeddings (note_path, embedding, model, embedded_at, content_hash) VALUES (?1, ?2, ?3, 't', 'h')",
                params![path, blob, model],
            )
            .unwrap();
        }
        for path in ["Notebooks/a.md", "Notebooks/b.md"] {
            c.execute(
                "INSERT INTO notes_fts_rows (path, fts_rowid) VALUES (?1, 1)",
                [path],
            )
            .unwrap();
        }
        let got = embeddings_by_model(&c, "model2vec/").unwrap();
        assert_eq!(got, [("Notebooks/a.md".to_string(), vec![0.5, 0.25])]);
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
