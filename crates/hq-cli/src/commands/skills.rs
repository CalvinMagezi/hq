use anyhow::{Context, Result, bail};
use clap::Subcommand;
use hq_core::config::HqConfig;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Subcommand)]
pub enum SkillsAction {
    /// Create a new skill
    Create {
        name: String,
        #[arg(short, long)]
        description: Option<String>,
        #[arg(short, long)]
        hints: Vec<String>,
        #[arg(short, long)]
        content: Option<String>,
    },
    /// List all skills
    List,
    /// Show a skill's SKILL.md contents
    Show { name: String },
    /// Print the path to a skill's SKILL.md for editing
    Edit { name: String },
    /// Approve a proposed skill
    Approve { name: String },
    /// Reject a proposed skill
    Reject { name: String },
    /// Delete a live skill
    Delete { name: String },
    /// Check every skill for configurations that silently do nothing
    Validate,
    /// Restore a skill's newest archived version, archiving the current one first
    Revert { name: String },
    /// Show a skill's audit trail and archived versions
    History { name: String },
    /// Let the post-session review improve a skill you wrote
    Adopt { name: String },
    /// Make an adopted skill read-only to the review again
    Unadopt { name: String },
}

pub async fn run(config: &HqConfig, action: SkillsAction) -> Result<()> {
    run_with_output(config, action, &mut std::io::stdout()).await
}

async fn run_with_output(
    config: &HqConfig,
    action: SkillsAction,
    output: &mut dyn Write,
) -> Result<()> {
    let skills_dir = hq_core::skills_dir(&config.vault_path);

    match action {
        SkillsAction::Create {
            name,
            description,
            hints,
            content,
        } => create_skill(&skills_dir, &name, description.as_deref(), &hints, content).await,
        SkillsAction::List => list_skills(&skills_dir, output).await,
        SkillsAction::Show { name } => show_skill(&skills_dir, &name).await,
        SkillsAction::Edit { name } => edit_skill(&skills_dir, &name, output).await,
        SkillsAction::Approve { name } => approve_skill(&skills_dir, &name, output).await,
        SkillsAction::Reject { name } => reject_skill(&skills_dir, &name).await,
        SkillsAction::Delete { name } => delete_skill(&skills_dir, &name).await,
        SkillsAction::Validate => validate_skills(&skills_dir, output).await,
        SkillsAction::Revert { name } => {
            validate_name(&name)?;
            let path = hq_tools::skill_audit::revert_skill(&skills_dir, &name)?;
            writeln!(output, "Reverted skill at {}", path.display())?;
            Ok(())
        }
        SkillsAction::History { name } => skill_history(&skills_dir, &name, output),
        SkillsAction::Adopt { name } => set_managed(&skills_dir, &name, true, output),
        SkillsAction::Unadopt { name } => set_managed(&skills_dir, &name, false, output),
    }
}

/// Enough of the content hash to tell versions apart by eye.
const SHA_PREFIX_CHARS: usize = 12;

fn skill_history(skills_dir: &Path, name: &str, output: &mut dyn Write) -> Result<()> {
    use hq_tools::skill_audit::{archived_versions, audit_history};

    validate_name(name)?;
    let entries = audit_history(skills_dir, name);
    let archives = archived_versions(skills_dir, name);
    if entries.is_empty() && archives.is_empty() {
        bail!("no audit entries or archived versions for skill '{name}'");
    }
    for e in &entries {
        let flags = if e.flags.is_empty() { String::new() } else { format!(" flags=[{}]", e.flags.join("; ")) };
        let run = e.run_id.as_deref().map(|r| format!(" run={r}")).unwrap_or_default();
        writeln!(
            output,
            "{}  {:?}  v{}  sha256={}{run}{flags}  {}",
            e.ts, e.disposition, e.version, e.sha256.get(..SHA_PREFIX_CHARS).unwrap_or(&e.sha256), e.reason
        )?;
    }
    for (n, path) in &archives {
        writeln!(output, "archive v{n}  {}", path.display())?;
    }
    Ok(())
}

fn set_managed(skills_dir: &Path, name: &str, managed: bool, output: &mut dyn Write) -> Result<()> {
    use hq_tools::skill_audit::{Disposition, audit_live};
    use hq_tools::skill_edit::{SkillEdit, edit_skill};

    validate_name(name)?;
    edit_skill(skills_dir, name, &SkillEdit { managed: Some(managed), ..Default::default() })?;
    let verb = if managed { "adopted" } else { "unadopted" };
    audit_live(skills_dir, name, Disposition::Adopted, &format!("{verb} by owner"))?;
    writeln!(output, "Skill '{name}' {verb}")?;
    Ok(())
}

/// Report every skill whose configuration means it can never fire.
async fn validate_skills(skills_dir: &Path, output: &mut dyn Write) -> Result<()> {
    use hq_tools::skills::Severity;

    let issues = hq_tools::skills::validate_skills(skills_dir);
    let live = hq_tools::skills::list_skills(skills_dir);
    let errors = issues
        .iter()
        .filter(|i| i.severity == Severity::Error)
        .count();
    let warnings = issues.len() - errors;

    writeln!(output, "{} skills in the catalog\n", live.len())?;
    for issue in &issues {
        let label = match issue.severity {
            Severity::Error => "FAIL",
            Severity::Warning => "warn",
        };
        writeln!(output, "  {label}  {}: {}", issue.skill, issue.message)?;
    }
    if issues.is_empty() {
        writeln!(output, "  no issues")?;
    }
    writeln!(output, "\n{errors} failures, {warnings} warnings")?;

    if errors > 0 {
        bail!("{errors} skill(s) are misconfigured and cannot fire");
    }
    Ok(())
}

fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("skill name cannot be empty");
    }
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        bail!("skill name cannot contain path separators or '..'");
    }
    Ok(())
}

fn skill_path(skills_dir: &Path, name: &str) -> PathBuf {
    skills_dir.join(name).join("SKILL.md")
}

fn proposed_skill_path(skills_dir: &Path, name: &str) -> PathBuf {
    skills_dir.join("_proposed").join(name).join("SKILL.md")
}

fn format_frontmatter(description: Option<&str>, hints: &[String]) -> String {
    let mut out = String::from("---\n");
    out.push_str(&format!(
        "description: \"{}\"\n",
        description.unwrap_or("").replace('"', "\\\"")
    ));
    out.push_str("autoLoad: false\n");
    if hints.is_empty() {
        out.push_str("hints: []\n");
    } else {
        out.push_str("hints:\n");
        for hint in hints {
            out.push_str(&format!("  - \"{}\"\n", hint.replace('"', "\\\"")));
        }
    }
    out.push_str("---\n");
    out
}

async fn create_skill(
    skills_dir: &Path,
    name: &str,
    description: Option<&str>,
    hints: &[String],
    content: Option<String>,
) -> Result<()> {
    validate_name(name)?;

    let dir = skills_dir.join(name);
    if dir.exists() {
        bail!("skill '{}' already exists", name);
    }

    std::fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create skill directory {}", dir.display()))?;

    let body = content.unwrap_or_else(|| format!("# {name}\n\nAdd skill instructions here."));
    let frontmatter = format_frontmatter(description, hints);
    let path = dir.join("SKILL.md");
    std::fs::write(&path, format!("{frontmatter}\n{body}"))
        .with_context(|| format!("failed to write {}", path.display()))?;

    println!("Created skill at {}", path.display());
    Ok(())
}

async fn list_skills(skills_dir: &Path, output: &mut dyn Write) -> Result<()> {
    // The canonical catalog, so this matches exactly what agents see: bundles
    // appear as single entries and their `bundleOnly` members do not.
    for skill in hq_tools::skills::list_skills(skills_dir) {
        writeln!(output, "{}  {}", skill.name, skill.description)?;
    }

    let proposed_dir = skills_dir.join("_proposed");
    if proposed_dir.exists() {
        let mut proposed: Vec<String> = Vec::new();
        for entry in std::fs::read_dir(&proposed_dir)
            .with_context(|| format!("failed to read {}", proposed_dir.display()))?
            .flatten()
        {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            if let Some(skill) = hq_tools::skills::parse_skill(&proposed_dir, &name) {
                proposed.push(format!("{}  {} [proposed]", skill.name, skill.description));
            }
        }
        proposed.sort();
        for line in proposed {
            writeln!(output, "{line}")?;
        }
    }

    Ok(())
}

async fn show_skill(skills_dir: &Path, name: &str) -> Result<()> {
    validate_name(name)?;

    let live = skill_path(skills_dir, name);
    let proposed = proposed_skill_path(skills_dir, name);

    let path = if live.exists() {
        live
    } else if proposed.exists() {
        proposed
    } else {
        bail!("skill '{}' not found", name);
    };

    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    print!("{content}");
    Ok(())
}

async fn edit_skill(skills_dir: &Path, name: &str, output: &mut dyn Write) -> Result<()> {
    validate_name(name)?;

    let live = skill_path(skills_dir, name);
    let proposed = proposed_skill_path(skills_dir, name);

    let path = if live.exists() {
        live
    } else if proposed.exists() {
        proposed
    } else {
        bail!("skill '{}' not found", name);
    };

    writeln!(output, "{}", path.display())?;
    writeln!(output, "Open this file in your editor to edit.")?;
    Ok(())
}

async fn approve_skill(skills_dir: &Path, name: &str, output: &mut dyn Write) -> Result<()> {
    validate_name(name)?;
    let live_file = hq_tools::skills::approve_proposed_skill(skills_dir, name)?;
    writeln!(output, "Approved skill at {}", live_file.display())?;
    Ok(())
}

async fn reject_skill(skills_dir: &Path, name: &str) -> Result<()> {
    validate_name(name)?;
    hq_tools::skills::reject_proposed_skill(skills_dir, name)?;
    println!("Rejected proposed skill '{}'", name);
    Ok(())
}

async fn delete_skill(skills_dir: &Path, name: &str) -> Result<()> {
    validate_name(name)?;

    let dir = skills_dir.join(name);
    if !dir.exists() {
        bail!("skill '{}' not found", name);
    }

    std::fs::remove_dir_all(&dir)
        .with_context(|| format!("failed to remove skill dir {}", dir.display()))?;

    println!("Deleted skill '{}'", name);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config(vault_path: &Path) -> HqConfig {
        HqConfig {
            vault_path: vault_path.to_path_buf(),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn create_and_list_skill() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let config = test_config(temp.path());

        run(
            &config,
            SkillsAction::Create {
                name: "test-skill".to_string(),
                description: Some("A test skill".to_string()),
                hints: vec!["rust".to_string(), "cli".to_string()],
                content: None,
            },
        )
        .await?;

        let mut output = Vec::new();
        run_with_output(&config, SkillsAction::List, &mut output).await?;
        let output = String::from_utf8(output)?;

        assert!(output.contains("test-skill"));
        assert!(output.contains("A test skill"));
        Ok(())
    }

    #[tokio::test]
    async fn approve_moves_from_proposed() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let config = test_config(temp.path());
        let skills_dir = temp.path().join("Skills");
        let proposed_dir = skills_dir.join("_proposed").join("proposed-skill");
        std::fs::create_dir_all(&proposed_dir)?;
        std::fs::write(
            proposed_dir.join("SKILL.md"),
            "---\ndescription: Proposed skill\nautoLoad: false\nhints: []\n---\n# Proposed\n",
        )?;

        run(
            &config,
            SkillsAction::Approve {
                name: "proposed-skill".to_string(),
            },
        )
        .await?;

        assert!(skills_dir.join("proposed-skill").join("SKILL.md").exists());
        assert!(!proposed_dir.exists());
        Ok(())
    }

    #[tokio::test]
    async fn reject_removes_proposed() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let config = test_config(temp.path());
        let proposed_dir = temp
            .path()
            .join("Skills")
            .join("_proposed")
            .join("reject-skill");
        std::fs::create_dir_all(&proposed_dir)?;
        std::fs::write(
            proposed_dir.join("SKILL.md"),
            "---\ndescription: Reject me\nautoLoad: false\nhints: []\n---\n",
        )?;

        run(
            &config,
            SkillsAction::Reject {
                name: "reject-skill".to_string(),
            },
        )
        .await?;

        assert!(!proposed_dir.exists());
        Ok(())
    }

    #[tokio::test]
    async fn revert_then_history_shows_the_trail() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let config = test_config(temp.path());
        let skill_dir = temp.path().join("Skills").join("hist");
        std::fs::create_dir_all(skill_dir.join("archive"))?;
        std::fs::write(skill_dir.join("archive/SKILL-v1.md"), "---\ndescription: Old\n---\nold\n")?;
        std::fs::write(skill_dir.join("SKILL.md"), "---\ndescription: New\n---\nnew\n")?;

        let mut out = Vec::new();
        run_with_output(&config, SkillsAction::Revert { name: "hist".to_string() }, &mut out).await?;
        assert!(std::fs::read_to_string(skill_dir.join("SKILL.md"))?.contains("old"));

        let mut out = Vec::new();
        run_with_output(&config, SkillsAction::History { name: "hist".to_string() }, &mut out).await?;
        let out = String::from_utf8(out)?;
        assert!(out.contains("Reverted"), "{out}");
        assert!(out.contains("archive v2"), "{out}");
        Ok(())
    }

    #[tokio::test]
    async fn adopt_and_unadopt_toggle_review_ownership() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let config = test_config(temp.path());
        let skills_dir = temp.path().join("Skills");
        let skill_dir = skills_dir.join("mine");
        std::fs::create_dir_all(&skill_dir)?;
        std::fs::write(skill_dir.join("SKILL.md"), "---\nname: mine\ndescription: Mine\n---\n# Mine\n")?;

        let mut out = Vec::new();
        run_with_output(&config, SkillsAction::Adopt { name: "mine".to_string() }, &mut out).await?;
        assert!(hq_tools::skill_edit::is_managed(&skills_dir, "mine"));
        assert!(std::fs::read_to_string(skill_dir.join("SKILL.md"))?.contains("name: mine"));

        run_with_output(&config, SkillsAction::Unadopt { name: "mine".to_string() }, &mut out).await?;
        assert!(!hq_tools::skill_edit::is_managed(&skills_dir, "mine"));
        let reasons: Vec<String> =
            hq_tools::skill_audit::audit_history(&skills_dir, "mine").into_iter().map(|e| e.reason).collect();
        assert_eq!(reasons, vec!["adopted by owner", "unadopted by owner"]);
        Ok(())
    }

    #[tokio::test]
    async fn delete_removes_skill() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let config = test_config(temp.path());
        let skill_dir = temp.path().join("Skills").join("delete-skill");
        std::fs::create_dir_all(&skill_dir)?;
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\ndescription: Delete me\nautoLoad: false\nhints: []\n---\n",
        )?;

        run(
            &config,
            SkillsAction::Delete {
                name: "delete-skill".to_string(),
            },
        )
        .await?;

        assert!(!skill_dir.exists());
        Ok(())
    }
}
