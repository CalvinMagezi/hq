//! Media kept once, by content.
//!
//! A picture is filed as `_media/cas/<first two hex>/<sha256>.<ext>`. The same bytes arriving
//! again (a re-forwarded file, the same logo in twenty documents) cost nothing. Notes link to the
//! file with an ordinary relative markdown link, so any viewer and any exporter works, and a
//! person can recover everything with a file manager.

use std::collections::HashSet;
use std::fs;

use crate::error::{Result, StoreError};
use crate::fsx::{self, sha256_bytes, sha256_file};
use crate::paths::{CAS_REL, SuitePaths};

/// What an object is filed under.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct CasRef {
    /// 64 lowercase hex characters.
    pub sha256: String,
    /// 1 to 5 lowercase letters or digits.
    pub ext: String,
}

/// Whether `put` wrote something.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stored {
    New,
    /// The same bytes were already there.
    Existing,
}

fn clean_ext(ext: &str) -> String {
    let e: String = ext
        .trim_start_matches('.')
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .take(5)
        .collect();
    if e.is_empty() { "bin".into() } else { e }
}

fn is_hex64(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

impl CasRef {
    pub fn new(sha256: &str, ext: &str) -> Option<CasRef> {
        is_hex64(sha256).then(|| CasRef {
            sha256: sha256.to_string(),
            ext: clean_ext(ext),
        })
    }

    /// The vault-relative path: `_media/cas/ab/abcd....webp`.
    pub fn rel(&self) -> String {
        format!(
            "{CAS_REL}/{}/{}.{}",
            &self.sha256[..2],
            self.sha256,
            self.ext
        )
    }

    /// Reads `_media/cas/ab/<64 hex>.<ext>` back into a reference, whatever text surrounds it.
    pub fn parse_rel(path: &str) -> Option<CasRef> {
        let start = path.find(&format!("{CAS_REL}/"))? + CAS_REL.len() + 1;
        let rest = &path[start..];
        let (shard, rest) = rest.split_once('/')?;
        let name_end = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '.'))
            .unwrap_or(rest.len());
        let (sha, ext) = rest[..name_end].split_once('.')?;
        (is_hex64(sha)
            && shard == &sha[..2]
            && !ext.is_empty()
            && ext.len() <= 5
            && !ext.contains('.'))
        .then(|| CasRef {
            sha256: sha.to_string(),
            ext: ext.to_string(),
        })
    }

    /// A markdown image link to this object from the note at `note_rel`, with the right number of
    /// `../` for where that note sits.
    pub fn markdown_image(&self, note_rel: &str, alt: &str) -> String {
        let depth = note_rel.matches('/').count();
        let up = "../".repeat(depth);
        let alt = alt.replace(['[', ']', '\n', '\r'], " ");
        format!("![{alt}]({up}{})", self.rel())
    }
}

/// Every object a piece of text links to. Finds links in any syntax (markdown, wiki, HTML),
/// since it looks for the path itself.
pub fn references_in(text: &str) -> HashSet<CasRef> {
    let mut found = HashSet::new();
    let needle = format!("{CAS_REL}/");
    let mut from = 0;
    while let Some(i) = text[from..].find(&needle) {
        let at = from + i;
        if let Some(r) =
            CasRef::parse_rel(&text[at..text.len().min(at + needle.len() + 3 + 64 + 8)])
        {
            found.insert(r);
        }
        from = at + needle.len();
    }
    found
}

/// The media store of one vault.
#[derive(Debug, Clone)]
pub struct Cas {
    paths: SuitePaths,
}

impl Cas {
    pub fn new(paths: SuitePaths) -> Self {
        Cas { paths }
    }

    fn file(&self, r: &CasRef) -> std::path::PathBuf {
        self.paths.vault().join(r.rel())
    }

    /// Stores `bytes`. If the same content is already there with the right size nothing is
    /// written; a damaged copy (wrong size) is replaced.
    pub fn put(&self, bytes: &[u8], ext: &str) -> Result<(CasRef, Stored)> {
        let r = CasRef {
            sha256: sha256_bytes(bytes),
            ext: clean_ext(ext),
        };
        let path = self.file(&r);
        // The store folder itself must not be a link.
        self.paths
            .resolve(&format!("{CAS_REL}/{}", &r.sha256[..2]))?;
        if fsx::write_new(&path, bytes)? {
            return Ok((r, Stored::New));
        }
        let have = fs::metadata(&path)
            .map_err(|e| StoreError::io(&path, e))?
            .len();
        if have != bytes.len() as u64 {
            fsx::atomic_write(&path, bytes)?;
            return Ok((r, Stored::New));
        }
        Ok((r, Stored::Existing))
    }

    pub fn has(&self, r: &CasRef) -> bool {
        self.file(r).is_file()
    }

    pub fn read(&self, r: &CasRef) -> Result<Vec<u8>> {
        let path = self.file(r);
        fs::read(&path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                StoreError::Missing(r.rel())
            } else {
                StoreError::io(&path, e)
            }
        })
    }

    /// Re-hashes the stored file. The only check that proves the bytes are the ones filed.
    pub fn verify(&self, r: &CasRef) -> Result<()> {
        let path = self.file(r);
        if !path.is_file() {
            return Err(StoreError::Missing(r.rel()));
        }
        let (hash, _) = sha256_file(&path)?;
        if hash == r.sha256 {
            Ok(())
        } else {
            Err(StoreError::Corrupt(r.rel()))
        }
    }

    /// Every object in the store.
    pub fn list(&self) -> Vec<CasRef> {
        let mut out = Vec::new();
        let Ok(shards) = fs::read_dir(self.paths.cas_dir()) else {
            return out;
        };
        for shard in shards.flatten() {
            let Ok(files) = fs::read_dir(shard.path()) else {
                continue;
            };
            for f in files.flatten() {
                let name = f.file_name().to_string_lossy().into_owned();
                if let Some(r) = CasRef::parse_rel(&format!(
                    "{CAS_REL}/{}/{name}",
                    shard.file_name().to_string_lossy()
                )) {
                    out.push(r);
                }
            }
        }
        out.sort();
        out
    }

    /// Objects no note links to, given every reference found in the vault's notes. Nothing is
    /// removed: the caller decides, and should only do so after a full scan of every note.
    pub fn unreferenced(&self, referenced: &HashSet<CasRef>) -> Vec<CasRef> {
        self.list()
            .into_iter()
            .filter(|r| !referenced.contains(r))
            .collect()
    }

    pub fn remove(&self, r: &CasRef) -> Result<()> {
        let path = self.file(r);
        fs::remove_file(&path).map_err(|e| StoreError::io(&path, e))
    }

    /// Bytes the store holds.
    pub fn total_bytes(&self) -> u64 {
        self.list()
            .iter()
            .filter_map(|r| fs::metadata(self.file(r)).ok())
            .map(|m| m.len())
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cas() -> (tempfile::TempDir, Cas) {
        let d = tempfile::tempdir().unwrap();
        let c = Cas::new(SuitePaths::new(d.path()));
        (d, c)
    }

    #[test]
    fn the_same_bytes_are_stored_once() {
        let (_d, c) = cas();
        let (a, first) = c.put(b"picture bytes", "webp").unwrap();
        let (b, again) = c.put(b"picture bytes", ".WEBP").unwrap();
        assert_eq!((first, again), (Stored::New, Stored::Existing));
        assert_eq!(a, b);
        assert_eq!(c.list(), vec![a.clone()]);
        assert_eq!(c.read(&a).unwrap(), b"picture bytes");
        assert!(a.rel().starts_with("_media/cas/") && a.rel().ends_with(".webp"));
        assert_eq!(c.total_bytes(), 13);
    }

    #[test]
    fn different_bytes_get_different_files() {
        let (_d, c) = cas();
        let (a, _) = c.put(b"one", "png").unwrap();
        let (b, _) = c.put(b"two", "png").unwrap();
        assert_ne!(a, b);
        assert_eq!(c.list().len(), 2);
    }

    #[test]
    fn verify_catches_a_changed_file_and_put_repairs_a_short_one() {
        let (d, c) = cas();
        let (r, _) = c.put(b"original bytes", "jpg").unwrap();
        c.verify(&r).unwrap();
        let path = d.path().join(r.rel());
        fs::write(&path, b"tampered byte!").unwrap(); // same length, wrong content
        assert!(matches!(c.verify(&r), Err(StoreError::Corrupt(_))));
        fs::write(&path, b"short").unwrap(); // wrong length: put repairs it
        let (_, stored) = c.put(b"original bytes", "jpg").unwrap();
        assert_eq!(stored, Stored::New);
        c.verify(&r).unwrap();
        c.remove(&r).unwrap();
        assert!(matches!(c.verify(&r), Err(StoreError::Missing(_))));
        assert!(matches!(c.read(&r), Err(StoreError::Missing(_))));
    }

    #[test]
    fn references_are_found_in_any_syntax() {
        let (_d, c) = cas();
        let (r1, _) = c.put(b"a", "png").unwrap();
        let (r2, _) = c.put(b"b", "webp").unwrap();
        let text = format!(
            "![x](../../{}) and [[{}]] and <img src=\"{}\"> and _media/cas/zz/not-a-hash.png",
            r1.rel(),
            r2.rel(),
            r1.rel()
        );
        let found = references_in(&text);
        assert_eq!(found, HashSet::from([r1, r2]));
        assert!(references_in("no media here").is_empty());
        assert!(references_in("_media/cas/").is_empty());
    }

    #[test]
    fn unreferenced_objects_are_listed_not_removed() {
        let (_d, c) = cas();
        let (keep, _) = c.put(b"keep", "png").unwrap();
        let (drop, _) = c.put(b"drop", "png").unwrap();
        let used = HashSet::from([keep.clone()]);
        assert_eq!(c.unreferenced(&used), vec![drop.clone()]);
        assert!(c.has(&drop));
    }

    #[test]
    fn links_count_the_folders_a_note_is_below() {
        let r = CasRef::new(&"ab".repeat(32), "webp").unwrap();
        assert_eq!(
            r.markdown_image("Top.md", "pic"),
            format!("![pic]({})", r.rel())
        );
        assert_eq!(
            r.markdown_image("Notebooks/Docs/Letter.md", "a [b]"),
            format!("![a  b ](../../{})", r.rel())
        );
    }

    #[test]
    fn hostile_names_do_not_parse() {
        let sha = "ab".repeat(32);
        assert!(CasRef::parse_rel(&format!("_media/cas/ab/{sha}.png")).is_some());
        assert!(
            CasRef::parse_rel(&format!("_media/cas/cd/{sha}.png")).is_none(),
            "wrong shard"
        );
        assert!(CasRef::parse_rel(&format!("_media/cas/ab/{sha}.")).is_none());
        assert!(CasRef::parse_rel(&format!("_media/cas/ab/{sha}.toolong")).is_none());
        assert!(CasRef::parse_rel(&format!("_media/cas/ab/{}.png", sha.to_uppercase())).is_none());
        assert!(CasRef::new("short", "png").is_none());
        assert_eq!(CasRef::new(&sha, "../../x").unwrap().ext, "x");
        assert_eq!(CasRef::new(&sha, "").unwrap().ext, "bin");
    }

    #[cfg(unix)]
    #[test]
    fn a_linked_store_folder_is_refused() {
        let outside = tempfile::tempdir().unwrap();
        let (d, c) = cas();
        fs::create_dir_all(d.path().join("_media")).unwrap();
        std::os::unix::fs::symlink(outside.path(), d.path().join("_media/cas")).unwrap();
        assert!(matches!(c.put(b"x", "png"), Err(StoreError::UnsafePath(_))));
        assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
    }
}
