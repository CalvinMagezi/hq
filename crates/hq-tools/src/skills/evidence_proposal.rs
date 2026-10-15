//! A reviewable proposal built from skill evidence.
//!
//! A proposal says what keeps happening, how often and over what period, points at examples by
//! id, suggests the kind of change that would address it, and lists the regression fixtures any
//! change has to keep passing. It is written to `skills/_proposed/<name>/PROPOSAL.md`.
//!
//! It never edits the skill and cannot be approved into one: it has no replacement text. It is
//! evidence for the owner or the skill reviewer to act on through the governed edit path, where
//! the fixtures gate the change. `hq skills reject <name>` dismisses it.

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use hq_db::skill_evidence::{
    MIN_INVOCATIONS, MIN_SESSIONS, Pattern, PatternKind, SkillEvidence, VersionComparison,
};

use super::fixtures::{FIXTURES_FILE, load_fixtures};
use super::proposals::validate_skill_name;

/// The file name inside `skills/_proposed/<name>/`.
pub const PROPOSAL_FILE: &str = "PROPOSAL.md";

fn likely_change(p: &Pattern) -> String {
    match p.kind {
        PatternKind::ReworkedOutcomes => format!(
            "Work that used this skill was reopened or rejected in {} loads. Read the referenced \
             tasks, find the step the reopened work lacked, tighten or add it in the skill, and \
             add it to `required_steps` in the fixtures so it cannot be dropped again.",
            p.invocations
        ),
        PatternKind::JudgedHarmful => format!(
            "The post-session review judged this skill harmful in {} loads. Check for an \
             outdated or over-broad step and narrow it; if the hints are why it loaded, narrow \
             them and add the unwanted phrases to `not_triggers`.",
            p.invocations
        ),
        PatternKind::LoadedButUnused => format!(
            "The skill loaded in {} sessions that then did not use it. Its hints probably match \
             work it does not fit: remove or sharpen the over-broad hints, and add the phrases \
             from those sessions to `not_triggers`.",
            p.invocations
        ),
        PatternKind::ReworkedForPurpose => format!(
            "Work of kind `{}` that used this skill keeps coming back ({} loads). The skill may \
             lack guidance for that kind of work, or such tasks may arrive without the context \
             the skill assumes. Add that guidance, or restrict the hints.",
            p.qualifier, p.invocations
        ),
        PatternKind::HintsDidNotActivate => format!(
            "The skill was loaded by hand in {} sessions although it is meant to load on its \
             hints. Add the phrasing people used as hints, and as `triggers` in the fixtures.",
            p.invocations
        ),
    }
}

fn pattern_title(p: &Pattern) -> String {
    match p.kind {
        PatternKind::ReworkedForPurpose => format!("{} ({})", p.kind.as_str(), p.qualifier),
        _ => p.kind.as_str().to_string(),
    }
}

fn comparison_line(c: &VersionComparison) -> String {
    let mean = |m: Option<f64>| m.map_or("no scored loads".to_string(), |v| format!("{v:.2}"));
    let short = |h: &Option<String>| {
        h.as_deref()
            .map_or("unrecorded".to_string(), |h| h.chars().take(12).collect())
    };
    format!(
        "- {} → {}: mean outcome {} ({} loads) → {} ({} loads){}",
        short(&c.before),
        short(&c.after),
        mean(c.before_mean),
        c.before_invocations,
        mean(c.after_mean),
        c.after_invocations,
        if c.small_sample {
            ". Small sample: anecdotal."
        } else {
            "."
        }
    )
}

/// The text of the proposal, or `None` when the evidence is below the threshold. A skill
/// with no finding gets no proposal, whatever it shows below the line.
pub fn render_proposal(
    skills_dir: &Path,
    evidence: &SkillEvidence,
    current_hash: Option<&str>,
) -> Option<String> {
    if !evidence.sufficient() {
        return None;
    }
    let name = &evidence.skill;
    let mut out = String::new();
    out.push_str(&format!("# Proposal: improve skill `{name}`\n\n"));
    out.push_str(&format!(
        "Generated {} from skill invocation records over the last {} days. The records hold ids, \
         hashes and fixed labels only, so this proposal quotes no prompt, document or task text.\n\n",
        chrono::Utc::now().format("%Y-%m-%d %H:%M UTC"),
        evidence.window_days
    ));
    out.push_str(
        "**This is a proposal, not a change.** It does not edit the skill and cannot be approved \
         into one. Make the change through `hq skills edit` or the skill review, where the \
         fixtures below gate it. `hq skills reject` dismisses this file.\n\n",
    );

    out.push_str("## Evidence\n\n");
    out.push_str(&format!(
        "A pattern is listed here only when it shows in at least {MIN_INVOCATIONS} loads across \
         at least {MIN_SESSIONS} sessions. Counts show what happened, not why: a pattern is a \
         reason to look, not proof of a cause.\n\n"
    ));
    for p in evidence.findings() {
        out.push_str(&format!(
            "### {}\n\n- {} loads across {} sessions, {} to {}\n- skill version: {}\n",
            pattern_title(p),
            p.invocations,
            p.sessions,
            p.first,
            p.last,
            p.skill_hash.as_deref().unwrap_or("not recorded"),
        ));
        if let (Some(now), Some(then)) = (current_hash, p.skill_hash.as_deref())
            && now != then
        {
            out.push_str("- the skill has changed since these loads\n");
        }
        out.push_str("- examples:\n");
        for e in &p.examples {
            out.push_str(&format!(
                "  - `{}` ({}){}{}\n",
                e.id,
                e.loaded_at,
                e.task_id
                    .as_deref()
                    .map(|t| format!(", task `{t}`"))
                    .unwrap_or_default(),
                e.outcome_ref
                    .as_deref()
                    .map(|r| format!(", outcome from `{r}`"))
                    .unwrap_or_default(),
            ));
        }
        out.push('\n');
    }

    if !evidence.comparisons.is_empty() {
        out.push_str("### Outcomes by skill version\n\n");
        for c in &evidence.comparisons {
            out.push_str(&comparison_line(c));
            out.push('\n');
        }
        out.push('\n');
    }

    out.push_str("## Likely change\n\n");
    for p in evidence.findings() {
        out.push_str(&format!("- {}\n", likely_change(p)));
    }

    out.push_str("\n## Regression fixtures\n\n");
    match load_fixtures(skills_dir, name) {
        Ok(Some(f)) if !f.is_empty() => {
            out.push_str(&format!(
                "`{FIXTURES_FILE}` already holds {} trigger, {} non-trigger and {} required-step \
                 fixtures. Any change has to keep passing them (`hq skills validate`); a review \
                 change that fails them is held in `_proposed/` instead of applied.\n",
                f.triggers.len(),
                f.not_triggers.len(),
                f.required_steps.len()
            ));
        }
        Ok(_) => out.push_str(&format!(
            "This skill has no `{FIXTURES_FILE}`. Before changing it, add fixtures for what the \
             change is meant to fix (a `required_steps` entry for a step to keep, `triggers` and \
             `not_triggers` for the hints) so the next change cannot undo it.\n"
        )),
        Err(e) => out.push_str(&format!("The fixtures could not be read: {e}.\n")),
    }
    Some(out)
}

/// Write the proposal for `evidence` into `skills/_proposed/<name>/PROPOSAL.md`. Returns the
/// path, or `None` when the evidence is below the threshold and nothing was written. A
/// replacement `SKILL.md` already held there is left alone.
pub fn write_evidence_proposal(
    skills_dir: &Path,
    evidence: &SkillEvidence,
    current_hash: Option<&str>,
) -> Result<Option<PathBuf>> {
    validate_skill_name(&evidence.skill)?;
    let Some(text) = render_proposal(skills_dir, evidence, current_hash) else {
        return Ok(None);
    };
    if !skills_dir.join(&evidence.skill).join("SKILL.md").is_file() {
        bail!(
            "skill '{}' does not exist, so there is nothing to propose a change to",
            evidence.skill
        );
    }
    let dir = skills_dir.join("_proposed").join(&evidence.skill);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(PROPOSAL_FILE);
    std::fs::write(&path, text)?;
    Ok(Some(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_db::skill_evidence::{Example, Pattern};

    fn pattern(kind: PatternKind, invocations: u32, sessions: u32) -> Pattern {
        Pattern {
            kind,
            qualifier: String::new(),
            skill_hash: Some("abc123".into()),
            invocations,
            sessions,
            first: "2026-10-01 10:00:00".into(),
            last: "2026-10-14 10:00:00".into(),
            examples: vec![Example {
                id: "inv_1234".into(),
                loaded_at: "2026-10-14 10:00:00".into(),
                task_id: Some("tk-9".into()),
                outcome_ref: Some("task:tk-9".into()),
            }],
            enough: invocations >= MIN_INVOCATIONS && sessions >= MIN_SESSIONS,
        }
    }

    fn evidence(patterns: Vec<Pattern>) -> SkillEvidence {
        SkillEvidence {
            skill: "sheets".into(),
            window_days: 30,
            invocations: 9,
            sessions: 4,
            versions: vec![],
            comparisons: vec![],
            patterns,
        }
    }

    fn skills_with_sheets() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("sheets")).unwrap();
        std::fs::write(
            dir.path().join("sheets/SKILL.md"),
            "---\nname: sheets\n---\nbody\n",
        )
        .unwrap();
        dir
    }

    #[test]
    fn below_the_threshold_there_is_no_proposal_and_nothing_is_written() {
        let dir = skills_with_sheets();
        let e = evidence(vec![pattern(PatternKind::ReworkedOutcomes, 4, 3)]);
        assert_eq!(render_proposal(dir.path(), &e, None), None);
        assert_eq!(write_evidence_proposal(dir.path(), &e, None).unwrap(), None);
        assert!(!dir.path().join("_proposed").exists());
    }

    #[test]
    fn a_finding_becomes_a_proposal_with_counts_range_examples_change_and_fixtures() {
        let dir = skills_with_sheets();
        let e = evidence(vec![pattern(PatternKind::ReworkedOutcomes, 6, 4)]);

        let path = write_evidence_proposal(dir.path(), &e, Some("abc123"))
            .unwrap()
            .unwrap();

        assert_eq!(
            path,
            dir.path().join("_proposed/sheets").join(PROPOSAL_FILE)
        );
        let text = std::fs::read_to_string(path).unwrap();
        assert!(text.contains("6 loads across 4 sessions"), "{text}");
        assert!(
            text.contains("2026-10-01 10:00:00 to 2026-10-14 10:00:00"),
            "{text}"
        );
        assert!(
            text.contains("`inv_1234`")
                && text.contains("task `tk-9`")
                && text.contains("`task:tk-9`"),
            "{text}"
        );
        assert!(text.contains("## Likely change"), "{text}");
        assert!(text.contains("required_steps"), "{text}");
        assert!(text.contains("has no `fixtures.yaml`"), "{text}");
        assert!(text.contains("not a change"), "{text}");
        assert!(text.contains("reason to look, not proof"), "{text}");
    }

    #[test]
    fn a_proposal_cannot_rewrite_the_skill_or_be_approved_into_one() {
        let dir = skills_with_sheets();
        let before = std::fs::read_to_string(dir.path().join("sheets/SKILL.md")).unwrap();
        let e = evidence(vec![pattern(PatternKind::ReworkedOutcomes, 6, 4)]);
        write_evidence_proposal(dir.path(), &e, None).unwrap();

        assert_eq!(
            std::fs::read_to_string(dir.path().join("sheets/SKILL.md")).unwrap(),
            before
        );
        // There is no replacement text, so approving finds nothing to promote.
        let err = crate::skills::approve_proposed_skill(dir.path(), "sheets")
            .unwrap_err()
            .to_string();
        assert!(err.contains("PROPOSAL.md"), "{err}");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("sheets/SKILL.md")).unwrap(),
            before
        );
        // Rejecting dismisses it.
        crate::skills::reject_proposed_skill(dir.path(), "sheets").unwrap();
        assert!(!dir.path().join("_proposed/sheets").exists());
    }

    #[test]
    fn existing_fixtures_are_named_as_the_bar_a_change_must_clear() {
        let dir = skills_with_sheets();
        std::fs::write(
            dir.path().join("sheets").join(FIXTURES_FILE),
            "triggers:\n  - fix this spreadsheet\nrequired_steps:\n  - back up\n  - confirm\n",
        )
        .unwrap();
        let e = evidence(vec![pattern(PatternKind::LoadedButUnused, 5, 3)]);
        let text = render_proposal(dir.path(), &e, None).unwrap();
        assert!(
            text.contains("1 trigger, 0 non-trigger and 2 required-step"),
            "{text}"
        );
        assert!(text.contains("hq skills validate"), "{text}");
    }

    #[test]
    fn a_held_replacement_skill_is_left_alone() {
        let dir = skills_with_sheets();
        std::fs::create_dir_all(dir.path().join("_proposed/sheets")).unwrap();
        std::fs::write(dir.path().join("_proposed/sheets/SKILL.md"), "held text").unwrap();
        let e = evidence(vec![pattern(PatternKind::JudgedHarmful, 5, 3)]);
        write_evidence_proposal(dir.path(), &e, None).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("_proposed/sheets/SKILL.md")).unwrap(),
            "held text"
        );
    }

    #[test]
    fn a_skill_that_does_not_exist_gets_no_proposal() {
        let dir = tempfile::tempdir().unwrap();
        let e = evidence(vec![pattern(PatternKind::ReworkedOutcomes, 6, 4)]);
        assert!(write_evidence_proposal(dir.path(), &e, None).is_err());
    }

    #[test]
    fn a_skill_name_that_escapes_the_folder_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let mut e = evidence(vec![pattern(PatternKind::ReworkedOutcomes, 6, 4)]);
        e.skill = "../outside".into();
        assert!(write_evidence_proposal(dir.path(), &e, None).is_err());
    }

    #[test]
    fn a_changed_skill_is_noted_and_version_outcomes_carry_their_sample_size() {
        let dir = skills_with_sheets();
        let mut e = evidence(vec![pattern(PatternKind::ReworkedOutcomes, 6, 4)]);
        e.comparisons = vec![VersionComparison {
            before: Some("abc123".into()),
            after: Some("def456".into()),
            before_invocations: 6,
            after_invocations: 2,
            before_mean: Some(0.5),
            after_mean: Some(1.0),
            small_sample: true,
        }];
        let text = render_proposal(dir.path(), &e, Some("def456")).unwrap();
        assert!(
            text.contains("the skill has changed since these loads"),
            "{text}"
        );
        assert!(
            text.contains("0.50 (6 loads) → 1.00 (2 loads). Small sample: anecdotal."),
            "{text}"
        );
    }
}
