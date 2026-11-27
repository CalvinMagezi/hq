//! Running the vault health scan and keeping what it found.
//!
//! The daemon, `hq vault health`, the web and phone screens and the `vault_health` tool all go
//! through here, so they show the same report. The scan itself is `hq_vault::health`; this adds
//! what the owner chose to ignore, and storing the result in `hq_db::vault_health`. Reading is
//! cheap (twenty thousand notes take about half a second); nothing here changes a note.

use anyhow::Result;
use hq_db::Database;
use hq_db::vault_health::{FixSummary, RunCounts, TrendPoint};
use hq_vault::fixes::{self, Context, Plan};
use hq_vault::health::{self, Counts, Ignored, Report, SimilarPair};
use hq_vault::relink::{Snapshot, snapshot};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// A scan with the time it ran.
#[derive(Debug, Clone, Serialize)]
pub struct Stored {
    /// RFC 3339.
    pub ran_at: String,
    pub report: Report,
    /// What the scan found to repair: the first few fixes, as the web reads them.
    pub fixes: Vec<serde_json::Value>,
    /// Counts of the certain (a) and likely (b) fixes, all of them, not only those listed.
    pub fix_a: i64,
    pub fix_b: i64,
    /// The notes it had to leave alone, and why.
    pub skipped: Vec<serde_json::Value>,
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

/// What one scan found: the report and the repairs it could make.
pub struct Scanned {
    pub report: Report,
    pub plan: Plan,
}

/// The most fixes a stored scan carries: enough for a screen, not a whole vault's worth.
const MAX_STORED_FIXES: usize = 200;

/// Scan the vault, plan the repairs, and keep both. Reads every note: run it off the async
/// runtime. A note seen to be renamed since the last scan is remembered for 30 days, so a link
/// to its old name can be followed.
pub fn scan_and_store(vault_path: &Path, db: &Database) -> Result<Stored> {
    let ignored = load_ignored(db)?;
    let snap = snapshot(vault_path);
    let now = chrono::Utc::now();
    let scanned = scan_snapshot(&snap, db, &ignored, now)?;
    let ran_at = now.to_rfc3339();
    let json = serde_json::to_string(&scanned.report)?;
    let counts = run_counts(&scanned.report.counts);
    let (summary_a, summary_b) = {
        let (a, b) = scanned.plan.counts();
        (a as i64, b as i64)
    };
    let summary = FixSummary {
        a: summary_a,
        b: summary_b,
        fixes: serde_json::to_string(
            &scanned
                .plan
                .fixes
                .iter()
                .take(MAX_STORED_FIXES)
                .collect::<Vec<_>>(),
        )?,
        skipped: serde_json::to_string(&scanned.plan.skipped)?,
    };
    db.with_conn({
        let ran_at = ran_at.clone();
        move |conn| hq_db::vault_health::insert_run(conn, &ran_at, &counts, &json, &summary)
    })?;
    let fixes = scanned
        .plan
        .fixes
        .iter()
        .take(MAX_STORED_FIXES)
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()?;
    let skipped = scanned
        .plan
        .skipped
        .iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Stored {
        ran_at,
        report: scanned.report,
        fixes,
        fix_a: summary_a,
        fix_b: summary_b,
        skipped,
    })
}

/// Everything a scan does except writing its result.
pub fn scan_snapshot(
    snap: &Snapshot,
    db: &Database,
    ignored: &Ignored,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<Scanned> {
    let mut report = health::scan_full(snap);
    report.apply_ignores(ignored);

    // Notes that moved since the last scan, remembered so a link to the old name can follow.
    let current: Vec<(String, String)> = snap
        .notes
        .iter()
        .map(|n| (n.path.clone(), fixes::text_hash(&n.text)))
        .collect();
    let previous = db.with_conn(hq_db::vault_health::seen)?;
    let found = detect_renames(&previous, &current);
    let at = now.to_rfc3339();
    db.with_conn({
        let (found, current, at) = (found.clone(), current.clone(), at.clone());
        move |conn| {
            hq_db::vault_health::add_renames(conn, &found, &at)?;
            hq_db::vault_health::replace_seen(conn, &current)
        }
    })?;
    let renames: HashMap<String, String> = db
        .with_conn(move |conn| hq_db::vault_health::renames(conn, now))?
        .into_iter()
        .collect();

    let (plan, _) = plan_with(snap, db, ignored, renames)?;
    report.similar = near_duplicates(db, snap, &report);
    report.counts.similar = report.similar.len();
    report.counts.similar_checked = similar_checked(db, snap);
    report
        .similar
        .retain(|p| !ignored.contains(&(health::kind::SIMILAR.to_string(), p.key())));
    report.counts.similar = report.similar.len();
    report.cap();
    Ok(Scanned { report, plan })
}

/// Plan repairs for `snap` with what the database knows: renames, notes an assistant wrote that
/// nobody has reviewed, and links the owner chose to ignore.
fn plan_with(
    snap: &Snapshot,
    db: &Database,
    ignored: &Ignored,
    renames: HashMap<String, String>,
) -> Result<(Plan, Context)> {
    let mut ctx = Context {
        renames,
        ..Context::default()
    };
    // The first pass says which notes would be touched; only those are looked up in the
    // provenance table.
    let first = fixes::plan(snap, &ctx);
    let candidates: HashSet<&str> = first
        .fixes
        .iter()
        .flat_map(|f| f.bases.keys().map(String::as_str))
        .chain(first.skipped.iter().map(|s| s.note.as_str()))
        .collect();
    let texts: HashMap<&str, &str> = snap
        .notes
        .iter()
        .map(|n| (n.path.as_str(), n.text.as_str()))
        .collect();
    let looked_up: Vec<(String, String)> = candidates
        .iter()
        .filter_map(|p| texts.get(p).map(|t| (p.to_string(), fixes::text_hash(t))))
        .collect();
    ctx.agent_unreviewed = db.with_conn(move |conn| {
        let mut out = HashSet::new();
        for (path, hash) in looked_up {
            if let Some(p) = hq_db::provenance::current(conn, &path, &hash)?
                && p.source_kind == hq_db::provenance::SourceKind::Agent
                && matches!(
                    p.review_state,
                    hq_db::provenance::ReviewState::Unreviewed
                        | hq_db::provenance::ReviewState::Disputed
                )
            {
                out.insert(path);
            }
        }
        Ok(out)
    })?;
    let mut plan = fixes::plan(snap, &ctx);
    // A link the owner said to leave alone is not offered for repair either.
    plan.fixes.retain(|f| {
        let key = format!("{}|{}", f.note, f.before);
        ![health::kind::DEAD_LINK, health::kind::AMBIGUOUS_LINK]
            .iter()
            .any(|k| ignored.contains(&(k.to_string(), key.clone())))
    });
    Ok((plan, ctx))
}

/// Notes seen to move: a path that was there last time and is not now, whose exact text is
/// now at one path that is new. Anything less certain is not a rename.
pub fn detect_renames(
    previous: &[(String, String)],
    current: &[(String, String)],
) -> Vec<(String, String)> {
    let now_paths: HashSet<&str> = current.iter().map(|(p, _)| p.as_str()).collect();
    let before_paths: HashSet<&str> = previous.iter().map(|(p, _)| p.as_str()).collect();
    let mut gone: HashMap<&str, Vec<&str>> = HashMap::new();
    for (path, hash) in previous {
        if !now_paths.contains(path.as_str()) {
            gone.entry(hash).or_default().push(path);
        }
    }
    let mut added: HashMap<&str, Vec<&str>> = HashMap::new();
    for (path, hash) in current {
        if !before_paths.contains(path.as_str()) {
            added.entry(hash).or_default().push(path);
        }
    }
    let mut out: Vec<(String, String)> = gone
        .iter()
        .filter_map(
            |(hash, olds)| match (olds.as_slice(), added.get(hash)?.as_slice()) {
                ([old], [new]) => Some((old.to_string(), new.to_string())),
                _ => None,
            },
        )
        .collect();
    out.sort();
    out
}

// ─── Notes that say almost the same thing ────────────────────────────────────

/// Pairs closer than this by the on-device model are looked at more closely.
const SIMILAR_COSINE: f32 = 0.97;
/// And they are listed only if this share of their words is the same.
const SIMILAR_WORDS: f32 = 0.8;
/// Comparing every pair is quadratic; past this many notes the check does not run.
const SIMILAR_MAX_NOTES: usize = 6000;
/// The on-device model's name starts with this. A remote model is never used for this: it
/// would send the notes to a provider.
const LOCAL_MODEL_PREFIX: &str = "model2vec/";

fn embedded_notes(db: &Database, snap: &Snapshot) -> Vec<(String, Vec<f32>)> {
    let present: HashSet<&str> = snap.notes.iter().map(|n| n.path.as_str()).collect();
    db.with_conn(|conn| hq_db::vault_health::embeddings_by_model(conn, LOCAL_MODEL_PREFIX))
        .unwrap_or_default()
        .into_iter()
        .filter(|(p, v)| present.contains(p.as_str()) && !v.is_empty())
        .collect()
}

fn similar_checked(db: &Database, snap: &Snapshot) -> usize {
    let n = embedded_notes(db, snap).len();
    if n > SIMILAR_MAX_NOTES { 0 } else { n }
}

fn words(text: &str) -> HashSet<String> {
    hq_core::frontmatter_utils::split_frontmatter(text)
        .1
        .split_whitespace()
        .map(|w| {
            w.trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .filter(|w| !w.is_empty())
        .collect()
}

/// Notes the on-device model finds near each other, confirmed on their whole text. Notes with
/// no vector from that model, or in a group of exact repeats, are not compared.
pub fn near_duplicates(db: &Database, snap: &Snapshot, report: &Report) -> Vec<SimilarPair> {
    let rows = embedded_notes(db, snap);
    if rows.len() < 2 || rows.len() > SIMILAR_MAX_NOTES {
        return Vec::new();
    }
    let dim = rows[0].1.len();
    let exact: HashSet<(&str, &str)> = report
        .duplicates
        .iter()
        .flat_map(|g| {
            g.notes
                .iter()
                .flat_map(|a| g.notes.iter().map(move |b| (a.as_str(), b.as_str())))
        })
        .collect();
    let texts: HashMap<&str, &str> = snap
        .notes
        .iter()
        .map(|n| (n.path.as_str(), n.text.as_str()))
        .collect();
    let rows: Vec<&(String, Vec<f32>)> = rows.iter().filter(|(_, v)| v.len() == dim).collect();
    let mut flat: Vec<f32> = Vec::with_capacity(rows.len() * dim);
    for (_, v) in &rows {
        flat.extend_from_slice(v);
    }
    let mut out = Vec::new();
    for (i, (a, va)) in rows.iter().enumerate() {
        let scores = hq_db::search::batch_cosine_similarity(va, &flat[(i + 1) * dim..], dim);
        for (offset, score) in scores.into_iter().enumerate() {
            if score < SIMILAR_COSINE {
                continue;
            }
            let b = &rows[i + 1 + offset].0;
            if exact.contains(&(a.as_str(), b.as_str())) {
                continue;
            }
            let (Some(ta), Some(tb)) = (texts.get(a.as_str()), texts.get(b.as_str())) else {
                continue;
            };
            let (wa, wb) = (words(ta), words(tb));
            let (small, big) = if wa.len() <= wb.len() {
                (&wa, &wb)
            } else {
                (&wb, &wa)
            };
            if small.len() < 8 {
                continue;
            }
            let shared = small.iter().filter(|w| big.contains(*w)).count() as f32;
            let union = (wa.len() + wb.len()) as f32 - shared;
            let share = shared / union;
            if share >= SIMILAR_WORDS {
                out.push(SimilarPair {
                    a: a.to_string(),
                    b: b.to_string(),
                    percent: (share * 100.0).round().min(100.0) as u8,
                });
            }
        }
    }
    out.sort_by(|x, y| {
        y.percent
            .cmp(&x.percent)
            .then_with(|| x.key().cmp(&y.key()))
    });
    out
}

// ─── Repairing ───────────────────────────────────────────────────────────────

/// A fresh plan from the notes as they are now.
pub fn plan_now(vault_path: &Path, db: &Database) -> Result<(Plan, Context)> {
    let ignored = load_ignored(db)?;
    let snap = snapshot(vault_path);
    let renames: HashMap<String, String> = db
        .with_conn(|conn| hq_db::vault_health::renames(conn, chrono::Utc::now()))?
        .into_iter()
        .collect();
    plan_with(&snap, db, &ignored, renames)
}

// ─── The Inbox ───────────────────────────────────────────────────────────────

use chrono::Datelike;
use hq_core::types::{VAULT_HEALTH_SOURCE, ValueItem, ValueKind};

/// The week's note about the notes, and the question that goes with it.
const DIGEST_PREFIX: &str = "vault-health-";
/// The offer to fix what HQ is certain about. The week is in the key, so a "no" is asked again
/// only next week, and only if there is still something to fix.
pub const FIX_PREFIX: &str = "vault-fix-";
/// Asked once, after a first yes: keep doing it without asking.
pub const AUTO_OFFER_KEY: &str = "vault-auto-fix-offer";

fn week_key(now: chrono::DateTime<chrono::Utc>) -> String {
    let iso = now.iso_week();
    format!("{}-W{:02}", iso.year(), iso.week())
}

/// Whether HQ may fix the certain things on its own: the config says so, or the owner said yes in
/// the Inbox.
pub fn auto_fix_on(db: &Database, config_on: bool) -> bool {
    config_on || hq_db::owner_prefs::vault_auto_fix_on(db)
}

/// What the Inbox should hold after a scan. Nothing when there is nothing to do: a note about
/// notes that are in order would only teach the owner to stop reading these.
pub fn inbox_items(
    stored: &Stored,
    now: chrono::DateTime<chrono::Utc>,
    auto_on: bool,
) -> Vec<ValueItem> {
    let c = &stored.report.counts;
    let week = week_key(now);
    let mut out = Vec::new();
    if c.problems() > 0 {
        let mut parts = Vec::new();
        for (n, one, many) in [
            (
                c.links.dead,
                "link leads to a note that is not there",
                "links lead to notes that are not there",
            ),
            (
                c.links.ambiguous,
                "link could mean more than one note",
                "links could mean more than one note",
            ),
            (
                c.links.broken_anchors,
                "link points at a heading that is gone",
                "links point at headings that are gone",
            ),
            (
                c.duplicates,
                "set of notes has the same text",
                "sets of notes have the same text",
            ),
            (
                c.frontmatter,
                "note has properties HQ cannot read",
                "notes have properties HQ cannot read",
            ),
        ] {
            if n > 0 {
                parts.push(format!("{n} {}", if n == 1 { one } else { many }));
            }
        }
        out.push(
            ValueItem::new(
                VAULT_HEALTH_SOURCE,
                ValueKind::Fyi,
                "Your notes need a look".to_string(),
                format!(
                    "{}. Open Notes, then Review, to see where.",
                    parts.join(", ")
                ),
            )
            .with_dedup_key(format!("{DIGEST_PREFIX}{week}"))
            .with_link("/notes/health".to_string()),
        );
    }
    if stored.fix_a > 0 && !auto_on {
        let n = stored.fix_a;
        out.push(
            ValueItem::new(
                VAULT_HEALTH_SOURCE,
                ValueKind::ActionNeeded,
                if n == 1 {
                    "1 link in your notes can be fixed".to_string()
                } else {
                    format!("{n} links in your notes can be fixed")
                },
                "HQ is sure which note each one means. Approve to fix them. The original of every note is saved first, and you can undo it from Notes."
                    .to_string(),
            )
            .with_dedup_key(format!("{FIX_PREFIX}{week}"))
            .with_link("/notes/health".to_string())
            .owner_only(true),
        );
    }
    out
}

/// Put the Inbox in order after a scan: add what is new, and close this week's offer if there
/// is nothing left to fix (it was done by hand, or by the daemon).
pub fn sync_inbox(db: &Database, stored: &Stored, auto_on: bool) -> Result<usize> {
    let now = chrono::Utc::now();
    let items = inbox_items(stored, now, auto_on);
    let count = items.len();
    for item in &items {
        hq_db::value_items::emit(db, item)?;
    }
    let week = week_key(now);
    if stored.fix_a == 0 || auto_on {
        let key = format!("{FIX_PREFIX}{week}");
        db.with_conn(move |c| {
            hq_db::value_items::dismiss_open_by_dedup_on(c, VAULT_HEALTH_SOURCE, &key)
        })?;
    }
    if stored.report.counts.problems() == 0 {
        let key = format!("{DIGEST_PREFIX}{week}");
        db.with_conn(move |c| {
            hq_db::value_items::dismiss_open_by_dedup_on(c, VAULT_HEALTH_SOURCE, &key)
        })?;
    }
    Ok(count)
}

/// The last stored scan. A stored report this version cannot read counts as none, so the next
/// scan replaces it.
pub fn latest(db: &Database) -> Result<Option<Stored>> {
    let Some(run) = db.with_conn(hq_db::vault_health::latest_run)? else {
        return Ok(None);
    };
    let Ok(report) = serde_json::from_str::<Report>(&run.report) else {
        return Ok(None);
    };
    let list =
        |text: &str| serde_json::from_str::<Vec<serde_json::Value>>(text).unwrap_or_default();
    Ok(Some(Stored {
        ran_at: run.ran_at,
        report,
        fixes: list(&run.fixes),
        fix_a: run.fix_a,
        fix_b: run.fix_b,
        skipped: list(&run.skipped),
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

    // ─── Repairs ─────────────────────────────────────────────────────────────

    use std::sync::Arc;

    const WORDS: &str = "enough words in this note to count as a real note and not be empty";

    fn repair_vault(files: &[(&str, &str)]) -> (tempfile::TempDir, Arc<Database>) {
        let dir = tempfile::tempdir().unwrap();
        for (rel, text) in files {
            let p = dir.path().join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        (dir, Arc::new(Database::open_memory().unwrap()))
    }

    /// Notes are fresh files, and a real run leaves anything changed in the last ten minutes alone.
    /// Backdating them is how a test stands in for notes that have been sitting there.
    fn age(dir: &Path) {
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        let mut stack = vec![dir.join("Notebooks")];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap().flatten() {
                if e.path().is_dir() {
                    stack.push(e.path());
                } else {
                    std::fs::File::options()
                        .write(true)
                        .open(e.path())
                        .unwrap()
                        .set_modified(old)
                        .unwrap();
                }
            }
        }
    }

    #[test]
    fn a_link_the_owner_ignored_is_not_offered_for_repair() {
        let (v, db) = repair_vault(&[
            ("Notebooks/New/Plan.md", &format!("{WORDS}, plans")),
            ("Notebooks/L.md", "see [[Old/Plan]]"),
        ]);
        age(v.path());
        ignore(&db, health::kind::DEAD_LINK, "Notebooks/L.md|[[Old/Plan]]").unwrap();
        let (plan, _) = plan_now(v.path(), &db).unwrap();
        assert!(plan.fixes.is_empty());
    }

    // ─── Renames and near-duplicates ─────────────────────────────────────────

    fn pair(p: &str, h: &str) -> (String, String) {
        (p.to_string(), h.to_string())
    }

    #[test]
    fn only_a_clear_one_to_one_move_of_the_same_text_is_a_rename() {
        let before = [
            pair("a.md", "h1"),
            pair("b.md", "h2"),
            pair("c.md", "h3"),
            pair("d.md", "h3"),
        ];
        let now = [
            pair("a2.md", "h1"),
            pair("b2.md", "h9"),
            pair("e.md", "h3"),
            pair("f.md", "h3"),
        ];
        assert_eq!(detect_renames(&before, &now), [pair("a.md", "a2.md")]);
        assert!(
            detect_renames(&[], &now).is_empty(),
            "the first scan has nothing to compare"
        );
    }

    fn put_embedding(db: &Database, path: &str, model: &str, v: &[f32]) {
        let (path, model, blob) = (
            path.to_string(),
            model.to_string(),
            hq_db::search::embedding_to_bytes(v),
        );
        db.with_conn(move |c| {
            c.execute(
                "INSERT INTO embeddings (note_path, embedding, model, embedded_at, content_hash) VALUES (?1, ?2, ?3, 't', 'h')",
                rusqlite::params![path, blob, model],
            )?;
            c.execute("INSERT INTO notes_fts_rows (path, fts_rowid) VALUES (?1, 1)", [path])?;
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn near_duplicates_need_the_on_device_model_and_the_same_words() {
        let a = "The quarterly numbers for the shop, and what they mean for stock and for the next hiring round.";
        let b = "The quarterly numbers for the shop, and what they mean for stock and for the next hiring round soon.";
        let c = "A completely different note about a trip to the coast in the dry season with the whole family.";
        let (v, db) = repair_vault(&[
            ("Notebooks/A.md", a),
            ("Notebooks/B.md", b),
            ("Notebooks/C.md", c),
            ("Notebooks/R.md", b),
        ]);
        let local = "model2vec/potion-base-8M@x";
        // A and B are near each other, and C has the same vector as A but other words.
        put_embedding(&db, "Notebooks/A.md", local, &[1.0, 0.0, 0.1]);
        put_embedding(&db, "Notebooks/B.md", local, &[1.0, 0.0, 0.1]);
        put_embedding(&db, "Notebooks/C.md", local, &[1.0, 0.0, 0.1]);
        // R repeats B word for word, but a remote model made its vector: it is not compared.
        put_embedding(&db, "Notebooks/R.md", "nomic-embed-text", &[1.0, 0.0, 0.1]);
        let snap = snapshot(v.path());
        let report = health::scan_full(&snap);
        let pairs = near_duplicates(&db, &snap, &report);
        assert_eq!(pairs.len(), 1, "{pairs:?}");
        assert_eq!(
            (pairs[0].a.as_str(), pairs[0].b.as_str()),
            ("Notebooks/A.md", "Notebooks/B.md")
        );
        assert!(pairs[0].percent >= 80);
        assert_eq!(similar_checked(&db, &snap), 3);
    }

    #[test]
    fn with_no_on_device_vectors_nothing_is_compared_and_it_says_so() {
        let (v, db) = repair_vault(&[
            ("Notebooks/A.md", "words words words"),
            ("Notebooks/B.md", "words words words"),
        ]);
        put_embedding(&db, "Notebooks/A.md", "nomic-embed-text", &[1.0, 0.0]);
        put_embedding(&db, "Notebooks/B.md", "nomic-embed-text", &[1.0, 0.0]);
        let stored = scan_and_store(v.path(), &db).unwrap();
        assert_eq!(stored.report.counts.similar_checked, 0);
        assert!(stored.report.similar.is_empty());
    }

    // ─── The Inbox ───────────────────────────────────────────────────────────

    fn stored_with(fix_a: i64, dead: usize) -> Stored {
        let mut report = Report::default();
        report.counts.links.dead = dead;
        Stored {
            ran_at: "2026-11-27T10:00:00Z".into(),
            report,
            fixes: vec![],
            fix_a,
            fix_b: 0,
            skipped: vec![],
        }
    }

    fn open_items(db: &Database) -> Vec<(String, String)> {
        db.with_conn(|c| {
            let mut stmt = c.prepare(
                "SELECT dedup_key, kind FROM value_items WHERE source_task = 'vault_health'
                 AND state IN ('pending','routed','delivered') ORDER BY dedup_key",
            )?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
        .unwrap()
    }

    #[test]
    fn a_healthy_vault_puts_nothing_in_the_inbox() {
        let now = chrono::Utc::now();
        assert!(inbox_items(&stored_with(0, 0), now, false).is_empty());
    }

    #[test]
    fn something_to_look_at_is_one_note_and_something_to_fix_is_one_question_and_both_stay_on_the_device()
     {
        let now = chrono::Utc::now();
        let items = inbox_items(&stored_with(3, 4), now, false);
        assert_eq!(items.len(), 2);
        let digest = items
            .iter()
            .find(|i| i.title == "Your notes need a look")
            .unwrap();
        assert_eq!(digest.kind, ValueKind::Fyi);
        assert!(
            digest
                .body
                .contains("4 links lead to notes that are not there"),
            "{}",
            digest.body
        );
        let ask = items
            .iter()
            .find(|i| i.kind == ValueKind::ActionNeeded)
            .unwrap();
        assert_eq!(ask.title, "3 links in your notes can be fixed");
        assert!(ask.owner_only);
        for i in &items {
            assert!(
                i.is_device_only(),
                "it names notes, so no push service sees it"
            );
            assert_eq!(i.link.as_deref(), Some("/notes/health"));
            assert!(!i.title.contains('\u{2014}') && !i.body.contains('\u{2014}'));
        }
        assert_eq!(
            inbox_items(&stored_with(1, 1), now, false)[1].title,
            "1 link in your notes can be fixed"
        );
    }

    #[test]
    fn nothing_is_asked_when_the_owner_already_allowed_it() {
        let items = inbox_items(&stored_with(3, 4), chrono::Utc::now(), true);
        assert!(items.iter().all(|i| i.kind != ValueKind::ActionNeeded));
    }

    #[test]
    fn the_week_is_asked_about_once_and_closed_when_there_is_nothing_left_to_fix() {
        let db = Database::open_memory().unwrap();
        sync_inbox(&db, &stored_with(2, 2), false).unwrap();
        sync_inbox(&db, &stored_with(2, 2), false).unwrap();
        assert_eq!(
            open_items(&db).len(),
            2,
            "the same week adds nothing the second time"
        );
        // Fixed by hand: both close.
        sync_inbox(&db, &stored_with(0, 0), false).unwrap();
        assert!(open_items(&db).is_empty());
    }
}
