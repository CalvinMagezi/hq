//! Regression fixtures stored beside a skill, so a change to the skill has something to be
//! tested against.
//!
//! A skill may have `skills/<name>/fixtures.yaml`:
//!
//! ```yaml
//! # Phrases that must make this skill's hints match (any one hint is enough).
//! triggers:
//!   - fix this spreadsheet
//!   - open the budget sheet
//! # Phrases that must not match any of its hints.
//! not_triggers:
//!   - write a poem about spring
//! # Steps the body must still contain. Compared ignoring case and runs of whitespace.
//! required_steps:
//!   - back up the original file
//!   - confirm before overwriting
//! ```
//!
//! Matching uses the same rule as the real hint index (`word_boundary_match` over the lowercased
//! phrase), so a fixture cannot pass here and fail in a session. A skill without a fixtures file
//! is checked by nothing here and validates as before. Fixtures are checked by
//! `hq skills validate`, and a change from the post-session review or a generated proposal must
//! pass them before it is applied: a change that fails is held in `skills/_proposed/` for
//! `hq skills approve` or `reject`.

use std::path::Path;

use serde::Deserialize;

use super::hints::word_boundary_match;

/// The file beside `SKILL.md`.
pub const FIXTURES_FILE: &str = "fixtures.yaml";
/// Entries per list, and characters per entry. Fixtures are small on purpose.
const MAX_ENTRIES: usize = 50;
const MAX_ENTRY_CHARS: usize = 200;

#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fixtures {
    #[serde(default)]
    pub triggers: Vec<String>,
    #[serde(default)]
    pub not_triggers: Vec<String>,
    #[serde(default)]
    pub required_steps: Vec<String>,
}

impl Fixtures {
    pub fn is_empty(&self) -> bool {
        self.triggers.is_empty() && self.not_triggers.is_empty() && self.required_steps.is_empty()
    }

    /// Why these fixtures themselves are unusable, if they are.
    fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (label, list) in [
            ("triggers", &self.triggers),
            ("not_triggers", &self.not_triggers),
            ("required_steps", &self.required_steps),
        ] {
            if list.len() > MAX_ENTRIES {
                out.push(format!(
                    "{label} has {} entries (at most {MAX_ENTRIES})",
                    list.len()
                ));
            }
            for entry in list {
                if entry.trim().is_empty() {
                    out.push(format!("{label} has an empty entry"));
                } else if entry.chars().count() > MAX_ENTRY_CHARS {
                    out.push(format!(
                        "a {label} entry is over {MAX_ENTRY_CHARS} characters"
                    ));
                }
            }
        }
        out
    }
}

/// Read the fixtures beside a skill. `Ok(None)` when it has none.
pub fn load_fixtures(skills_dir: &Path, name: &str) -> Result<Option<Fixtures>, String> {
    if name.is_empty() || name.contains('/') || name.contains('\\') || name.contains("..") {
        return Ok(None);
    }
    let path = skills_dir.join(name).join(FIXTURES_FILE);
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{FIXTURES_FILE} cannot be read: {e}")),
    };
    serde_yaml::from_str::<Fixtures>(&raw)
        .map(Some)
        .map_err(|e| format!("{FIXTURES_FILE} is not valid: {e}"))
}

/// Lowercased, whitespace collapsed to single spaces.
fn squash(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// The hints and the body of a SKILL.md text, read the way the catalog reads a skill.
fn hints_and_body(text: &str) -> (Vec<String>, String) {
    let (frontmatter, body) = hq_core::frontmatter_utils::split_frontmatter(text);
    let hints = frontmatter
        .and_then(|fm| serde_yaml::from_str::<serde_yaml::Value>(fm).ok())
        .and_then(|v| v.get("hints").and_then(|h| h.as_sequence()).cloned())
        .map(|seq| {
            seq.iter()
                .filter_map(|h| h.as_str().map(str::to_lowercase))
                .collect()
        })
        .unwrap_or_default();
    (hints, body.to_string())
}

/// Every way `text`, a SKILL.md, breaks `fixtures`. Empty when it passes.
pub fn check_text(text: &str, fixtures: &Fixtures) -> Vec<String> {
    let mut failures = fixtures.problems();
    let (hints, body) = hints_and_body(text);
    let body = squash(&body);

    for phrase in &fixtures.triggers {
        let lowered = phrase.to_lowercase();
        if !hints.iter().any(|h| word_boundary_match(&lowered, h)) {
            failures.push(format!("must trigger but no hint matches: {phrase:?}"));
        }
    }
    for phrase in &fixtures.not_triggers {
        let lowered = phrase.to_lowercase();
        if let Some(hint) = hints.iter().find(|h| word_boundary_match(&lowered, h)) {
            failures.push(format!(
                "must not trigger but hint {hint:?} matches: {phrase:?}"
            ));
        }
    }
    for step in &fixtures.required_steps {
        if !body.contains(&squash(step)) {
            failures.push(format!("required step is missing from the body: {step:?}"));
        }
    }
    failures
}

/// Every way `candidate`, a proposed new SKILL.md for `name`, breaks that skill's fixtures.
/// Empty when it passes or the skill has none. An unreadable fixtures file is a failure: a
/// change should not slip through because its tests could not be read.
pub fn candidate_failures(skills_dir: &Path, name: &str, candidate: &str) -> Vec<String> {
    match load_fixtures(skills_dir, name) {
        Ok(Some(fixtures)) => check_text(candidate, &fixtures),
        Ok(None) => Vec::new(),
        Err(e) => vec![e],
    }
}

/// [`candidate_failures`] for the skill as it stands on disk.
pub fn skill_failures(skills_dir: &Path, name: &str) -> Vec<String> {
    let live = skills_dir.join(name).join("SKILL.md");
    match std::fs::read_to_string(live) {
        Ok(text) => candidate_failures(skills_dir, name, &text),
        // No SKILL.md (a bundle, or gone): nothing to test.
        Err(_) => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SKILL: &str = "---\nname: sheets\ndescription: Work with spreadsheets\nhints:\n  - spreadsheet\n  - budget sheet\nautoLoad: true\n---\n\n# Sheets\n\n1. Back up the original file.\n2. Edit the copy.\n3. Confirm before   overwriting.\n";

    fn fixtures() -> Fixtures {
        Fixtures {
            triggers: vec![
                "Please fix this spreadsheet".into(),
                "open the budget sheet".into(),
            ],
            not_triggers: vec![
                "write a poem about spring".into(),
                "spreadsheets are fun".into(),
            ],
            required_steps: vec![
                "back up the original file".into(),
                "confirm before overwriting".into(),
            ],
        }
    }

    #[test]
    fn a_skill_that_meets_its_fixtures_passes() {
        assert_eq!(check_text(SKILL, &fixtures()), Vec::<String>::new());
    }

    #[test]
    fn dropping_a_required_step_fails() {
        let patched = SKILL.replace("1. Back up the original file.\n", "");
        let failures = check_text(&patched, &fixtures());
        assert_eq!(failures.len(), 1, "{failures:?}");
        assert!(
            failures[0].contains("back up the original file"),
            "{failures:?}"
        );
    }

    #[test]
    fn steps_match_ignoring_case_and_runs_of_whitespace() {
        // "Confirm before   overwriting" in the skill matches "confirm before overwriting".
        assert!(check_text(SKILL, &fixtures()).is_empty());
        let reflowed = SKILL.replace(
            "Back up the original file.",
            "BACK UP\nthe   original file.",
        );
        assert!(check_text(&reflowed, &fixtures()).is_empty());
    }

    #[test]
    fn losing_a_hint_breaks_the_phrase_that_needed_it() {
        let patched = SKILL.replace("  - budget sheet\n", "");
        let failures = check_text(&patched, &fixtures());
        assert!(
            failures.iter().any(|f| f.contains("open the budget sheet")),
            "{failures:?}"
        );
    }

    #[test]
    fn a_new_hint_that_matches_a_forbidden_phrase_fails() {
        let patched = SKILL.replace("  - budget sheet\n", "  - budget sheet\n  - poem\n");
        let failures = check_text(&patched, &fixtures());
        assert!(
            failures
                .iter()
                .any(|f| f.contains("must not trigger") && f.contains("poem")),
            "{failures:?}"
        );
    }

    #[test]
    fn matching_follows_the_real_word_boundary_rule() {
        // "spreadsheets" does not match the hint "spreadsheet": a whole word is needed.
        let f = Fixtures {
            triggers: vec!["spreadsheets".into()],
            ..Default::default()
        };
        assert_eq!(check_text(SKILL, &f).len(), 1);
        let f = Fixtures {
            not_triggers: vec!["spreadsheets".into()],
            ..Default::default()
        };
        assert!(check_text(SKILL, &f).is_empty());
    }

    #[test]
    fn a_skill_without_fixtures_is_not_checked() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("sheets")).unwrap();
        std::fs::write(dir.path().join("sheets/SKILL.md"), SKILL).unwrap();
        assert_eq!(load_fixtures(dir.path(), "sheets"), Ok(None));
        assert!(candidate_failures(dir.path(), "sheets", "no frontmatter at all").is_empty());
        assert!(skill_failures(dir.path(), "sheets").is_empty());
    }

    #[test]
    fn fixtures_are_read_from_the_file_beside_the_skill() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("sheets")).unwrap();
        std::fs::write(dir.path().join("sheets/SKILL.md"), SKILL).unwrap();
        std::fs::write(
            dir.path().join("sheets").join(FIXTURES_FILE),
            "triggers:\n  - fix this spreadsheet\nrequired_steps:\n  - edit the copy\n",
        )
        .unwrap();
        assert!(skill_failures(dir.path(), "sheets").is_empty());

        let broken = SKILL.replace("2. Edit the copy.\n", "");
        let failures = candidate_failures(dir.path(), "sheets", &broken);
        assert_eq!(failures.len(), 1);
    }

    #[test]
    fn a_fixtures_file_that_cannot_be_read_blocks_a_change_instead_of_waving_it_through() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("sheets")).unwrap();
        std::fs::write(
            dir.path().join("sheets").join(FIXTURES_FILE),
            "triggers: [unclosed",
        )
        .unwrap();
        let failures = candidate_failures(dir.path(), "sheets", SKILL);
        assert_eq!(failures.len(), 1);
        assert!(failures[0].contains("not valid"), "{failures:?}");

        std::fs::write(
            dir.path().join("sheets").join(FIXTURES_FILE),
            "tirggers:\n  - typo\n",
        )
        .unwrap();
        assert!(
            !candidate_failures(dir.path(), "sheets", SKILL).is_empty(),
            "an unknown key is an error, not silence"
        );
    }

    #[test]
    fn unusable_fixtures_are_reported() {
        let f = Fixtures {
            triggers: vec!["  ".into()],
            required_steps: vec!["x".repeat(MAX_ENTRY_CHARS + 1)],
            ..Default::default()
        };
        let failures = check_text(SKILL, &f);
        assert!(
            failures.iter().any(|m| m.contains("empty entry")),
            "{failures:?}"
        );
        assert!(failures.iter().any(|m| m.contains("over")), "{failures:?}");
    }

    #[test]
    fn a_skill_name_that_escapes_the_skills_folder_has_no_fixtures() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load_fixtures(dir.path(), "../x"), Ok(None));
        assert_eq!(load_fixtures(dir.path(), ""), Ok(None));
    }
}
