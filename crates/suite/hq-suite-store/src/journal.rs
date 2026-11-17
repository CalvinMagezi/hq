//! The shrink journal: one small file per job saying how far it got.
//!
//! Shrinking a file is several steps, each safe to repeat: store the pictures, write the note,
//! check the note, set the original aside, record the saving. The journal records a step only
//! after it is done, and every step can be run again from where the record says. So after a crash
//! at any moment the job is resumed from its last recorded step, and the original is either still
//! where it was or in the trash path the journal already named. The one rule the steps enforce
//! between them: the original is never moved until the record is written and checked.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::cas::CasRef;
use crate::error::{Result, StoreError};
use crate::fsx;
use crate::paths::SuitePaths;

/// Bumped when the file's meaning changes. A newer file than this build understands is left
/// alone and reported, never rewritten.
pub const JOURNAL_VERSION: u32 = 1;

/// How far a job got. Order matters: a job only moves forward, apart from the ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    /// Every picture is in the media store and verified, and the note is drafted: its text sits
    /// in the journal folder. Nothing in the vault has changed except pictures no note links to.
    Planned,
    /// The note is written.
    NoteWritten,
    /// The note was read back and checked, and every picture it links to is present and intact.
    Verified,
    /// The person agreed to set the original aside, and the journal names where it goes.
    Consented,
    /// The original has left its place.
    OriginalMoved,
    /// Finished and recorded in the note.
    Done,
    /// Undone: the original is back where it was.
    Undone,
    /// Stopped before the original was touched.
    Abandoned,
}

impl Step {
    /// Whether the job has nothing left to do.
    pub fn is_terminal(self) -> bool {
        matches!(self, Step::Done | Step::Undone | Step::Abandoned)
    }
}

/// What happens to the original. More ways (a cold sink) arrive with milestone M3.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Disposition {
    /// Into the vault's trash at this vault-relative path, decided when consent is given.
    Trash { target: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaPlan {
    pub cas: CasRef,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Job {
    pub version: u32,
    pub id: String,
    /// The original, vault-relative.
    pub source: String,
    pub source_sha256: String,
    pub source_bytes: u64,
    /// The note that becomes the record, vault-relative.
    pub note: String,
    pub media: Vec<MediaPlan>,
    /// Hash of the drafted note text; the check reads the written note back and compares.
    pub note_sha256: String,
    /// The day the job was prepared, as `YYYY-MM-DD`. Fixed at the start so resuming on another
    /// day writes the same note.
    pub day: String,
    /// What the importer made the note from, kept for the preview.
    pub source_format: String,
    /// Which kinds of loss the importer reported, each once, as the importer names them.
    pub loss_kinds: Vec<String>,
    /// `native`, `safe`, `lossy` or `view`, as projected when the job was prepared.
    pub fidelity: String,
    /// Pictures that were recompressed, and pictures kept as they were.
    pub pictures_recompressed: u32,
    pub pictures_kept: u32,
    pub disposition: Option<Disposition>,
    pub step: Step,
    /// Original size minus the record's size (note plus new pictures), once known.
    pub saved_bytes: Option<i64>,
    pub failure: Option<String>,
    /// Set when the note is one that already existed (made by `hq vault import`, or by an earlier
    /// shrink that was undone): the hash of its text as it was, which the draft was built from.
    /// The text itself is kept beside the job so that giving up can put it back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adopted_note_sha256: Option<String>,
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

impl Job {
    /// The id for shrinking the file with this hash. The same file always gets the same id, so
    /// asking twice resumes or reports the first job instead of starting a second.
    pub fn id_for(source_sha256: &str) -> String {
        format!("sj-{}", &source_sha256[..source_sha256.len().min(16)])
    }

    /// Moves the job to `to`, if that is a step it may take from where it is. Repeating the
    /// current step is allowed, since every step is repeatable.
    pub fn advance(&mut self, to: Step) -> Result<()> {
        use Step::*;
        let from = self.step;
        let ok = to == from
            || matches!(
                (from, to),
                (Planned, NoteWritten)
                    | (NoteWritten, Verified)
                    | (Verified, Consented)
                    | (Consented, OriginalMoved)
                    | (OriginalMoved, Done)
                    | (Done | OriginalMoved, Undone)
            )
            // Before the original is touched a job may be given up.
            || (to == Abandoned && from < OriginalMoved);
        if !ok {
            return Err(StoreError::Journal(format!(
                "job {} cannot go from {from:?} to {to:?}",
                self.id
            )));
        }
        self.step = to;
        Ok(())
    }
}

/// The folder of job files.
#[derive(Debug, Clone)]
pub struct Journal {
    dir: PathBuf,
}

impl Journal {
    pub fn new(paths: &SuitePaths) -> Self {
        Journal {
            dir: paths.journal_dir(),
        }
    }

    fn file(&self, id: &str) -> Result<PathBuf> {
        if !valid_id(id) {
            return Err(StoreError::Journal(format!("`{id}` is not a job id")));
        }
        Ok(self.dir.join(format!("{id}.json")))
    }

    /// Writes a new job. Fails if the id is taken.
    pub fn create(&self, job: &Job) -> Result<()> {
        let bytes =
            serde_json::to_vec_pretty(job).map_err(|e| StoreError::Journal(e.to_string()))?;
        if fsx::write_new(&self.file(&job.id)?, &bytes)? {
            Ok(())
        } else {
            Err(StoreError::Exists(format!("job {}", job.id)))
        }
    }

    /// Records the job's current state, replacing the file whole.
    pub fn save(&self, job: &Job) -> Result<()> {
        let bytes =
            serde_json::to_vec_pretty(job).map_err(|e| StoreError::Journal(e.to_string()))?;
        fsx::atomic_write(&self.file(&job.id)?, &bytes)
    }

    pub fn load(&self, id: &str) -> Result<Job> {
        let path = self.file(id)?;
        let text = fs::read_to_string(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                StoreError::Missing(format!("job {id}"))
            } else {
                StoreError::io(&path, e)
            }
        })?;
        let job: Job = serde_json::from_str(&text)
            .map_err(|e| StoreError::Journal(format!("job {id} is unreadable: {e}")))?;
        if job.version > JOURNAL_VERSION {
            return Err(StoreError::Journal(format!(
                "job {id} was written by a newer version of HQ"
            )));
        }
        if job.id != id {
            return Err(StoreError::Journal(format!(
                "job file {id} names another job"
            )));
        }
        Ok(job)
    }

    /// Every readable job, oldest id first, and the ids of files that could not be read. A damaged
    /// file is reported, never deleted: it may be the only record of where an original went.
    pub fn list(&self) -> (Vec<Job>, Vec<String>) {
        let mut jobs = Vec::new();
        let mut damaged = Vec::new();
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return (jobs, damaged);
        };
        let mut ids: Vec<String> = entries
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                name.strip_suffix(".json").map(str::to_string)
            })
            .collect();
        ids.sort();
        for id in ids {
            match self.load(&id) {
                Ok(j) => jobs.push(j),
                Err(_) => damaged.push(id),
            }
        }
        (jobs, damaged)
    }

    /// Jobs that still have something to do.
    pub fn unfinished(&self) -> Vec<Job> {
        self.list()
            .0
            .into_iter()
            .filter(|j| !j.step.is_terminal())
            .collect()
    }

    fn draft_file(&self, id: &str) -> Result<PathBuf> {
        self.file(id).map(|f| f.with_extension("md"))
    }

    fn prior_file(&self, id: &str) -> Result<PathBuf> {
        self.file(id).map(|f| f.with_extension("prior"))
    }

    /// Keeps the drafted note's text beside the job, so a resumed job writes exactly it.
    pub fn write_draft(&self, id: &str, text: &str) -> Result<()> {
        fsx::atomic_write(&self.draft_file(id)?, text.as_bytes())
    }

    pub fn read_draft(&self, id: &str) -> Result<String> {
        let path = self.draft_file(id)?;
        fs::read_to_string(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                StoreError::Missing(format!("draft of job {id}"))
            } else {
                StoreError::io(&path, e)
            }
        })
    }

    /// Keeps the text an adopted note had before this job changed it.
    pub fn write_prior(&self, id: &str, text: &str) -> Result<()> {
        fsx::atomic_write(&self.prior_file(id)?, text.as_bytes())
    }

    pub fn read_prior(&self, id: &str) -> Result<String> {
        let path = self.prior_file(id)?;
        fs::read_to_string(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                StoreError::Missing(format!("earlier note of job {id}"))
            } else {
                StoreError::io(&path, e)
            }
        })
    }

    pub fn remove_prior(&self, id: &str) {
        if let Ok(path) = self.prior_file(id) {
            let _ = fs::remove_file(path);
        }
    }

    /// Removes a draft once its job is finished. Missing is fine.
    pub fn remove_draft(&self, id: &str) {
        if let Ok(path) = self.draft_file(id) {
            let _ = fs::remove_file(path);
        }
    }

    /// Clears unfinished temporary files a crash left in the journal folder.
    pub fn sweep(&self) -> usize {
        fsx::sweep_partials(&self.dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn job(id: &str) -> Job {
        Job {
            version: JOURNAL_VERSION,
            id: id.to_string(),
            source: "Notebooks/Inbox/Imports/a.docx".into(),
            source_sha256: "ab".repeat(32),
            source_bytes: 1000,
            note: "Notebooks/Inbox/Imports/a.md".into(),
            media: vec![MediaPlan {
                cas: CasRef::new(&"cd".repeat(32), "webp").unwrap(),
                bytes: 100,
            }],
            note_sha256: "ef".repeat(32),
            day: "2026-11-16".into(),
            source_format: "docx".into(),
            loss_kinds: vec![],
            fidelity: "safe".into(),
            pictures_recompressed: 1,
            pictures_kept: 0,
            disposition: None,
            step: Step::Planned,
            saved_bytes: None,
            failure: None,
            adopted_note_sha256: None,
        }
    }

    fn journal() -> (tempfile::TempDir, Journal) {
        let d = tempfile::tempdir().unwrap();
        let j = Journal::new(&SuitePaths::new(d.path()));
        (d, j)
    }

    #[test]
    fn a_job_round_trips_through_its_file() {
        let (_d, j) = journal();
        let mut a = job("sj-0001");
        j.create(&a).unwrap();
        assert!(matches!(j.create(&a), Err(StoreError::Exists(_))));
        a.advance(Step::NoteWritten).unwrap();
        a.disposition = Some(Disposition::Trash {
            target: "_trash/2026-11-16/x".into(),
        });
        j.save(&a).unwrap();
        assert_eq!(j.load("sj-0001").unwrap(), a);
        assert!(matches!(j.load("sj-9999"), Err(StoreError::Missing(_))));
    }

    #[test]
    fn steps_only_go_forward_and_repeat_safely() {
        let mut a = job("sj-1");
        for s in [
            Step::NoteWritten,
            Step::NoteWritten,
            Step::Verified,
            Step::Consented,
            Step::OriginalMoved,
            Step::Done,
        ] {
            a.advance(s).unwrap();
        }
        assert!(a.advance(Step::Planned).is_err());
        assert!(
            a.advance(Step::Abandoned).is_err(),
            "once the original has moved there is no walking away"
        );
        a.advance(Step::Undone).unwrap();
        assert!(a.advance(Step::Done).is_err());

        let mut b = job("sj-2");
        assert!(b.advance(Step::Verified).is_err(), "no skipping");
        assert!(b.advance(Step::Done).is_err());
        b.advance(Step::Abandoned).unwrap();
        assert!(b.step.is_terminal());

        let mut c = job("sj-3");
        for s in [Step::NoteWritten, Step::Verified, Step::Consented] {
            c.advance(s).unwrap();
        }
        c.advance(Step::Abandoned).unwrap();
    }

    #[test]
    fn ids_are_derived_from_the_file_and_checked() {
        assert_eq!(Job::id_for(&"ab".repeat(32)), "sj-abababababababab");
        let (_d, j) = journal();
        for bad in ["", "../x", "A", "a/b", "a.json", &"a".repeat(65)] {
            assert!(j.load(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn listing_reports_damage_and_never_deletes_it() {
        let (d, j) = journal();
        j.create(&job("sj-a")).unwrap();
        fs::write(d.path().join("_data/suite/journal/sj-b.json"), "{ not json").unwrap();
        let mut newer = job("sj-c");
        newer.version = JOURNAL_VERSION + 1;
        fs::write(
            d.path().join("_data/suite/journal/sj-c.json"),
            serde_json::to_string(&newer).unwrap(),
        )
        .unwrap();
        let (jobs, damaged) = j.list();
        assert_eq!(jobs.len(), 1);
        assert_eq!(damaged, vec!["sj-b".to_string(), "sj-c".to_string()]);
        assert!(d.path().join("_data/suite/journal/sj-b.json").exists());
        assert_eq!(j.unfinished().len(), 1);
    }

    #[test]
    fn finished_jobs_are_not_unfinished_and_sweep_clears_partials() {
        let (d, j) = journal();
        let mut a = job("sj-done");
        j.create(&a).unwrap();
        a.advance(Step::Abandoned).unwrap();
        j.save(&a).unwrap();
        assert!(j.unfinished().is_empty());
        fs::write(
            d.path()
                .join(format!("_data/suite/journal/.sj-x{}9-9", fsx::PARTIAL_MARK)),
            "p",
        )
        .unwrap();
        assert_eq!(j.sweep(), 1);
    }

    #[test]
    fn a_draft_is_kept_beside_its_job_and_is_not_a_job() {
        let (_d, j) = journal();
        j.create(&job("sj-d")).unwrap();
        j.write_draft("sj-d", "# note\n").unwrap();
        assert_eq!(j.read_draft("sj-d").unwrap(), "# note\n");
        assert_eq!(j.list().0.len(), 1, "the draft is not listed as a job");
        j.remove_draft("sj-d");
        j.remove_draft("sj-d");
        assert!(matches!(j.read_draft("sj-d"), Err(StoreError::Missing(_))));
    }

    #[test]
    fn a_job_file_naming_another_job_is_refused() {
        let (d, j) = journal();
        let a = job("sj-real");
        fs::create_dir_all(d.path().join("_data/suite/journal")).unwrap();
        fs::write(
            d.path().join("_data/suite/journal/sj-fake.json"),
            serde_json::to_string(&a).unwrap(),
        )
        .unwrap();
        assert!(j.load("sj-fake").is_err());
    }
}
