//! Two kinds of agent run in the host, and HQ hands them different work.
//!
//! A **coding agent** (Claude Code, Codex, Cursor, ...) edits repositories. A
//! **general-purpose agent** (Hermes, OpenClaw, ...) is a full
//! assistant with its own tool loop, skills and memory; HQ never hands it code
//! or repository changes, but it can take watching a coding session, a round of
//! parallel research, or a peer review. The split is kept on the harness spec,
//! and every session records what it was started for as its `purpose`, so the
//! refusal is mechanical and not a matter of prompt wording.

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HarnessCategory {
    /// Edits code in a project directory.
    Coding,
    /// A general assistant: research, review and monitoring, never code work.
    GeneralPurpose,
}

impl HarnessCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            HarnessCategory::Coding => "coding",
            HarnessCategory::GeneralPurpose => "general_purpose",
        }
    }

    /// What a session of this category is for when the caller names nothing.
    pub fn default_purpose(self) -> Purpose {
        match self {
            HarnessCategory::Coding => Purpose::Code,
            HarnessCategory::GeneralPurpose => Purpose::General,
        }
    }

    /// Whether a harness of this category may be started for `purpose`. A coding
    /// agent takes any job; a general-purpose agent takes everything but code.
    pub fn accepts(self, purpose: Purpose) -> bool {
        !(self == HarnessCategory::GeneralPurpose && purpose == Purpose::Code)
    }

    /// `accepts` as an error that tells the caller where the work goes instead.
    pub fn check(self, harness: &str, purpose: Purpose) -> Result<()> {
        if self.accepts(purpose) {
            return Ok(());
        }
        bail!(
            "'{harness}' is a general-purpose agent, and code or repository changes never go to one. \
             Start a coding agent for that work ({}). A general-purpose agent takes purpose {}.",
            coding_names().join(", "),
            Purpose::GENERAL_PURPOSE
                .iter()
                .map(|p| p.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

fn coding_names() -> Vec<&'static str> {
    super::spec::names_in(HarnessCategory::Coding)
}

/// What a session was started to do. Stored on the session, reported in every
/// view of it, and used to decide what its lifecycle means for its task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Purpose {
    /// Code or repository changes.
    Code,
    /// Watching other sessions and reporting on them.
    Monitor,
    /// Reading, searching and summarising.
    Research,
    /// Peer review of another agent's work.
    Review,
    /// Anything else a general assistant does.
    General,
}

impl Purpose {
    pub const ALL: [Purpose; 5] = [
        Purpose::Code,
        Purpose::Monitor,
        Purpose::Research,
        Purpose::Review,
        Purpose::General,
    ];

    /// The purposes a general-purpose agent may be started for.
    pub const GENERAL_PURPOSE: [Purpose; 4] = [
        Purpose::Monitor,
        Purpose::Research,
        Purpose::Review,
        Purpose::General,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Purpose::Code => "code",
            Purpose::Monitor => "monitor",
            Purpose::Research => "research",
            Purpose::Review => "review",
            Purpose::General => "general",
        }
    }

    pub fn parse(text: &str) -> Result<Purpose> {
        let wanted = text.trim().to_ascii_lowercase();
        match Self::ALL.iter().find(|p| p.as_str() == wanted) {
            Some(p) => Ok(*p),
            None => bail!(
                "unknown purpose '{text}' (one of: {})",
                Self::ALL
                    .iter()
                    .map(|p| p.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }

    /// Parse an optional argument: blank means "the harness's default".
    pub fn parse_opt(text: Option<&str>) -> Result<Option<Purpose>> {
        text.map(str::trim)
            .filter(|t| !t.is_empty())
            .map(Purpose::parse)
            .transpose()
    }

    /// Whether a session with this purpose does its task's own work, so its
    /// lifecycle moves the task (launched: in progress, finished: ready for
    /// review, exited: blocked). A monitor or reviewer is about someone else's
    /// work: its events are recorded as comments and never move the task.
    pub fn does_the_tasks_work(self) -> bool {
        !matches!(self, Purpose::Monitor | Purpose::Review)
    }

    /// Whether a session with this purpose may be given other sessions to
    /// observe: their finished turns, blocks and exits are relayed to it.
    pub fn may_observe(self) -> bool {
        matches!(self, Purpose::Monitor | Purpose::Review)
    }
}

/// Which purpose a new session has: the one asked for, else the harness's
/// default, checked against what the harness may be used for.
pub fn purpose_for(harness: &super::Harness, requested: Option<Purpose>) -> Result<Purpose> {
    let category = harness.spec.category;
    let purpose = requested.unwrap_or_else(|| category.default_purpose());
    category.check(&harness.name, purpose)?;
    Ok(purpose)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_general_purpose_agent_refuses_code_and_takes_the_rest() {
        let gp = HarnessCategory::GeneralPurpose;
        assert!(!gp.accepts(Purpose::Code));
        for p in Purpose::GENERAL_PURPOSE {
            assert!(gp.accepts(p), "{p:?}");
        }
        for p in Purpose::ALL {
            assert!(HarnessCategory::Coding.accepts(p), "{p:?}");
        }
    }

    #[test]
    fn the_refusal_names_where_code_goes() {
        let err = HarnessCategory::GeneralPurpose
            .check("hermes", Purpose::Code)
            .unwrap_err()
            .to_string();
        assert!(err.contains("claude-code"), "{err}");
        assert!(err.contains("monitor, research, review, general"), "{err}");
        assert!(
            !err.contains("hermes,"),
            "a general-purpose agent is not offered for code: {err}"
        );
    }

    #[test]
    fn purposes_round_trip_and_blank_means_default() {
        for p in Purpose::ALL {
            assert_eq!(Purpose::parse(p.as_str()).unwrap(), p);
        }
        assert_eq!(Purpose::parse(" Review ").unwrap(), Purpose::Review);
        assert!(
            Purpose::parse("coding")
                .unwrap_err()
                .to_string()
                .contains("one of")
        );
        assert_eq!(Purpose::parse_opt(None).unwrap(), None);
        assert_eq!(Purpose::parse_opt(Some("  ")).unwrap(), None);
        assert_eq!(
            Purpose::parse_opt(Some("monitor")).unwrap(),
            Some(Purpose::Monitor)
        );
    }

    #[test]
    fn only_monitors_and_reviewers_observe_and_they_never_move_a_task() {
        for p in Purpose::ALL {
            assert_eq!(p.may_observe(), !p.does_the_tasks_work(), "{p:?}");
        }
        assert!(Purpose::Research.does_the_tasks_work());
        assert!(Purpose::Monitor.may_observe());
    }

    #[test]
    fn purpose_for_defaults_by_category_and_checks() {
        let cfg = hq_core::config::AgentHostConfig::default();
        let claude = super::super::resolve_in(&cfg, "claude-code").unwrap();
        let hermes = super::super::resolve_in(&cfg, "hermes").unwrap();
        assert_eq!(purpose_for(&claude, None).unwrap(), Purpose::Code);
        assert_eq!(purpose_for(&hermes, None).unwrap(), Purpose::General);
        assert_eq!(
            purpose_for(&claude, Some(Purpose::Review)).unwrap(),
            Purpose::Review
        );
        assert_eq!(
            purpose_for(&hermes, Some(Purpose::Monitor)).unwrap(),
            Purpose::Monitor
        );
        assert!(purpose_for(&hermes, Some(Purpose::Code)).is_err());
    }

    #[test]
    fn a_profile_on_a_general_purpose_base_keeps_the_refusal() {
        let cfg: hq_core::config::AgentHostConfig = serde_yaml::from_str(
            "harness_profiles:\n  my-hermes:\n    base: hermes\n    command: hermes-wrapped\n",
        )
        .unwrap();
        let h = super::super::resolve_in(&cfg, "my-hermes").unwrap();
        let err = purpose_for(&h, Some(Purpose::Code))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("'my-hermes' is a general-purpose agent"),
            "{err}"
        );
    }
}
