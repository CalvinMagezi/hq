//! Repairing notes from the notes check, and taking the repairs back.
//!
//! The scan, the plan and the stored report are `vault_health`; this is the part that writes. It
//! sits above `provenance` and `vault_reorg` because a repair is recorded as HQ's own upkeep and
//! a removed note leaves search the way a deleted one does. Applying and undoing are for the
//! owner (the Inbox, the CLI, an owner-only route) and the daemon with the owner's standing
//! permission; neither is an agent tool.

use crate::vault_health::{AUTO_OFFER_KEY, FIX_PREFIX, plan_now};
use anyhow::{Result, bail};
use hq_core::types::{VAULT_HEALTH_SOURCE, ValueItem, ValueKind};
use hq_db::Database;
use hq_db::vault_health::{Repair, Source};
use hq_vault::repair::{self, Options};
use serde::Serialize;
use std::collections::HashSet;
use std::path::Path;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault_health::{auto_fix_on, scan_and_store, sync_inbox};
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
}
