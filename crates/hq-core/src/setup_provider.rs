//! The providers a first-run setup can configure, shared by `hq onboard` and the web setup screen.

use crate::config::HqConfig;

pub const CHEAP_OPENROUTER_MODEL: &str = "openai/gpt-6-luna";
pub const CHEAP_ANTHROPIC_MODEL: &str = "anthropic/claude-haiku-5.5";
pub const CHEAP_GOOGLE_MODEL: &str = "google/gemini-2.5-flash";

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SetupProvider {
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
}
