//! Rendering skills and writing, approving, or rejecting proposals.

use anyhow::Result;
use std::path::{Path, PathBuf};

use super::parse::*;

/// Render a SkillDefinition back to SKILL.md text (frontmatter + body).
pub fn render_skill(skill: &SkillDefinition) -> String {
    let mut fm = String::new();
    fm.push_str("---\n");
    fm.push_str(&format!(
        "description: \"{}\"\n",
        escape_yaml(&skill.description)
    ));
    fm.push_str(&format!("autoLoad: {}\n", skill.auto_load));
    if skill.load_full {
        fm.push_str("loadFull: true\n");
    }
    if skill.bundle_only {
        fm.push_str("bundleOnly: true\n");
    }
    if !skill.hints.is_empty() {
        fm.push_str("hints:\n");
        for h in &skill.hints {
            fm.push_str(&format!("  - {}\n", h));
        }
    }
    if !skill.next_skills.is_empty() {
        fm.push_str("nextSkills:\n");
        for n in &skill.next_skills {
            fm.push_str(&format!("  - {}\n", n));
        }
    }
    fm.push_str("provenance:\n");
    fm.push_str(&format!("  mintedBy: {}\n", skill.provenance.minted_by));
    if let Some(run_id) = &skill.provenance.minted_from_run_id {
        fm.push_str(&format!("  mintedFromRunId: \"{}\"\n", escape_yaml(run_id)));
    }
    fm.push_str(&format!("  version: {}\n", skill.provenance.version));
    fm.push_str("---\n\n");
    fm.push_str(&skill.content);
    fm
}

fn escape_yaml(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Stage already-rendered SKILL.md text under `_proposed/<name>/`.
pub fn write_proposal_text(skills_dir: &Path, name: &str, text: &str) -> Result<PathBuf> {
    validate_skill_name(name)?;
    let dir = skills_dir.join("_proposed").join(name);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("SKILL.md");
    std::fs::write(&path, text)?;
    Ok(path)
}

pub(crate) fn validate_skill_name(name: &str) -> Result<()> {
    if name.is_empty() || name.contains('/') || name.contains('\\') || name.contains("..") {
        anyhow::bail!("invalid skill name: {}", name);
    }
    Ok(())
}

/// Promote `<skills_dir>/_proposed/<name>/SKILL.md` to `<skills_dir>/<name>/SKILL.md`.
///
/// If a live skill with the same name already exists, its `SKILL.md` is archived
/// under `<name>/archive/` first rather than overwritten. Shared by the `hq skills
/// approve` CLI command and the Telegram approve/reject reply flow so both paths
/// move a proposal the exact same way.
pub fn approve_proposed_skill(skills_dir: &Path, name: &str) -> Result<PathBuf> {
    validate_skill_name(name)?;

    let proposed_dir = skills_dir.join("_proposed").join(name);
    let proposed_file = proposed_dir.join("SKILL.md");
    if !proposed_file.exists() {
        anyhow::bail!("proposed skill '{}' not found", name);
    }

    let live_dir = skills_dir.join(name);
    let live_file = live_dir.join("SKILL.md");

    crate::skill_audit::archive_current(skills_dir, name)?;
    std::fs::create_dir_all(&live_dir)?;
    std::fs::rename(&proposed_file, &live_file)?;
    std::fs::remove_dir(&proposed_dir)?;
    crate::skill_audit::audit_live(
        skills_dir,
        name,
        crate::skill_audit::Disposition::Adopted,
        "approved",
    )?;

    Ok(live_file)
}

/// Discard `<skills_dir>/_proposed/<name>/`, dropping the draft entirely.
pub fn reject_proposed_skill(skills_dir: &Path, name: &str) -> Result<()> {
    validate_skill_name(name)?;

    let proposed_dir = skills_dir.join("_proposed").join(name);
    if !proposed_dir.exists() {
        anyhow::bail!("proposed skill '{}' not found", name);
    }
    std::fs::remove_dir_all(&proposed_dir)?;
    Ok(())
}
