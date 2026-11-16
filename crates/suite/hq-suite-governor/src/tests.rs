use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use hq_suite_shrink::ShrinkError;
use hq_suite_store::journal::{JOURNAL_VERSION, MediaPlan};

use super::*;

const NOTE: &str = "Notebooks/a.md";
const SOURCE: &str = "Notebooks/a.docx";
const TRASHED: &str = "_trash/2026-01-01/Notebooks/a.docx";

/// Far enough in the future that every picture in a test counts as old.
fn later() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(4_000_000_000)
}

struct FsNotes(SuitePaths);

impl Notes for FsNotes {
    fn read(&self, rel: &str) -> Result<Option<String>, ShrinkError> {
        match fs::read_to_string(self.0.resolve(rel).unwrap()) {
            Ok(t) => Ok(Some(t)),
            Err(_) => Ok(None),
        }
    }
    fn write(&self, rel: &str, text: &str) -> Result<(), ShrinkError> {
        fs::write(self.0.resolve(rel).unwrap(), text).map_err(|e| ShrinkError::Notes(e.to_string()))
    }
    fn remove(&self, rel: &str) -> Result<(), ShrinkError> {
        fs::remove_file(self.0.resolve(rel).unwrap()).map_err(|e| ShrinkError::Notes(e.to_string()))
    }
}

fn setup() -> (tempfile::TempDir, SuitePaths) {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir_all(d.path().join("Notebooks")).unwrap();
    let p = SuitePaths::new(d.path());
    (d, p)
}

fn picture(paths: &SuitePaths, tag: u8) -> CasRef {
    Cas::new(paths.clone()).put(&[tag; 100], "webp").unwrap().0
}

fn note_text(state: &str, links: &[&CasRef]) -> String {
    let mut t =
        format!("---\nsuite_original_state: {state}\nsuite_saved_bytes: 10\n---\n\nText.\n\n");
    for r in links {
        t.push_str(&r.markdown_image(NOTE, "p"));
        t.push('\n');
    }
    t
}

fn job(id: &str, step: Step, day: &str, media: Vec<MediaPlan>) -> Job {
    Job {
        version: JOURNAL_VERSION,
        id: id.into(),
        source: SOURCE.into(),
        source_sha256: "0".repeat(64),
        source_bytes: 1000,
        note: NOTE.into(),
        media,
        note_sha256: "0".repeat(64),
        day: day.into(),
        source_format: "docx".into(),
        loss_kinds: vec![],
        fidelity: "safe".into(),
        pictures_recompressed: 0,
        pictures_kept: 0,
        disposition: Some(Disposition::Trash {
            target: TRASHED.into(),
        }),
        step,
        saved_bytes: Some(10),
        failure: None,
        adopted_note_sha256: None,
    }
}

fn put_original(d: &tempfile::TempDir, bytes: usize) {
    let p = d.path().join(TRASHED);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, vec![7u8; bytes]).unwrap();
}

fn state_of(d: &tempfile::TempDir) -> String {
    let t = fs::read_to_string(d.path().join(NOTE)).unwrap();
    frontmatter::get(
        hq_suite_md::Document::parse(&t).frontmatter(),
        KEY_ORIGINAL_STATE,
    )
    .unwrap()
}

fn run(paths: &SuitePaths, s: &Settings) -> Outcome {
    govern(paths, &FsNotes(paths.clone()), s, "2026-03-01", later()).unwrap()
}

#[test]
fn a_picture_nobody_links_to_goes_and_a_linked_one_stays() {
    let (d, p) = setup();
    let kept = picture(&p, 1);
    let orphan = picture(&p, 2);
    fs::write(d.path().join(NOTE), note_text("trash", &[&kept])).unwrap();
    let out = run(&p, &Settings::default());
    assert_eq!(out.media_removed, 1);
    let cas = Cas::new(p.clone());
    assert!(cas.has(&kept));
    assert!(!cas.has(&orphan));
}

#[test]
fn a_picture_in_a_trashed_note_is_still_wanted() {
    let (d, p) = setup();
    let r = picture(&p, 1);
    let trashed = d.path().join("_trash/2026-01-01/Notebooks");
    fs::create_dir_all(&trashed).unwrap();
    fs::write(trashed.join("b.md"), note_text("kept", &[&r])).unwrap();
    assert_eq!(run(&p, &Settings::default()).media_removed, 0);
    assert!(Cas::new(p).has(&r));
}

#[test]
fn a_picture_an_unfinished_job_needs_stays() {
    let (_d, p) = setup();
    let r = picture(&p, 1);
    let plan = MediaPlan {
        cas: r.clone(),
        bytes: 100,
    };
    Journal::new(&p)
        .create(&job("sj-a", Step::Planned, "2026-02-01", vec![plan]))
        .unwrap();
    assert_eq!(run(&p, &Settings::default()).media_removed, 0);
    assert!(Cas::new(p).has(&r));
}

#[test]
fn a_very_new_picture_is_left_alone() {
    let (d, p) = setup();
    let r = picture(&p, 1);
    let modified = fs::metadata(d.path().join(r.rel()))
        .unwrap()
        .modified()
        .unwrap();
    let soon = modified + Duration::from_secs(60);
    let out = govern(
        &p,
        &FsNotes(p.clone()),
        &Settings::default(),
        "2026-03-01",
        soon,
    )
    .unwrap();
    assert_eq!(out.media_removed, 0);
    assert!(Cas::new(p).has(&r));
}

#[cfg(unix)]
#[test]
fn nothing_is_removed_when_a_note_cannot_be_read() {
    let (d, p) = setup();
    let orphan = picture(&p, 2);
    let note = d.path().join(NOTE);
    fs::write(&note, note_text("kept", &[])).unwrap();
    fs::set_permissions(&note, fs::Permissions::from_mode(0o000)).unwrap();
    let out = run(&p, &Settings::default());
    fs::set_permissions(&note, fs::Permissions::from_mode(0o644)).unwrap();
    if fs::read_to_string(&note).is_ok() && out.skipped.is_empty() {
        // Running as a user that ignores file modes (root): the guard cannot be provoked here.
        return;
    }
    assert_eq!(out.media_removed, 0);
    assert!(out.skipped.contains(&"a note could not be read"));
    assert!(Cas::new(p).has(&orphan));
}

#[test]
fn a_note_whose_original_aged_out_of_the_trash_is_marked_gone() {
    let (d, p) = setup();
    fs::write(d.path().join(NOTE), note_text("trash", &[])).unwrap();
    Journal::new(&p)
        .create(&job("sj-a", Step::Done, "2026-01-01", vec![]))
        .unwrap();
    let out = run(&p, &Settings::default());
    assert_eq!(out.marked_gone, 1);
    assert_eq!(state_of(&d), "gone");
    assert_eq!(
        run(&p, &Settings::default()).marked_gone,
        0,
        "a second pass changes nothing"
    );
}

#[test]
fn a_note_whose_original_is_still_in_the_trash_is_left() {
    let (d, p) = setup();
    fs::write(d.path().join(NOTE), note_text("trash", &[])).unwrap();
    put_original(&d, 500);
    Journal::new(&p)
        .create(&job("sj-a", Step::Done, "2026-01-01", vec![]))
        .unwrap();
    assert_eq!(run(&p, &Settings::default()).marked_gone, 0);
    assert_eq!(state_of(&d), "trash");
}

#[test]
fn over_budget_the_cache_goes_before_any_original() {
    let (d, p) = setup();
    fs::write(d.path().join(NOTE), note_text("trash", &[])).unwrap();
    put_original(&d, 500);
    Journal::new(&p)
        .create(&job("sj-a", Step::Done, "2026-01-01", vec![]))
        .unwrap();
    fs::create_dir_all(p.cache_dir()).unwrap();
    fs::write(p.cache_dir().join("thumb"), vec![0u8; 400]).unwrap();
    let s = Settings {
        budget_bytes: Some(600),
        ..Settings::default()
    };
    let out = run(&p, &s);
    assert_eq!(out.cache_bytes_freed, 400);
    assert_eq!(out.originals_expired, 0);
    assert!(!out.over_budget);
    assert!(trash::exists(&p, TRASHED));
}

#[test]
fn over_budget_an_original_past_its_window_goes_and_its_note_says_so() {
    let (d, p) = setup();
    fs::write(d.path().join(NOTE), note_text("trash", &[])).unwrap();
    put_original(&d, 500);
    Journal::new(&p)
        .create(&job("sj-a", Step::Done, "2026-01-01", vec![]))
        .unwrap();
    let s = Settings {
        budget_bytes: Some(100),
        ..Settings::default()
    };
    let out = run(&p, &s);
    assert_eq!(out.originals_expired, 1);
    assert_eq!(out.original_bytes_freed, 500);
    assert!(!trash::exists(&p, TRASHED));
    assert_eq!(state_of(&d), "gone");
}

#[test]
fn an_original_inside_its_window_is_never_taken_and_the_pass_says_it_is_over() {
    let (d, p) = setup();
    fs::write(d.path().join(NOTE), note_text("trash", &[])).unwrap();
    put_original(&d, 500);
    Journal::new(&p)
        .create(&job("sj-a", Step::Done, "2026-02-20", vec![]))
        .unwrap();
    let s = Settings {
        budget_bytes: Some(100),
        ..Settings::default()
    };
    let out = run(&p, &s);
    assert_eq!(out.originals_expired, 0);
    assert!(out.over_budget);
    assert!(trash::exists(&p, TRASHED));
}

#[test]
fn a_damaged_job_file_stops_the_pass_before_anything_is_removed() {
    let (d, p) = setup();
    let orphan = picture(&p, 2);
    fs::create_dir_all(p.journal_dir()).unwrap();
    fs::write(p.journal_dir().join("sj-bad.json"), "{ not json").unwrap();
    fs::write(d.path().join(NOTE), note_text("kept", &[])).unwrap();
    let out = run(&p, &Settings::default());
    assert_eq!(out.skipped, vec!["a job file is damaged"]);
    assert!(Cas::new(p).has(&orphan));
}

#[test]
fn the_report_is_in_plain_words() {
    let r = Report {
        media_bytes: 5 * 1024 * 1024,
        cache_bytes: 0,
        cold_bytes: 2 * 1024 * 1024,
    };
    let text = describe(&r, Some(4 * 1024 * 1024));
    assert!(text.contains("5.0 MB of pictures"), "{text}");
    assert!(text.contains("over the 4.0 MB"), "{text}");
}
