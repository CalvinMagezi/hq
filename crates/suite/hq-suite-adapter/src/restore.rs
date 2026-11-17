//! Getting originals back after the phone is gone.
//!
//! A restored vault has the notes, and each note whose original was sealed into a sink says so
//! (`suite_original_state: offloaded`, the sink, and the original's `source` path and
//! `source_sha256`). That is enough to fetch every such original from the sink and put it back
//! where it was, with no job journal: the journal is not part of what a backup needs to carry.

use std::path::Path;

use anyhow::Result;
use hq_suite_md::{Document, frontmatter, meta::KEY_ORIGINAL_SINK, meta::KEY_ORIGINAL_STATE};
use hq_suite_shrink::set_original_kept;
use hq_suite_store::{ColdStore, SuitePaths, fsx};

use crate::shrink::VaultNotes;

/// Folders no note lives in (media, the suite's own data, the trash).
const SKIP_TOP: &[&str] = &["_media", "_data", "_trash", ".git"];

/// What a restore did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Restored {
    /// Originals fetched from the sink and put back.
    pub restored: Vec<String>,
    /// Originals already at their place with the right bytes; their notes were marked kept.
    pub already_there: Vec<String>,
    /// Something else is at that place, so nothing was written.
    pub in_the_way: Vec<String>,
    /// Not in the sink, or damaged there.
    pub failed: Vec<(String, String)>,
}

/// Every `.md` under the vault (outside the folders above), as vault-relative paths.
fn notes_of(vault: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![(vault.to_path_buf(), String::new())];
    while let Some((dir, rel)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let child = if rel.is_empty() {
                name.clone()
            } else {
                format!("{rel}/{name}")
            };
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() && !(rel.is_empty() && SKIP_TOP.contains(&name.as_str())) {
                stack.push((e.path(), child));
            } else if ft.is_file() && name.ends_with(".md") {
                out.push(child);
            }
        }
    }
    out.sort();
    out
}

/// Fetches every original `sink` holds for a note of this vault and puts it back at its source.
pub fn restore_originals(vault: &Path, sink: &dyn ColdStore) -> Result<Restored> {
    let paths = SuitePaths::new(vault);
    let notes = VaultNotes::new(paths.clone());
    let mut report = Restored::default();
    for rel in notes_of(vault) {
        let Ok(text) = std::fs::read_to_string(vault.join(&rel)) else {
            continue;
        };
        let doc = Document::parse(&text);
        let fm = doc.frontmatter();
        let (Some("offloaded"), Some(named)) = (
            frontmatter::get(fm, KEY_ORIGINAL_STATE).as_deref(),
            frontmatter::get(fm, KEY_ORIGINAL_SINK),
        ) else {
            continue;
        };
        if named != sink.id() {
            continue;
        }
        let (Some(source), Some(sha)) = (
            frontmatter::get(fm, "source"),
            frontmatter::get(fm, "source_sha256"),
        ) else {
            report
                .failed
                .push((rel, "the note does not say which file it came from".into()));
            continue;
        };
        let Ok(dest) = paths.resolve(&source) else {
            report
                .failed
                .push((source, "not a path inside the vault".into()));
            continue;
        };
        if dest.exists() {
            let same = fsx::sha256_file(&dest).is_ok_and(|(h, _)| h == sha);
            if same {
                set_original_kept(&notes, &rel)?;
                report.already_there.push(source);
            } else {
                report.in_the_way.push(source);
            }
            continue;
        }
        match sink.get(&sha) {
            Ok(bytes) => {
                if !fsx::write_new(&dest, &bytes)? {
                    report.in_the_way.push(source);
                    continue;
                }
                set_original_kept(&notes, &rel)?;
                report.restored.push(source);
            }
            Err(e) => report.failed.push((source, e.to_string())),
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_suite_store::{ColdError, ColdResult};
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Mem(Mutex<HashMap<String, Vec<u8>>>);
    impl ColdStore for Mem {
        fn id(&self) -> &str {
            "card"
        }
        fn put(&self, sha: &str, b: &[u8]) -> ColdResult<()> {
            self.0.lock().unwrap().insert(sha.into(), b.to_vec());
            Ok(())
        }
        fn get(&self, sha: &str) -> ColdResult<Vec<u8>> {
            self.0
                .lock()
                .unwrap()
                .get(sha)
                .cloned()
                .ok_or_else(|| ColdError("not in the sink".into()))
        }
        fn has(&self, sha: &str) -> bool {
            self.0.lock().unwrap().contains_key(sha)
        }
    }

    fn note(vault: &Path, rel: &str, sink: &str, source: &str, sha: &str) {
        let text = format!(
            "---\ntitle: T\nsource: {source}\nsource_sha256: {sha}\nsuite_original_state: offloaded\nsuite_original_sink: {sink}\nsuite_saved_bytes: 10\n---\n\nBody\n"
        );
        let p = vault.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    fn sha(b: &[u8]) -> String {
        fsx::sha256_bytes(b)
    }

    #[test]
    fn originals_come_back_to_their_places_and_their_notes_say_so() {
        let d = tempfile::tempdir().unwrap();
        let sink = Mem::default();
        let (a, b) = (b"first original".to_vec(), b"second original".to_vec());
        sink.put(&sha(&a), &a).unwrap();
        sink.put(&sha(&b), &b).unwrap();
        note(
            d.path(),
            "Notebooks/x/a.md",
            "card",
            "Notebooks/x/a.docx",
            &sha(&a),
        );
        note(
            d.path(),
            "Notebooks/y/b.md",
            "card",
            "Notebooks/y/b.xlsx",
            &sha(&b),
        );
        // Another sink's note, and a plain note, are not touched.
        note(
            d.path(),
            "Notebooks/z/c.md",
            "other",
            "Notebooks/z/c.docx",
            &sha(b"c"),
        );
        std::fs::write(d.path().join("Notebooks/plain.md"), "just text").unwrap();

        let r = restore_originals(d.path(), &sink).unwrap();
        assert_eq!(r.restored.len(), 2, "{r:?}");
        assert_eq!(
            std::fs::read(d.path().join("Notebooks/x/a.docx")).unwrap(),
            a
        );
        assert_eq!(
            std::fs::read(d.path().join("Notebooks/y/b.xlsx")).unwrap(),
            b
        );
        let after = std::fs::read_to_string(d.path().join("Notebooks/x/a.md")).unwrap();
        assert!(after.contains("suite_original_state: kept"), "{after}");
        assert!(!after.contains("suite_original_sink"), "{after}");
        assert!(
            std::fs::read_to_string(d.path().join("Notebooks/z/c.md"))
                .unwrap()
                .contains("offloaded")
        );
        // Running it again finds nothing left to do.
        let again = restore_originals(d.path(), &sink).unwrap();
        assert_eq!(again, Restored::default());
    }

    #[test]
    fn a_file_in_the_way_is_never_overwritten_and_a_missing_object_is_reported() {
        let d = tempfile::tempdir().unwrap();
        let sink = Mem::default();
        let a = b"original".to_vec();
        sink.put(&sha(&a), &a).unwrap();
        note(
            d.path(),
            "Notebooks/a.md",
            "card",
            "Notebooks/a.docx",
            &sha(&a),
        );
        std::fs::write(
            d.path().join("Notebooks/a.docx"),
            b"someone saved another file here",
        )
        .unwrap();
        note(
            d.path(),
            "Notebooks/m.md",
            "card",
            "Notebooks/m.docx",
            &sha(b"lost"),
        );
        let r = restore_originals(d.path(), &sink).unwrap();
        assert_eq!(r.in_the_way, vec!["Notebooks/a.docx".to_string()]);
        assert_eq!(r.failed.len(), 1);
        assert_eq!(
            std::fs::read(d.path().join("Notebooks/a.docx")).unwrap(),
            b"someone saved another file here"
        );
        assert!(!d.path().join("Notebooks/m.docx").exists());
    }

    #[test]
    fn an_original_already_at_its_place_just_marks_the_note() {
        let d = tempfile::tempdir().unwrap();
        let sink = Mem::default();
        let a = b"same bytes".to_vec();
        note(
            d.path(),
            "Notebooks/a.md",
            "card",
            "Notebooks/a.docx",
            &sha(&a),
        );
        std::fs::write(d.path().join("Notebooks/a.docx"), &a).unwrap();
        let r = restore_originals(d.path(), &sink).unwrap();
        assert_eq!(r.already_there, vec!["Notebooks/a.docx".to_string()]);
        assert!(
            std::fs::read_to_string(d.path().join("Notebooks/a.md"))
                .unwrap()
                .contains("suite_original_state: kept")
        );
    }
}
