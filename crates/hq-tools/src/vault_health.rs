//! Running the vault health scan and keeping what it found.
//!
//! The daemon, `hq vault health`, the web and phone screens and the `vault_health` tool all go
//! through here, so they show the same report. The scan itself is `hq_vault::health`; this adds
//! what the owner chose to ignore, and storing the result in `hq_db::vault_health`. Reading is
//! cheap (twenty thousand notes take about half a second); nothing here changes a note.

use anyhow::{Result, bail};
use hq_db::Database;
use hq_db::vault_health::{FixSummary, Repair, RunCounts, Source, TrendPoint};
use hq_vault::fixes::{self, Context, Plan};
use hq_vault::health::{self, Counts, Ignored, Report, SimilarPair};
use hq_vault::relink::{Snapshot, snapshot};
use hq_vault::repair::{self, Options};
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

/// Which fixes to do.
pub enum Which {
    /// These, by id (as the last scan listed them).
    Ids(HashSet<String>),
    /// Every tier A fix: the ones that are certain.
    TierA,
    /// Every fix, the likely ones too.
    All,
}

/// What a repair run did, and the row kept for undoing it.
#[derive(Debug, Serialize)]
pub struct Applied {
    #[serde(flatten)]
    pub done: repair::Done,
    pub repair: Option<Repair>,
}

/// A run's notes are written one at a time and its index is updated afterwards; two runs at once
/// would interleave both.
static RUNNING: std::sync::Mutex<()> = std::sync::Mutex::new(());

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

/// Do the chosen fixes: plan again from the notes as they are, so an id from an old scan that no
/// longer fits is simply not done. Each rewritten note is saved first, re-indexed, and recorded
/// as written by HQ's own upkeep; a removed note leaves search and the trash keeps it.
pub fn apply_repairs(
    vault_path: &Path,
    db: &std::sync::Arc<Database>,
    which: &Which,
    source: Source,
) -> Result<Applied> {
    let _only_one = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
    let (plan, ctx) = plan_now(vault_path, db)?;
    let chosen: Vec<&hq_vault::fixes::Fix> = match which {
        Which::Ids(ids) => plan.select(ids),
        Which::TierA => plan.tier_a(),
        Which::All => plan.fixes.iter().collect(),
    };
    let now = chrono::Local::now();
    let opts = Options {
        run_id: format!(
            "{}-{:04x}",
            now.format("%Y%m%d-%H%M%S"),
            now.timestamp_subsec_nanos() & 0xffff
        ),
        day: now.format("%Y-%m-%d").to_string(),
        now: std::time::SystemTime::now(),
        quiet_secs: QUIET_SECS,
        max_notes: MAX_NOTES_PER_RUN,
    };
    let done = repair::apply(vault_path, &chosen, &ctx, &opts)?;

    let recorder = crate::provenance::Recorder::new(db.clone(), vault_path.to_path_buf());
    let writer = crate::provenance::Writer::system("vault-health");
    for rel in &done.rewritten {
        recorder.record(rel, &writer);
        let (vault, rel) = (vault_path.to_path_buf(), rel.clone());
        let _ = db.with_conn(move |c| hq_db::search::reconcile_path(c, &vault, &rel).map(|_| ()));
    }
    let manifest = if done.dir.is_empty() {
        None
    } else {
        std::fs::read_to_string(vault_path.join(&done.dir).join("manifest.json"))
            .ok()
            .and_then(|t| serde_json::from_str::<repair::Manifest>(&t).ok())
    };
    if let Some(m) = &manifest {
        for t in &m.trashed {
            crate::vault_reorg::forget_trashed(
                vault_path,
                Some(db),
                &t.note,
                &t.trash_path,
                source == Source::Owner,
            );
        }
    }
    let row = (!done.dir.is_empty()).then(|| Repair {
        run_id: done.run_id.clone(),
        dir: done.dir.clone(),
        ran_at: chrono::Utc::now().to_rfc3339(),
        source: source.as_str().to_string(),
        fixes: done.fixes.len() as i64,
        rewritten: done.rewritten.len() as i64,
        trashed: done.trashed.len() as i64,
        undone_at: None,
        edited_since: 0,
    });
    if let Some(row) = &row {
        let row = row.clone();
        db.with_conn(move |c| hq_db::vault_health::insert_repair(c, &row))?;
    }
    Ok(Applied { done, repair: row })
}

/// Take back a run. Notes edited since are left as they are and counted.
pub fn undo_repair(
    vault_path: &Path,
    db: &std::sync::Arc<Database>,
    run_id: &str,
) -> Result<repair::Undone> {
    let _only_one = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
    let id = run_id.to_string();
    let Some(row) = db.with_conn(move |c| hq_db::vault_health::repair(c, &id))? else {
        bail!("no such repair");
    };
    if row.undone_at.is_some() {
        bail!("that repair was already taken back");
    }
    let undone = repair::undo(vault_path, &row.dir)?;
    let recorder = crate::provenance::Recorder::new(db.clone(), vault_path.to_path_buf());
    let writer = crate::provenance::Writer::system("vault-health-undo");
    for rel in undone.restored.iter().chain(undone.untrashed.iter()) {
        recorder.record(rel, &writer);
        let (vault, rel) = (vault_path.to_path_buf(), rel.clone());
        let _ = db.with_conn(move |c| hq_db::search::reconcile_path(c, &vault, &rel).map(|_| ()));
    }
    let (id, at, left) = (
        run_id.to_string(),
        chrono::Utc::now().to_rfc3339(),
        undone.edited_since.len() as i64,
    );
    db.with_conn(move |c| hq_db::vault_health::mark_undone(c, &id, &at, left))?;
    Ok(undone)
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

/// What the owner's answer to one of these Inbox items does. `val_id` is the item; only an
/// approval acts, and only on this feature's own items.
///
/// - the offer to fix: do the certain fixes now, then ask once whether to keep doing it;
/// - the offer to keep fixing: remember the yes.
pub fn answer_inbox(
    vault_path: &Path,
    db: &std::sync::Arc<Database>,
    val_id: &str,
    approved: bool,
) -> Result<()> {
    if !approved {
        return Ok(());
    }
    let id = val_id.to_string();
    let found: Option<(String, String)> = db.with_conn(move |c| {
        use rusqlite::OptionalExtension;
        Ok(c.query_row(
            "SELECT source_task, COALESCE(dedup_key, '') FROM value_items WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?)
    })?;
    let Some((source, key)) = found else {
        return Ok(());
    };
    if source != VAULT_HEALTH_SOURCE {
        return Ok(());
    }
    if key.starts_with(FIX_PREFIX) {
        apply_repairs(vault_path, db, &Which::TierA, Source::Owner)?;
        if !hq_db::value_items::any_with_dedup(db, AUTO_OFFER_KEY)? {
            hq_db::value_items::emit(
                db,
                &ValueItem::new(
                    VAULT_HEALTH_SOURCE,
                    ValueKind::ActionNeeded,
                    "Fix these automatically from now on?".to_string(),
                    "Next time HQ finds a link it is sure about, it can fix it without asking. It still saves the original, and you can undo any of it. Approve to allow this, or leave it to keep being asked."
                        .to_string(),
                )
                .with_dedup_key(AUTO_OFFER_KEY.to_string())
                .with_link("/notes/health".to_string())
                .owner_only(true),
            )?;
        }
    } else if key == AUTO_OFFER_KEY {
        hq_db::owner_prefs::set(db, hq_db::owner_prefs::VAULT_AUTO_FIX, "true")?;
    }
    Ok(())
}

/// A note edited more recently than this is not touched.
const QUIET_SECS: u64 = 600;
/// The most notes one run changes.
pub const MAX_NOTES_PER_RUN: usize = 50;
/// How many of the daemon's latest runs the brake looks at, and the fewest it needs to judge.
const BRAKE_WINDOW: i64 = 10;
const BRAKE_MIN_RUNS: i64 = 5;
/// Past this share of them taken back, the daemon stops repairing on its own.
const BRAKE_MAX_UNDONE_PERCENT: i64 = 5;

/// Whether people have been undoing what the daemon repairs on its own, often enough that it
/// should stop and leave the rest to them.
pub fn auto_fix_braked(db: &Database) -> Result<bool> {
    let (total, undone) = db.with_conn(|c| hq_db::vault_health::auto_undo_rate(c, BRAKE_WINDOW))?;
    Ok(total >= BRAKE_MIN_RUNS && undone * 100 > total * BRAKE_MAX_UNDONE_PERCENT)
}

/// The daemon's own repair: every certain fix, unless the brake is on. `None` when it did not run.
pub fn run_auto(vault_path: &Path, db: &std::sync::Arc<Database>) -> Result<Option<Applied>> {
    if auto_fix_braked(db)? {
        return Ok(None);
    }
    let applied = apply_repairs(vault_path, db, &Which::TierA, Source::Auto)?;
    Ok(Some(applied))
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

    use hq_vault::fixes::Tier;
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

    fn read(dir: &Path, rel: &str) -> String {
        std::fs::read_to_string(dir.join(rel)).unwrap()
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
    fn a_rename_is_found_between_two_scans_and_the_links_to_the_old_name_then_follow() {
        let (v, db) = repair_vault(&[
            ("Notebooks/Budget.md", &format!("{WORDS}, budgets")),
            ("Notebooks/L.md", "see [[Budget]] and more"),
        ]);
        age(v.path());
        scan_and_store(v.path(), &db).unwrap();
        // Renamed outside HQ: the same text under a new name.
        std::fs::rename(
            v.path().join("Notebooks/Budget.md"),
            v.path().join("Notebooks/Budget 2026.md"),
        )
        .unwrap();
        age(v.path());
        let stored = scan_and_store(v.path(), &db).unwrap();
        assert_eq!(stored.report.counts.links.dead, 1);

        let (plan, _) = plan_now(v.path(), &db).unwrap();
        assert_eq!(plan.fixes.len(), 1, "{:?}", plan.fixes);
        assert_eq!(plan.fixes[0].kind, hq_vault::fixes::Kind::RenamedNote);
        assert_eq!(plan.fixes[0].tier, Tier::A);

        let applied = apply_repairs(v.path(), &db, &Which::TierA, Source::Owner).unwrap();
        assert_eq!(applied.done.rewritten, ["Notebooks/L.md"]);
        assert_eq!(
            read(v.path(), "Notebooks/L.md"),
            "see [[Budget 2026]] and more"
        );
        assert_eq!(
            scan_and_store(v.path(), &db)
                .unwrap()
                .report
                .counts
                .problems(),
            0
        );
        let row = applied.repair.expect("a row to undo by");
        assert_eq!(
            (row.fixes, row.rewritten, row.source.as_str()),
            (1, 1, "owner")
        );

        // The rewritten note is recorded as HQ's upkeep, not left as whoever wrote it before.
        let hash = hq_db::search::content_hash(&read(v.path(), "Notebooks/L.md"));
        let rec = db
            .with_conn(|c| hq_db::provenance::current(c, "Notebooks/L.md", &hash))
            .unwrap()
            .expect("recorded");
        assert_eq!(rec.actor_id, "system:vault-health");
    }

    #[test]
    fn undo_puts_the_notes_back_and_the_run_can_be_taken_back_only_once() {
        let (v, db) = repair_vault(&[
            ("Notebooks/New/Plan.md", &format!("{WORDS}, plans")),
            ("Notebooks/L.md", "see [[Old/Plan]]"),
        ]);
        age(v.path());
        let applied = apply_repairs(v.path(), &db, &Which::TierA, Source::Owner).unwrap();
        assert_eq!(read(v.path(), "Notebooks/L.md"), "see [[New/Plan]]");
        let id = applied.repair.unwrap().run_id;
        let undone = undo_repair(v.path(), &db, &id).unwrap();
        assert_eq!(undone.restored, ["Notebooks/L.md"]);
        assert_eq!(read(v.path(), "Notebooks/L.md"), "see [[Old/Plan]]");
        assert!(
            undo_repair(v.path(), &db, &id).is_err(),
            "already taken back"
        );
        assert!(undo_repair(v.path(), &db, "no-such-run").is_err());
        let rows = db
            .with_conn(|c| hq_db::vault_health::repairs(c, 10))
            .unwrap();
        assert!(rows[0].undone_at.is_some());
    }

    #[test]
    fn a_note_an_assistant_wrote_and_nobody_reviewed_is_not_rewritten_but_says_why() {
        let (v, db) = repair_vault(&[
            ("Notebooks/New/Plan.md", &format!("{WORDS}, plans")),
            ("Notebooks/Agent.md", "see [[Old/Plan]]"),
            ("Notebooks/Mine.md", "see [[Old/Plan]]"),
        ]);
        age(v.path());
        let recorder = crate::provenance::Recorder::new(db.clone(), v.path().to_path_buf());
        recorder.record(
            "Notebooks/Agent.md",
            &crate::provenance::Writer {
                actor_id: Some("hs-1".into()),
                source_kind: hq_db::provenance::SourceKind::Agent,
                session_id: None,
                task_id: None,
            },
        );
        let (plan, _) = plan_now(v.path(), &db).unwrap();
        assert_eq!(
            plan.fixes
                .iter()
                .map(|f| f.note.as_str())
                .collect::<Vec<_>>(),
            ["Notebooks/Mine.md"]
        );
        assert_eq!(plan.skipped[0].note, "Notebooks/Agent.md");
        assert_eq!(
            plan.skipped[0].why,
            hq_vault::fixes::SkipReason::AgentUnreviewed
        );

        apply_repairs(v.path(), &db, &Which::TierA, Source::Owner).unwrap();
        assert_eq!(read(v.path(), "Notebooks/Agent.md"), "see [[Old/Plan]]");
        assert_eq!(read(v.path(), "Notebooks/Mine.md"), "see [[New/Plan]]");
    }

    #[test]
    fn only_the_chosen_fixes_are_done_and_a_stale_id_does_nothing() {
        let (v, db) = repair_vault(&[
            ("Notebooks/New/Plan.md", &format!("{WORDS}, plans")),
            (
                "Notebooks/Quarterly Review.md",
                &format!("{WORDS}, reviews"),
            ),
            ("Notebooks/L.md", "[[Old/Plan]] and [[Quarterly Reveiw]]"),
        ]);
        age(v.path());
        let (plan, _) = plan_now(v.path(), &db).unwrap();
        assert_eq!(plan.fixes.len(), 2);
        let near = plan.fixes.iter().find(|f| f.tier == Tier::B).unwrap();
        let ids: HashSet<String> = [near.id.clone(), "not-an-id".to_string()]
            .into_iter()
            .collect();
        apply_repairs(v.path(), &db, &Which::Ids(ids), Source::Owner).unwrap();
        assert_eq!(
            read(v.path(), "Notebooks/L.md"),
            "[[Old/Plan]] and [[Quarterly Review]]"
        );
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

    #[test]
    fn the_daemon_stops_repairing_once_people_keep_undoing_it() {
        let (v, db) = repair_vault(&[
            ("Notebooks/New/Plan.md", &format!("{WORDS}, plans")),
            ("Notebooks/L.md", "see [[Old/Plan]]"),
        ]);
        age(v.path());
        assert!(!auto_fix_braked(&db).unwrap());
        let insert = |id: &str, n: u32, undone: bool| {
            let row = Repair {
                run_id: id.into(),
                dir: format!("_trash/2026-11-0{n}/link-repair/{id}"),
                ran_at: format!("2026-11-0{n}T00:00:00Z"),
                source: "auto".into(),
                fixes: 1,
                rewritten: 1,
                trashed: 0,
                undone_at: undone.then(|| "t".to_string()),
                edited_since: 0,
            };
            db.with_conn(move |c| hq_db::vault_health::insert_repair(c, &row))
                .unwrap();
        };
        for n in 1..=5 {
            insert(&format!("ok{n}"), n, false);
        }
        assert!(!auto_fix_braked(&db).unwrap(), "five runs, none undone");
        insert("bad", 6, true);
        assert!(
            auto_fix_braked(&db).unwrap(),
            "one of the last six taken back is over five percent"
        );
        assert!(run_auto(v.path(), &db).unwrap().is_none());
        assert_eq!(
            read(v.path(), "Notebooks/L.md"),
            "see [[Old/Plan]]",
            "nothing was done"
        );
    }

    #[test]
    fn the_daemons_own_run_does_the_certain_fixes_and_no_others() {
        let (v, db) = repair_vault(&[
            ("Notebooks/New/Plan.md", &format!("{WORDS}, plans")),
            (
                "Notebooks/Quarterly Review.md",
                &format!("{WORDS}, reviews"),
            ),
            ("Notebooks/L.md", "[[Old/Plan]] and [[Quarterly Reveiw]]"),
        ]);
        age(v.path());
        let applied = run_auto(v.path(), &db).unwrap().expect("ran");
        assert_eq!(applied.repair.unwrap().source, "auto");
        assert_eq!(
            read(v.path(), "Notebooks/L.md"),
            "[[New/Plan]] and [[Quarterly Reveiw]]"
        );
    }

    #[test]
    fn a_removed_repeat_leaves_search_and_comes_back_on_undo() {
        let text = "The same long paragraph about the quarterly numbers and what they mean.";
        let (v, db) = repair_vault(&[
            ("Notebooks/Keep.md", text),
            ("Notebooks/Dup.md", text),
            ("Notebooks/L.md", "[[Keep]] and [[Dup]]"),
            ("Notebooks/M.md", "[[Keep]]"),
        ]);
        age(v.path());
        for rel in ["Notebooks/Keep.md", "Notebooks/Dup.md"] {
            let (vault, rel) = (v.path().to_path_buf(), rel.to_string());
            db.with_conn(move |c| hq_db::search::reconcile_path(c, &vault, &rel).map(|_| ()))
                .unwrap();
        }
        let indexed = |db: &Database, p: &str| {
            let p = p.to_string();
            db.with_conn(move |c| {
                Ok(c.query_row(
                    "SELECT COUNT(*) FROM notes_fts_rows WHERE path = ?1",
                    [p],
                    |r| r.get::<_, i64>(0),
                )?)
            })
            .unwrap()
        };
        assert_eq!(indexed(&db, "Notebooks/Dup.md"), 1);
        let (plan, _) = plan_now(v.path(), &db).unwrap();
        let merge = plan
            .fixes
            .iter()
            .find(|f| f.kind == hq_vault::fixes::Kind::MergeDuplicate)
            .unwrap();
        let ids: HashSet<String> = [merge.id.clone()].into_iter().collect();
        let applied = apply_repairs(v.path(), &db, &Which::Ids(ids), Source::Owner).unwrap();
        assert_eq!(applied.done.trashed, ["Notebooks/Dup.md"]);
        assert_eq!(indexed(&db, "Notebooks/Dup.md"), 0, "gone from search");
        assert_eq!(read(v.path(), "Notebooks/L.md"), "[[Keep]] and [[Keep]]");

        undo_repair(v.path(), &db, &applied.repair.unwrap().run_id).unwrap();
        assert!(v.path().join("Notebooks/Dup.md").exists());
        assert_eq!(indexed(&db, "Notebooks/Dup.md"), 1, "and back");
        assert_eq!(read(v.path(), "Notebooks/L.md"), "[[Keep]] and [[Dup]]");
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

    #[test]
    fn approving_the_offer_fixes_the_links_then_asks_once_to_keep_doing_it_and_a_second_yes_is_remembered()
     {
        let (v, db) = repair_vault(&[
            ("Notebooks/New/Plan.md", &format!("{WORDS}, plans")),
            ("Notebooks/L.md", "see [[Old/Plan]]"),
        ]);
        age(v.path());
        let stored = scan_and_store(v.path(), &db).unwrap();
        assert_eq!(stored.fix_a, 1);
        sync_inbox(&db, &stored, false).unwrap();
        let fix_id = id_of(&db, FIX_PREFIX);

        // A no, or a dismissal, changes nothing.
        answer_inbox(v.path(), &db, &fix_id, false).unwrap();
        assert_eq!(read(v.path(), "Notebooks/L.md"), "see [[Old/Plan]]");

        answer_inbox(v.path(), &db, &fix_id, true).unwrap();
        assert_eq!(read(v.path(), "Notebooks/L.md"), "see [[New/Plan]]");
        assert!(
            !hq_db::owner_prefs::vault_auto_fix_on(&db),
            "one yes is not a standing yes"
        );
        let offer = id_of(&db, AUTO_OFFER_KEY);

        // Asked once: approving the first offer a second time does not ask again.
        answer_inbox(v.path(), &db, &fix_id, true).unwrap();
        assert_eq!(
            open_items(&db)
                .iter()
                .filter(|(k, _)| k == AUTO_OFFER_KEY)
                .count(),
            1
        );

        answer_inbox(v.path(), &db, &offer, true).unwrap();
        assert!(hq_db::owner_prefs::vault_auto_fix_on(&db));
        assert!(auto_fix_on(&db, false));
    }

    #[test]
    fn an_approval_of_anything_else_does_nothing_here() {
        let (v, db) = repair_vault(&[("Notebooks/a.md", "x")]);
        hq_db::value_items::emit(
            &db,
            &ValueItem::new("somebody_else", ValueKind::ActionNeeded, "t", "b")
                .with_dedup_key(format!("{FIX_PREFIX}2026-W48")),
        )
        .unwrap();
        let id = db
            .with_conn(|c| {
                Ok(c.query_row(
                    "SELECT id FROM value_items WHERE source_task = 'somebody_else'",
                    [],
                    |r| r.get::<_, String>(0),
                )?)
            })
            .unwrap();
        answer_inbox(v.path(), &db, &id, true).unwrap();
        assert!(!hq_db::owner_prefs::vault_auto_fix_on(&db));
        assert!(answer_inbox(v.path(), &db, "no-such-item", true).is_ok());
    }

    fn id_of(db: &Database, prefix: &str) -> String {
        let like = format!("{prefix}%");
        db.with_conn(move |c| {
            Ok(c.query_row(
                "SELECT id FROM value_items WHERE source_task = 'vault_health' AND dedup_key LIKE ?1
                 AND state IN ('pending','routed','delivered')",
                [like],
                |r| r.get::<_, String>(0),
            )?)
        })
        .unwrap()
    }
}
