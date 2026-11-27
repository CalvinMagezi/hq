//! Writing the fixes `fixes.rs` planned, and taking them back.
//!
//! Every run copies each note it rewrites to `_trash/<day>/link-repair/<run>/` first and ends
//! with a `manifest.json` there that names, per note, the hash before and after. A note that
//! is no longer the text the fix was planned from is left alone. A note changed in the last
//! few minutes is left alone, since an editor or a sync tool may still be writing it. A fix
//! that touches several notes (removing a repeat and rewriting the links to it) is done whole
//! or its removal is not done at all.
//!
//! [`undo`] puts a note back only while it is still exactly what the run wrote. Anything
//! edited since is listed and left as the person has it.

use crate::fixes::{Context, Fix, SkipReason, Skipped, may_touch, text_hash};
use crate::links::{Edit, apply_edits};
use crate::note_lock::lock_note;
use crate::notes::write_atomic;
use crate::rel_path::RelPath;
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::{Duration, SystemTime};

const TRASH: &str = "_trash";
const REPAIR_DIR: &str = "link-repair";
const MANIFEST: &str = "manifest.json";

/// How a run is set up. The clock and the ids come in so a test needs neither.
#[derive(Debug, Clone)]
pub struct Options {
    /// `YYYYMMDD-HHMMSS-xxxx`: names the run's folder.
    pub run_id: String,
    /// `YYYY-MM-DD`, the trash folder the run lives in (the day purging counts from).
    pub day: String,
    pub now: SystemTime,
    /// A note edited more recently than this many seconds ago is not touched.
    pub quiet_secs: u64,
    /// The most notes one run rewrites or removes.
    pub max_notes: usize,
}

/// What a run did to one rewritten note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub note: String,
    pub before: String,
    pub after: String,
    /// Where the original was copied, relative to the vault.
    pub backup: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Trashed {
    pub note: String,
    /// Where it is now, under `_trash/`.
    pub trash_path: String,
    pub hash: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub run_id: String,
    pub day: String,
    /// The ids of the fixes this run did.
    pub fixes: Vec<String>,
    pub entries: Vec<Entry>,
    pub trashed: Vec<Trashed>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Done {
    pub run_id: String,
    /// The run's folder, relative to the vault. Empty when nothing was changed.
    pub dir: String,
    /// Ids of the fixes that were done.
    pub fixes: Vec<String>,
    /// Notes whose text was rewritten.
    pub rewritten: Vec<String>,
    /// Notes moved to the trash.
    pub trashed: Vec<String>,
    pub skipped: Vec<Skipped>,
}

fn read(vault: &Path, rel: &str) -> Option<String> {
    std::fs::read_to_string(vault.join(rel)).ok()
}

fn modified_ago(vault: &Path, rel: &str, now: SystemTime) -> Option<Duration> {
    let m = std::fs::metadata(vault.join(rel)).ok()?.modified().ok()?;
    now.duration_since(m).ok().or(Some(Duration::ZERO))
}

/// Why `fix` cannot be done now, if it cannot: a note it needs is gone, changed, protected or
/// too fresh.
fn blocked(vault: &Path, fix: &Fix, ctx: &Context, opts: &Options) -> Option<(String, SkipReason)> {
    for (note, base) in &fix.bases {
        let Some(text) = read(vault, note) else {
            return Some((note.clone(), SkipReason::Changed));
        };
        if &text_hash(&text) != base {
            return Some((note.clone(), SkipReason::Changed));
        }
        if let Err(why) = may_touch(note, &text, ctx) {
            return Some((note.clone(), why));
        }
        if modified_ago(vault, note, opts.now).is_some_and(|d| d.as_secs() < opts.quiet_secs) {
            return Some((note.clone(), SkipReason::RecentlyEdited));
        }
    }
    None
}

/// Do `fixes`. A fix that cannot be done is listed in `skipped` with why; the rest are done.
pub fn apply(vault: &Path, fixes: &[&Fix], ctx: &Context, opts: &Options) -> Result<Done> {
    let mut done = Done {
        run_id: opts.run_id.clone(),
        ..Done::default()
    };

    // 1. Which fixes can be done at all.
    let mut chosen: Vec<&Fix> = Vec::new();
    let mut touched: BTreeSet<&str> = BTreeSet::new();
    for fix in fixes {
        if let Some((note, why)) = blocked(vault, fix, ctx, opts) {
            done.skipped.push(Skipped { note, why });
            continue;
        }
        let mut next = touched.clone();
        next.extend(fix.bases.keys().map(String::as_str));
        if next.len() > opts.max_notes {
            continue;
        }
        touched = next;
        chosen.push(fix);
    }

    // 2. Drop a fix whose words overlap an earlier one's, for every note it touches.
    let texts: BTreeMap<&str, String> = touched
        .iter()
        .filter_map(|n| read(vault, n).map(|t| (*n, t)))
        .collect();
    let mut accepted: Vec<&Fix> = Vec::new();
    let mut edits: BTreeMap<&str, Vec<Edit>> = BTreeMap::new();
    for fix in chosen {
        let mut trial = edits.clone();
        for (note, e) in &fix.edits {
            trial
                .entry(note.as_str())
                .or_default()
                .extend(e.iter().cloned());
        }
        let fits = fix.edits.keys().all(|n| {
            texts
                .get(n.as_str())
                .is_some_and(|t| apply_edits(t, &trial[n.as_str()]).is_some())
        });
        if fits {
            edits = trial;
            accepted.push(fix);
        } else {
            done.skipped.push(Skipped {
                note: fix.note.clone(),
                why: SkipReason::Conflict,
            });
        }
    }
    if accepted.is_empty() {
        return Ok(done);
    }

    // 3. Write each note once, original first.
    let run_dir = format!("{TRASH}/{}/{REPAIR_DIR}/{}", opts.day, opts.run_id);
    let mut manifest = Manifest {
        run_id: opts.run_id.clone(),
        day: opts.day.clone(),
        ..Manifest::default()
    };
    let mut failed: BTreeSet<String> = BTreeSet::new();
    for (note, note_edits) in &edits {
        let base = accepted
            .iter()
            .find_map(|f| f.bases.get(*note))
            .cloned()
            .unwrap_or_default();
        let _held = lock_note(note);
        let Some(current) = read(vault, note) else {
            failed.insert(note.to_string());
            continue;
        };
        // Judged again with the note held: the check above was a moment ago.
        if text_hash(&current) != base {
            done.skipped.push(Skipped {
                note: note.to_string(),
                why: SkipReason::Changed,
            });
            failed.insert(note.to_string());
            continue;
        }
        let Some(new_text) = apply_edits(&current, note_edits) else {
            failed.insert(note.to_string());
            continue;
        };
        let backup = format!("{run_dir}/{note}");
        let backup_path = vault.join(&backup);
        if let Some(parent) = backup_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&backup_path, &current)?;
        write_atomic(&vault.join(note), new_text.as_bytes())?;
        manifest.entries.push(Entry {
            note: note.to_string(),
            before: base,
            after: text_hash(&new_text),
            backup,
        });
        done.rewritten.push(note.to_string());
    }

    // 4. Remove what a merge made unnecessary, only if every note that pointed at it moved on.
    for fix in &accepted {
        let all_written = fix.edits.keys().all(|n| !failed.contains(n));
        if !all_written {
            continue;
        }
        for gone in &fix.trash {
            let _held = lock_note(gone);
            let Some(current) = read(vault, gone) else {
                continue;
            };
            let hash = text_hash(&current);
            if fix.bases.get(gone) != Some(&hash) {
                done.skipped.push(Skipped {
                    note: gone.clone(),
                    why: SkipReason::Changed,
                });
                continue;
            }
            let trash_path = crate::reorg::trash_note(vault, gone, false)?;
            manifest.trashed.push(Trashed {
                note: gone.clone(),
                trash_path,
                hash,
            });
            done.trashed.push(gone.clone());
        }
    }

    // 5. Say what was done, so it can be undone.
    manifest.fixes = accepted
        .iter()
        .filter(|f| f.edits.keys().all(|n| !failed.contains(n)))
        .map(|f| f.id.clone())
        .collect();
    done.fixes = manifest.fixes.clone();
    if manifest.entries.is_empty() && manifest.trashed.is_empty() {
        return Ok(done);
    }
    let manifest_path = vault.join(&run_dir).join(MANIFEST);
    if let Some(parent) = manifest_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    write_atomic(
        &manifest_path,
        serde_json::to_string_pretty(&manifest)?.as_bytes(),
    )?;
    done.dir = run_dir;
    Ok(done)
}

/// What [`undo`] did.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Undone {
    /// Notes put back as they were.
    pub restored: Vec<String>,
    /// Notes edited since the run, left as they are.
    pub edited_since: Vec<String>,
    /// Notes brought back from the trash, by the path they have now.
    pub untrashed: Vec<String>,
    /// Anything that could not be put back, with why.
    pub failed: Vec<String>,
}

/// A run folder as the caller names it, checked to be one: `_trash/<day>/link-repair/<run>`.
fn run_dir(dir: &str) -> Result<String> {
    let norm = RelPath::parse(dir)?.normalized();
    let parts: Vec<&str> = norm.split('/').collect();
    let ok = parts.len() == 4
        && parts[0] == TRASH
        && parts[2] == REPAIR_DIR
        && chrono::NaiveDate::parse_from_str(parts[1], "%Y-%m-%d").is_ok()
        && !parts[3].is_empty();
    if !ok {
        bail!("'{dir}' is not a repair run");
    }
    Ok(norm)
}

/// Take back the run in `dir` (as [`Done::dir`] gave it).
pub fn undo(vault: &Path, dir: &str) -> Result<Undone> {
    let dir = run_dir(dir)?;
    let manifest: Manifest =
        serde_json::from_str(&std::fs::read_to_string(vault.join(&dir).join(MANIFEST))?)?;
    let mut out = Undone::default();

    for e in &manifest.entries {
        // The backup must be what the manifest says, and still inside this run's folder.
        if !e.backup.starts_with(&format!("{dir}/")) || RelPath::parse(&e.backup).is_err() {
            out.failed
                .push(format!("{}: the backup is not where it should be", e.note));
            continue;
        }
        let _held = lock_note(&e.note);
        let Some(current) = read(vault, &e.note) else {
            out.edited_since.push(e.note.clone());
            continue;
        };
        let now = text_hash(&current);
        if now == e.before {
            out.restored.push(e.note.clone());
            continue;
        }
        if now != e.after {
            out.edited_since.push(e.note.clone());
            continue;
        }
        match read(vault, &e.backup) {
            Some(original) if text_hash(&original) == e.before => {
                write_atomic(&vault.join(&e.note), original.as_bytes())?;
                out.restored.push(e.note.clone());
            }
            _ => out
                .failed
                .push(format!("{}: the saved copy is missing or changed", e.note)),
        }
    }

    for t in &manifest.trashed {
        match crate::reorg::restore_note(vault, &t.trash_path) {
            Ok(r) => out.untrashed.push(r.path),
            Err(e) => out.failed.push(format!("{}: {e}", t.note)),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixes::plan;
    use crate::relink::snapshot;
    use std::collections::HashSet;
    use std::fs;

    const WORDS: &str = "enough words in this note to count as a real note and not be empty";

    fn vault(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (rel, text) in files {
            let p = dir.path().join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, text).unwrap();
        }
        dir
    }

    fn opts() -> Options {
        Options {
            run_id: "20261127-100000-ab12".into(),
            day: chrono::Local::now().format("%Y-%m-%d").to_string(),
            now: SystemTime::now(),
            quiet_secs: 0,
            max_notes: 50,
        }
    }

    /// Every file under `root` (outside `_trash`) with its text, for comparing two states.
    fn tree(root: &Path) -> BTreeMap<String, String> {
        let mut out = BTreeMap::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in fs::read_dir(&d).unwrap().flatten() {
                let p = e.path();
                let rel = p
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                if rel.starts_with(TRASH) {
                    continue;
                }
                if p.is_dir() {
                    stack.push(p);
                } else {
                    out.insert(rel, fs::read_to_string(&p).unwrap());
                }
            }
        }
        out
    }

    fn broken_vault() -> tempfile::TempDir {
        vault(&[
            ("Notebooks/New/Plan.md", &format!("{WORDS}, plans")),
            (
                "Notebooks/L.md",
                "see [[Old/Plan#Goals]] and [t](Old/Plan.md)",
            ),
            ("Notebooks/M.md", "also [[Old/Plan]]"),
        ])
    }

    #[test]
    fn applying_rewrites_the_links_saves_the_originals_and_writes_a_manifest() {
        let v = broken_vault();
        let p = plan(&snapshot(v.path()), &Context::default());
        let all: Vec<&Fix> = p.fixes.iter().collect();
        let done = apply(v.path(), &all, &Context::default(), &opts()).unwrap();
        assert_eq!(done.rewritten, ["Notebooks/L.md", "Notebooks/M.md"]);
        assert_eq!(done.fixes.len(), 3);
        assert_eq!(
            fs::read_to_string(v.path().join("Notebooks/L.md")).unwrap(),
            "see [[New/Plan#Goals]] and [t](New/Plan.md)"
        );
        let m: Manifest = serde_json::from_str(
            &fs::read_to_string(v.path().join(&done.dir).join(MANIFEST)).unwrap(),
        )
        .unwrap();
        assert_eq!(m.entries.len(), 2);
        let backup = fs::read_to_string(v.path().join(&m.entries[0].backup)).unwrap();
        assert_eq!(text_hash(&backup), m.entries[0].before);
        // Planning again after the run finds nothing left to do.
        assert!(
            plan(&snapshot(v.path()), &Context::default())
                .fixes
                .is_empty()
        );
    }

    #[test]
    fn undo_gives_back_every_note_exactly() {
        let v = broken_vault();
        let before = tree(v.path());
        let p = plan(&snapshot(v.path()), &Context::default());
        let done = apply(
            v.path(),
            &p.fixes.iter().collect::<Vec<_>>(),
            &Context::default(),
            &opts(),
        )
        .unwrap();
        assert_ne!(tree(v.path()), before);
        let undone = undo(v.path(), &done.dir).unwrap();
        assert_eq!(undone.restored.len(), 2);
        assert!(undone.edited_since.is_empty() && undone.failed.is_empty());
        assert_eq!(tree(v.path()), before);
    }

    #[test]
    fn undo_leaves_a_note_edited_since_and_restores_the_rest() {
        let v = broken_vault();
        let p = plan(&snapshot(v.path()), &Context::default());
        let done = apply(
            v.path(),
            &p.fixes.iter().collect::<Vec<_>>(),
            &Context::default(),
            &opts(),
        )
        .unwrap();
        fs::write(
            v.path().join("Notebooks/M.md"),
            "also [[New/Plan]] and my own new thought",
        )
        .unwrap();
        let undone = undo(v.path(), &done.dir).unwrap();
        assert_eq!(undone.restored, ["Notebooks/L.md"]);
        assert_eq!(undone.edited_since, ["Notebooks/M.md"]);
        assert_eq!(
            fs::read_to_string(v.path().join("Notebooks/M.md")).unwrap(),
            "also [[New/Plan]] and my own new thought"
        );
    }

    #[test]
    fn undoing_twice_changes_nothing_the_second_time() {
        let v = broken_vault();
        let p = plan(&snapshot(v.path()), &Context::default());
        let done = apply(
            v.path(),
            &p.fixes.iter().collect::<Vec<_>>(),
            &Context::default(),
            &opts(),
        )
        .unwrap();
        undo(v.path(), &done.dir).unwrap();
        let again = undo(v.path(), &done.dir).unwrap();
        assert!(again.edited_since.is_empty() && again.failed.is_empty());
        assert_eq!(
            again.restored.len(),
            2,
            "already as it was counts as restored"
        );
    }

    #[test]
    fn a_note_edited_after_planning_is_left_alone_and_reported() {
        let v = broken_vault();
        let p = plan(&snapshot(v.path()), &Context::default());
        fs::write(
            v.path().join("Notebooks/M.md"),
            "also [[Old/Plan]] plus a new line",
        )
        .unwrap();
        let done = apply(
            v.path(),
            &p.fixes.iter().collect::<Vec<_>>(),
            &Context::default(),
            &opts(),
        )
        .unwrap();
        assert_eq!(done.rewritten, ["Notebooks/L.md"]);
        assert!(
            done.skipped
                .iter()
                .any(|s| s.note == "Notebooks/M.md" && s.why == SkipReason::Changed)
        );
        assert_eq!(
            fs::read_to_string(v.path().join("Notebooks/M.md")).unwrap(),
            "also [[Old/Plan]] plus a new line"
        );
    }

    #[test]
    fn a_note_changed_a_moment_ago_is_not_touched() {
        let v = broken_vault();
        let p = plan(&snapshot(v.path()), &Context::default());
        let mut o = opts();
        o.quiet_secs = 600;
        let done = apply(
            v.path(),
            &p.fixes.iter().collect::<Vec<_>>(),
            &Context::default(),
            &o,
        )
        .unwrap();
        assert!(done.rewritten.is_empty());
        assert!(
            done.skipped
                .iter()
                .all(|s| s.why == SkipReason::RecentlyEdited)
        );
        assert!(
            done.dir.is_empty(),
            "nothing was written, so there is no run folder"
        );
        assert!(!v.path().join(TRASH).exists());
        // The same notes, judged ten minutes later, are fair game.
        o.now += Duration::from_secs(601);
        let later = apply(
            v.path(),
            &p.fixes.iter().collect::<Vec<_>>(),
            &Context::default(),
            &o,
        )
        .unwrap();
        assert_eq!(later.rewritten.len(), 2);
    }

    #[test]
    fn a_note_that_became_pinned_after_planning_is_not_rewritten() {
        let v = broken_vault();
        let p = plan(&snapshot(v.path()), &Context::default());
        let pinned = "---\npinned: true\n---\nsee [[Old/Plan#Goals]] and [t](Old/Plan.md)";
        fs::write(v.path().join("Notebooks/L.md"), pinned).unwrap();
        let done = apply(
            v.path(),
            &p.fixes.iter().collect::<Vec<_>>(),
            &Context::default(),
            &opts(),
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(v.path().join("Notebooks/L.md")).unwrap(),
            pinned
        );
        assert_eq!(done.rewritten, ["Notebooks/M.md"]);
    }

    #[test]
    fn a_run_is_capped_at_the_most_notes_it_may_change() {
        let mut files: Vec<(String, String)> =
            vec![("Notebooks/New/Plan.md".into(), format!("{WORDS}, plans"))];
        for i in 0..5 {
            files.push((format!("Notebooks/L{i}.md"), "see [[Old/Plan]]".into()));
        }
        let refs: Vec<(&str, &str)> = files
            .iter()
            .map(|(a, b)| (a.as_str(), b.as_str()))
            .collect();
        let v = vault(&refs);
        let p = plan(&snapshot(v.path()), &Context::default());
        let mut o = opts();
        o.max_notes = 3;
        let done = apply(
            v.path(),
            &p.fixes.iter().collect::<Vec<_>>(),
            &Context::default(),
            &o,
        )
        .unwrap();
        assert_eq!(done.rewritten.len(), 3);
        // The rest are still there to plan.
        assert_eq!(
            plan(&snapshot(v.path()), &Context::default()).fixes.len(),
            2
        );
    }

    #[test]
    fn removing_a_repeat_rewrites_its_links_then_trashes_it_and_undo_brings_both_back() {
        let text = "The same long paragraph about the quarterly numbers and what they mean.";
        let v = vault(&[
            ("Notebooks/Keep.md", text),
            ("Notebooks/Dup.md", text),
            ("Notebooks/L.md", "[[Keep]] and [[Dup]]"),
            ("Notebooks/M.md", "[[Keep]]"),
        ]);
        let before = tree(v.path());
        let p = plan(&snapshot(v.path()), &Context::default());
        let merge: Vec<&Fix> = p.fixes.iter().filter(|f| f.trash.len() == 1).collect();
        assert_eq!(merge.len(), 1);
        let done = apply(v.path(), &merge, &Context::default(), &opts()).unwrap();
        assert_eq!(done.trashed, ["Notebooks/Dup.md"]);
        assert!(!v.path().join("Notebooks/Dup.md").exists());
        assert_eq!(
            fs::read_to_string(v.path().join("Notebooks/L.md")).unwrap(),
            "[[Keep]] and [[Keep]]"
        );

        let undone = undo(v.path(), &done.dir).unwrap();
        assert_eq!(undone.untrashed, ["Notebooks/Dup.md"]);
        assert_eq!(tree(v.path()), before);
    }

    #[test]
    fn a_repeat_is_not_removed_if_a_note_that_linked_to_it_could_not_be_rewritten() {
        let text = "The same long paragraph about the quarterly numbers and what they mean.";
        let v = vault(&[
            ("Notebooks/Keep.md", text),
            ("Notebooks/Dup.md", text),
            ("Notebooks/L.md", "[[Dup]]"),
            ("Notebooks/M.md", "[[Keep]]"),
            ("Notebooks/N.md", "[[Keep]]"),
        ]);
        let p = plan(&snapshot(v.path()), &Context::default());
        let merge: Vec<&Fix> = p.fixes.iter().filter(|f| f.trash.len() == 1).collect();
        // L changes after the plan, so the fix is turned away before anything is written. (The
        // second look under the lock is for an edit that lands in the moments after the first.)
        let mut o = opts();
        o.quiet_secs = 0;
        fs::write(v.path().join("Notebooks/L.md"), "[[Dup]] edited").unwrap();
        let done = apply(v.path(), &merge, &Context::default(), &o).unwrap();
        assert!(done.trashed.is_empty());
        assert!(v.path().join("Notebooks/Dup.md").exists());
    }

    #[test]
    fn two_fixes_for_the_same_words_do_not_both_apply() {
        let v = vault(&[("Notebooks/N.md", "abc def")]);
        let text = "abc def";
        let mk = |id: &str, with: &str| Fix {
            id: id.into(),
            tier: crate::fixes::Tier::A,
            kind: crate::fixes::Kind::StalePath,
            note: "Notebooks/N.md".into(),
            line: Some(1),
            before: "abc".into(),
            after: with.into(),
            edits: BTreeMap::from([(
                "Notebooks/N.md".to_string(),
                vec![Edit {
                    range: 0..3,
                    text: with.into(),
                }],
            )]),
            trash: vec![],
            bases: BTreeMap::from([("Notebooks/N.md".to_string(), text_hash(text))]),
        };
        let (a, b) = (mk("a", "XYZ"), mk("b", "QRS"));
        let done = apply(v.path(), &[&a, &b], &Context::default(), &opts()).unwrap();
        assert_eq!(done.fixes, ["a"]);
        assert!(done.skipped.iter().any(|s| s.why == SkipReason::Conflict));
        assert_eq!(
            fs::read_to_string(v.path().join("Notebooks/N.md")).unwrap(),
            "XYZ def"
        );
    }

    #[test]
    fn undo_refuses_anything_that_is_not_a_run_folder() {
        let v = vault(&[("Notebooks/a.md", "x")]);
        for bad in [
            "Notebooks",
            "_trash",
            "_trash/2026-11-27",
            "_trash/2026-11-27/Notebooks/a.md",
            "_trash/2026-11-27/link-repair",
            "_trash/not-a-day/link-repair/run",
            "../outside",
            "_trash/2026-11-27/link-repair/run/extra",
        ] {
            assert!(undo(v.path(), bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_manifest_that_points_its_backup_elsewhere_restores_nothing() {
        let v = broken_vault();
        let p = plan(&snapshot(v.path()), &Context::default());
        let done = apply(
            v.path(),
            &p.fixes.iter().collect::<Vec<_>>(),
            &Context::default(),
            &opts(),
        )
        .unwrap();
        let mpath = v.path().join(&done.dir).join(MANIFEST);
        let mut m: Manifest = serde_json::from_str(&fs::read_to_string(&mpath).unwrap()).unwrap();
        m.entries[0].backup = "Notebooks/M.md".into();
        fs::write(&mpath, serde_json::to_string(&m).unwrap()).unwrap();
        let undone = undo(v.path(), &done.dir).unwrap();
        assert_eq!(undone.failed.len(), 1);
        assert!(!undone.restored.contains(&m.entries[0].note));
    }

    #[test]
    fn ids_pick_only_the_chosen_fixes() {
        let v = broken_vault();
        let p = plan(&snapshot(v.path()), &Context::default());
        let one: HashSet<String> = [p.fixes[0].id.clone()].into_iter().collect();
        let chosen = p.select(&one);
        assert_eq!(chosen.len(), 1);
        let done = apply(v.path(), &chosen, &Context::default(), &opts()).unwrap();
        assert_eq!(done.fixes, [p.fixes[0].id.clone()]);
    }
}
