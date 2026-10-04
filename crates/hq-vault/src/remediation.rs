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

pub(crate) fn repoint_wikilinks(vault_path: &Path, old_stem: &str, new_stem: &str) -> anyhow::Result<u32> {
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
        let out = repoint_content("[[Old]] [[old|Alias]] [[Other]] [[broken", "old", "New").unwrap();
        assert_eq!(out, "[[New]] [[New|Alias]] [[Other]] [[broken");
        assert!(repoint_content("[[Other]]", "old", "New").is_none());
    }
}
