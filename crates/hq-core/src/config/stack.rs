//! The Agentic Stack (`stack:` in the config): named general-purpose agents
//! that pull the tasks assigned to them and run each one as a single bounded
//! job in the host. The terms come from Kola Academy's Agentic Freelancing
//! programme: a *stack* of *specialists*, each with a lane, directed toward
//! outcomes a person reviews. See `docs/plans/agentic-stack.md`.

use serde::{Deserialize, Serialize};

/// Minutes a run may take before the dispatcher stops it.
pub const DEFAULT_RUN_MINUTES: u64 = 30;
pub const MIN_RUN_MINUTES: u64 = 5;
pub const MAX_RUN_MINUTES: u64 = 240;

/// Runs one specialist may start in a rolling day.
pub const DEFAULT_RUNS_PER_DAY: u64 = 24;
pub const MAX_RUNS_PER_DAY: u64 = 500;

/// Longest a specialist's name may be: it becomes a session label and a lease actor.
pub const MAX_NAME_LEN: usize = 32;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StackConfig {
    #[serde(default)]
    pub specialists: Vec<Specialist>,
}

/// One agent in the stack. Tasks assigned to `name` are its queue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Specialist {
    /// The assignee name tasks carry, and the name in `hq stack` and the
    /// session label (`stack/<name>`). Lowercase letters, digits, `-` and `_`.
    pub name: String,

    /// A built-in general-purpose harness (`hermes`, `openclaw`) or a profile
    /// built on one. A coding harness is refused: the stack is for work that
    /// never touches code.
    pub harness: String,

    /// The purposes this specialist takes, first is the default for a task that
    /// hints at none: `research`, `review`, `monitor`, `general`. Empty means
    /// every purpose its category allows, with `general` as the default.
    #[serde(default)]
    pub purposes: Vec<String>,

    /// Where runs start. Unset: a `stack-<name>` folder in the host's HQ folder
    /// (`~/Documents/HQ` on that machine), made when missing. Must pass the same
    /// deny and allow lists as any other session folder.
    #[serde(default)]
    pub cwd: Option<String>,

    /// Host to run on (`native`, `local` or a paired host). Unset: the default host.
    #[serde(default)]
    pub host: Option<String>,

    /// Minutes before a run is stopped. Clamped to 5..=240.
    #[serde(default = "default_run_minutes")]
    pub run_minutes: u64,

    /// Runs this specialist may start in a rolling 24 hours. Clamped to 1..=500.
    #[serde(default = "default_runs_per_day")]
    pub runs_per_day: u64,

    /// `false` keeps the entry but starts nothing.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_run_minutes() -> u64 {
    DEFAULT_RUN_MINUTES
}

fn default_runs_per_day() -> u64 {
    DEFAULT_RUNS_PER_DAY
}

fn default_true() -> bool {
    true
}

impl Specialist {
    pub fn run_minutes(&self) -> u64 {
        self.run_minutes.clamp(MIN_RUN_MINUTES, MAX_RUN_MINUTES)
    }

    pub fn runs_per_day(&self) -> u64 {
        self.runs_per_day.clamp(1, MAX_RUNS_PER_DAY)
    }

    /// The session label every run of this specialist carries.
    pub fn label(&self) -> String {
        format!("stack/{}", self.name)
    }

    /// Whether `name` can serve as an assignee, a label and a lease actor.
    pub fn valid_name(name: &str) -> bool {
        let mut chars = name.chars();
        let first_ok = chars.next().is_some_and(|c| c.is_ascii_lowercase());
        first_ok
            && name.len() <= MAX_NAME_LEN
            && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
    }
}

impl StackConfig {
    /// What is wrong with the block, as one line per problem. Harness and
    /// purpose names are checked where the harness specs live (`hq-tools`).
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for s in &self.specialists {
            if !Specialist::valid_name(&s.name) {
                out.push(format!(
                    "stack: specialist name '{}' must be 1 to {MAX_NAME_LEN} lowercase letters, digits, '-' or '_' and start with a letter",
                    s.name
                ));
            }
            if !seen.insert(s.name.clone()) {
                out.push(format!("stack: specialist '{}' is listed twice", s.name));
            }
            if s.harness.trim().is_empty() {
                out.push(format!("stack: specialist '{}' names no harness", s.name));
            }
        }
        out
    }

    pub fn get(&self, name: &str) -> Option<&Specialist> {
        self.specialists.iter().find(|s| s.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_block_is_an_empty_stack() {
        let cfg: StackConfig = serde_yaml::from_str("{}").unwrap();
        assert!(cfg.specialists.is_empty());
        assert!(cfg.problems().is_empty());
    }

    #[test]
    fn a_specialist_needs_only_a_name_and_a_harness() {
        let cfg: StackConfig =
            serde_yaml::from_str("specialists:\n  - name: hermes\n    harness: hermes\n").unwrap();
        let s = &cfg.specialists[0];
        assert_eq!(s.run_minutes(), DEFAULT_RUN_MINUTES);
        assert_eq!(s.runs_per_day(), DEFAULT_RUNS_PER_DAY);
        assert!(s.enabled && s.purposes.is_empty() && s.cwd.is_none());
        assert_eq!(s.label(), "stack/hermes");
        assert!(cfg.problems().is_empty());
    }

    #[test]
    fn limits_are_clamped_and_names_checked() {
        let cfg: StackConfig = serde_yaml::from_str(
            "specialists:\n  - name: Hermes!\n    harness: hermes\n    run_minutes: 1\n    runs_per_day: 0\n  - name: ok\n    harness: ''\n  - name: ok\n    harness: openclaw\n",
        )
        .unwrap();
        assert_eq!(cfg.specialists[0].run_minutes(), MIN_RUN_MINUTES);
        assert_eq!(cfg.specialists[0].runs_per_day(), 1);
        let problems = cfg.problems();
        assert_eq!(problems.len(), 3, "{problems:?}");
        assert!(problems[0].contains("'Hermes!'"));
        assert!(problems[1].contains("names no harness"));
        assert!(problems[2].contains("listed twice"));
    }
}
