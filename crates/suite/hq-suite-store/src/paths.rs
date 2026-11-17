use std::path::{Path, PathBuf};

use hq_suite_md::safe_rel_path;

use crate::error::{Result, StoreError};

/// Vault-relative folder that holds media stored by hash.
pub const CAS_REL: &str = "_media/cas";
/// Vault-relative folder for the suite's own state. Under `_data`, which the vault keeps from
/// agents and its watcher.
pub const DATA_REL: &str = "_data/suite";
/// The vault's own trash.
pub const TRASH_REL: &str = "_trash";

/// Where the suite keeps things, given the vault's root. The one place a vault path is built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuitePaths {
    vault: PathBuf,
}

impl SuitePaths {
    pub fn new(vault: impl Into<PathBuf>) -> Self {
        SuitePaths {
            vault: vault.into(),
        }
    }

    pub fn vault(&self) -> &Path {
        &self.vault
    }

    pub fn cas_dir(&self) -> PathBuf {
        self.vault.join(CAS_REL)
    }

    pub fn journal_dir(&self) -> PathBuf {
        self.vault.join(DATA_REL).join("journal")
    }

    pub fn cache_dir(&self) -> PathBuf {
        self.vault.join(DATA_REL).join("cache")
    }

    pub fn trash_dir(&self) -> PathBuf {
        self.vault.join(TRASH_REL)
    }

    /// Turns a vault-relative path into a real one, refusing anything that could leave the vault:
    /// absolute paths, `..`, backslashes, and any existing part of the path that is a link.
    pub fn resolve(&self, rel: &str) -> Result<PathBuf> {
        safe_rel_path(rel).map_err(|_| StoreError::UnsafePath(rel.to_string()))?;
        let mut cur = self.vault.clone();
        for part in rel.split('/') {
            cur.push(part);
            match std::fs::symlink_metadata(&cur) {
                Ok(m) if m.file_type().is_symlink() => {
                    return Err(StoreError::UnsafePath(rel.to_string()));
                }
                Ok(_) => {}
                // Parts that do not exist yet cannot be links; the rest will be created by us.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(StoreError::io(&cur, e)),
            }
        }
        Ok(cur)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_inside_the_vault_and_refuses_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let p = SuitePaths::new(dir.path());
        assert_eq!(
            p.resolve("Notebooks/a.docx").unwrap(),
            dir.path().join("Notebooks/a.docx")
        );
        for bad in ["", "/etc/passwd", "../x", "a/../../x", "a\\b", "C:/x"] {
            assert!(
                matches!(p.resolve(bad), Err(StoreError::UnsafePath(_))),
                "{bad:?}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_link_in_the_path_is_refused() {
        let outside = tempfile::tempdir().unwrap();
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("Notebooks")).unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("Notebooks/escape")).unwrap();
        let p = SuitePaths::new(dir.path());
        assert!(matches!(
            p.resolve("Notebooks/escape/file.docx"),
            Err(StoreError::UnsafePath(_))
        ));
        assert!(p.resolve("Notebooks/fine/file.docx").is_ok());
    }

    #[test]
    fn the_suites_folders_sit_where_the_plan_says() {
        let p = SuitePaths::new("/vault");
        assert_eq!(p.cas_dir(), PathBuf::from("/vault/_media/cas"));
        assert_eq!(p.journal_dir(), PathBuf::from("/vault/_data/suite/journal"));
        assert_eq!(p.cache_dir(), PathBuf::from("/vault/_data/suite/cache"));
        assert_eq!(p.trash_dir(), PathBuf::from("/vault/_trash"));
    }
}
