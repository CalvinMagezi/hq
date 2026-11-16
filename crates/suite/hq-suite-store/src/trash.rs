//! Originals set aside in the vault's own trash.
//!
//! A file goes to `_trash/<day>/<its vault path>`, the same layout `vault_delete` uses for notes,
//! so the vault's 30-day retention applies to it and a person can find it with a file manager.
//! Moving is a rename, so it is atomic, and nothing here deletes.

use std::path::Path;

use crate::error::{Result, StoreError};
use crate::fsx;
use crate::paths::{SuitePaths, TRASH_REL};

/// Where a file would go: `_trash/<day>/<rel>`, with `-2`, `-3` ... before the extension if that
/// name is taken, so two files of the same name on the same day never overwrite each other.
pub fn trash_target(paths: &SuitePaths, rel: &str, day: &str) -> Result<String> {
    check_day(day)?;
    let base = format!("{TRASH_REL}/{day}/{rel}");
    paths.resolve(&base)?;
    if !paths.vault().join(&base).exists() {
        return Ok(base);
    }
    let (stem, ext) = match base.rfind('.') {
        Some(dot) if dot > base.rfind('/').unwrap_or(0) => (&base[..dot], &base[dot..]),
        _ => (base.as_str(), ""),
    };
    for n in 2..1000 {
        let candidate = format!("{stem}-{n}{ext}");
        if !paths.vault().join(&candidate).exists() {
            return Ok(candidate);
        }
    }
    Err(StoreError::Exists(base))
}

fn check_day(day: &str) -> Result<()> {
    let b = day.as_bytes();
    let ok = b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b.iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit());
    if ok {
        Ok(())
    } else {
        Err(StoreError::UnsafePath(day.to_string()))
    }
}

/// Moves the file at `rel` to `target` (from [`trash_target`]). Refuses to replace anything.
pub fn move_to(paths: &SuitePaths, rel: &str, target: &str) -> Result<()> {
    let from = paths.resolve(rel)?;
    if !target.starts_with(&format!("{TRASH_REL}/")) {
        return Err(StoreError::UnsafePath(target.to_string()));
    }
    let to = paths.resolve(target)?;
    fsx::move_file(&from, &to)
}

/// Puts a trashed file back at `dest`, which must be free.
pub fn restore(paths: &SuitePaths, trash_rel: &str, dest: &str) -> Result<()> {
    if !trash_rel.starts_with(&format!("{TRASH_REL}/")) {
        return Err(StoreError::UnsafePath(trash_rel.to_string()));
    }
    let from = paths.resolve(trash_rel)?;
    let to = paths.resolve(dest)?;
    fsx::move_file(&from, &to)
}

/// Whether a vault file exists as a regular file.
pub fn exists(paths: &SuitePaths, rel: &str) -> bool {
    paths
        .resolve(rel)
        .map(|p| Path::new(&p).is_file())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn vault() -> (tempfile::TempDir, SuitePaths) {
        let d = tempfile::tempdir().unwrap();
        let p = SuitePaths::new(d.path());
        (d, p)
    }

    fn put(d: &tempfile::TempDir, rel: &str, body: &str) {
        let p = d.path().join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, body).unwrap();
    }

    #[test]
    fn a_file_moves_to_the_vaults_trash_layout_and_back() {
        let (d, p) = vault();
        put(&d, "Notebooks/Inbox/Imports/a.docx", "original");
        let t = trash_target(&p, "Notebooks/Inbox/Imports/a.docx", "2026-11-16").unwrap();
        assert_eq!(t, "_trash/2026-11-16/Notebooks/Inbox/Imports/a.docx");
        move_to(&p, "Notebooks/Inbox/Imports/a.docx", &t).unwrap();
        assert!(!exists(&p, "Notebooks/Inbox/Imports/a.docx"));
        assert_eq!(fs::read_to_string(d.path().join(&t)).unwrap(), "original");
        restore(&p, &t, "Notebooks/Inbox/Imports/a.docx").unwrap();
        assert_eq!(
            fs::read_to_string(d.path().join("Notebooks/Inbox/Imports/a.docx")).unwrap(),
            "original"
        );
    }

    #[test]
    fn same_day_same_name_is_numbered_not_overwritten() {
        let (d, p) = vault();
        put(&d, "Notebooks/a.docx", "first");
        let t1 = trash_target(&p, "Notebooks/a.docx", "2026-11-16").unwrap();
        move_to(&p, "Notebooks/a.docx", &t1).unwrap();
        put(&d, "Notebooks/a.docx", "second");
        let t2 = trash_target(&p, "Notebooks/a.docx", "2026-11-16").unwrap();
        assert_eq!(t2, "_trash/2026-11-16/Notebooks/a-2.docx");
        move_to(&p, "Notebooks/a.docx", &t2).unwrap();
        assert_eq!(fs::read_to_string(d.path().join(&t1)).unwrap(), "first");
        assert_eq!(fs::read_to_string(d.path().join(&t2)).unwrap(), "second");
    }

    #[test]
    fn restoring_over_a_file_is_refused_and_loses_nothing() {
        let (d, p) = vault();
        put(&d, "Notebooks/a.docx", "trashed");
        let t = trash_target(&p, "Notebooks/a.docx", "2026-11-16").unwrap();
        move_to(&p, "Notebooks/a.docx", &t).unwrap();
        put(&d, "Notebooks/a.docx", "someone made a new one");
        assert!(matches!(
            restore(&p, &t, "Notebooks/a.docx"),
            Err(StoreError::Exists(_))
        ));
        assert_eq!(fs::read_to_string(d.path().join(&t)).unwrap(), "trashed");
    }

    #[test]
    fn days_and_destinations_are_checked() {
        let (d, p) = vault();
        put(&d, "Notebooks/a.docx", "x");
        for bad in [
            "",
            "yesterday",
            "2026-1-16",
            "2026/11/16",
            "../../etc",
            "2026-11-16/x",
        ] {
            assert!(
                trash_target(&p, "Notebooks/a.docx", bad).is_err(),
                "{bad:?}"
            );
        }
        assert!(move_to(&p, "Notebooks/a.docx", "Elsewhere/a.docx").is_err());
        assert!(restore(&p, "Notebooks/a.docx", "Other/a.docx").is_err());
        assert!(exists(&p, "Notebooks/a.docx"));
    }
}
