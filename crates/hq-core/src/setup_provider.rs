//! The providers a first-run setup can configure, shared by `hq onboard` and the web setup screen.

use crate::config::HqConfig;

pub const CHEAP_OPENROUTER_MODEL: &str = "openai/gpt-6-luna";
pub const CHEAP_ANTHROPIC_MODEL: &str = "anthropic/claude-haiku-5.5";
pub const CHEAP_GOOGLE_MODEL: &str = "google/gemini-2.5-flash";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SetupProvider {
    #[default]
    OpenRouter,
    Anthropic,
    Google,
}

impl SetupProvider {
    /// A fresh install starts on a low-cost model because every turn carries a large
    /// system prompt and tool schema; users opt in to premium models later.
    pub fn cheap_model(self) -> &'static str {
        match self {
            Self::OpenRouter => CHEAP_OPENROUTER_MODEL,
            Self::Anthropic => CHEAP_ANTHROPIC_MODEL,
            Self::Google => CHEAP_GOOGLE_MODEL,
        }
    }

    pub fn set_key(self, config: &mut HqConfig, key: String) {
        match self {
            Self::OpenRouter => config.openrouter_api_key = Some(key),
            Self::Anthropic => config.anthropic_api_key = Some(key),
            Self::Google => config.google_ai_api_key = Some(key),
        }
    }
}

/// Provider whose cheap model a fresh config should start on: OpenRouter when its key is
/// present or nothing else is, otherwise the single direct provider that has a key.
pub fn default_provider(openrouter: bool, anthropic: bool, google: bool) -> SetupProvider {
    match (openrouter, anthropic, google) {
        (false, true, _) => SetupProvider::Anthropic,
        (false, false, true) => SetupProvider::Google,
        _ => SetupProvider::OpenRouter,
    }
}

/// The router alias a config carries until someone picks a model.
const ROUTER_DEFAULT_MODEL: &str = "relay";

/// Which providers hold a key, in the order `default_provider` takes them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KeyedProviders {
    pub openrouter: bool,
    pub anthropic: bool,
    pub google: bool,
}

impl KeyedProviders {
    fn any(self) -> bool {
        self.openrouter || self.anthropic || self.google
    }

    fn has(self, provider: SetupProvider) -> bool {
        match provider {
            SetupProvider::OpenRouter => self.openrouter,
            SetupProvider::Anthropic => self.anthropic,
            SetupProvider::Google => self.google,
        }
    }
}

/// The provider whose cheap model `model` is, when it is one a setup writes.
fn cheap_model_provider(model: &str) -> Option<SetupProvider> {
    [
        SetupProvider::OpenRouter,
        SetupProvider::Anthropic,
        SetupProvider::Google,
    ]
    .into_iter()
    .find(|p| p.cheap_model() == model)
}

/// Move a default model nobody chose (the router alias, a local model, or a setup's cheap model
/// whose provider has no key) onto the cheap model of a provider that does. Returns the new model
/// when it changed. A model the owner picked is never touched.
pub fn follow_keys(config: &mut HqConfig, keyed: KeyedProviders) -> Option<&'static str> {
    if !keyed.any() {
        return None;
    }
    let current = config.default_model.as_str();
    let stock = current == ROUTER_DEFAULT_MODEL || current.starts_with("ollama/");
    let served = match cheap_model_provider(current) {
        Some(provider) => keyed.has(provider),
        None if stock => false,
        None => return None,
    };
    if served {
        return None;
    }
    let target = default_provider(keyed.openrouter, keyed.anthropic, keyed.google).cheap_model();
    config.default_model = target.to_string();
    Some(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openrouter_wins_unless_a_direct_key_is_the_only_one() {
        assert_eq!(
            default_provider(true, true, true),
            SetupProvider::OpenRouter
        );
        assert_eq!(
            default_provider(false, true, true),
            SetupProvider::Anthropic
        );
        assert_eq!(default_provider(false, false, true), SetupProvider::Google);
        assert_eq!(
            default_provider(false, false, false),
            SetupProvider::OpenRouter
        );
    }

    #[test]
    fn set_key_fills_only_that_providers_field() {
        let mut c = HqConfig::default();
        SetupProvider::Anthropic.set_key(&mut c, "k".into());
        assert_eq!(c.anthropic_api_key.as_deref(), Some("k"));
        assert!(c.openrouter_api_key.is_none() && c.google_ai_api_key.is_none());
    }

    fn with_model(model: &str) -> HqConfig {
        HqConfig {
            default_model: model.to_string(),
            ..HqConfig::default()
        }
    }

    const ANTHROPIC_ONLY: KeyedProviders = KeyedProviders {
        openrouter: false,
        anthropic: true,
        google: false,
    };

    #[test]
    fn a_stock_default_follows_the_only_key() {
        for stock in ["relay", "ollama/qwen3:8b", CHEAP_OPENROUTER_MODEL] {
            let mut c = with_model(stock);
            assert_eq!(
                follow_keys(&mut c, ANTHROPIC_ONLY),
                Some(CHEAP_ANTHROPIC_MODEL)
            );
            assert_eq!(c.default_model, CHEAP_ANTHROPIC_MODEL, "{stock}");
        }
    }

    #[test]
    fn a_chosen_or_served_default_is_left_alone() {
        let mut chosen = with_model("anthropic/claude-opus-5.5");
        assert_eq!(follow_keys(&mut chosen, ANTHROPIC_ONLY), None);
        let mut served = with_model(CHEAP_ANTHROPIC_MODEL);
        assert_eq!(follow_keys(&mut served, ANTHROPIC_ONLY), None);
        let mut keyless = with_model("relay");
        assert_eq!(follow_keys(&mut keyless, KeyedProviders::default()), None);
        assert_eq!(keyless.default_model, "relay");
    }
}
