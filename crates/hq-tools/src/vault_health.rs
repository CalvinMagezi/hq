//! Running the vault health scan and keeping what it found.
//!
//! The daemon, `hq vault health`, the web and phone screens and the `vault_health` tool all go
//! through here, so they show the same report. The scan itself is `hq_vault::health`; this adds
//! what the owner chose to ignore, and storing the result in `hq_db::vault_health`. Reading is
//! cheap (twenty thousand notes take about half a second); nothing here changes a note.

use anyhow::Result;
use hq_db::Database;
use hq_db::vault_health::{RunCounts, TrendPoint};
use hq_vault::health::{self, Counts, Ignored, Report};
use serde::Serialize;
use std::path::Path;

/// A scan with the time it ran.
#[derive(Debug, Clone, Serialize)]
pub struct Stored {
    /// RFC 3339.
    pub ran_at: String,
    pub report: Report,
}

/// What the owner chose to leave alone.
pub fn load_ignored(db: &Database) -> Result<Ignored> {
    Ok(db
        .with_conn(hq_db::vault_health::ignores)?
        .into_iter()
        .collect())
}

/// Scan the vault now: ignored items dropped, the lists capped, the counts exact. Reads every
/// note; run it off the async runtime. Nothing is stored.
pub fn scan_now(vault_path: &Path, ignored: &Ignored) -> Report {
    let mut report = scan_full_now(vault_path, ignored);
    report.cap();
    report
}

/// Like [`scan_now`] with every example listed, for a caller that filters before it shows.
pub fn scan_full_now(vault_path: &Path, ignored: &Ignored) -> Report {
    let mut report = health::scan_full(&hq_vault::relink::snapshot(vault_path));
    report.apply_ignores(ignored);
    report
}

/// Scan and keep the result. Returns what was stored.
pub fn scan_and_store(vault_path: &Path, db: &Database) -> Result<Stored> {
    let ignored = load_ignored(db)?;
    let report = scan_now(vault_path, &ignored);
    let ran_at = chrono::Utc::now().to_rfc3339();
    let json = serde_json::to_string(&report)?;
    let counts = run_counts(&report.counts);
    db.with_conn({
        let ran_at = ran_at.clone();
        move |conn| hq_db::vault_health::insert_run(conn, &ran_at, &counts, &json)
    })?;
    Ok(Stored { ran_at, report })
}

/// The last stored scan. A stored report this version cannot read counts as none, so the next
/// scan replaces it.
pub fn latest(db: &Database) -> Result<Option<Stored>> {
    let Some(run) = db.with_conn(hq_db::vault_health::latest_run)? else {
        return Ok(None);
    };
    Ok(serde_json::from_str::<Report>(&run.report)
        .ok()
        .map(|report| Stored {
            ran_at: run.ran_at,
            report,
        }))
}

/// The last `limit` scans' counts, oldest first.
pub fn trend(db: &Database, limit: i64) -> Result<Vec<TrendPoint>> {
    db.with_conn(move |conn| hq_db::vault_health::trend(conn, limit))
}

/// Leave one item out of the report from the next scan on.
pub fn ignore(db: &Database, kind: &str, key: &str) -> Result<()> {
    let (kind, key) = (kind.to_string(), key.to_string());
    let at = chrono::Utc::now().to_rfc3339();
    db.with_conn(move |conn| hq_db::vault_health::add_ignore(conn, &kind, &key, &at))
}

/// Count an ignored item again from the next scan on. Returns whether it had been ignored.
pub fn unignore(db: &Database, kind: &str, key: &str) -> Result<bool> {
    let (kind, key) = (kind.to_string(), key.to_string());
    db.with_conn(move |conn| hq_db::vault_health::remove_ignore(conn, &kind, &key))
}

fn run_counts(c: &Counts) -> RunCounts {
    let n = |v: usize| v as i64;
    RunCounts {
        notes: n(c.notes),
        links: n(c.links.total),
        dead: n(c.links.dead),
        ambiguous: n(c.links.ambiguous),
        broken_anchors: n(c.links.broken_anchors),
        loose: n(c.loose),
        duplicates: n(c.duplicates),
        empty: n(c.empty),
        frontmatter: n(c.frontmatter),
        unread: n(c.unread),
    }
}

/// The report as a note a person can read in the vault (`_system/VAULT-HEALTH.md`).
pub fn render_markdown(stored: &Stored) -> String {
    let r = &stored.report;
    let c = &r.counts;
    let mut out = format!(
        "---\nchecked_at: {at}\nnotes: {notes}\nlinks: {links}\ndead_links: {dead}\n\
         ambiguous_links: {amb}\nbroken_anchors: {anchors}\nloose_notes: {loose}\n\
         duplicate_groups: {dup}\nempty_notes: {empty}\nfrontmatter_problems: {fm}\n---\n\n\
         # Vault health\n\nLast check: {at}. HQ rewrites this note at every check; edits here are lost.\n\n",
        at = stored.ran_at,
        notes = c.notes,
        links = c.links.total,
        dead = c.links.dead,
        amb = c.links.ambiguous,
        anchors = c.links.broken_anchors,
        loose = c.loose,
        dup = c.duplicates,
        empty = c.empty,
        fm = c.frontmatter,
    );
    if c.problems() == 0 {
        out.push_str("Every link reaches a note, and nothing needs fixing.\n");
    }
    let mut section = |title: &str, lines: Vec<String>, total: usize| {
        if total == 0 {
            return;
        }
        out.push_str(&format!("## {title} ({total})\n\n"));
        for l in lines.iter().take(20) {
            out.push_str(&format!("- {l}\n"));
        }
        if total > 20 {
            out.push_str(&format!("- and {} more\n", total - 20));
        }
        out.push('\n');
    };
    section(
        "Links that reach no note",
        r.dead_links
            .iter()
            .map(|d| match &d.did_you_mean {
                Some(near) => format!(
                    "`{}` line {}: `{}` (did you mean `{near}`?)",
                    d.note, d.line, d.link
                ),
                None => format!("`{}` line {}: `{}`", d.note, d.line, d.link),
            })
            .collect(),
        c.links.dead,
    );
    section(
        "Links that fit more than one note",
        r.ambiguous_links
            .iter()
            .map(|a| {
                format!(
                    "`{}` line {}: `{}` ({} notes)",
                    a.note,
                    a.line,
                    a.link,
                    a.candidates.len()
                )
            })
            .collect(),
        c.links.ambiguous,
    );
    section(
        "Links to a heading that is not there",
        r.broken_anchors
            .iter()
            .map(|b| format!("`{}` line {}: `{}`", b.note, b.line, b.link))
            .collect(),
        c.links.broken_anchors,
    );
    section(
        "Notes with the same text",
        r.duplicates
            .iter()
            .map(|g| {
                g.notes
                    .iter()
                    .map(|n| format!("`{n}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .collect(),
        c.duplicates,
    );
    section(
        "Properties that do not read",
        r.frontmatter
            .iter()
            .map(|f| format!("`{}`: {:?}", f.note, f.problem))
            .collect(),
        c.frontmatter,
    );
    section(
        "Notes nothing links to",
        r.loose.iter().map(|p| format!("`{p}`")).collect(),
        c.loose,
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vault() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (rel, text) in [
            (
                "Notebooks/A.md",
                "see [[Gone]] and [[B]], with enough words to be a real note",
            ),
            (
                "Notebooks/B.md",
                "linked from A, with enough words to be a real note too",
            ),
        ] {
            let p = dir.path().join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        dir
    }

    #[test]
    fn a_scan_is_stored_and_read_back_whole() {
        let v = vault();
        let db = Database::open_memory().unwrap();
        assert!(latest(&db).unwrap().is_none());
        let stored = scan_and_store(v.path(), &db).unwrap();
        assert_eq!(stored.report.counts.links.dead, 1);
        let back = latest(&db).unwrap().expect("stored");
        assert_eq!(back.report, stored.report);
        assert_eq!(back.ran_at, stored.ran_at);
        assert_eq!(trend(&db, 10).unwrap().len(), 1);
    }

    #[test]
    fn an_ignored_item_is_left_out_of_the_next_scan_until_it_is_counted_again() {
        let v = vault();
        let db = Database::open_memory().unwrap();
        let first = scan_and_store(v.path(), &db).unwrap();
        let key = first.report.dead_links[0].key();
        ignore(&db, health::kind::DEAD_LINK, &key).unwrap();
        let second = scan_and_store(v.path(), &db).unwrap();
        assert_eq!(second.report.counts.problems(), 0);
        assert!(unignore(&db, health::kind::DEAD_LINK, &key).unwrap());
        assert_eq!(
            scan_and_store(v.path(), &db)
                .unwrap()
                .report
                .counts
                .links
                .dead,
            1
        );
    }

    #[test]
    fn the_readable_note_names_each_problem_and_says_when_there_is_none() {
        let v = vault();
        let db = Database::open_memory().unwrap();
        let text = render_markdown(&scan_and_store(v.path(), &db).unwrap());
        assert!(text.contains("## Links that reach no note (1)"), "{text}");
        assert!(
            text.contains("`Notebooks/A.md` line 1: `[[Gone]]`"),
            "{text}"
        );
        assert!(
            !text.contains('\u{2014}'),
            "no em dashes in a note HQ writes"
        );

        let clean = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(clean.path().join("Notebooks")).unwrap();
        std::fs::write(
            clean.path().join("Notebooks/Only.md"),
            "[[Only]] is a note about itself, long enough",
        )
        .unwrap();
        let text = render_markdown(&scan_and_store(clean.path(), &db).unwrap());
        assert!(text.contains("nothing needs fixing"), "{text}");
    }
}
