//! What the apps draw for one file: its state, the sentences to show and the actions that make
//! sense, as plain data. The REST routes and any other front end call these; none of them holds
//! shrink logic of its own.

use std::path::Path;

use anyhow::Result;
use hq_suite_shrink::{OriginalAt, Preview, Shrinker};
use hq_suite_store::{Job, Journal, Step, SuitePaths};
use serde::Serialize;

use crate::shrink::{VaultNotes, friendly, shrinker, today, vault_relative};
use crate::text;

/// Where a file stands, as the apps name it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// Nothing has been done; shrinking can be tried.
    Ready,
    /// A record is written and checked; the original has not moved. Waiting for an answer.
    Prepared,
    /// The record stands and the original is in the trash.
    Shrunk,
}

#[derive(Debug, Clone, Serialize)]
pub struct Actions {
    pub shrink: bool,
    pub confirm: bool,
    pub abandon: bool,
    pub undo: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileStatus {
    pub path: String,
    pub state: State,
    pub job: Option<String>,
    /// The note that is the record, once there is one.
    pub note: Option<String>,
    pub saved_bytes: Option<i64>,
    /// One sentence for a card.
    pub headline: String,
    /// More lines for the sheet that asks for a decision.
    pub details: Vec<String>,
    pub actions: Actions,
}

fn actions(state: State) -> Actions {
    Actions {
        shrink: state == State::Ready,
        confirm: state == State::Prepared,
        abandon: state == State::Prepared,
        undo: state == State::Shrunk,
    }
}

/// The job that says most about `path`: one in progress, else a finished one that still holds.
fn job_for(paths: &SuitePaths, path: &str) -> Option<Job> {
    let (jobs, _) = Journal::new(paths).list();
    let mine = |j: &&Job| j.source == path;
    jobs.iter()
        .filter(mine)
        .find(|j| !j.step.is_terminal())
        .or_else(|| jobs.iter().filter(mine).find(|j| j.step == Step::Done))
        .cloned()
}

fn status_of(s: &Shrinker<'_>, path: &str, job: Option<Job>) -> FileStatus {
    let name = path.rsplit('/').next().unwrap_or(path).to_string();
    let Some(job) = job else {
        return FileStatus {
            path: path.into(),
            state: State::Ready,
            job: None,
            note: None,
            saved_bytes: None,
            headline: format!("{name} can be made smaller."),
            details: vec![],
            actions: actions(State::Ready),
        };
    };
    let preview: Preview = s
        .preview(&job.id)
        .unwrap_or_else(|_| unreachable_preview(&job));
    let (state, headline) = match (job.step, preview.original_at) {
        (Step::Done, OriginalAt::Trash) => (
            State::Shrunk,
            format!(
                "{name} was made smaller. {} saved; the original is in the trash for 30 days.",
                text::size(preview.saved_bytes.max(0) as u64)
            ),
        ),
        (Step::Done, OriginalAt::Sink) => (
            State::Shrunk,
            format!(
                "{name} was made smaller. {} saved; the original is sealed in your sink and comes back with hq suite undo.",
                text::size(preview.saved_bytes.max(0) as u64)
            ),
        ),
        (Step::Done, _) => (
            State::Shrunk,
            format!("{name} was made smaller. The original is no longer in the trash."),
        ),
        _ => (
            State::Prepared,
            format!(
                "{name} can shrink from {} to {}.",
                text::size(preview.source_bytes),
                text::size(preview.record_bytes)
            ),
        ),
    };
    let details = if state == State::Prepared {
        text::preview(&preview)
            .lines()
            .skip(1)
            .map(|l| l.trim().to_string())
            .collect()
    } else {
        vec![]
    };
    FileStatus {
        path: path.into(),
        state,
        note: Some(job.note.clone()),
        saved_bytes: job.saved_bytes,
        job: Some(job.id),
        headline,
        details,
        actions: actions(state),
    }
}

/// A preview built from the job alone, for when the shrinker cannot be asked.
fn unreachable_preview(job: &Job) -> Preview {
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
        original_at: OriginalAt::Nowhere,
    }
}

fn rel(vault: &Path, path: &str) -> Result<String> {
    vault_relative(vault, Path::new(path))
}

/// Where `path` stands now.
pub fn status(vault: &Path, path: &str) -> Result<FileStatus> {
    let rel = rel(vault, path)?;
    let paths = SuitePaths::new(vault);
    let notes = VaultNotes::new(paths.clone());
    let job = job_for(&paths, &rel);
    if job.is_none() && !hq_suite_store::trash::exists(&paths, &rel) {
        anyhow::bail!("That file is not in the vault.");
    }
    Ok(status_of(&shrinker(&paths, &notes), &rel, job))
}

/// Writes the record and checks it; the original does not move. The answer holds the sentences to
/// show before asking.
pub async fn prepare(vault: &Path, path: &str) -> Result<FileStatus> {
    let rel = rel(vault, path)?;
    let paths = SuitePaths::new(vault);
    let notes = VaultNotes::new(paths.clone());
    let s = shrinker(&paths, &notes);
    let preview = s.prepare(&rel, &today()).await.map_err(friendly)?;
    let job = Journal::new(&paths).load(&preview.job).ok();
    Ok(status_of(&s, &rel, job))
}

fn after(
    vault: &Path,
    job: &str,
    step: impl FnOnce(&Shrinker<'_>) -> Result<()>,
) -> Result<FileStatus> {
    let paths = SuitePaths::new(vault);
    let notes = VaultNotes::new(paths.clone());
    let s = shrinker(&paths, &notes);
    let source = Journal::new(&paths)
        .load(job)
        .map(|j| j.source)
        .unwrap_or_default();
    step(&s)?;
    let current = job_for(&paths, &source);
    Ok(status_of(&s, &source, current))
}

/// The person agreed: the original goes to the trash.
pub fn confirm(vault: &Path, job: &str) -> Result<FileStatus> {
    after(vault, job, |s| {
        s.confirm(job, &today()).map(drop).map_err(friendly)
    })
}

/// The person chose to keep the original in a sink instead of the trash.
pub fn offload(vault: &Path, job: &str, sink: &str, code: &str) -> Result<FileStatus> {
    after(vault, job, |_| {
        crate::sinks::offload_with_code(vault, job, sink, code)
    })
}

/// The person said no: the new note is removed and nothing moved.
pub fn abandon(vault: &Path, job: &str) -> Result<FileStatus> {
    after(vault, job, |s| s.abandon(job).map_err(friendly))
}

/// Brings the original back from the trash.
pub fn undo(vault: &Path, job: &str) -> Result<FileStatus> {
    after(vault, job, |s| s.undo(job).map_err(friendly))
}

/// Largest picture the media route hands out. Recompressed pictures are far below it; the cap only
/// stops a hostile or odd file from filling the phone's memory.
const MAX_MEDIA_BYTES: u64 = 16 * 1024 * 1024;

/// A picture from the media store by its vault path (`_media/cas/ab/<hash>.<ext>`), with the type
/// to send it as. Anything that is not exactly a store path is refused.
pub fn media(vault: &Path, path: &str) -> Result<(Vec<u8>, &'static str)> {
    let r = hq_suite_store::CasRef::parse_rel(path)
        .ok_or_else(|| anyhow::anyhow!("That is not a stored picture."))?;
    let kind = match path.rsplit('.').next().unwrap_or("") {
        "webp" => "image/webp",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        _ => anyhow::bail!("That kind of picture is not shown."),
    };
    let paths = SuitePaths::new(vault);
    let file = paths.resolve(&r.rel())?;
    if std::fs::metadata(&file)
        .map_err(|_| anyhow::anyhow!("That picture is not there."))?
        .len()
        > MAX_MEDIA_BYTES
    {
        anyhow::bail!("That picture is too large to show.");
    }
    let bytes = hq_suite_store::Cas::new(paths)
        .read(&r)
        .map_err(|_| anyhow::anyhow!("That picture is not there."))?;
    Ok((bytes, kind))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stored_picture_is_served_and_nothing_else_is() {
        let d = tempfile::tempdir().unwrap();
        let paths = SuitePaths::new(d.path());
        let (r, _) = hq_suite_store::Cas::new(paths)
            .put(b"picture", "webp")
            .unwrap();
        let (bytes, kind) = media(d.path(), &r.rel()).unwrap();
        assert_eq!((bytes.as_slice(), kind), (&b"picture"[..], "image/webp"));
        for bad in [
            "Notebooks/a.docx",
            "_media/cas/ab/not-a-hash.webp",
            "_media/cas/../../secret.webp",
            "_media/web/upload.png",
        ] {
            assert!(media(d.path(), bad).is_err(), "{bad}");
        }
    }
}
