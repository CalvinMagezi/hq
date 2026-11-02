use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::notes::write_atomic;

/// Every `.md` file under `dir`, recursively. Unreadable directories are skipped.
fn md_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for path in entries.flatten().map(|e| e.path()) {
        if path.is_dir() {
            md_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "md") {
            out.push(path);
        }
    }
}

pub fn count_notes_and_stems(
    dir: &Path,
    total: &mut u32,
    broken: &mut u32,
    stems: &mut HashSet<String>,
    paths: &mut Vec<PathBuf>,
) {
    let start = paths.len();
    md_files(dir, paths);
    for path in &paths[start..] {
        *total += 1;
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            stems.insert(stem.to_lowercase());
        }
        if let Ok(content) = std::fs::read_to_string(path) {
            let trimmed = content.trim();
            if trimmed.starts_with("---") && trimmed[3..].find("---").is_none() {
                *broken += 1;
            }
        }
    }
}

pub fn extract_wikilinks(content: &str) -> Vec<String> {
    let mut links = Vec::new();
    let mut remaining = content;
    while let Some(start) = remaining.find("[[") {
        remaining = &remaining[start + 2..];
        if let Some(end) = remaining.find("]]") {
            let target = &remaining[..end];
            let target = target.split('|').next().unwrap_or(target).trim();
            if !target.is_empty() && target.len() < 200 {
                links.push(target.to_string());
            }
            remaining = &remaining[end + 2..];
        } else {
            break;
        }
    }
    links
}

/// `content` with every `[[old_stem]]` or `[[old_stem|alias]]` pointed at `new_stem`,
/// or None when nothing matched.
fn repoint_content(content: &str, old_lower: &str, new_stem: &str) -> Option<String> {
    let mut out = String::with_capacity(content.len());
    let mut remaining = content;
    let mut changed = false;
    while let Some(start) = remaining.find("[[") {
        out.push_str(&remaining[..start]);
        remaining = &remaining[start + 2..];
        let Some(end) = remaining.find("]]") else {
            out.push_str("[[");
            break;
        };
        let inner = &remaining[..end];
        let (target, alias) = match inner.split_once('|') {
            Some((t, a)) => (t, Some(a.trim())),
            None => (inner, None),
        };
        if target.trim().to_lowercase() == old_lower {
            let alias = alias.map(|a| format!("|{a}")).unwrap_or_default();
            out.push_str(&format!("[[{new_stem}{alias}]]"));
            changed = true;
        } else {
            out.push_str(&format!("[[{inner}]]"));
        }
        remaining = &remaining[end + 2..];
    }
    out.push_str(remaining);
    changed.then_some(out)
}

pub(crate) fn repoint_wikilinks(
    vault_path: &Path,
    old_stem: &str,
    new_stem: &str,
) -> anyhow::Result<u32> {
    let mut paths = Vec::new();
    md_files(&vault_path.join("Notebooks"), &mut paths);

    let old_lower = old_stem.to_lowercase();
    let mut modified = 0u32;
    for path in &paths {
        let Ok(content) = std::fs::read_to_string(path) else {
            continue;
        };
        let mtime_before = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        let Some(new_content) = repoint_content(&content, &old_lower, new_stem) else {
            continue;
        };
        // Skip a file someone else edited between our read and write.
        let mtime_now = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        if mtime_before == mtime_now && write_atomic(path, new_content.as_bytes()).is_ok() {
            modified += 1;
        }
    }
    Ok(modified)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repoint_keeps_aliases_and_other_links() {
        let out =
            repoint_content("[[Old]] [[old|Alias]] [[Other]] [[broken", "old", "New").unwrap();
        assert_eq!(out, "[[New]] [[New|Alias]] [[Other]] [[broken");
        assert!(repoint_content("[[Other]]", "old", "New").is_none());
    }
}

/// What [`repair_nested_frontmatter`] found and, when applied, changed.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NestedFrontmatterReport {
    pub scanned: u32,
    /// Vault-relative paths of the notes with a nested block (repaired when applied).
    pub nested: Vec<String>,
}

/// Merge the frontmatter block an older `vault_write_note` left at the start of a note's body
/// into the note's real frontmatter. Only that exact shape is touched: an outer frontmatter
/// holding `title` and nothing else, followed by a body that opens with a YAML map. When `apply`
/// is set each original is copied under `backup_dir` (same relative path) before it is rewritten.
pub fn repair_nested_frontmatter(
    vault: &Path,
    backup_dir: &Path,
    apply: bool,
) -> anyhow::Result<NestedFrontmatterReport> {
    let mut files = Vec::new();
    md_files(&vault.join("Notebooks"), &mut files);
    let mut report = NestedFrontmatterReport::default();
    for path in files {
        report.scanned += 1;
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Some(repaired) = unnest(&raw) else {
            continue;
        };
        let rel = path
            .strip_prefix(vault)
            .unwrap_or(&path)
            .to_string_lossy()
            .into_owned();
        if apply {
            let backup = backup_dir.join(&rel);
            if let Some(parent) = backup.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&backup, &raw)?;
            write_atomic(&path, repaired.as_bytes())?;
        }
        report.nested.push(rel);
    }
    Ok(report)
}

/// The note rewritten with one frontmatter, or `None` when it is not the nested shape.
fn unnest(raw: &str) -> Option<String> {
    let (Some(outer), body) = hq_core::frontmatter_utils::split_frontmatter(raw) else {
        return None;
    };
    let outer: std::collections::HashMap<String, serde_yaml::Value> =
        serde_yaml::from_str(outer).ok()?;
    if outer.len() != 1 {
        return None;
    }
    let title = outer.get("title")?.as_str()?.to_string();
    let inner = body.trim_start();
    let (Some(block), _) = hq_core::frontmatter_utils::split_frontmatter(inner) else {
        return None;
    };
    let parsed: std::collections::HashMap<String, serde_yaml::Value> =
        serde_yaml::from_str(block).ok()?;
    if parsed.is_empty() {
        return None;
    }
    let merged = crate::frontmatter::merge_leading_frontmatter(title, inner, Vec::new());
    crate::frontmatter::serialize(&merged.frontmatter, &merged.content).ok()
}

#[cfg(test)]
mod nested_frontmatter_tests {
    use super::*;

    const NESTED: &str = "---\ntitle: Edge notes\n---\n\n---\ntitle: \"Agent-HQ on the Edge\"\nstatus: exploratory\ntags:\n  - edge-ai\n  - android\n---\n\n# Body\n\ntext\n";

    fn vault_with(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (rel, body) in files {
            let path = dir.path().join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, body).unwrap();
        }
        dir
    }

    #[test]
    fn a_nested_block_becomes_the_notes_frontmatter_and_the_original_is_kept() {
        let vault = vault_with(&[("Notebooks/a.md", NESTED)]);
        let backup = vault.path().join("_trash/x");
        let report = repair_nested_frontmatter(vault.path(), &backup, true).unwrap();
        assert_eq!(report.nested, ["Notebooks/a.md"]);

        let fixed = std::fs::read_to_string(vault.path().join("Notebooks/a.md")).unwrap();
        assert_eq!(fixed.matches("---").count(), 2, "{fixed}");
        let tags = hq_core::frontmatter_utils::extract_tags_from_frontmatter(&fixed);
        assert_eq!(tags, ["edge-ai", "android"]);
        assert!(
            fixed.contains("title: Edge notes"),
            "the outer title wins: {fixed}"
        );
        assert!(fixed.contains("status: exploratory"));
        assert!(fixed.trim_end().ends_with("# Body\n\ntext"));
        assert_eq!(
            std::fs::read_to_string(backup.join("Notebooks/a.md")).unwrap(),
            NESTED
        );

        let again = repair_nested_frontmatter(vault.path(), &backup, true).unwrap();
        assert!(again.nested.is_empty(), "a repaired note is left alone");
    }

    #[test]
    fn other_shapes_are_never_touched() {
        let notes = [
            // An outer frontmatter with more than a title was not written by the old tool.
            (
                "Notebooks/rich.md",
                "---\ntitle: T\ntags:\n  - a\n---\n\n---\nx: 1\n---\nbody\n",
            ),
            // A horizontal rule after a normal frontmatter.
            (
                "Notebooks/rule.md",
                "---\ntitle: T\n---\n\ntext\n\n---\n\nmore\n",
            ),
            // A block that is not YAML.
            (
                "Notebooks/bad.md",
                "---\ntitle: T\n---\n\n---\n: : [\n---\nbody\n",
            ),
            ("Notebooks/plain.md", "no frontmatter at all\n"),
        ];
        let vault = vault_with(&notes);
        let report =
            repair_nested_frontmatter(vault.path(), &vault.path().join("_trash/x"), true).unwrap();
        assert_eq!(report.scanned, 4);
        assert!(report.nested.is_empty(), "{:?}", report.nested);
        for (rel, body) in notes {
            assert_eq!(
                std::fs::read_to_string(vault.path().join(rel)).unwrap(),
                body
            );
        }
    }

    #[test]
    fn a_dry_run_reports_without_writing() {
        let vault = vault_with(&[("Notebooks/a.md", NESTED)]);
        let report =
            repair_nested_frontmatter(vault.path(), &vault.path().join("_trash/x"), false).unwrap();
        assert_eq!(report.nested.len(), 1);
        assert_eq!(
            std::fs::read_to_string(vault.path().join("Notebooks/a.md")).unwrap(),
            NESTED
        );
        assert!(!vault.path().join("_trash").exists());
    }
}
