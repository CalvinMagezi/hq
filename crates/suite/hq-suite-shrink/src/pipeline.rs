//! Shrinking one file: from a document in the vault to a small record, with the original set
//! aside only after the record is written, checked, and the person has agreed.
//!
//! The steps, each safe to repeat (`hq_suite_store::journal` records how far a job got):
//!
//! 1. **prepare**: read the file, store its pictures (recompressed, once, by hash), draft the
//!    note, record the job, write the note into the vault, and check it. The original has not been
//!    touched. The result is a [`Preview`] for the person to look at.
//! 2. **confirm**: on consent, check the record again, then move the original to the vault's
//!    trash, then mark the note. Never before the check, never if it failed.
//! 3. **undo**: put the original back, within the trash's retention.
//!
//! After a crash at any moment, [`Shrinker::recover`] finishes what was agreed to and leaves what
//! was not. The rule it protects: the original is at its place or in the trash path the journal
//! named, and the record is complete whenever the original is not at its place.

use std::collections::{HashMap, HashSet};
use std::ops::ControlFlow;
use std::sync::Arc;

use hq_convert::ingest::ConvertOptions;
use hq_convert::ingest::sniff::{Kind, sniff};
use hq_suite_ingest::{FailureKind, ImageFormat, for_each_media, read_document};
use hq_suite_md::frontmatter::{self, Scalar};
use hq_suite_md::meta::{
    KEY_FIDELITY, KEY_KIND, KEY_ORIGINAL_BYTES, KEY_ORIGINAL_STATE, KEY_SAVED_BYTES, KEY_SHRUNK_AT,
};
use hq_suite_md::{Document, Op, Patch, Severity, apply, has_errors, validate};
use hq_suite_store::cas::references_in;
use hq_suite_store::fsx::sha256_bytes;
use hq_suite_store::journal::MediaPlan;
use hq_suite_store::{
    Cas, ColdStore, Disposition, Job, Journal, Step, StoreError, SuitePaths, trash,
};
use serde::Serialize;

use crate::codec::{self, Encoder};
use crate::measure::loss_label;

/// How the vault's notes are read and written. The adapter implements this over the vault client,
/// so a write takes the per-note lock and reaches the search index; tests use a plain folder.
pub trait Notes: Send + Sync {
    fn read(&self, rel: &str) -> Result<Option<String>, ShrinkError>;
    /// Writes the whole note, atomically.
    fn write(&self, rel: &str, text: &str) -> Result<(), ShrinkError>;
    fn remove(&self, rel: &str) -> Result<(), ShrinkError>;

    /// Reads the note, lets `change` produce its new text, and writes that if there is one, all
    /// under one lock where the store has locks. The default is read then write, which is only
    /// as safe as the store; the vault's own implementation holds the note's lock throughout, so
    /// an edit made elsewhere in between cannot be overwritten.
    fn modify(
        &self,
        rel: &str,
        change: &dyn Fn(&str) -> Result<Option<String>, ShrinkError>,
    ) -> Result<(), ShrinkError> {
        let text = self
            .read(rel)?
            .ok_or_else(|| ShrinkError::NotVerified("the note is missing".into()))?;
        match change(&text)? {
            Some(next) => self.write(rel, &next),
            None => Ok(()),
        }
    }

    /// Removes the note only if its text still hashes to `sha256`, under one lock where there is
    /// one. Returns whether it was removed.
    fn remove_if_unchanged(&self, rel: &str, sha256: &str) -> Result<bool, ShrinkError> {
        match self.read(rel)? {
            Some(text) if sha256_bytes(text.as_bytes()) == sha256 => {
                self.remove(rel)?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }
}

/// Where a crash can be simulated. [`NoFaults`] in production.
pub trait Faults: Send + Sync {
    fn point(&self, name: &'static str) -> Result<(), ShrinkError>;
}

pub struct NoFaults;

impl Faults for NoFaults {
    fn point(&self, _name: &'static str) -> Result<(), ShrinkError> {
        Ok(())
    }
}

/// Why a file was not shrunk, in terms a person can act on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum Refusal {
    /// A password protects it. Open it, save an unprotected copy, and try that.
    Encrypted,
    /// A scan or photo with no text layer. These keep their page images and are not shrunk yet.
    NeedsOcr,
    Unsupported,
    TooLarge,
    /// The file lives in a folder HQ keeps for itself (`_media`, `_trash`, `_data`, ...).
    ProtectedFolder,
    /// The record would not be smaller than the original.
    NotSmaller {
        saved_bytes: i64,
    },
    /// The converted text would not be valid hqmd; the first problem is named.
    Unrepresentable(String),
    /// It was shrunk before and the job is finished (undo it first).
    AlreadyShrunk,
    Unreadable,
}

#[derive(Debug, thiserror::Error)]
pub enum ShrinkError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("not shrunk: {0:?}")]
    Refused(Refusal),
    /// The original changed after the plan was made; nothing was moved.
    #[error("the original changed since it was read")]
    SourceChanged,
    /// The record did not check out; the original was left where it is.
    #[error("the record did not check out: {0}")]
    NotVerified(String),
    /// A note exists where this job wants to write one, and it is not this job's.
    #[error("{0} already exists")]
    NoteConflict(String),
    #[error("no such job")]
    NoSuchJob,
    #[error("job is {0:?}; {1}")]
    WrongStep(Step, &'static str),
    /// The original is neither at its place nor in the trash. Reported loudly; never repaired by guessing.
    #[error("the original is missing")]
    OriginalMissing,
    #[error("note store: {0}")]
    Notes(String),
    /// The cold sink could not store, find or return the original.
    #[error("the sink: {0}")]
    Sink(String),
    /// A simulated crash (tests).
    #[error("crashed at {0}")]
    Crashed(&'static str),
}

/// How pictures are recompressed. The encoder was picked by measurement (docs/suite/MEASURE.md).
#[derive(Debug, Clone, Copy)]
pub struct ImagePolicy {
    /// For photographs (JPEG sources).
    pub photo: Encoder,
    /// For graphics (PNG and WebP sources): lossless, since text and line art show every artefact.
    pub graphic: Encoder,
    pub max_long_edge: u32,
    pub max_pixels: u64,
}

impl Default for ImagePolicy {
    fn default() -> Self {
        ImagePolicy {
            photo: Encoder::WebpLossy {
                quality: crate::codec::DEFAULT_WEBP_QUALITY,
            },
            graphic: Encoder::WebpLossless,
            max_long_edge: 1600,
            max_pixels: 50_000_000,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Options {
    pub image: ImagePolicy,
    pub convert: ConvertOptions,
    pub max_source_bytes: u64,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            image: ImagePolicy::default(),
            // No OCR and no outside programs: a shrink reads only what is in the file.
            convert: ConvertOptions {
                ocr: hq_convert::ingest::OcrMode::Off,
                external_office: false,
                ..ConvertOptions::default()
            },
            max_source_bytes: 64 * 1024 * 1024,
        }
    }
}

/// Sets a note's `suite_original_state` and `suite_saved_bytes`, leaving every other byte. Also
/// used by the governor when an original has aged out of the trash.
pub fn set_original_state(
    notes: &dyn Notes,
    note: &str,
    state: &str,
    saved: i64,
) -> Result<(), ShrinkError> {
    notes.modify(note, &|text| {
        let doc = Document::parse(text);
        let patch = Patch {
            base_revision: None,
            ops: vec![
                Op::SetMeta {
                    key: KEY_ORIGINAL_STATE.into(),
                    value: Scalar::Str(state.into()),
                },
                Op::SetMeta {
                    key: KEY_SAVED_BYTES.into(),
                    value: Scalar::Int(saved),
                },
            ],
        };
        let next = apply(&doc, &patch)
            .map_err(|e| ShrinkError::Notes(e.to_string()))?
            .render();
        Ok((next != text).then_some(next))
    })
}

/// Marks a note's original as sealed in `sink`.
pub fn set_offloaded(
    notes: &dyn Notes,
    note: &str,
    sink: &str,
    saved: i64,
) -> Result<(), ShrinkError> {
    patch_meta(
        notes,
        note,
        vec![
            Op::SetMeta {
                key: KEY_ORIGINAL_STATE.into(),
                value: Scalar::Str("offloaded".into()),
            },
            Op::SetMeta {
                key: hq_suite_md::meta::KEY_ORIGINAL_SINK.into(),
                value: Scalar::Str(sink.into()),
            },
            Op::SetMeta {
                key: KEY_SAVED_BYTES.into(),
                value: Scalar::Int(saved),
            },
        ],
    )
}

/// Marks a note's original as back in its place, and forgets the sink.
pub fn set_original_kept(notes: &dyn Notes, note: &str) -> Result<(), ShrinkError> {
    patch_meta(
        notes,
        note,
        vec![
            Op::SetMeta {
                key: KEY_ORIGINAL_STATE.into(),
                value: Scalar::Str("kept".into()),
            },
            Op::RemoveMeta {
                key: hq_suite_md::meta::KEY_ORIGINAL_SINK.into(),
            },
            Op::SetMeta {
                key: KEY_SAVED_BYTES.into(),
                value: Scalar::Int(0),
            },
        ],
    )
}

fn patch_meta(notes: &dyn Notes, note: &str, ops: Vec<Op>) -> Result<(), ShrinkError> {
    notes.modify(note, &|text| {
        let patch = Patch {
            base_revision: None,
            ops: ops.clone(),
        };
        let next = apply(&Document::parse(text), &patch)
            .map_err(|e| ShrinkError::Notes(e.to_string()))?
            .render();
        Ok((next != text).then_some(next))
    })
}

/// Where the original is right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OriginalAt {
    Source,
    Trash,
    /// Sealed in a sink the owner keeps; fetched back on undo.
    Sink,
    Nowhere,
}

/// What the person sees before agreeing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Preview {
    pub job: String,
    pub source: String,
    pub note: String,
    pub step: Step,
    pub fidelity: String,
    pub source_format: String,
    /// What the importer reports it could not carry, by kind (`charts_omitted`, ...).
    pub loss_kinds: Vec<String>,
    pub source_bytes: u64,
    pub record_bytes: u64,
    pub saved_bytes: i64,
    pub pictures_recompressed: u32,
    pub pictures_kept: u32,
    pub original_at: OriginalAt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Outcome {
    pub job: String,
    pub saved_bytes: i64,
    /// Where the original is now, in the vault's trash.
    pub trash_path: String,
}

/// What recovery did for a job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recovered {
    /// Brought back to the point where it waits for the person's agreement.
    ReadyForConsent,
    /// The agreement had been given; the job was carried through.
    Finished,
    /// Could not be carried on; the reason. Nothing was moved.
    Stuck(String),
}

pub struct Shrinker<'a> {
    pub paths: SuitePaths,
    pub notes: &'a dyn Notes,
    pub faults: &'a dyn Faults,
    pub options: Options,
}

fn refuse(r: Refusal) -> ShrinkError {
    ShrinkError::Refused(r)
}

/// Folders HQ keeps for itself. A file there is not the person's document.
fn is_protected(rel: &str) -> bool {
    rel.split('/')
        .next()
        .is_some_and(|top| top.starts_with('_'))
}

fn note_rel_for(source: &str, taken: impl Fn(&str) -> bool) -> String {
    let stem = match source.rfind('.') {
        Some(dot) if dot > source.rfind('/').unwrap_or(0) => &source[..dot],
        _ => source,
    };
    let first = format!("{stem}.md");
    if !taken(&first) {
        return first;
    }
    (2..100)
        .map(|n| format!("{stem}-{n}.md"))
        .find(|c| !taken(c))
        .unwrap_or_else(|| format!("{stem}-{}.md", &sha256_bytes(source.as_bytes())[..8]))
}

fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

struct Stored1 {
    plans: Vec<MediaPlan>,
    /// Bytes of objects that were not in the store before this job.
    new_bytes: u64,
    recompressed: u32,
    kept: u32,
}

/// Recompresses and stores every picture in a package. Runs on the blocking pool.
fn store_pictures(
    bytes: &[u8],
    kind: Kind,
    cas: &Cas,
    policy: ImagePolicy,
    limits: &hq_convert::ingest::Limits,
    shared: &HashSet<hq_suite_store::CasRef>,
) -> Result<Stored1, ShrinkError> {
    let mut out = Stored1 {
        plans: Vec::new(),
        new_bytes: 0,
        recompressed: 0,
        kept: 0,
    };
    let mut failure: Option<ShrinkError> = None;
    let mut seen: HashMap<String, ()> = HashMap::new();
    let scan = for_each_media(bytes, kind, limits, |m| {
        if seen.insert(m.sha256.clone(), ()).is_some() {
            return ControlFlow::Continue(());
        }
        let encoder = match m.format {
            ImageFormat::Jpeg => Some(policy.photo),
            ImageFormat::Png | ImageFormat::Webp => Some(policy.graphic),
            _ => None,
        };
        // A picture is recompressed only when that gives something smaller and readable;
        // otherwise the original bytes are kept as they are.
        let mut chosen: (Vec<u8>, &str) = (m.bytes.to_vec(), ext_of(m.part, m.format));
        let mut did = false;
        if let Some(enc) = encoder
            && let Ok(img) = codec::decode_limited(m.bytes, policy.max_pixels)
        {
            let img = codec::fit_long_edge(img, policy.max_long_edge);
            if let Ok(done) = codec::encode(&img, enc)
                && (done.bytes.len() as u64) < m.bytes.len() as u64
            {
                chosen = (done.bytes, enc.extension());
                did = true;
            }
        }
        match cas.put(&chosen.0, chosen.1) {
            Ok((r, _)) => {
                // A picture another job's record already links costs this one nothing. One only an
                // earlier, unfinished attempt of this job stored does cost it, so a retry never
                // makes the saving look larger.
                if !shared.contains(&r) {
                    out.new_bytes += chosen.0.len() as u64;
                }
                out.plans.push(MediaPlan {
                    cas: r,
                    bytes: chosen.0.len() as u64,
                });
                if did {
                    out.recompressed += 1;
                } else {
                    out.kept += 1;
                }
                ControlFlow::Continue(())
            }
            Err(e) => {
                failure = Some(e.into());
                ControlFlow::Break(())
            }
        }
    })
    .map_err(|_| refuse(Refusal::Unreadable))?;
    let _ = scan;
    match failure {
        Some(e) => Err(e),
        None => Ok(out),
    }
}

fn ext_of(part: &str, format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Jpeg => "jpg",
        ImageFormat::Png => "png",
        ImageFormat::Gif => "gif",
        ImageFormat::Webp => "webp",
        ImageFormat::Tiff => "tif",
        ImageFormat::Bmp => "bmp",
        ImageFormat::Emf => "emf",
        ImageFormat::Wmf => "wmf",
        ImageFormat::Svg => "svg",
        ImageFormat::Other => {
            let _ = part;
            "bin"
        }
    }
}

impl<'a> Shrinker<'a> {
    pub fn new(
        paths: SuitePaths,
        notes: &'a dyn Notes,
        faults: &'a dyn Faults,
        options: Options,
    ) -> Self {
        Shrinker {
            paths,
            notes,
            faults,
            options,
        }
    }

    fn journal(&self) -> Journal {
        Journal::new(&self.paths)
    }

    fn cas(&self) -> Cas {
        Cas::new(self.paths.clone())
    }

    fn point(&self, name: &'static str) -> Result<(), ShrinkError> {
        self.faults.point(name)
    }

    /// Where the original of `job` is.
    pub fn original_at(&self, job: &Job) -> OriginalAt {
        if trash::exists(&self.paths, &job.source) {
            return OriginalAt::Source;
        }
        match &job.disposition {
            Some(Disposition::Trash { target }) if trash::exists(&self.paths, target) => {
                OriginalAt::Trash
            }
            Some(Disposition::Sink { .. }) => OriginalAt::Sink,
            _ => OriginalAt::Nowhere,
        }
    }

    /// The promise, checkable at any moment: the original is somewhere, and if it is not at its
    /// place the record is complete. Tests assert it after every simulated crash.
    pub fn nothing_lost(&self, job: &Job) -> bool {
        self.nothing_lost_with(job, None)
    }

    /// As [`nothing_lost`](Self::nothing_lost), asking `sink` whether it holds an original that
    /// was sealed into it.
    pub fn nothing_lost_with(&self, job: &Job, sink: Option<&dyn ColdStore>) -> bool {
        match self.original_at(job) {
            OriginalAt::Source => true,
            OriginalAt::Trash => self.verify_record(job, false).is_ok(),
            OriginalAt::Sink => match (&job.disposition, sink) {
                (Some(Disposition::Sink { sha256, .. }), Some(s)) => {
                    s.has(sha256) && self.verify_record(job, false).is_ok()
                }
                _ => false,
            },
            OriginalAt::Nowhere => false,
        }
    }

    // -----------------------------------------------------------------------------------------
    // prepare

    /// Steps 1 to 5: store the pictures, draft and write the note, check it. The original is not
    /// touched. Asking again for a file already prepared returns its preview.
    pub async fn prepare(&self, source: &str, day: &str) -> Result<Preview, ShrinkError> {
        if is_protected(source) {
            return Err(refuse(Refusal::ProtectedFolder));
        }
        let path = self.paths.resolve(source)?;
        let meta = std::fs::metadata(&path).map_err(|_| refuse(Refusal::Unreadable))?;
        if !meta.is_file() {
            return Err(refuse(Refusal::Unreadable));
        }
        if meta.len() > self.options.max_source_bytes {
            return Err(refuse(Refusal::TooLarge));
        }
        let data = std::fs::read(&path).map_err(|_| refuse(Refusal::Unreadable))?;
        let sha = sha256_bytes(&data);
        let id = Job::id_for(&sha);

        let journal = self.journal();
        journal.sweep();
        match journal.load(&id) {
            Ok(job) if job.step == Step::Done => return Err(refuse(Refusal::AlreadyShrunk)),
            Ok(job) if !job.step.is_terminal() => {
                // Same file, a job in progress: carry it to the preview.
                let mut job = job;
                self.run_to_verified(&mut job)?;
                return Ok(self.preview_of(&job));
            }
            Ok(_) | Err(StoreError::Missing(_)) => {}
            Err(e) => return Err(e.into()),
        }

        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_string();
        let kind = sniff(&data, &ext).kind;
        if kind == Kind::Image {
            return Err(refuse(Refusal::NeedsOcr));
        }
        let data = Arc::new(data);
        let doc = read_document((*data).clone(), &ext, &self.options.convert)
            .await
            .map_err(|f| {
                refuse(match f.kind {
                    FailureKind::Encrypted => Refusal::Encrypted,
                    FailureKind::NeedsOcr => Refusal::NeedsOcr,
                    FailureKind::TooLarge => Refusal::TooLarge,
                    FailureKind::Unsupported | FailureKind::NeedsLibreOffice => {
                        Refusal::Unsupported
                    }
                    FailureKind::Error => Refusal::Unreadable,
                })
            })?;

        let pictures = {
            let shared: HashSet<_> = journal
                .list()
                .0
                .iter()
                .filter(|j| j.id != id && j.step != Step::Abandoned)
                .flat_map(|j| j.media.iter().map(|m| m.cas.clone()))
                .collect();
            let (d, cas, policy, limits) = (
                Arc::clone(&data),
                self.cas(),
                self.options.image,
                self.options.convert.limits.clone(),
            );
            tokio::task::spawn_blocking(move || {
                store_pictures(&d, kind, &cas, policy, &limits, &shared)
            })
            .await
            .map_err(|_| refuse(Refusal::Unreadable))??
        };
        self.point("pictures_stored")?;

        let adoptable = self.adoptable_note(source, &sha)?;
        let note_rel = match &adoptable {
            Some((rel, _)) => rel.clone(),
            None => note_rel_for(source, |c| self.notes.read(c).ok().flatten().is_some()),
        };
        let title = doc
            .title
            .as_deref()
            .map(one_line)
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| {
                let name = source.rsplit('/').next().unwrap_or(source);
                one_line(name.rsplit_once('.').map_or(name, |(s, _)| s))
            });
        let text = match &adoptable {
            Some((_, existing)) => {
                self.draft_adopted(existing, &note_rel, data.len() as u64, day, &doc, &pictures)?
            }
            None => self.draft_note(
                source,
                &note_rel,
                &sha,
                data.len() as u64,
                &title,
                day,
                &doc,
                &pictures,
            )?,
        };

        let parsed = Document::parse(&text);
        let diags = validate(&parsed);
        if has_errors(&diags) {
            let first = diags.iter().find(|d| d.severity == Severity::Error);
            return Err(refuse(Refusal::Unrepresentable(
                first.map_or_else(String::new, |d| d.message.clone()),
            )));
        }

        // An adopted note already took its own bytes; only what this job adds counts against
        // the original.
        let note_added = match &adoptable {
            Some((_, existing)) => text.len() as i64 - existing.len() as i64,
            None => text.len() as i64,
        };
        let saved = data.len() as i64 - note_added - pictures.new_bytes as i64;
        if saved <= 0 {
            return Err(refuse(Refusal::NotSmaller { saved_bytes: saved }));
        }

        let mut job = Job {
            version: hq_suite_store::journal::JOURNAL_VERSION,
            id: id.clone(),
            source: source.to_string(),
            source_sha256: sha,
            source_bytes: data.len() as u64,
            note: note_rel,
            media: pictures.plans,
            note_sha256: sha256_bytes(text.as_bytes()),
            day: day.to_string(),
            source_format: doc.projection.kind.label().to_string(),
            loss_kinds: doc
                .projection
                .loss_kinds
                .iter()
                .map(|k| loss_label(*k))
                .collect(),
            fidelity: doc.projection.fidelity.as_str().to_string(),
            pictures_recompressed: pictures.recompressed,
            pictures_kept: pictures.kept,
            disposition: None,
            step: Step::Planned,
            saved_bytes: Some(saved),
            failure: None,
            adopted_note_sha256: adoptable.as_ref().map(|(_, t)| sha256_bytes(t.as_bytes())),
        };
        if let Some((_, existing)) = &adoptable {
            // A second request for the same file may have gone further while this one worked:
            // then the note is already its draft, and its earlier text is the one on file.
            if let Ok(mut other) = journal.load(&id)
                && !other.step.is_terminal()
            {
                self.run_to_verified(&mut other)?;
                return Ok(self.preview_of(&other));
            }
            journal.write_prior(&id, existing)?;
        }
        journal.write_draft(&id, &text)?;
        self.point("draft_written")?;
        // A finished job under this id (undone or abandoned) is replaced by the new one.
        if journal.load(&id).is_ok() {
            journal.save(&job)?;
        } else {
            journal.create(&job)?;
        }
        self.point("job_created")?;

        self.run_to_verified(&mut job)?;
        Ok(self.preview_of(&job))
    }

    /// The note an earlier import (or an undone shrink) already made for this exact file, if the
    /// name a new record would take is held by one. Its text is returned as it is now.
    fn adoptable_note(
        &self,
        source: &str,
        sha: &str,
    ) -> Result<Option<(String, String)>, ShrinkError> {
        let rel = note_rel_for(source, |_| false);
        let Some(text) = self.notes.read(&rel)? else {
            return Ok(None);
        };
        let same_file = frontmatter::get(Document::parse(&text).frontmatter(), "source_sha256")
            .is_some_and(|v| v == sha);
        Ok(same_file.then_some((rel, text)))
    }

    /// The existing note with the suite's keys set and any picture it does not yet show added in a
    /// "Pictures" section at the end. Every other byte of the note is the person's and stays.
    fn draft_adopted(
        &self,
        existing: &str,
        note_rel: &str,
        bytes: u64,
        day: &str,
        doc: &hq_suite_ingest::DocumentText,
        pictures: &Stored1,
    ) -> Result<String, ShrinkError> {
        let set = |key: &str, value: Scalar| Op::SetMeta {
            key: key.into(),
            value,
        };
        let patch = Patch {
            base_revision: None,
            ops: vec![
                set(KEY_KIND, Scalar::Str("doc".into())),
                set(
                    KEY_FIDELITY,
                    Scalar::Str(doc.projection.fidelity.as_str().into()),
                ),
                set(KEY_ORIGINAL_STATE, Scalar::Str("kept".into())),
                set(KEY_ORIGINAL_BYTES, Scalar::Int(bytes as i64)),
                set(KEY_SHRUNK_AT, Scalar::Str(day.to_string())),
            ],
        };
        let mut text = apply(&Document::parse(existing), &patch)
            .map_err(|e| ShrinkError::Notes(e.to_string()))?
            .render();
        let linked = references_in(&text);
        let missing: Vec<_> = pictures
            .plans
            .iter()
            .filter(|p| !linked.contains(&p.cas))
            .collect();
        if !missing.is_empty() {
            if !text.ends_with('\n') {
                text.push('\n');
            }
            if !text.contains("\n## Pictures\n") {
                text.push_str("\n## Pictures\n");
            }
            for (i, p) in missing.iter().enumerate() {
                text.push('\n');
                text.push_str(
                    &p.cas
                        .markdown_image(note_rel, &format!("Picture {}", i + 1)),
                );
                text.push('\n');
            }
        }
        Ok(text)
    }

    #[allow(clippy::too_many_arguments)]
    fn draft_note(
        &self,
        source: &str,
        note_rel: &str,
        sha: &str,
        bytes: u64,
        title: &str,
        day: &str,
        doc: &hq_suite_ingest::DocumentText,
        pictures: &Stored1,
    ) -> Result<String, ShrinkError> {
        let mut fm = String::new();
        let mut set = |k: &str, v: Scalar| -> Result<(), ShrinkError> {
            fm = frontmatter::set(&fm, k, &v, "\n")
                .map_err(|e| ShrinkError::Notes(e.to_string()))?;
            Ok(())
        };
        set("title", Scalar::Str(title.to_string()))?;
        set("source", Scalar::Str(source.to_string()))?;
        set("source_sha256", Scalar::Str(sha.to_string()))?;
        set(
            "source_format",
            Scalar::Str(doc.projection.kind.label().to_string()),
        )?;
        set("converter", Scalar::Str(doc.projection.engine.clone()))?;
        set("noteType", Scalar::Str("imported-document".into()))?;
        set(KEY_KIND, Scalar::Str("doc".into()))?;
        set(
            KEY_FIDELITY,
            Scalar::Str(doc.projection.fidelity.as_str().into()),
        )?;
        set(KEY_ORIGINAL_STATE, Scalar::Str("kept".into()))?;
        set(KEY_ORIGINAL_BYTES, Scalar::Int(bytes as i64))?;
        set(KEY_SHRUNK_AT, Scalar::Str(day.to_string()))?;
        let mut body = doc.markdown.trim_end().to_string();
        body.push('\n');
        if !pictures.plans.is_empty() {
            body.push_str("\n## Pictures\n");
            for (i, p) in pictures.plans.iter().enumerate() {
                body.push('\n');
                body.push_str(
                    &p.cas
                        .markdown_image(note_rel, &format!("Picture {}", i + 1)),
                );
                body.push('\n');
            }
        }
        Ok(format!("{fm}\n{body}"))
    }

    // -----------------------------------------------------------------------------------------
    // the steps shared by prepare, recovery and confirm

    /// Writes the note and checks it. Safe to repeat from any point up to `Verified`.
    fn run_to_verified(&self, job: &mut Job) -> Result<(), ShrinkError> {
        let journal = self.journal();
        if job.step == Step::Planned {
            let draft = journal.read_draft(&job.id)?;
            if sha256_bytes(draft.as_bytes()) != job.note_sha256 {
                return Err(ShrinkError::NotVerified(
                    "the saved draft was changed".into(),
                ));
            }
            match &job.adopted_note_sha256 {
                // The note was there before: replace it only if it is still exactly what the
                // draft was built from, under the note's own lock.
                Some(before) => self.notes.modify(&job.note, &|existing| {
                    let now = sha256_bytes(existing.as_bytes());
                    if now == job.note_sha256 {
                        Ok(None)
                    } else if &now == before {
                        Ok(Some(draft.clone()))
                    } else {
                        Err(ShrinkError::NoteConflict(job.note.clone()))
                    }
                })?,
                None => match self.notes.read(&job.note)? {
                    None => self.notes.write(&job.note, &draft)?,
                    Some(existing) if sha256_bytes(existing.as_bytes()) == job.note_sha256 => {}
                    Some(_) => return Err(ShrinkError::NoteConflict(job.note.clone())),
                },
            }
            self.point("note_written")?;
            job.advance(Step::NoteWritten)?;
            journal.save(job)?;
            self.point("note_step_saved")?;
        }
        if job.step == Step::NoteWritten {
            self.verify_record(job, true)?;
            job.advance(Step::Verified)?;
            journal.save(job)?;
            self.point("verified_saved")?;
        }
        Ok(())
    }

    /// The check that stands between a record and the original's move: the note as written is the
    /// drafted one, it is valid hqmd, every picture it links to is present and intact, and (while
    /// the original is still at its place) the original is the file the job started from.
    fn verify_record(&self, job: &Job, expect_untouched_note: bool) -> Result<(), ShrinkError> {
        let bad = |m: &str| ShrinkError::NotVerified(m.to_string());
        let text = self
            .notes
            .read(&job.note)?
            .ok_or_else(|| bad("the note is missing"))?;
        if expect_untouched_note && sha256_bytes(text.as_bytes()) != job.note_sha256 {
            return Err(bad("the note differs from the draft"));
        }
        if has_errors(&validate(&Document::parse(&text))) {
            return Err(bad("the note is not valid"));
        }
        let linked = references_in(&text);
        let cas = self.cas();
        for plan in &job.media {
            if !linked.contains(&plan.cas) {
                return Err(bad("the note does not link a picture it should"));
            }
            cas.verify(&plan.cas)
                .map_err(|_| bad("a picture is missing or damaged"))?;
        }
        if trash::exists(&self.paths, &job.source) {
            let path = self.paths.resolve(&job.source)?;
            let (hash, _) = hq_suite_store::fsx::sha256_file(&path)?;
            if hash != job.source_sha256 {
                return Err(ShrinkError::SourceChanged);
            }
        }
        Ok(())
    }

    // -----------------------------------------------------------------------------------------
    // confirm

    /// On the person's agreement: check the record once more, move the original to the trash, mark
    /// the note. Can be called again after a crash and carries on from where the journal says.
    pub fn confirm(&self, job_id: &str, day: &str) -> Result<Outcome, ShrinkError> {
        let journal = self.journal();
        let mut job = journal.load(job_id).map_err(|_| ShrinkError::NoSuchJob)?;
        match job.step {
            Step::Planned | Step::NoteWritten => {
                self.run_to_verified(&mut job)?;
            }
            Step::Undone | Step::Abandoned => {
                return Err(ShrinkError::WrongStep(job.step, "start again with prepare"));
            }
            _ => {}
        }
        if job.step == Step::Verified {
            self.verify_record(&job, true)?;
            let target = trash::trash_target(&self.paths, &job.source, day)?;
            job.disposition = Some(Disposition::Trash { target });
            job.advance(Step::Consented)?;
            journal.save(&job)?;
            self.point("consent_saved")?;
        }
        let Some(Disposition::Trash { target }) = job.disposition.clone() else {
            return Err(ShrinkError::WrongStep(job.step, "no agreement is recorded"));
        };
        if job.step == Step::Consented {
            if trash::exists(&self.paths, &job.source) {
                // The note was checked above or by a previous run; check again, since time has
                // passed, and refuse if the original is no longer the one that was read.
                self.verify_record(&job, false)?;
                trash::move_to(&self.paths, &job.source, &target)?;
            } else if !trash::exists(&self.paths, &target) {
                return Err(ShrinkError::OriginalMissing);
            }
            self.point("moved")?;
            job.advance(Step::OriginalMoved)?;
            journal.save(&job)?;
            self.point("moved_saved")?;
        }
        if job.step == Step::OriginalMoved {
            self.mark_note(&job, "trash", job.saved_bytes.unwrap_or(0))?;
            self.point("meta_written")?;
            job.advance(Step::Done)?;
            journal.save(&job)?;
            journal.remove_draft(&job.id);
            journal.remove_prior(&job.id);
            self.point("done_saved")?;
        }
        Ok(Outcome {
            job: job.id.clone(),
            saved_bytes: job.saved_bytes.unwrap_or(0),
            trash_path: target,
        })
    }

    /// Sets the note's `suite_original_state` and `suite_saved_bytes`, leaving every other byte.
    fn mark_note(&self, job: &Job, state: &str, saved: i64) -> Result<(), ShrinkError> {
        set_original_state(self.notes, &job.note, state, saved)
    }

    // -----------------------------------------------------------------------------------------
    // undo, abandon, recover

    /// Puts the original back where it was and marks the note. Needs the original to still be in
    /// the trash (it stays 30 days) and the old place to be free.
    pub fn undo(&self, job_id: &str) -> Result<(), ShrinkError> {
        let journal = self.journal();
        let mut job = journal.load(job_id).map_err(|_| ShrinkError::NoSuchJob)?;
        if !matches!(job.step, Step::Done | Step::OriginalMoved | Step::Undone) {
            return Err(ShrinkError::WrongStep(job.step, "there is nothing to undo"));
        }
        let Some(Disposition::Trash { target }) = job.disposition.clone() else {
            return Err(ShrinkError::WrongStep(
                job.step,
                "no original was set aside",
            ));
        };
        if trash::exists(&self.paths, &target) {
            trash::restore(&self.paths, &target, &job.source)?;
        } else if !trash::exists(&self.paths, &job.source) {
            return Err(ShrinkError::OriginalMissing);
        }
        self.point("restored")?;
        self.mark_note(&job, "kept", 0)?;
        self.point("undo_note_marked")?;
        job.advance(Step::Undone)?;
        journal.save(&job)?;
        self.point("undone_saved")?;
        Ok(())
    }

    /// Steps 6 and 7 for a sink: seal the original into `sink` and read it back, and only then
    /// remove it from the phone. The original is at its place or in the sink, verified, whenever
    /// it is not at its place; a crash anywhere is finished by calling this again.
    pub fn offload(&self, job_id: &str, sink: &dyn ColdStore) -> Result<Outcome, ShrinkError> {
        let journal = self.journal();
        let mut job = journal.load(job_id).map_err(|_| ShrinkError::NoSuchJob)?;
        match job.step {
            Step::Planned | Step::NoteWritten => self.run_to_verified(&mut job)?,
            Step::Undone | Step::Abandoned => {
                return Err(ShrinkError::WrongStep(job.step, "start again with prepare"));
            }
            _ => {}
        }
        if job.step == Step::Verified {
            self.verify_record(&job, true)?;
            let path = self.paths.resolve(&job.source)?;
            let bytes = std::fs::read(&path).map_err(|_| refuse(Refusal::Unreadable))?;
            let sha = sha256_bytes(&bytes);
            if sha != job.source_sha256 {
                return Err(ShrinkError::SourceChanged);
            }
            sink.put(&sha, &bytes)
                .map_err(|e| ShrinkError::Sink(e.to_string()))?;
            self.point("sealed")?;
            job.disposition = Some(Disposition::Sink {
                sink: sink.id().to_string(),
                sha256: sha,
            });
            job.advance(Step::Consented)?;
            journal.save(&job)?;
            self.point("consent_saved")?;
        }
        let Some(Disposition::Sink {
            sink: named,
            sha256,
        }) = job.disposition.clone()
        else {
            return Err(ShrinkError::WrongStep(
                job.step,
                "no sink was chosen for this job",
            ));
        };
        if named != sink.id() {
            return Err(ShrinkError::WrongStep(
                job.step,
                "this job names another sink",
            ));
        }
        if job.step == Step::Consented {
            if trash::exists(&self.paths, &job.source) {
                self.verify_record(&job, false)?;
                if !sink.has(&sha256) {
                    return Err(ShrinkError::NotVerified(
                        "the sink does not hold the original".into(),
                    ));
                }
                let path = self.paths.resolve(&job.source)?;
                std::fs::remove_file(path).map_err(|e| ShrinkError::Notes(e.to_string()))?;
            } else if !sink.has(&sha256) {
                return Err(ShrinkError::OriginalMissing);
            }
            self.point("moved")?;
            job.advance(Step::OriginalMoved)?;
            journal.save(&job)?;
            self.point("moved_saved")?;
        }
        if job.step == Step::OriginalMoved {
            set_offloaded(self.notes, &job.note, &named, job.saved_bytes.unwrap_or(0))?;
            self.point("meta_written")?;
            job.advance(Step::Done)?;
            journal.save(&job)?;
            journal.remove_draft(&job.id);
            journal.remove_prior(&job.id);
            self.point("done_saved")?;
        }
        Ok(Outcome {
            job: job.id.clone(),
            saved_bytes: job.saved_bytes.unwrap_or(0),
            trash_path: format!("sink:{named}"),
        })
    }

    /// As [`undo`](Self::undo), fetching the original back from `sink` when that is where it went.
    pub fn undo_with(&self, job_id: &str, sink: &dyn ColdStore) -> Result<(), ShrinkError> {
        let journal = self.journal();
        let mut job = journal.load(job_id).map_err(|_| ShrinkError::NoSuchJob)?;
        let Some(Disposition::Sink {
            sink: named,
            sha256,
        }) = job.disposition.clone()
        else {
            return self.undo(job_id);
        };
        if !matches!(job.step, Step::Done | Step::OriginalMoved | Step::Undone) {
            return Err(ShrinkError::WrongStep(job.step, "there is nothing to undo"));
        }
        if named != sink.id() {
            return Err(ShrinkError::WrongStep(
                job.step,
                "this job names another sink",
            ));
        }
        if !trash::exists(&self.paths, &job.source) {
            let bytes = sink
                .get(&sha256)
                .map_err(|e| ShrinkError::Sink(e.to_string()))?;
            if sha256_bytes(&bytes) != sha256 {
                return Err(ShrinkError::NotVerified(
                    "the sink returned other bytes".into(),
                ));
            }
            let path = self.paths.resolve(&job.source)?;
            if !hq_suite_store::fsx::write_new(&path, &bytes)? {
                return Err(ShrinkError::NoteConflict(job.source.clone()));
            }
        }
        self.point("restored")?;
        set_original_kept(self.notes, &job.note)?;
        self.point("undo_note_marked")?;
        job.advance(Step::Undone)?;
        journal.save(&job)?;
        self.point("undone_saved")?;
        Ok(())
    }

    /// The person said no before anything moved: the note this job wrote is removed if it is still
    /// exactly as drafted, and the job is closed. Pictures stay for the next cleanup, which
    /// removes only those no note links to.
    pub fn abandon(&self, job_id: &str) -> Result<(), ShrinkError> {
        let journal = self.journal();
        let mut job = journal.load(job_id).map_err(|_| ShrinkError::NoSuchJob)?;
        if job.step >= Step::OriginalMoved {
            return Err(ShrinkError::WrongStep(
                job.step,
                "the original has already moved; use undo",
            ));
        }
        if job.adopted_note_sha256.is_some() {
            // The note was the person's before this job: put its text back, if nobody has
            // edited it since the draft was written.
            if self.notes.read(&job.note)?.is_some() {
                let prior = journal.read_prior(&job.id)?;
                self.notes.modify(&job.note, &|text| {
                    Ok((sha256_bytes(text.as_bytes()) == job.note_sha256).then(|| prior.clone()))
                })?;
            }
        } else {
            self.notes
                .remove_if_unchanged(&job.note, &job.note_sha256)?;
        }
        job.advance(Step::Abandoned)?;
        journal.save(&job)?;
        journal.remove_draft(&job.id);
        journal.remove_prior(&job.id);
        Ok(())
    }

    /// Carries every unfinished job as far as it may go without asking anyone: up to the
    /// preview if the person has not agreed, through to the end if they have.
    pub fn recover(&self) -> Vec<(String, Recovered)> {
        let journal = self.journal();
        journal.sweep();
        let mut out = Vec::new();
        for job in journal.unfinished() {
            let id = job.id.clone();
            let result = match job.step {
                Step::Planned | Step::NoteWritten => {
                    let mut job = job;
                    match self.run_to_verified(&mut job) {
                        Ok(()) => Recovered::ReadyForConsent,
                        Err(e) => Recovered::Stuck(e.to_string()),
                    }
                }
                Step::Verified => Recovered::ReadyForConsent,
                Step::Consented | Step::OriginalMoved
                    if matches!(job.disposition, Some(Disposition::Sink { .. })) =>
                {
                    Recovered::Stuck("it needs its sink: run offload for it again".into())
                }
                Step::Consented | Step::OriginalMoved => match self.confirm(&id, &job.day) {
                    Ok(_) => Recovered::Finished,
                    Err(e) => Recovered::Stuck(e.to_string()),
                },
                _ => continue,
            };
            out.push((id, result));
        }
        out
    }

    // -----------------------------------------------------------------------------------------

    pub fn preview(&self, job_id: &str) -> Result<Preview, ShrinkError> {
        let job = self
            .journal()
            .load(job_id)
            .map_err(|_| ShrinkError::NoSuchJob)?;
        Ok(self.preview_of(&job))
    }

    fn preview_of(&self, job: &Job) -> Preview {
        let saved = job.saved_bytes.unwrap_or(0);
        Preview {
            job: job.id.clone(),
            source: job.source.clone(),
            note: job.note.clone(),
            step: job.step,
            fidelity: job.fidelity.clone(),
            source_format: job.source_format.clone(),
            loss_kinds: job.loss_kinds.clone(),
            source_bytes: job.source_bytes,
            record_bytes: (job.source_bytes as i64 - saved).max(0) as u64,
            saved_bytes: saved,
            pictures_recompressed: job.pictures_recompressed,
            pictures_kept: job.pictures_kept,
            original_at: self.original_at(job),
        }
    }
}
