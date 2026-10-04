//! In-place SKILL.md edits that keep every frontmatter key the renderer does not know.

use std::path::Path;

use anyhow::{Result, bail};
use serde_yaml::{Mapping, Value};

use crate::skill_audit::{archive_current, rollback};
use crate::skills::{Severity, hint_problem, parse_skill, validate_skill, validate_skill_name};

/// `provenance.mintedBy` on skills the post-session reviewer wrote.
pub const REVIEWER: &str = "skill-review";
/// Frontmatter flag that hands a skill the owner wrote to the reviewer.
pub const MANAGED_KEY: &str = "managed";
/// A patch may not grow a body past this; existing larger bodies may still shrink.
pub const MAX_PATCHED_BODY_CHARS: usize = 20_000;
const SKILL_FILE: &str = "SKILL.md";
const FENCE: &str = "---";

/// What to change. Every field is optional; an edit that changes nothing is an error.
#[derive(Debug, Default)]
pub struct SkillEdit<'a> {
    /// Replace `old` with `new` in the body; `old` must occur exactly once.
    pub patch: Option<(&'a str, &'a str)>,
    pub add_hints: &'a [String],
    /// `Some(true)` sets `managed: true`, `Some(false)` removes the key.
    pub managed: Option<bool>,
}

/// The applied edit, for the audit line.
#[derive(Debug, PartialEq)]
pub struct Edited {
    pub text: String,
    pub version: u32,
    pub hints_added: Vec<String>,
}

fn split_frontmatter(raw: &str) -> Result<(Mapping, &str)> {
    let (Some(fm), body) = hq_core::frontmatter_utils::split_frontmatter(raw) else {
        bail!("SKILL.md has no closed frontmatter block");
    };
    Ok((serde_yaml::from_str(fm)?, body))
}

fn read_live(skills_dir: &Path, name: &str) -> Result<String> {
    validate_skill_name(name)?;
    Ok(std::fs::read_to_string(
        skills_dir.join(name).join(SKILL_FILE),
    )?)
}

/// Whether the reviewer may edit this skill: it wrote it, or the owner adopted it.
pub fn is_managed(skills_dir: &Path, name: &str) -> bool {
    let Some(fm) = read_live(skills_dir, name)
        .ok()
        .and_then(|raw| split_frontmatter(&raw).ok().map(|(fm, _)| fm))
    else {
        return false;
    };
    let minted = fm
        .get("provenance")
        .and_then(|p| p.get("mintedBy"))
        .and_then(Value::as_str);
    minted == Some(REVIEWER) || fm.get(MANAGED_KEY).and_then(Value::as_bool) == Some(true)
}

fn patch_body(body: &str, old: &str, new: &str) -> Result<String> {
    match body.matches(old).count() {
        _ if old.is_empty() => bail!("patch `old` text is empty"),
        0 => bail!("patch `old` text not found in the skill body"),
        1 => {}
        n => bail!("patch `old` text matches {n} times; it must match exactly once"),
    }
    let patched = body.replacen(old, new, 1);
    if patched.len() > MAX_PATCHED_BODY_CHARS && patched.len() > body.len() {
        bail!("patched body would exceed {MAX_PATCHED_BODY_CHARS} chars");
    }
    Ok(patched)
}

/// Adds usable, new hints; turns on auto-loading so they can fire.
fn add_hints(fm: &mut Mapping, wanted: &[String], has_summary: bool) -> Vec<String> {
    let mut hints: Vec<Value> = match fm.get("hints") {
        Some(Value::Sequence(seq)) => seq.clone(),
        _ => Vec::new(),
    };
    let mut added = Vec::new();
    for hint in wanted.iter().map(|h| h.trim().to_lowercase()) {
        let fresh = !hints.iter().any(|h| h.as_str() == Some(hint.as_str()));
        if fresh && hint_problem(&hint).is_none() && !added.contains(&hint) {
            hints.push(Value::String(hint.clone()));
            added.push(hint);
        }
    }
    if added.is_empty() {
        return added;
    }
    fm.insert("hints".into(), Value::Sequence(hints));
    if fm.get("autoLoad").and_then(Value::as_bool) != Some(true) {
        fm.insert("autoLoad".into(), Value::Bool(true));
        // autoLoad without loadFull needs a SUMMARY.md, or validation rejects it.
        if !has_summary {
            fm.insert("loadFull".into(), Value::Bool(true));
        }
    }
    added
}

fn bump_version(fm: &mut Mapping) -> u32 {
    let mut provenance = match fm.get("provenance") {
        Some(Value::Mapping(p)) => p.clone(),
        _ => Mapping::new(),
    };
    let next = provenance
        .get("version")
        .and_then(Value::as_u64)
        .unwrap_or(1) as u32
        + 1;
    provenance.insert("version".into(), Value::Number(next.into()));
    fm.insert("provenance".into(), Value::Mapping(provenance));
    next
}

/// Compute the edited SKILL.md text without writing it.
pub fn edited_text(skills_dir: &Path, name: &str, edit: &SkillEdit) -> Result<Edited> {
    let raw = read_live(skills_dir, name)?;
    let (mut fm, body) = split_frontmatter(&raw)?;
    let body = match edit.patch {
        Some((old, new)) => patch_body(body, old, new)?,
        None => body.to_string(),
    };
    let has_summary = skills_dir.join(name).join("SUMMARY.md").is_file();
    let hints_added = add_hints(&mut fm, edit.add_hints, has_summary);
    match edit.managed {
        Some(true) => {
            fm.insert(MANAGED_KEY.into(), Value::Bool(true));
        }
        Some(false) => {
            fm.remove(MANAGED_KEY);
        }
        None => {}
    }
    let content_changed = edit.patch.is_some() || !hints_added.is_empty();
    let version = if content_changed {
        bump_version(&mut fm)
    } else {
        parse_skill(skills_dir, name).map_or(1, |s| s.provenance.version)
    };
    let text = format!("{FENCE}\n{}{FENCE}\n{body}", serde_yaml::to_string(&fm)?);
    if text == raw {
        bail!("edit changes nothing in skill {name:?}");
    }
    Ok(Edited {
        text,
        version,
        hints_added,
    })
}

fn error_messages(skills_dir: &Path, name: &str) -> Vec<String> {
    validate_skill(skills_dir, name)
        .into_iter()
        .filter(|i| i.severity == Severity::Error)
        .map(|i| i.message)
        .collect()
}

/// Write a live SKILL.md, archiving the prior version first. Rolls back on any
/// validation error the prior version did not already have, so a machine
/// missing one of a skill's binaries can still edit it.
pub fn write_live(skills_dir: &Path, name: &str, text: &str) -> Result<()> {
    validate_skill_name(name)?;
    let live = skills_dir.join(name).join(SKILL_FILE);
    let before = if live.exists() {
        error_messages(skills_dir, name)
    } else {
        Vec::new()
    };
    let archived = archive_current(skills_dir, name)?;
    std::fs::create_dir_all(skills_dir.join(name))?;
    std::fs::write(&live, text)?;
    let new_errors: Vec<String> = error_messages(skills_dir, name)
        .into_iter()
        .filter(|e| !before.contains(e))
        .collect();
    if new_errors.is_empty() {
        return Ok(());
    }
    rollback(skills_dir, name, archived)?;
    bail!(
        "skill {name:?} failed validation and was reverted: {}",
        new_errors.join("; ")
    )
}

/// Edit a live skill in place and return what was written.
pub fn edit_skill(skills_dir: &Path, name: &str, edit: &SkillEdit) -> Result<Edited> {
    let edited = edited_text(skills_dir, name, edit)?;
    write_live(skills_dir, name, &edited.text)?;
    Ok(edited)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"---
name: gws-docs-workflow
description: "Create, read, and repeatedly update Google Docs on this host via the gws CLI. Use when asked to write a vault note into Google Drive, build a Doc, append sections to an existing Doc, or sync markdown content into a Google Doc."
metadata:
  author: hq
  version: "1.1"
  verified: "2026-09-17"
  requires:
    bins:
      - sh
  skills:
    - gws-shared
    - gws-docs
    - gws-docs-write
hints:
  - google doc
  - google docs
  - gws docs
  - batchupdate
autoLoad: true
---

# Google Docs workflow

1. Create the doc with gws docs create.
2. Append sections with batchUpdate.

---

## Notes
- Always set the title.
"#;

    fn fixture(dir: &Path) {
        std::fs::create_dir_all(dir.join("gws-docs-workflow")).unwrap();
        std::fs::write(dir.join("gws-docs-workflow").join(SKILL_FILE), FIXTURE).unwrap();
    }

    fn live(dir: &Path) -> String {
        std::fs::read_to_string(dir.join("gws-docs-workflow").join(SKILL_FILE)).unwrap()
    }

    fn frontmatter(dir: &Path) -> Mapping {
        split_frontmatter(&live(dir)).unwrap().0
    }

    #[test]
    fn a_patch_keeps_every_frontmatter_key_and_bumps_the_version() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        let edit = SkillEdit {
            patch: Some((
                "Always set the title.",
                "Always set the title.\n- Bold the headers.",
            )),
            ..Default::default()
        };
        let edited = edit_skill(dir.path(), "gws-docs-workflow", &edit).unwrap();
        assert_eq!(edited.version, 2);

        let fm = frontmatter(dir.path());
        assert_eq!(
            fm.get("name").and_then(Value::as_str),
            Some("gws-docs-workflow")
        );
        let metadata = fm.get("metadata").unwrap();
        assert_eq!(metadata.get("version"), Some(&Value::String("1.1".into())));
        assert_eq!(metadata.get("author").and_then(Value::as_str), Some("hq"));
        assert_eq!(metadata["requires"]["bins"][0].as_str(), Some("sh"));
        assert_eq!(metadata["skills"].as_sequence().unwrap().len(), 3);
        assert_eq!(fm.get("hints").unwrap().as_sequence().unwrap().len(), 4);

        let skill = parse_skill(dir.path(), "gws-docs-workflow").unwrap();
        assert_eq!(skill.provenance.version, 2);
        assert!(skill.auto_load);
        assert!(skill.content.contains("Bold the headers."));
        assert!(
            skill.content.contains("## Notes"),
            "the body rule must not end the frontmatter"
        );
        assert!(
            dir.path()
                .join("gws-docs-workflow/archive/SKILL-v1.md")
                .exists()
        );
    }

    #[test]
    fn a_missing_or_ambiguous_old_text_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        for old in ["not in the body", "the", ""] {
            let edit = SkillEdit {
                patch: Some((old, "x")),
                ..Default::default()
            };
            assert!(
                edit_skill(dir.path(), "gws-docs-workflow", &edit).is_err(),
                "{old:?}"
            );
        }
        assert_eq!(live(dir.path()), FIXTURE);
        assert!(!dir.path().join("gws-docs-workflow/archive").exists());
    }

    #[test]
    fn hints_are_filtered_and_turn_on_auto_loading() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("s")).unwrap();
        let body = "x\n".repeat(12);
        std::fs::write(
            dir.path().join("s/SKILL.md"),
            format!("---\ndescription: \"d\"\n---\n{body}"),
        )
        .unwrap();
        let hints = vec![
            "Column Widths".to_string(),
            "ui".to_string(),
            "the".to_string(),
        ];
        let edited = edit_skill(
            dir.path(),
            "s",
            &SkillEdit {
                add_hints: &hints,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(edited.hints_added, vec!["column widths".to_string()]);
        let skill = parse_skill(dir.path(), "s").unwrap();
        assert_eq!(skill.hints, vec!["column widths".to_string()]);
        assert!(skill.auto_load && skill.load_full);
        assert_eq!(skill.provenance.version, 2);

        let again = SkillEdit {
            add_hints: &hints,
            ..Default::default()
        };
        assert!(
            edit_skill(dir.path(), "s", &again).is_err(),
            "no new hints means no change"
        );
    }

    #[test]
    fn adopting_marks_a_skill_managed_and_unadopting_clears_it() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        assert!(!is_managed(dir.path(), "gws-docs-workflow"));
        edit_skill(
            dir.path(),
            "gws-docs-workflow",
            &SkillEdit {
                managed: Some(true),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(is_managed(dir.path(), "gws-docs-workflow"));
        edit_skill(
            dir.path(),
            "gws-docs-workflow",
            &SkillEdit {
                managed: Some(false),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(!is_managed(dir.path(), "gws-docs-workflow"));
        assert!(!is_managed(dir.path(), "../escape"));
    }

    #[test]
    fn an_edit_that_breaks_validation_is_rolled_back() {
        let dir = tempfile::tempdir().unwrap();
        fixture(dir.path());
        let broken = FIXTURE.replacen("description: ", "summary: ", 1);
        assert!(write_live(dir.path(), "gws-docs-workflow", &broken).is_err());
        assert_eq!(live(dir.path()), FIXTURE);
    }
}
