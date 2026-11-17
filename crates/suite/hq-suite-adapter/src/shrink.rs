//! `hq suite shrink`, `confirm`, `abandon`, `undo`, `jobs` and `recover`: the shrink pipeline over
//! a real vault.

use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use hq_suite_shrink::{NoFaults, Notes, Options, Refusal, ShrinkError, Shrinker};
use hq_suite_store::{Journal, SuitePaths, fsx};

use crate::text;

/// Reads and writes notes through the vault's own per-note lock, so a shrink cannot interleave
/// with a save made elsewhere. The vault watcher and the 30-minute scan keep search fresh.
pub struct VaultNotes {
    paths: SuitePaths,
}

impl VaultNotes {
    pub fn new(paths: SuitePaths) -> Self {
        VaultNotes { paths }
    }
}

fn notes_err(e: impl std::fmt::Display) -> ShrinkError {
    ShrinkError::Notes(e.to_string())
}

impl Notes for VaultNotes {
    fn read(&self, rel: &str) -> Result<Option<String>, ShrinkError> {
        let _guard = hq_vault::note_lock::lock_note(rel);
        match std::fs::read_to_string(self.paths.resolve(rel)?) {
            Ok(t) => Ok(Some(t)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(notes_err(e)),
        }
    }

    fn write(&self, rel: &str, text: &str) -> Result<(), ShrinkError> {
        let _guard = hq_vault::note_lock::lock_note(rel);
        Ok(fsx::atomic_write(
            &self.paths.resolve(rel)?,
            text.as_bytes(),
        )?)
    }

    fn remove(&self, rel: &str) -> Result<(), ShrinkError> {
        let _guard = hq_vault::note_lock::lock_note(rel);
        std::fs::remove_file(self.paths.resolve(rel)?).map_err(notes_err)
    }

    /// Read, change and write under one hold of the note's lock, so a save made elsewhere in
    /// between cannot be lost.
    fn modify(
        &self,
        rel: &str,
        change: &dyn Fn(&str) -> Result<Option<String>, ShrinkError>,
    ) -> Result<(), ShrinkError> {
        let _guard = hq_vault::note_lock::lock_note(rel);
        let path = self.paths.resolve(rel)?;
        let text = std::fs::read_to_string(&path)
            .map_err(|e| ShrinkError::NotVerified(format!("the note cannot be read: {e}")))?;
        if let Some(next) = change(&text)? {
            fsx::atomic_write(&path, next.as_bytes())?;
        }
        Ok(())
    }

    fn remove_if_unchanged(&self, rel: &str, sha256: &str) -> Result<bool, ShrinkError> {
        let _guard = hq_vault::note_lock::lock_note(rel);
        let path = self.paths.resolve(rel)?;
        match std::fs::read(&path) {
            Ok(bytes) if fsx::sha256_bytes(&bytes) == sha256 => {
                std::fs::remove_file(&path).map_err(notes_err)?;
                Ok(true)
            }
            Ok(_) => Ok(false),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(notes_err(e)),
        }
    }
}

/// A path the person typed, as a vault-relative one. Absolute paths must lie inside the vault.
pub fn vault_relative(vault: &Path, file: &Path) -> Result<String> {
    let rel: PathBuf = if file.is_absolute() {
        let root = vault
            .canonicalize()
            .context("the vault folder cannot be read")?;
        let abs = file
            .canonicalize()
            .with_context(|| format!("{} cannot be read", file.display()))?;
        abs.strip_prefix(&root)
            .map(Path::to_path_buf)
            .map_err(|_| anyhow::anyhow!("{} is not inside the vault", file.display()))?
    } else {
        file.to_path_buf()
    };
    let text = rel
        .to_str()
        .context("the path is not valid text")?
        .replace('\\', "/");
    Ok(text.trim_start_matches("./").to_string())
}

pub(crate) fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

fn explain(r: &Refusal) -> String {
    match r {
        Refusal::Encrypted => {
            "It is password protected. Save an unprotected copy and try that.".into()
        }
        Refusal::NeedsOcr => "It is a scan or a photo. Those are kept as they are for now.".into(),
        Refusal::Unsupported => "HQ cannot read this kind of file yet.".into(),
        Refusal::TooLarge => "It is too large to read safely.".into(),
        Refusal::ProtectedFolder => "It is in a folder HQ keeps for itself.".into(),
        Refusal::NotSmaller { .. } => {
            "The note would not be smaller than the file, so there is nothing to save.".into()
        }
        Refusal::Unrepresentable(why) => {
            format!("The converted text would not be a valid note ({why}).")
        }
        Refusal::AlreadyShrunk => {
            "It was already made smaller. Undo that first if you want to redo it.".into()
        }
        Refusal::Unreadable => "It could not be read.".into(),
    }
}

pub(crate) fn friendly(e: ShrinkError) -> anyhow::Error {
    match e {
        ShrinkError::Refused(r) => anyhow::anyhow!("Not shrunk. {}", explain(&r)),
        ShrinkError::SourceChanged => anyhow::anyhow!(
            "The original changed after it was read, so nothing was moved. Run hq suite shrink again."
        ),
        other => anyhow::anyhow!("{other}"),
    }
}

pub(crate) fn shrinker<'a>(paths: &SuitePaths, notes: &'a VaultNotes) -> Shrinker<'a> {
    Shrinker::new(paths.clone(), notes, &NoFaults, Options::default())
}

/// `hq suite shrink <file>`: prepare, show the preview, and move the original only on a yes.
pub async fn shrink(vault: &Path, file: &Path, yes: bool) -> Result<()> {
    let rel = vault_relative(vault, file)?;
    let paths = SuitePaths::new(vault);
    let notes = VaultNotes::new(paths.clone());
    let s = shrinker(&paths, &notes);
    let day = today();
    let preview = s.prepare(&rel, &day).await.map_err(friendly)?;
    print!("{}", text::preview(&preview));
    let go = if yes {
        true
    } else if std::io::stdin().is_terminal() {
        print!("{}", text::ask_to_move());
        std::io::stdout().flush()?;
        let mut line = String::new();
        std::io::stdin().lock().read_line(&mut line)?;
        matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes")
    } else {
        print!("{}", text::next_steps(&preview.job));
        return Ok(());
    };
    if go {
        let out = s.confirm(&preview.job, &day).map_err(friendly)?;
        print!(
            "{}",
            text::moved(&preview.source, &out.trash_path, out.saved_bytes)
                .replace("<job>", &preview.job)
        );
    } else {
        print!("{}", text::next_steps(&preview.job));
    }
    Ok(())
}

pub fn confirm(vault: &Path, job: &str) -> Result<()> {
    let paths = SuitePaths::new(vault);
    let notes = VaultNotes::new(paths.clone());
    let s = shrinker(&paths, &notes);
    let out = s.confirm(job, &today()).map_err(friendly)?;
    let source = Journal::new(&paths)
        .load(job)
        .map(|j| j.source)
        .unwrap_or_default();
    print!(
        "{}",
        text::moved(&source, &out.trash_path, out.saved_bytes).replace("<job>", job)
    );
    Ok(())
}

pub fn abandon(vault: &Path, job: &str) -> Result<()> {
    let paths = SuitePaths::new(vault);
    let notes = VaultNotes::new(paths.clone());
    shrinker(&paths, &notes).abandon(job).map_err(friendly)?;
    print!("{}", text::abandoned());
    Ok(())
}

pub fn undo(vault: &Path, job: &str) -> Result<()> {
    let paths = SuitePaths::new(vault);
    let notes = VaultNotes::new(paths.clone());
    shrinker(&paths, &notes).undo(job).map_err(friendly)?;
    let source = Journal::new(&paths)
        .load(job)
        .map(|j| j.source)
        .unwrap_or_default();
    print!("{}", text::undone(&source));
    Ok(())
}

pub fn jobs(vault: &Path) -> Result<()> {
    let paths = SuitePaths::new(vault);
    let notes = VaultNotes::new(paths.clone());
    let s = shrinker(&paths, &notes);
    let (jobs, damaged) = Journal::new(&paths).list();
    if jobs.is_empty() && damaged.is_empty() {
        print!("{}", text::no_jobs());
        return Ok(());
    }
    for j in jobs {
        let at = s.original_at(&j);
        println!("{}  {}  {}", j.id, j.source, text::step_words(j.step, at));
    }
    for id in damaged {
        println!("{id}  (this job's file cannot be read; it was left alone)");
    }
    Ok(())
}

pub fn recover(vault: &Path) -> Result<()> {
    let paths = SuitePaths::new(vault);
    let notes = VaultNotes::new(paths.clone());
    let done = shrinker(&paths, &notes).recover();
    print!("{}", text::recovered(done.len()));
    for (id, what) in done {
        println!("  {id}: {what:?}");
    }
    Ok(())
}

/// Used by tests and by callers that need the plain refusal text.
pub fn refusal_text(r: &Refusal) -> String {
    explain(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_made_relative_to_the_vault() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("Notebooks")).unwrap();
        std::fs::write(d.path().join("Notebooks/a.docx"), b"x").unwrap();
        assert_eq!(
            vault_relative(d.path(), Path::new("Notebooks/a.docx")).unwrap(),
            "Notebooks/a.docx"
        );
        assert_eq!(
            vault_relative(d.path(), Path::new("./Notebooks/a.docx")).unwrap(),
            "Notebooks/a.docx"
        );
        assert_eq!(
            vault_relative(d.path(), &d.path().join("Notebooks/a.docx")).unwrap(),
            "Notebooks/a.docx"
        );
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("b.docx"), b"x").unwrap();
        assert!(vault_relative(d.path(), &outside.path().join("b.docx")).is_err());
    }

    #[test]
    fn modify_changes_one_note_under_its_lock_and_removal_checks_the_text() {
        let d = tempfile::tempdir().unwrap();
        let paths = SuitePaths::new(d.path());
        let notes = VaultNotes::new(paths);
        notes.write("a.md", "one").unwrap();
        notes
            .modify("a.md", &|t| Ok(Some(format!("{t} two"))))
            .unwrap();
        assert_eq!(notes.read("a.md").unwrap().as_deref(), Some("one two"));
        notes.modify("a.md", &|_| Ok(None)).unwrap();
        assert_eq!(notes.read("a.md").unwrap().as_deref(), Some("one two"));
        assert!(notes.modify("missing.md", &|_| Ok(None)).is_err());
        assert!(
            !notes
                .remove_if_unchanged("a.md", &fsx::sha256_bytes(b"other"))
                .unwrap()
        );
        assert!(
            notes
                .remove_if_unchanged("a.md", &fsx::sha256_bytes(b"one two"))
                .unwrap()
        );
        assert_eq!(notes.read("a.md").unwrap(), None);
        assert!(!notes.remove_if_unchanged("a.md", "x").unwrap());
    }

    #[test]
    fn every_refusal_has_a_plain_sentence() {
        for r in [
            Refusal::Encrypted,
            Refusal::NeedsOcr,
            Refusal::Unsupported,
            Refusal::TooLarge,
            Refusal::ProtectedFolder,
            Refusal::NotSmaller { saved_bytes: -5 },
            Refusal::Unrepresentable("x".into()),
            Refusal::AlreadyShrunk,
            Refusal::Unreadable,
        ] {
            let t = refusal_text(&r);
            assert!(t.ends_with('.') && !t.contains('\u{2014}'), "{t}");
        }
    }
}
