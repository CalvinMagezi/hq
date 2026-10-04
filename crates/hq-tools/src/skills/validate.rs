//! Skill validation: issues, hint problems, and per-skill checks.

use std::collections::HashSet;
use std::path::Path;

use super::parse::*;
use crate::skill_bundle::load_skill_bundles;

/// Hints too short or too common to carry signal. `word_boundary_match`
/// matches the raw token, so a hint of "ui" or "note" fires on nearly every
/// message and the skill loads when it has nothing to contribute.
const HINT_STOPWORDS: &[&str] = &[
    "the", "and", "for", "with", "this", "that", "you", "your", "can", "use", "get", "set", "new",
    "all", "any", "not", "but", "how", "why", "who", "was", "are", "has", "had", "one", "two",
    "note", "notes", "file", "code", "task", "work", "make", "help", "run", "add",
];

/// Minimum body lines before a skill counts as more than a stub.
pub(super) const MIN_SKILL_BODY_LINES: usize = 10;

/// Byte ceiling on the combined catalog descriptions. Every one of these ships
/// in every system prompt on every surface.
const MAX_CATALOG_BYTES: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone)]
pub struct SkillIssue {
    pub skill: String,
    pub severity: Severity,
    pub message: String,
}

impl SkillIssue {
    fn error(skill: &str, message: impl Into<String>) -> Self {
        Self {
            skill: skill.to_string(),
            severity: Severity::Error,
            message: message.into(),
        }
    }
    fn warning(skill: &str, message: impl Into<String>) -> Self {
        Self {
            skill: skill.to_string(),
            severity: Severity::Warning,
            message: message.into(),
        }
    }
}

/// Check every skill for the configurations that silently do nothing.
///
/// Most of these rules exist because a skill can look configured and never
/// fire: `autoLoad` without hints can never match, and a non-`loadFull`
/// autoLoad skill without a `SUMMARY.md` used to match and then vanish.
/// Both shapes were present in the live vault.
pub fn validate_skills(skills_dir: &Path) -> Vec<SkillIssue> {
    let mut issues = Vec::new();
    let Some(names) = skill_names(skills_dir) else {
        return vec![SkillIssue::error(
            "<skills dir>",
            format!("cannot read {}", skills_dir.display()),
        )];
    };

    let known: HashSet<String> = names.iter().cloned().collect();
    let mut catalog_bytes = 0usize;

    for name in &names {
        let Some(skill) = parse_skill(skills_dir, name) else {
            issues.push(SkillIssue::error(name, "SKILL.md missing or unparseable"));
            continue;
        };

        if !skill.bundle_only {
            catalog_bytes += skill.name.len() + skill.description.len() + 8;
        }
        issues.extend(check_skill(skills_dir, &skill, &known));
    }

    for bundle in load_skill_bundles(&bundles_dir(skills_dir)) {
        catalog_bytes += bundle.name.len() + bundle.description.len() + 8;
        for member in &bundle.skills {
            if parse_skill(skills_dir, member).is_none() {
                issues.push(SkillIssue::error(
                    &bundle.name,
                    format!("bundle member {member:?} does not exist"),
                ));
            }
        }
    }

    if catalog_bytes > MAX_CATALOG_BYTES {
        issues.push(SkillIssue::warning(
            "<catalog>",
            format!(
                "catalog descriptions total {catalog_bytes} bytes (over {MAX_CATALOG_BYTES}); \
                 this ships in every system prompt — prune or bundle"
            ),
        ));
    }

    issues
}

/// Validate one live skill, for gating a write before it is kept.
pub fn validate_skill(skills_dir: &Path, name: &str) -> Vec<SkillIssue> {
    let Some(skill) = parse_skill(skills_dir, name) else {
        return vec![SkillIssue::error(name, "SKILL.md missing or unparseable")];
    };
    let known: HashSet<String> = skill_names(skills_dir)
        .unwrap_or_default()
        .into_iter()
        .collect();
    check_skill(skills_dir, &skill, &known)
}

fn skill_names(skills_dir: &Path) -> Option<Vec<String>> {
    let mut names: Vec<String> = std::fs::read_dir(skills_dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        // A deleted skill keeps only its archive so `hq skills revert` can bring it back.
        .filter(|p| p.join("SKILL.md").exists() || !p.join("archive").is_dir())
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .filter(|n| !n.starts_with('_') && n != "skill-bundles")
        .collect();
    names.sort();
    Some(names)
}

fn check_skill(
    skills_dir: &Path,
    skill: &SkillDefinition,
    known: &HashSet<String>,
) -> Vec<SkillIssue> {
    let mut issues = Vec::new();
    let name = skill.name.as_str();

    for bin in &skill.requires_bins {
        if which::which(bin).is_err() {
            issues.push(SkillIssue::error(
                name,
                format!("requires.bins lists {bin:?}, which is not on PATH"),
            ));
        }
    }

    if skill.hints.is_empty() && !skill.bundle_only && !skill.auto_load {
        issues.push(SkillIssue::warning(
            name,
            "no hints, so it can only load if the model chooses it from the catalog",
        ));
    }

    if skill.description.trim().is_empty() || skill.description == format!("Skill: {name}") {
        issues.push(SkillIssue::error(
            name,
            "no `description` in frontmatter — the catalog entry is what agents \
             use to decide whether to load this",
        ));
    }

    if skill.auto_load && skill.hints.is_empty() {
        issues.push(SkillIssue::error(
            name,
            "autoLoad is true but hints is empty — this skill can never fire",
        ));
    }

    if skill.auto_load && !skill.load_full && !skills_dir.join(name).join("SUMMARY.md").is_file() {
        issues.push(SkillIssue::error(
            name,
            "autoLoad with loadFull false needs a SUMMARY.md — without one the \
             skill matches and is then dropped",
        ));
    }

    for hint in &skill.hints {
        if let Some(problem) = hint_problem(hint) {
            issues.push(SkillIssue::error(name, problem));
        }
    }

    let body_lines = skill
        .content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .count();
    if body_lines < MIN_SKILL_BODY_LINES {
        issues.push(SkillIssue::warning(
            name,
            format!("body is only {body_lines} non-empty lines — is this a stub?"),
        ));
    }

    for next in &skill.next_skills {
        if !known.contains(next) && !is_bundle_skill(skills_dir, next) {
            issues.push(SkillIssue::error(
                name,
                format!("nextSkills references {next:?}, which does not exist"),
            ));
        }
    }
    issues
}

/// Why a hint would misfire, or `None` when it is usable.
pub fn hint_problem(hint: &str) -> Option<String> {
    let h = hint.trim();
    if h.len() < 3 {
        Some(format!(
            "hint {h:?} is too short — it will match constantly"
        ))
    } else if HINT_STOPWORDS.contains(&h) {
        Some(format!(
            "hint {h:?} is a common word — it will match constantly"
        ))
    } else if h != hint.to_lowercase() {
        Some(format!("hint {hint:?} must be lowercase"))
    } else {
        None
    }
}
