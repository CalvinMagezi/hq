use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const SITE_NOTIFY_GATE: &str = "notify_gate";
pub const SITE_MEMORY_TURN: &str = "memory_turn";
pub const SITE_EMAIL_FYI: &str = "email_fyi";
/// Sharpens `task_create_from_note`'s pick of an existing Space/Folder/List for a
/// promoted vault note. Off by default: the tool's own tag/folder-name heuristic is
/// the incumbent behavior and works with no decisions config at all.
pub const SITE_TASK_PLACEMENT: &str = "task_placement";

const DEFAULT_TIMEOUT_MS: u64 = 3000;
const DEFAULT_ENDPOINT: &str = "https://openrouter.ai/api/alpha/decisions";
const DEFAULT_CREDENTIAL_ENV: &str = "OPENROUTER_API_KEY";
// Pinned to a version, not the `~typesafe/jev-latest` alias, because thresholds
// are calibrated per model version. OpenRouter rejects `~typesafe/jev-1.13`.
const DEFAULT_MODEL: &str = "typesafe/jev-1.13";

/// What a decision site does with the model's answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum DecisionMode {
    /// Never call the model for this site.
    #[default]
    Off,
    /// Call it in the background and log beside the incumbent decision.
    Shadow,
    /// Act on the answer. Any error still falls back to the incumbent path.
    Enforce,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionSite {
    #[serde(default)]
    pub mode: DecisionMode,
    /// Overrides the consumer's built-in threshold when set.
    #[serde(default)]
    pub threshold: Option<f64>,
}

/// One HTTP route to a structured-decision endpoint. Routes are tried in order,
/// so a moved URL or a second provider is a config edit, not a code change.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionRoute {
    /// Full URL, not a base: the OpenRouter and TypeSafe routes differ in path.
    pub endpoint: String,
    /// Model id exactly as this route expects it (ids differ between routes).
    pub model: String,
    /// Name of the env var holding the API key. Read at construction only.
    pub credential_env: String,
}

impl Default for DecisionRoute {
    fn default() -> Self {
        Self {
            endpoint: DEFAULT_ENDPOINT.to_string(),
            model: DEFAULT_MODEL.to_string(),
            credential_env: DEFAULT_CREDENTIAL_ENV.to_string(),
        }
    }
}

/// Fast structured-decision model (a "System 1" classifier such as TypeSafe's
/// Jev) used to gate work before an expensive generative call. Off by default.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionsConfig {
    #[serde(default)]
    pub enabled: bool,

    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,

    #[serde(default = "default_routes")]
    pub routes: Vec<DecisionRoute>,

    /// Per-consumer overrides, keyed by site name. A site that is absent uses
    /// its built-in default mode, so new sites need no edit to an existing config.
    #[serde(default)]
    pub sites: BTreeMap<String, DecisionSite>,
}

fn default_timeout_ms() -> u64 {
    DEFAULT_TIMEOUT_MS
}

fn default_routes() -> Vec<DecisionRoute> {
    vec![DecisionRoute::default()]
}

/// Mode a site runs in when the config does not mention it.
fn builtin_mode(site: &str) -> DecisionMode {
    match site {
        SITE_MEMORY_TURN | SITE_EMAIL_FYI => DecisionMode::Enforce,
        SITE_NOTIFY_GATE => DecisionMode::Shadow,
        _ => DecisionMode::Off,
    }
}

impl Default for DecisionsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            timeout_ms: default_timeout_ms(),
            routes: default_routes(),
            sites: BTreeMap::new(),
        }
    }
}

impl DecisionsConfig {
    pub fn site_mode(&self, site: &str) -> DecisionMode {
        self.sites
            .get(site)
            .map(|s| s.mode)
            .unwrap_or_else(|| builtin_mode(site))
    }

    pub fn site_threshold(&self, site: &str, builtin: f64) -> f64 {
        self.sites
            .get(site)
            .and_then(|s| s.threshold)
            .unwrap_or(builtin)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_by_default_with_documented_site_modes() {
        let cfg = DecisionsConfig::default();
        assert!(!cfg.enabled);
        assert_eq!(cfg.site_mode(SITE_MEMORY_TURN), DecisionMode::Enforce);
        assert_eq!(cfg.site_mode(SITE_NOTIFY_GATE), DecisionMode::Shadow);
        assert_eq!(cfg.site_mode(SITE_EMAIL_FYI), DecisionMode::Enforce);
        assert_eq!(cfg.site_mode("unknown-site"), DecisionMode::Off);
    }

    #[test]
    fn partial_yaml_keeps_defaults_and_overrides_threshold() {
        let cfg: DecisionsConfig = serde_yaml::from_str(
            "enabled: true\nsites:\n  notify_gate: { mode: enforce, threshold: 0.9 }\n",
        )
        .unwrap();
        assert!(cfg.enabled);
        assert_eq!(cfg.routes.len(), 1);
        assert_eq!(cfg.site_mode(SITE_NOTIFY_GATE), DecisionMode::Enforce);
        assert_eq!(cfg.site_threshold(SITE_NOTIFY_GATE, 0.8), 0.9);
        assert_eq!(cfg.site_threshold(SITE_MEMORY_TURN, 0.15), 0.15);
        // A config that lists only some sites still gets built-in modes for the rest.
        assert_eq!(cfg.site_mode(SITE_EMAIL_FYI), DecisionMode::Enforce);
    }
}
