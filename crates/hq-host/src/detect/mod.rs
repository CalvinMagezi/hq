//! Agent state detection: which of idle, working, blocked or unknown an agent
//! is in, judged from its screen and terminal title by per-agent rule files.
//!
//! The rule file format follows the one documented by herdr (see
//! docs/provenance/herdr.md); the engine here is HQ's own. Each rule names a
//! region of the screen and a gate of phrases or patterns. The matching rule
//! with the highest priority decides (the earlier rule wins a tie). An agent
//! with a rule file and no matching rule is idle.

mod region;

use regex::Regex;
pub use region::Input;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// The newest rule-file features this engine understands.
pub const ENGINE_VERSION: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    Idle,
    Working,
    Blocked,
    Unknown,
}

impl AgentState {
    pub fn as_str(self) -> &'static str {
        match self {
            AgentState::Idle => "idle",
            AgentState::Working => "working",
            AgentState::Blocked => "blocked",
            AgentState::Unknown => "unknown",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "idle" => Some(AgentState::Idle),
            "working" => Some(AgentState::Working),
            "blocked" => Some(AgentState::Blocked),
            "unknown" => Some(AgentState::Unknown),
            _ => None,
        }
    }
}

/// The result of looking at one screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detection {
    pub state: AgentState,
    /// The rule that decided, or `None` when nothing matched and the default applied.
    pub rule: Option<String>,
    pub priority: i32,
    /// The rule asks callers to keep the previous state (a transcript viewer, say).
    pub skip_state_update: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawManifest {
    id: String,
    // Informational fields of the rule-file format.
    #[allow(dead_code)]
    version: Option<String>,
    min_engine_version: Option<u32>,
    #[allow(dead_code)]
    updated_at: Option<String>,
    #[serde(default)]
    aliases: Vec<String>,
    #[serde(default)]
    rules: Vec<RawRule>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRule {
    id: String,
    state: Option<AgentState>,
    #[serde(default)]
    priority: i32,
    #[serde(default = "default_region")]
    region: String,
    // Hints for UIs; detection does not use them.
    #[serde(default)]
    #[allow(dead_code)]
    visible_idle: bool,
    #[serde(default)]
    #[allow(dead_code)]
    visible_blocker: bool,
    #[serde(default)]
    #[allow(dead_code)]
    visible_working: bool,
    #[serde(default)]
    skip_state_update: bool,
    #[serde(flatten)]
    gate: RawGate,
}

fn default_region() -> String {
    "whole_recent".to_string()
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawGate {
    #[serde(default)]
    all: Vec<RawGate>,
    #[serde(default)]
    any: Vec<RawGate>,
    #[serde(default, rename = "not")]
    none: Vec<RawGate>,
    #[serde(default)]
    contains: Vec<String>,
    #[serde(default)]
    regex: Vec<String>,
    #[serde(default)]
    line_regex: Vec<String>,
}

struct Gate {
    all: Vec<Gate>,
    any: Vec<Gate>,
    none: Vec<Gate>,
    /// Lowercased: phrases match without regard to case.
    contains: Vec<String>,
    regex: Vec<Regex>,
    line_regex: Vec<Regex>,
}

struct Rule {
    id: String,
    state: AgentState,
    priority: i32,
    region: String,
    skip_state_update: bool,
    gate: Gate,
}

struct Compiled {
    id: String,
    rules: Vec<Rule>,
}

fn compile_gate(raw: &RawGate) -> Result<Gate, String> {
    let nested = |gates: &[RawGate]| {
        gates
            .iter()
            .map(compile_gate)
            .collect::<Result<Vec<_>, _>>()
    };
    let patterns = |list: &[String]| {
        list.iter()
            .map(|p| Regex::new(p).map_err(|e| format!("pattern {p:?}: {e}")))
            .collect::<Result<Vec<_>, _>>()
    };
    Ok(Gate {
        all: nested(&raw.all)?,
        any: nested(&raw.any)?,
        none: nested(&raw.none)?,
        contains: raw.contains.iter().map(|c| c.to_lowercase()).collect(),
        regex: patterns(&raw.regex)?,
        line_regex: patterns(&raw.line_regex)?,
    })
}

fn compile(raw: RawManifest) -> Result<(Compiled, Vec<String>), String> {
    if let Some(min) = raw.min_engine_version
        && min > ENGINE_VERSION
    {
        return Err(format!(
            "{} needs engine version {min}, this engine is {ENGINE_VERSION}",
            raw.id
        ));
    }
    let rules = raw
        .rules
        .iter()
        .map(|r| {
            Ok(Rule {
                id: r.id.clone(),
                state: r.state.unwrap_or(AgentState::Unknown),
                priority: r.priority,
                region: r.region.clone(),
                skip_state_update: r.skip_state_update,
                gate: compile_gate(&r.gate).map_err(|e| format!("rule {}: {e}", r.id))?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok((Compiled { id: raw.id, rules }, raw.aliases))
}

fn gate_matches(gate: &Gate, text: &str, lower: &str) -> bool {
    gate.contains.iter().all(|c| lower.contains(c.as_str()))
        && gate.regex.iter().all(|r| r.is_match(text))
        && gate
            .line_regex
            .iter()
            .all(|r| text.lines().any(|l| r.is_match(l)))
        && gate.all.iter().all(|g| gate_matches(g, text, lower))
        && (gate.any.is_empty() || gate.any.iter().any(|g| gate_matches(g, text, lower)))
        && !gate.none.iter().any(|g| gate_matches(g, text, lower))
}

/// The rule files, by agent.
pub struct Detector {
    by_id: HashMap<String, Compiled>,
    aliases: HashMap<String, String>,
}

const BUILTIN: &[&str] = &[
    include_str!("manifests/claude.toml"),
    include_str!("manifests/codex.toml"),
];

impl Detector {
    /// The rule files shipped in the binary.
    pub fn builtin() -> Self {
        let mut detector = Self {
            by_id: HashMap::new(),
            aliases: HashMap::new(),
        };
        for text in BUILTIN {
            detector.add(text).expect("built-in rule files are valid");
        }
        detector
    }

    /// Parses and adds (or replaces) one rule file; returns its id.
    pub fn add(&mut self, text: &str) -> Result<String, String> {
        let raw: RawManifest = toml::from_str(text).map_err(|e| e.to_string())?;
        let (compiled, aliases) = compile(raw)?;
        let id = compiled.id.clone();
        for alias in aliases {
            self.aliases.insert(alias, id.clone());
        }
        self.by_id.insert(id.clone(), compiled);
        Ok(id)
    }

    /// Adds every `*.toml` in `dir`, replacing built-ins of the same id. Files
    /// that do not parse are skipped and reported, never fatal.
    pub fn load_overrides(&mut self, dir: &Path) -> Vec<String> {
        let mut problems = Vec::new();
        let Ok(entries) = std::fs::read_dir(dir) else {
            return problems;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "toml") {
                continue;
            }
            let result = std::fs::read_to_string(&path)
                .map_err(|e| e.to_string())
                .and_then(|text| self.add(&text));
            if let Err(e) = result {
                problems.push(format!("{}: {e}", path.display()));
            }
        }
        problems
    }

    fn find(&self, agent: &str) -> Option<&Compiled> {
        let id = self.aliases.get(agent).map_or(agent, String::as_str);
        self.by_id.get(id)
    }

    pub fn knows(&self, agent: &str) -> bool {
        self.find(agent).is_some()
    }

    /// The state of `agent` given its screen, or `None` when there is no rule
    /// file for it.
    pub fn detect(&self, agent: &str, input: Input<'_>) -> Option<Detection> {
        let compiled = self.find(agent)?;
        let mut best: Option<&Rule> = None;
        for rule in &compiled.rules {
            let text = region::region(input, &rule.region);
            if !gate_matches(&rule.gate, text, &text.to_lowercase()) {
                continue;
            }
            if best.is_none_or(|b| rule.priority > b.priority) {
                best = Some(rule);
            }
        }
        Some(match best {
            Some(rule) => Detection {
                state: rule.state,
                rule: Some(rule.id.clone()),
                priority: rule.priority,
                skip_state_update: rule.skip_state_update,
            },
            None => Detection {
                state: AgentState::Idle,
                rule: None,
                priority: 0,
                skip_state_update: false,
            },
        })
    }
}

#[cfg(test)]
mod tests;
