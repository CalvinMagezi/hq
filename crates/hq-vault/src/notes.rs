use anyhow::{Context, Result};
use chrono::Utc;
use hq_core::types::Note;
use std::io::Write;
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;
use tracing::debug;

use crate::frontmatter;

/// Read a note from the vault by relative path.
///
/// If the path has no extension and the exact file doesn't exist, tries
/// appending `.md` automatically (Obsidian-style wikilink tolerance).
pub(crate) fn read_note(vault_path: &Path, rel_path: &str) -> Result<Note> {
    let exact = vault_path.join(rel_path);
    let (full_path, effective_rel) = if exact.exists() {
        (exact, rel_path.to_string())
    } else if Path::new(rel_path).extension().is_none() {
        let with_ext = format!("{rel_path}.md");
        let candidate = vault_path.join(&with_ext);
        if candidate.exists() {
            (candidate, with_ext)
        } else {
            (exact, rel_path.to_string())
        }
    } else {
        (exact, rel_path.to_string())
    };
    let rel_path = effective_rel.as_str();
    let raw = std::fs::read_to_string(&full_path)
        .with_context(|| format!("reading note: {}", rel_path))?;

    let (fm, content) = frontmatter::parse(&raw)?;

    let title = fm
        .get("title")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| extract_title_from_content(&content, rel_path));

    let metadata = std::fs::metadata(&full_path)?;
    let modified_at = metadata
        .modified()
        .map(chrono::DateTime::<Utc>::from)
        .unwrap_or_else(|_| Utc::now());

    let tags: Vec<String> = fm
        .get("tags")
        .and_then(|v| {
            if let serde_yaml::Value::Sequence(seq) = v {
                Some(
                    seq.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect(),
                )
            } else {
                None
            }
        })
        .unwrap_or_default();

    let pinned = fm.get("pinned").and_then(|v| v.as_bool()).unwrap_or(false);

    Ok(Note {
        path: rel_path.to_string(),
        title,
        content,
        frontmatter: fm,
        note_type: None,
        tags,
        pinned,
        source: None,
        embedding_status: None,
        created_at: None,
        updated_at: None,
        modified_at,
    })
}

/// Replace a file so readers see the old or the new content, never a partial
/// write. The temp file sits in the target's directory, so the rename stays on
/// one filesystem (no EXDEV), and its unique name keeps concurrent writers from
/// racing on a shared .tmp path; dropped without persist() it is removed.
pub fn write_atomic(path: &Path, contents: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("vault path has no parent directory"))?;
    let mut tmp = NamedTempFile::new_in(parent)?;
    tmp.write_all(contents)?;
    // The temp file is created 0600; keep an existing note's mode so an edit
    // does not tighten a 0644 file.
    if let Ok(meta) = std::fs::metadata(path) {
        tmp.as_file().set_permissions(meta.permissions())?;
    }
    tmp.persist(path)
        .map_err(|e| anyhow::anyhow!("failed to persist vault write: {}", e.error))?;
    Ok(())
}

/// Write a note to the vault.
pub(crate) fn write_note(vault_path: &Path, rel_path: &str, note: &Note) -> Result<()> {
    let full_path = vault_path.join(rel_path);

    if let Some(parent) = full_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let raw = frontmatter::serialize(&note.frontmatter, &note.content)?;
    write_atomic(&full_path, raw.as_bytes())?;
    debug!(path = %rel_path, "wrote note");
    Ok(())
}

/// List all markdown files in a directory (non-recursive).
pub(crate) fn list_notes(vault_path: &Path, dir: &str) -> Result<Vec<String>> {
    let full_path = vault_path.join(dir);
    if !full_path.exists() {
        return Ok(Vec::new());
    }

    let mut paths = Vec::new();
    for entry in std::fs::read_dir(&full_path)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "md")
            && let Ok(rel) = path.strip_prefix(vault_path)
        {
            paths.push(rel.to_string_lossy().to_string());
        }
    }
    paths.sort();
    Ok(paths)
}

/// List all markdown files recursively.
pub(crate) fn list_notes_recursive(vault_path: &Path, dir: &str) -> Result<Vec<String>> {
    let full_path = vault_path.join(dir);
    if !full_path.exists() {
        return Ok(Vec::new());
    }

    let mut paths = Vec::new();
    walk_dir(&full_path, vault_path, &mut paths)?;
    paths.sort();
    Ok(paths)
}

fn walk_dir(dir: &Path, vault_root: &Path, out: &mut Vec<String>) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            // Skip hidden directories and _data
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            if !name.starts_with('.') && name != "_data" {
                walk_dir(&path, vault_root, out)?;
            }
        } else if path.extension().is_some_and(|ext| ext == "md")
            && let Ok(rel) = path.strip_prefix(vault_root)
        {
            out.push(rel.to_string_lossy().to_string());
        }
    }
    Ok(())
}

fn extract_title_from_content(content: &str, rel_path: &str) -> String {
    // Try to find first heading
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(heading) = trimmed.strip_prefix("# ") {
            return heading.trim().to_string();
        }
    }
    // Fallback to filename without extension
    PathBuf::from(rel_path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| rel_path.to_string())
}

// ─── Outline & Section Utilities ───────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct NoteHeading {
    pub level: usize,
    pub title: String,
    pub start_line: usize,
    pub end_line: usize,
    pub line_count: usize,
    pub char_count: usize,
    pub snippet: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NoteSection {
    pub heading: String,
    pub level: usize,
    pub start_line: usize,
    pub end_line: usize,
    pub content: String,
}

/// Extract structured heading outline from markdown content.
pub(crate) fn extract_outline(content: &str) -> Vec<NoteHeading> {
    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() {
        return Vec::new();
    }

    struct RawHeading {
        level: usize,
        title: String,
        line_idx: usize,
    }

    let mut raw_headings = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            let hash_count = trimmed.chars().take_while(|c| *c == '#').count();
            if (1..=6).contains(&hash_count) {
                let rest = trimmed[hash_count..].trim();
                if !rest.is_empty() {
                    raw_headings.push(RawHeading {
                        level: hash_count,
                        title: rest.to_string(),
                        line_idx: idx,
                    });
                }
            }
        }
    }

    if raw_headings.is_empty() {
        return Vec::new();
    }

    let mut headings = Vec::new();
    let total_lines = lines.len();

    for (i, h) in raw_headings.iter().enumerate() {
        let start_line = h.line_idx + 1;
        let end_line = if i + 1 < raw_headings.len() {
            raw_headings[i + 1].line_idx
        } else {
            total_lines
        };

        let section_lines = &lines[h.line_idx..end_line];
        let line_count = section_lines.len();
        let char_count: usize = section_lines.iter().map(|l| l.len() + 1).sum();

        let snippet = section_lines
            .iter()
            .skip(1)
            .find(|l| !l.trim().is_empty())
            .copied()
            .unwrap_or("")
            .trim()
            .to_string();

        headings.push(NoteHeading {
            level: h.level,
            title: h.title.clone(),
            start_line,
            end_line,
            line_count,
            char_count,
            snippet,
        });
    }

    headings
}

/// Case-insensitive heading lookup that accepts the title with or without its `#` marks.
fn find_heading<'a>(outline: &'a [NoteHeading], target: &str) -> Option<&'a NoteHeading> {
    let normalized = target.trim().trim_start_matches('#').trim().to_lowercase();
    outline.iter().find(|h| h.title.to_lowercase() == normalized)
}

/// Read a specific section by heading title.
pub(crate) fn read_section(content: &str, target_heading: &str) -> Option<NoteSection> {
    let outline = extract_outline(content);
    let target = find_heading(&outline, target_heading)?;
    let lines: Vec<&str> = content.lines().collect();

    let section_lines = &lines[(target.start_line - 1)..target.end_line];
    let body = section_lines.join("\n");

    Some(NoteSection {
        heading: target.title.clone(),
        level: target.level,
        start_line: target.start_line,
        end_line: target.end_line,
        content: body,
    })
}

/// Append content to note body, optionally targeted under a heading.
pub(crate) fn append_to_note(content: &str, extra: &str, target_heading: Option<&str>) -> String {
    let extra_trimmed = extra.trim();
    if extra_trimmed.is_empty() {
        return content.to_string();
    }

    let Some(target) = target_heading else {
        let mut res = content.trim_end().to_string();
        res.push_str("\n\n");
        res.push_str(extra_trimmed);
        res.push('\n');
        return res;
    };

    let outline = extract_outline(content);
    let found = find_heading(&outline, target);

    let lines: Vec<&str> = content.lines().collect();

    let insert_line_idx = match found {
        Some(h) => {
            // Find last non-empty line inside section (between start_line and end_line)
            let end = h.end_line.min(lines.len());
            let mut last_idx = h.start_line;
            for idx in (h.start_line..end).rev() {
                if !lines[idx].trim().is_empty() {
                    last_idx = idx + 1;
                    break;
                }
            }
            last_idx
        }
        None => lines.len(),
    };

    let mut new_lines = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        if idx == insert_line_idx {
            new_lines.push(extra_trimmed);
        }
        new_lines.push(*line);
    }
    if insert_line_idx >= lines.len() {
        if !lines.is_empty() && !lines.last().unwrap().trim().is_empty() {
            new_lines.push("");
        }
        new_lines.push(extra_trimmed);
    }

    let mut res = new_lines.join("\n");
    if !res.ends_with('\n') {
        res.push('\n');
    }
    res
}

/// Replace the body under a target heading without modifying other sections or YAML frontmatter.
pub(crate) fn patch_section(
    content: &str,
    target_heading: &str,
    new_body: &str,
) -> anyhow::Result<String> {
    let outline = extract_outline(content);
    let target = find_heading(&outline, target_heading)
        .ok_or_else(|| anyhow::anyhow!("Heading '{}' not found in note", target_heading))?;

    let lines: Vec<&str> = content.lines().collect();
    let mut new_lines = Vec::new();

    // Copy lines up to heading line
    for line in &lines[..target.start_line] {
        new_lines.push(*line);
    }

    // Insert new body
    let body_trimmed = new_body.trim();
    if !body_trimmed.is_empty() {
        new_lines.push(body_trimmed);
    }

    // Copy lines after target section
    if target.end_line < lines.len() {
        for line in &lines[target.end_line..] {
            new_lines.push(*line);
        }
    }

    let mut res = new_lines.join("\n");
    if !res.ends_with('\n') {
        res.push('\n');
    }
    Ok(res)
}

/// Extract all WikiLinks (`[[Note]]` or `[[Note|Alias]]`) and Markdown links (`[text](note.md)`).
pub(crate) fn extract_links(content: &str) -> Vec<String> {
    let mut links: Vec<String> = Vec::new();
    for target in crate::remediation::extract_wikilinks(content) {
        if !links.contains(&target) {
            links.push(target);
        }
    }

    let mut rest = content;
    while let Some(start) = rest.find("](") {
        rest = &rest[start + 2..];
        if let Some(end) = rest.find(')') {
            let target = rest[..end].trim();
            if !target.starts_with("http://")
                && !target.starts_with("https://")
                && !target.starts_with('#')
                && !target.is_empty()
            {
                let clean = target.strip_suffix(".md").unwrap_or(target).to_string();
                if !links.contains(&clean) {
                    links.push(clean);
                }
            }
            rest = &rest[end + 1..];
        }
    }

    links
}

#[cfg(test)]
mod atomic_write_tests {
    use super::*;
    use chrono::Utc;
    use std::collections::HashMap;

    #[cfg(unix)]
    #[test]
    fn write_atomic_keeps_an_existing_files_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.md");
        std::fs::write(&path, "old").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        write_atomic(&path, b"new").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o644);
    }
    use tempfile::TempDir;

    fn test_note(path: &str) -> Note {
        Note {
            title: "Test".to_string(),
            content: "# Test\nHello".to_string(),
            path: path.to_string(),
            frontmatter: HashMap::new(),
            note_type: None,
            tags: Vec::new(),
            pinned: false,
            source: None,
            embedding_status: None,
            created_at: None,
            updated_at: None,
            modified_at: Utc::now(),
        }
    }

    #[test]
    fn write_note_leaves_no_tmp_sidecar_on_success() {
        let dir = TempDir::new().unwrap();
        let vault = dir.path().to_path_buf();

        write_note(&vault, "test.md", &test_note("test.md")).unwrap();

        // NamedTempFile uses opaque random names (no .tmp suffix), but verify
        // neither the old-style sidecar nor any stray temp file remains.
        let tmp = vault.join("test.md.tmp");
        assert!(
            !tmp.exists(),
            ".tmp sidecar must not persist after successful write"
        );
        assert!(vault.join("test.md").exists(), "target file must exist");
    }

    #[test]
    fn write_note_cleans_up_tmp_on_failure() {
        let dir = TempDir::new().unwrap();
        let vault = dir.path().to_path_buf();

        // Create the target path as a directory to force persist() to fail
        // (can't atomically replace a directory with a file).
        let target_dir = vault.join("test.md");
        std::fs::create_dir_all(&target_dir).unwrap();

        let result = write_note(&vault, "test.md", &test_note("test.md"));
        assert!(
            result.is_err(),
            "Expected write to fail when target is a directory"
        );

        // No temp files should remain in the vault directory — NamedTempFile
        // cleans up on drop when persist() was not called or failed.
        let leftover: Vec<_> = std::fs::read_dir(&vault)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                // Exclude the directory we created on purpose
                e.path() != target_dir
            })
            .collect();
        assert!(
            leftover.is_empty(),
            "Temp files leaked after failed write: {:?}",
            leftover.iter().map(|e| e.file_name()).collect::<Vec<_>>()
        );
    }
}

#[cfg(test)]
mod section_and_link_tests {
    use super::*;

    #[test]
    fn test_extract_outline() {
        let content = "# Title\nFirst paragraph.\n\n## Section 1\nSection 1 text.\n\n### Subsection A\nSub text.\n\n## Section 2\nSection 2 text.";
        let outline = extract_outline(content);
        assert_eq!(outline.len(), 4);
        assert_eq!(outline[0].title, "Title");
        assert_eq!(outline[0].level, 1);
        assert_eq!(outline[1].title, "Section 1");
        assert_eq!(outline[1].level, 2);
        assert_eq!(outline[2].title, "Subsection A");
        assert_eq!(outline[2].level, 3);
        assert_eq!(outline[3].title, "Section 2");
        assert_eq!(outline[3].level, 2);
    }

    #[test]
    fn test_read_section() {
        let content = "# Title\nIntro.\n\n## Key Decisions\n- Decision 1\n- Decision 2\n\n## Next Steps\nDo work.";
        let section = read_section(content, "Key Decisions").unwrap();
        assert_eq!(section.heading, "Key Decisions");
        assert!(section.content.contains("Decision 1"));
        assert!(!section.content.contains("Next Steps"));
    }

    #[test]
    fn test_append_to_note() {
        let content = "# Daily Log\n\n## Tasks\n- Task 1\n\n## Notes\nSome note.";
        let updated = append_to_note(content, "- Task 2", Some("Tasks"));
        assert!(updated.contains("- Task 1\n- Task 2"));

        let appended_end = append_to_note(content, "End entry.", None);
        assert!(appended_end.ends_with("End entry.\n"));
    }

    #[test]
    fn test_patch_section() {
        let content = "# Note\n\n## Status\nDrafting.\n\n## Content\nHello.";
        let patched = patch_section(content, "Status", "Complete.").unwrap();
        assert!(patched.contains("## Status\nComplete."));
        assert!(patched.contains("## Content\nHello."));
        assert!(!patched.contains("Drafting."));
    }

    #[test]
    fn test_extract_links() {
        let content =
            "Check out [[Project Alpha]] and [[Architecture|Arch Doc]] or [Link](docs/beta.md).";
        let links = extract_links(content);
        assert_eq!(links, vec!["Project Alpha", "Architecture", "docs/beta"]);
    }
}
