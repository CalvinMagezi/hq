//! The shrink promise, tested by breaking it at every step.
//!
//! A simulated crash is injected at each recorded point of prepare, confirm and undo. After each,
//! before anything recovers, the original must be at its place or in the trash with a complete
//! record. Then recovery runs and the job must reach its end state with nothing lost.

use std::io::{Cursor, Write};
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use hq_suite_md::{Document, has_errors, validate};
use hq_suite_shrink::pipeline::{Faults, NoFaults, Notes, Options, OriginalAt, Recovered, Refusal};
use hq_suite_shrink::{ShrinkError, Shrinker};
use hq_suite_store::cas::references_in;
use hq_suite_store::{Cas, Journal, Step, SuitePaths, fsx};
use image::{DynamicImage, ImageFormat, RgbImage};

const SOURCE: &str = "Notebooks/Inbox/Imports/report.docx";
const DAY: &str = "2026-11-16";

struct FolderNotes(SuitePaths);

impl Notes for FolderNotes {
    fn read(&self, rel: &str) -> Result<Option<String>, ShrinkError> {
        match std::fs::read_to_string(self.0.resolve(rel)?) {
            Ok(t) => Ok(Some(t)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(ShrinkError::Notes(e.to_string())),
        }
    }
    fn write(&self, rel: &str, text: &str) -> Result<(), ShrinkError> {
        Ok(fsx::atomic_write(&self.0.resolve(rel)?, text.as_bytes())?)
    }
    fn remove(&self, rel: &str) -> Result<(), ShrinkError> {
        std::fs::remove_file(self.0.resolve(rel)?).map_err(|e| ShrinkError::Notes(e.to_string()))
    }
}

/// Fires at the `n`th point it is asked about and records every point's name.
struct CrashAt {
    n: usize,
    seen: AtomicUsize,
    names: Mutex<Vec<&'static str>>,
}

impl CrashAt {
    fn never() -> Self {
        Self::at(usize::MAX)
    }
    fn at(n: usize) -> Self {
        CrashAt {
            n,
            seen: AtomicUsize::new(0),
            names: Mutex::new(Vec::new()),
        }
    }
    fn names(&self) -> Vec<&'static str> {
        self.names.lock().unwrap().clone()
    }
}

impl Faults for CrashAt {
    fn point(&self, name: &'static str) -> Result<(), ShrinkError> {
        self.names.lock().unwrap().push(name);
        if self.seen.fetch_add(1, Ordering::SeqCst) == self.n {
            return Err(ShrinkError::Crashed(name));
        }
        Ok(())
    }
}

fn picture(w: u32, h: u32, seed: u32, fmt: ImageFormat) -> Vec<u8> {
    let mut img = RgbImage::new(w, h);
    for (x, y, p) in img.enumerate_pixels_mut() {
        let n = ((x * 31 + y * 17 + seed) % 29) as u8;
        *p = image::Rgb([
            (x % 251) as u8 ^ n,
            (y % 241) as u8,
            ((x + y + seed) % 233) as u8,
        ]);
    }
    let mut out = Vec::new();
    DynamicImage::ImageRgb8(img)
        .write_to(&mut Cursor::new(&mut out), fmt)
        .unwrap();
    out
}

const TYPES: &[u8] = br#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/></Types>"#;

/// A docx whose text is `text` and whose pictures are `pics` (stored without compression, as a
/// careless producer might, so there is something to save).
fn docx(text: &str, pics: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let body = format!(
        r#"<?xml version="1.0"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:body></w:document>"#
    );
    let mut buf = Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        let o = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        z.start_file("[Content_Types].xml", o).unwrap();
        z.write_all(TYPES).unwrap();
        z.start_file("word/document.xml", o).unwrap();
        z.write_all(body.as_bytes()).unwrap();
        for (name, data) in pics {
            z.start_file(format!("word/media/{name}"), o).unwrap();
            z.write_all(data).unwrap();
        }
        z.finish().unwrap();
    }
    buf.into_inner()
}

fn big_pictures() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("image1.jpeg", picture(1200, 900, 1, ImageFormat::Jpeg)),
        ("image2.png", picture(500, 400, 2, ImageFormat::Png)),
    ]
}

fn vault_with(rel: &str, bytes: &[u8]) -> (tempfile::TempDir, SuitePaths) {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, bytes).unwrap();
    let paths = SuitePaths::new(d.path());
    (d, paths)
}

fn shrinker<'a>(
    paths: &SuitePaths,
    notes: &'a FolderNotes,
    faults: &'a dyn Faults,
) -> Shrinker<'a> {
    Shrinker::new(paths.clone(), notes, faults, Options::default())
}

fn only_job(paths: &SuitePaths) -> hq_suite_store::Job {
    let jobs = Journal::new(paths).list().0;
    assert_eq!(jobs.len(), 1, "{jobs:?}");
    jobs.into_iter().next().unwrap()
}

/// Everything that must hold once a shrink is finished.
fn assert_finished(paths: &SuitePaths, original: &[u8]) {
    let job = only_job(paths);
    assert_eq!(job.step, Step::Done);
    assert!(
        !paths.vault().join(SOURCE).exists(),
        "the original left its place"
    );
    let Some(hq_suite_store::Disposition::Trash { target }) = &job.disposition else {
        panic!("no trash target");
    };
    assert_eq!(
        std::fs::read(paths.vault().join(target)).unwrap(),
        original,
        "the original is whole in the trash"
    );
    let note = std::fs::read_to_string(paths.vault().join(&job.note)).unwrap();
    let doc = Document::parse(&note);
    assert!(!has_errors(&validate(&doc)));
    let (meta, problems) = doc.meta();
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(meta.original_state, Some(hq_suite_md::OriginalState::Trash));
    assert_eq!(meta.saved_bytes, job.saved_bytes);
    let cas = Cas::new(paths.clone());
    for r in references_in(&note) {
        cas.verify(&r).unwrap();
    }
    assert!(
        Journal::new(paths).read_draft(&job.id).is_err(),
        "the draft is cleaned up"
    );
}

#[tokio::test]
async fn a_document_is_shrunk_checked_and_its_original_set_aside() {
    let original = docx("Quarterly report", &big_pictures());
    let (d, paths) = vault_with(SOURCE, &original);
    let notes = FolderNotes(paths.clone());
    let s = shrinker(&paths, &notes, &NoFaults);

    let preview = s.prepare(SOURCE, DAY).await.unwrap();
    assert_eq!(preview.step, Step::Verified);
    assert_eq!(preview.original_at, OriginalAt::Source);
    assert!(preview.saved_bytes > 0, "{preview:?}");
    assert!(preview.record_bytes < preview.source_bytes);
    assert_eq!(preview.note, "Notebooks/Inbox/Imports/report.md");
    assert_eq!(preview.pictures_recompressed, 2, "{preview:?}");
    assert!(
        d.path().join(SOURCE).exists(),
        "nothing is moved before agreement"
    );
    let note = std::fs::read_to_string(d.path().join(&preview.note)).unwrap();
    assert!(note.contains("Quarterly report"));
    assert!(note.contains("suite_original_state: kept"));
    assert!(note.contains("![Picture 1](../../../_media/cas/"), "{note}");

    // Asking again for the same file returns the same job and writes no second note.
    let again = s.prepare(SOURCE, DAY).await.unwrap();
    assert_eq!(again, preview);
    assert_eq!(Journal::new(&paths).list().0.len(), 1);

    let outcome = s.confirm(&preview.job, DAY).unwrap();
    assert_eq!(outcome.trash_path, format!("_trash/{DAY}/{SOURCE}"));
    assert_finished(&paths, &original);
    assert!(matches!(
        s.prepare(&outcome.trash_path, DAY).await,
        Err(ShrinkError::Refused(Refusal::ProtectedFolder))
    ));

    s.undo(&preview.job).unwrap();
    assert_eq!(std::fs::read(d.path().join(SOURCE)).unwrap(), original);
    let note = std::fs::read_to_string(d.path().join(&preview.note)).unwrap();
    assert!(note.contains("suite_original_state: kept"), "{note}");
    assert_eq!(only_job(&paths).step, Step::Undone);
}

#[tokio::test]
async fn the_same_picture_in_two_documents_is_stored_once() {
    let pics = big_pictures();
    let a = docx("First", &pics);
    let b = docx("Second", &pics);
    let (d, paths) = vault_with("Notebooks/a.docx", &a);
    std::fs::write(d.path().join("Notebooks/b.docx"), &b).unwrap();
    let notes = FolderNotes(paths.clone());
    let s = shrinker(&paths, &notes, &NoFaults);
    let first = s.prepare("Notebooks/a.docx", DAY).await.unwrap();
    let objects = Cas::new(paths.clone()).list().len();
    let second = s.prepare("Notebooks/b.docx", DAY).await.unwrap();
    assert_eq!(
        Cas::new(paths.clone()).list().len(),
        objects,
        "no new pictures were stored"
    );
    assert!(
        second.record_bytes < first.record_bytes / 4,
        "the second record costs only its text: {} vs {}",
        second.record_bytes,
        first.record_bytes
    );
}

#[tokio::test]
async fn refusals_are_specific_and_touch_nothing() {
    // A protected folder.
    let (_d, paths) = vault_with("_media/inbound.docx", &docx("x", &[]));
    let notes = FolderNotes(paths.clone());
    let s = shrinker(&paths, &notes, &NoFaults);
    assert!(matches!(
        s.prepare("_media/inbound.docx", DAY).await,
        Err(ShrinkError::Refused(Refusal::ProtectedFolder))
    ));
    // A path that leaves the vault.
    assert!(s.prepare("../outside.docx", DAY).await.is_err());
    // Not there.
    assert!(matches!(
        s.prepare("Notebooks/none.docx", DAY).await,
        Err(ShrinkError::Refused(Refusal::Unreadable))
    ));
    // A photo is not a text document yet.
    let (d, paths) = vault_with(
        "Notebooks/photo.jpg",
        &picture(200, 200, 3, ImageFormat::Jpeg),
    );
    let notes = FolderNotes(paths.clone());
    let s = shrinker(&paths, &notes, &NoFaults);
    assert!(matches!(
        s.prepare("Notebooks/photo.jpg", DAY).await,
        Err(ShrinkError::Refused(Refusal::NeedsOcr))
    ));
    // A file whose record would be bigger.
    std::fs::write(d.path().join("Notebooks/tiny.csv"), "a,b\n1,2\n").unwrap();
    assert!(matches!(
        s.prepare("Notebooks/tiny.csv", DAY).await,
        Err(ShrinkError::Refused(Refusal::NotSmaller { .. }))
    ));
    // Refusals leave no journal, no note.
    assert!(Journal::new(&paths).list().0.is_empty());
    assert!(!d.path().join("Notebooks/tiny.md").exists());
}

#[tokio::test]
async fn an_original_that_changed_is_never_moved() {
    let original = docx("Version one", &big_pictures());
    let (d, paths) = vault_with(SOURCE, &original);
    let notes = FolderNotes(paths.clone());
    let s = shrinker(&paths, &notes, &NoFaults);
    let preview = s.prepare(SOURCE, DAY).await.unwrap();
    let edited = docx("Version two, edited meanwhile", &big_pictures());
    std::fs::write(d.path().join(SOURCE), &edited).unwrap();
    assert!(matches!(
        s.confirm(&preview.job, DAY),
        Err(ShrinkError::SourceChanged)
    ));
    assert_eq!(std::fs::read(d.path().join(SOURCE)).unwrap(), edited);
    assert_eq!(only_job(&paths).step, Step::Verified);
}

#[tokio::test]
async fn a_damaged_or_missing_picture_stops_the_move() {
    for damage in ["flip", "delete"] {
        let original = docx("Report", &big_pictures());
        let (d, paths) = vault_with(SOURCE, &original);
        let notes = FolderNotes(paths.clone());
        let s = shrinker(&paths, &notes, &NoFaults);
        let preview = s.prepare(SOURCE, DAY).await.unwrap();
        let obj = Cas::new(paths.clone()).list().into_iter().next().unwrap();
        let file = d.path().join(obj.rel());
        match damage {
            "flip" => {
                let mut b = std::fs::read(&file).unwrap();
                b[10] ^= 0xFF;
                std::fs::write(&file, b).unwrap();
            }
            _ => std::fs::remove_file(&file).unwrap(),
        }
        assert!(
            matches!(
                s.confirm(&preview.job, DAY),
                Err(ShrinkError::NotVerified(_))
            ),
            "{damage}"
        );
        assert_eq!(
            std::fs::read(d.path().join(SOURCE)).unwrap(),
            original,
            "{damage}"
        );
    }
}

#[tokio::test]
async fn a_note_edited_after_the_preview_stops_the_move() {
    let original = docx("Report", &big_pictures());
    let (d, paths) = vault_with(SOURCE, &original);
    let notes = FolderNotes(paths.clone());
    let s = shrinker(&paths, &notes, &NoFaults);
    let preview = s.prepare(SOURCE, DAY).await.unwrap();
    let note_path = d.path().join(&preview.note);
    let mut text = std::fs::read_to_string(&note_path).unwrap();
    text.push_str("\nSomeone typed here.\n");
    std::fs::write(&note_path, text).unwrap();
    assert!(matches!(
        s.confirm(&preview.job, DAY),
        Err(ShrinkError::NotVerified(_))
    ));
    assert!(d.path().join(SOURCE).exists());
}

#[tokio::test]
async fn abandoning_removes_the_untouched_note_and_keeps_the_original() {
    let original = docx("Report", &big_pictures());
    let (d, paths) = vault_with(SOURCE, &original);
    let notes = FolderNotes(paths.clone());
    let s = shrinker(&paths, &notes, &NoFaults);
    let preview = s.prepare(SOURCE, DAY).await.unwrap();
    s.abandon(&preview.job).unwrap();
    assert!(!d.path().join(&preview.note).exists());
    assert_eq!(std::fs::read(d.path().join(SOURCE)).unwrap(), original);
    assert_eq!(only_job(&paths).step, Step::Abandoned);
    // After the move there is no abandoning, only undo.
    let again = s.prepare(SOURCE, DAY).await.unwrap();
    s.confirm(&again.job, DAY).unwrap();
    assert!(matches!(
        s.abandon(&again.job),
        Err(ShrinkError::WrongStep(..))
    ));
}

#[tokio::test]
async fn undo_refuses_to_overwrite_a_new_file_at_the_old_place() {
    let original = docx("Report", &big_pictures());
    let (d, paths) = vault_with(SOURCE, &original);
    let notes = FolderNotes(paths.clone());
    let s = shrinker(&paths, &notes, &NoFaults);
    let preview = s.prepare(SOURCE, DAY).await.unwrap();
    s.confirm(&preview.job, DAY).unwrap();
    std::fs::write(d.path().join(SOURCE), b"someone saved a new file here").unwrap();
    assert!(s.undo(&preview.job).is_err());
    assert_eq!(
        std::fs::read(d.path().join(SOURCE)).unwrap(),
        b"someone saved a new file here"
    );
    let trash = format!("_trash/{DAY}/{SOURCE}");
    assert_eq!(std::fs::read(d.path().join(trash)).unwrap(), original);
}

// ---------------------------------------------------------------------------------------------
// The crash matrix

fn names_of_clean_run(original: &[u8], with_undo: bool) -> (Vec<&'static str>, usize, usize) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (_d, paths) = vault_with(SOURCE, original);
    let notes = FolderNotes(paths.clone());
    let faults = CrashAt::never();
    let s = shrinker(&paths, &notes, &faults);
    let preview = rt.block_on(s.prepare(SOURCE, DAY)).unwrap();
    let after_prepare = faults.names().len();
    s.confirm(&preview.job, DAY).unwrap();
    let after_confirm = faults.names().len();
    if with_undo {
        s.undo(&preview.job).unwrap();
    }
    (faults.names(), after_prepare, after_confirm)
}

#[test]
fn a_crash_at_any_point_of_prepare_and_confirm_loses_nothing() {
    let original = docx("Crash test", &big_pictures());
    let (names, _after_prepare, _) = names_of_clean_run(&original, false);
    assert!(names.len() >= 10, "{names:?}");
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    for k in 0..names.len() {
        let (d, paths) = vault_with(SOURCE, &original);
        let notes = FolderNotes(paths.clone());
        let faults = CrashAt::at(k);
        let s = shrinker(&paths, &notes, &faults);

        let crashed = match rt.block_on(s.prepare(SOURCE, DAY)) {
            Ok(preview) => s.confirm(&preview.job, DAY).err(),
            Err(e) => Some(e),
        };
        assert!(
            matches!(crashed, Some(ShrinkError::Crashed(n)) if n == names[k]),
            "point {k} ({})",
            names[k]
        );

        // Right after the crash, before anything recovers.
        if let Some(job) = Journal::new(&paths).list().0.into_iter().next() {
            let clean = shrinker(&paths, &notes, &NoFaults);
            assert!(
                clean.nothing_lost(&job),
                "point {k} ({}): lost the original",
                names[k]
            );
            // The original leaves its place at the point named "moved", and not before.
            let moved_at = names.iter().position(|n| *n == "moved").unwrap();
            assert_eq!(
                d.path().join(SOURCE).exists(),
                k < moved_at,
                "point {k} ({})",
                names[k]
            );
        } else {
            assert!(
                d.path().join(SOURCE).exists(),
                "point {k} ({}): no job, so the original stays",
                names[k]
            );
        }

        // A restart. Recovery carries on what was agreed and waits for what was not.
        let after = shrinker(&paths, &notes, &NoFaults);
        let actions = after.recover();
        let job = Journal::new(&paths).list().0.into_iter().next();
        match job {
            None => {
                // The crash came before the job was recorded. Starting over works.
                after_prepare_again(&rt, &after, &d);
            }
            Some(job) => {
                if job.step != Step::Done {
                    assert!(
                        matches!(
                            actions.first().map(|a| &a.1),
                            Some(Recovered::ReadyForConsent)
                        ),
                        "point {k} ({}): {actions:?}",
                        names[k]
                    );
                    after.confirm(&job.id, DAY).unwrap();
                }
            }
        }
        assert_finished(&paths, &original);
    }
}

fn after_prepare_again(rt: &tokio::runtime::Runtime, s: &Shrinker<'_>, d: &tempfile::TempDir) {
    assert!(d.path().join(SOURCE).exists());
    let preview = rt.block_on(s.prepare(SOURCE, DAY)).unwrap();
    s.confirm(&preview.job, DAY).unwrap();
}

#[test]
fn a_crash_at_any_point_of_undo_loses_nothing() {
    let original = docx("Undo crash test", &big_pictures());
    let (names, _, after_confirm) = names_of_clean_run(&original, true);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    for k in after_confirm..names.len() {
        let (d, paths) = vault_with(SOURCE, &original);
        let notes = FolderNotes(paths.clone());
        let clean = shrinker(&paths, &notes, &NoFaults);
        let preview = rt.block_on(clean.prepare(SOURCE, DAY)).unwrap();
        clean.confirm(&preview.job, DAY).unwrap();

        let faults = CrashAt::at(k - after_confirm + 0);
        let s = shrinker(&paths, &notes, &faults);
        // The fault counter starts at zero for the undo alone.
        let err = s.undo(&preview.job).unwrap_err();
        assert!(matches!(err, ShrinkError::Crashed(_)), "point {k}: {err:?}");

        let job = only_job(&paths);
        assert!(
            clean.nothing_lost(&job),
            "undo point {k} ({}): lost the original",
            names[k]
        );
        clean.undo(&preview.job).unwrap();
        assert_eq!(
            std::fs::read(d.path().join(SOURCE)).unwrap(),
            original,
            "undo point {k}"
        );
        let note = std::fs::read_to_string(d.path().join(&preview.note)).unwrap();
        assert!(
            note.contains("suite_original_state: kept"),
            "undo point {k}"
        );
        assert_eq!(only_job(&paths).step, Step::Undone);
    }
}

#[test]
fn a_damaged_job_file_is_reported_and_blocks_nothing_else() {
    let original = docx("Report", &big_pictures());
    let (d, paths) = vault_with(SOURCE, &original);
    let jdir = d.path().join("_data/suite/journal");
    std::fs::create_dir_all(&jdir).unwrap();
    std::fs::write(jdir.join("sj-broken.json"), "{ nope").unwrap();
    let notes = FolderNotes(paths.clone());
    let s = shrinker(&paths, &notes, &NoFaults);
    assert!(s.recover().is_empty());
    assert!(
        jdir.join("sj-broken.json").exists(),
        "a damaged job file is never deleted"
    );
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    assert!(rt.block_on(s.prepare(SOURCE, DAY)).is_ok());
}

#[allow(dead_code)]
fn unused(_: &Path) {}
