//! The notes and attachments a link can name, listed one way.
//!
//! The search index, the health report and the fix planner must agree on what a note is, so
//! this follows the index's rules (`hq_db::search::reconcile`): hidden entries and the trash
//! are skipped, symlinks are never followed, and `_meta.md` and editor lock files are not
//! notes. `hq-db` does not depend on this crate, so the rules are repeated here and a test
//! pins them.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// The folder the index covers and the one links resolve into.
pub const NOTES_DIR: &str = "Notebooks";

/// Folders that hold files a note embeds or links to, besides `Notebooks/` itself.
pub const ATTACHMENT_DIRS: &[&str] = &["_attachments", "_media", "Exports"];

/// One file found by [`walk`].
#[derive(Debug, Clone)]
pub struct VaultFile {
    /// Vault-relative, `/`-separated.
    pub rel: String,
    pub abs: PathBuf,
    pub size: u64,
    pub modified: Option<SystemTime>,
}

/// What a walk found.
#[derive(Debug, Default)]
pub struct Walked {
    /// Notes, sorted by `rel`.
    pub notes: Vec<VaultFile>,
    /// Non-note files under `Notebooks/` and [`ATTACHMENT_DIRS`], sorted by `rel`.
    pub attachments: Vec<String>,
    /// Folders that could not be listed. A note in one is unknown, not absent.
    pub unlisted: Vec<String>,
}

/// Whether a directory is skipped: hidden ones (`.git`, `.obsidian`) and the trash.
pub fn is_excluded_dir(name: &str) -> bool {
    name.starts_with('.') || name == "_trash"
}

/// Whether a file name is a note: `.md`, not hidden, not `_meta.md`.
pub fn is_note_file(name: &str) -> bool {
    !name.starts_with('.')
        && name != "_meta.md"
        && Path::new(name).extension().is_some_and(|e| e == "md")
}

/// List the notes under `Notebooks/` and the attachments around them. `lite` drops the
/// attachment folders that start with `_`, which Lite hides from the person.
pub fn walk(vault: &Path, lite: bool) -> Walked {
    let mut out = Walked::default();
    walk_dir(vault, &vault.join(NOTES_DIR), true, &mut out);
    for dir in ATTACHMENT_DIRS {
        if lite && dir.starts_with('_') {
            continue;
        }
        walk_dir(vault, &vault.join(dir), false, &mut out);
    }
    out.notes.sort_by(|a, b| a.rel.cmp(&b.rel));
    out.attachments.sort();
    out.attachments.dedup();
    out
}

fn walk_dir(vault: &Path, root: &Path, collect_notes: bool, out: &mut Walked) {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) => {
                // A folder that does not exist is simply empty. Anything else is unknown.
                if e.kind() != std::io::ErrorKind::NotFound {
                    out.unlisted.push(rel_string(&dir, vault));
                }
                continue;
            }
        };
        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                out.unlisted.push(rel_string(&entry.path(), vault));
                continue;
            };
            if file_type.is_symlink() {
                continue;
            }
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let path = entry.path();
            if file_type.is_dir() {
                if !is_excluded_dir(name) {
                    stack.push(path);
                }
            } else if file_type.is_file() && !name.starts_with('.') {
                let rel = rel_string(&path, vault);
                if collect_notes && is_note_file(name) {
                    let meta = entry.metadata().ok();
                    out.notes.push(VaultFile {
                        rel,
                        abs: path,
                        size: meta.as_ref().map_or(0, |m| m.len()),
                        modified: meta.and_then(|m| m.modified().ok()),
                    });
                } else if !is_note_file(name) {
                    out.attachments.push(rel);
                }
            }
        }
    }
}

fn rel_string(path: &Path, vault: &Path) -> String {
    path.strip_prefix(vault)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn touch(root: &Path, rel: &str) {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, "x").unwrap();
    }

    #[test]
    fn notes_follow_the_index_rules() {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        for rel in [
            "Notebooks/a.md",
            "Notebooks/Sub/b.md",
            "Notebooks/_meta.md",
            "Notebooks/.hidden.md",
            "Notebooks/.git/c.md",
            "Notebooks/_trash/d.md",
            "Notebooks/pic.png",
            "_attachments/scan.pdf",
            "_system/SOUL.md",
            "Elsewhere/e.md",
        ] {
            touch(r, rel);
        }
        let w = walk(r, false);
        let notes: Vec<&str> = w.notes.iter().map(|n| n.rel.as_str()).collect();
        assert_eq!(notes, ["Notebooks/Sub/b.md", "Notebooks/a.md"]);
        // `_meta.md` is a file that exists but not a note, so a link to it is not dangling.
        assert_eq!(
            w.attachments,
            [
                "Notebooks/_meta.md",
                "Notebooks/pic.png",
                "_attachments/scan.pdf"
            ]
        );
        assert!(w.unlisted.is_empty());
    }

    #[test]
    fn lite_leaves_out_the_attachment_folders_it_hides() {
        let dir = tempfile::tempdir().unwrap();
        touch(dir.path(), "Notebooks/a.md");
        touch(dir.path(), "_attachments/scan.pdf");
        touch(dir.path(), "Exports/2026-11/out.pdf");
        let w = walk(dir.path(), true);
        assert_eq!(w.attachments, ["Exports/2026-11/out.pdf"]);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_is_never_followed() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        touch(outside.path(), "secret.md");
        touch(dir.path(), "Notebooks/a.md");
        std::os::unix::fs::symlink(outside.path(), dir.path().join("Notebooks/link")).unwrap();
        std::os::unix::fs::symlink(
            outside.path().join("secret.md"),
            dir.path().join("Notebooks/linked.md"),
        )
        .unwrap();
        let w = walk(dir.path(), false);
        let notes: Vec<&str> = w.notes.iter().map(|n| n.rel.as_str()).collect();
        assert_eq!(notes, ["Notebooks/a.md"]);
    }
}
