//! The one place a call's dollar cost is decided. The router ledger and the session budget both
//! price through [`price_call`], so they cannot disagree.

use crate::models::{calculate_cost_with_cache, get_model_info};

/// Providers that run on the owner's hardware: tokens are tracked, dollars are zero by design.
const LOCAL_PROVIDERS: &[&str] = &["ollama", "turboquant"];
/// Providers billed through a subscription quota rather than per token.
const FLAT_RATE_PROVIDERS: &[&str] = &["copilot"];
const LOCAL_HOSTS: &[&str] = &["localhost", "127.0.0.1", "[::1]", "0.0.0.0"];

/// How a provider bills, which decides whether a missing price is a gap or simply not applicable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderClass {
    Metered,
    Local,
    Flat,
}

impl ProviderClass {
    /// Classify by the name a router registers a provider under.
    pub fn of_name(name: &str) -> Self {
        if LOCAL_PROVIDERS.contains(&name) {
            ProviderClass::Local
        } else if FLAT_RATE_PROVIDERS.contains(&name) {
            ProviderClass::Flat
        } else {
            ProviderClass::Metered
        }
    }

    /// A subscription run still counts at list price against a session's USD cap, so a runaway
    /// loop on a flat-rate plan can trip it. The ledger records the same run as `flat`.
    pub fn for_session_budget(self) -> Self {
        match self {
            ProviderClass::Flat => ProviderClass::Metered,
            other => other,
        }
    }

    /// Classify a configured backend by its endpoint: a loopback host is local.
    pub fn of_endpoint(endpoint: Option<&str>) -> Self {
        let rest = endpoint
            .and_then(|e| e.split("://").nth(1))
            .unwrap_or_default();
        let host = match rest.strip_prefix('[') {
            Some(v6) => v6
                .split(']')
                .next()
                .map(|h| format!("[{h}]"))
                .unwrap_or_default(),
            None => rest
                .split(['/', ':'])
                .next()
                .unwrap_or_default()
                .to_string(),
        };
        let host = host.as_str();
        if LOCAL_HOSTS.contains(&host) {
            ProviderClass::Local
        } else {
            ProviderClass::Metered
        }
    }
}

/// Token counts of one completed call. `input` includes cached prompt tokens.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Usage {
    pub input: u32,
    pub output: u32,
    pub cache_read: u32,
    pub cache_write: u32,
    /// Reasoning tokens, already counted inside `output`.
    pub reasoning: u32,
    /// What the provider says it billed for the call, when it says.
    pub billed_usd: Option<f64>,
}

/// Where a recorded cost came from. Stored as text in `task_outcomes.cost_source`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CostSource {
    /// The provider's own billed figure.
    Provider,
    /// Tokens times the static price table.
    Table,
    /// No price known, so the dollar figure is not a real zero.
    Unpriced,
    /// Local inference: genuinely zero dollars.
    Free,
    /// Subscription quota: dollars are not per call.
    Flat,
    /// The call failed, so nothing was billed.
    NotBilled,
}

impl CostSource {
    pub fn as_str(self) -> &'static str {
        match self {
            CostSource::Provider => "provider",
            CostSource::Table => "table",
            CostSource::Unpriced => "unpriced",
            CostSource::Free => "free",
            CostSource::Flat => "flat",
            CostSource::NotBilled => "none",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PricedCall {
    pub usd: f64,
    pub source: CostSource,
}

/// Price one call. A provider-billed figure in `usage` wins over the price table.
pub fn price_call(class: ProviderClass, model: &str, usage: &Usage) -> PricedCall {
    if let Some(usd) = usage.billed_usd {
        return PricedCall {
            usd,
            source: CostSource::Provider,
        };
    }
    match class {
        ProviderClass::Local => {
            return PricedCall {
                usd: 0.0,
                source: CostSource::Free,
            };
        }
        ProviderClass::Flat => {
            return PricedCall {
                usd: 0.0,
                source: CostSource::Flat,
            };
        }
        ProviderClass::Metered => {}
    }
    if get_model_info(model).is_none() {
        return PricedCall {
            usd: 0.0,
            source: CostSource::Unpriced,
        };
    }
    PricedCall {
        usd: calculate_cost_with_cache(
            model,
            usage.input,
            usage.output,
            usage.cache_read,
            usage.cache_write,
        ),
        source: CostSource::Table,
    }
}

/// Price a finished call whether or not the provider reported usage. Tokens that were counted
/// are billed even when the call failed or was cancelled afterwards.
pub fn price_outcome(
    class: ProviderClass,
    model: &str,
    usage: Option<&Usage>,
    failed: bool,
) -> PricedCall {
    if let Some(u) = usage {
        return price_call(class, model, u);
    }
    let source = match class {
        ProviderClass::Local => CostSource::Free,
        ProviderClass::Flat => CostSource::Flat,
        ProviderClass::Metered if failed => CostSource::NotBilled,
        ProviderClass::Metered => CostSource::Unpriced,
    };
    PricedCall { usd: 0.0, source }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KNOWN: &str = "anthropic/claude-haiku-5.5";

    fn usage(input: u32, output: u32, cache_read: u32) -> Usage {
        Usage {
            input,
            output,
            cache_read,
            ..Usage::default()
        }
    }

    #[test]
    fn cache_reads_are_billed_cheaper_than_fresh_input() {
        let cold = price_call(ProviderClass::Metered, KNOWN, &usage(100_000, 1_000, 0));
        let warm = price_call(
            ProviderClass::Metered,
            KNOWN,
            &usage(100_000, 1_000, 90_000),
        );
        assert_eq!(cold.source, CostSource::Table);
        assert!(warm.usd < cold.usd);
    }

    #[test]
    fn an_unknown_model_is_unpriced_not_free() {
        let p = price_call(
            ProviderClass::Metered,
            "nobody/never-heard-of-it",
            &usage(10, 10, 0),
        );
        assert_eq!(p.source, CostSource::Unpriced);
    }

    #[test]
    fn local_and_subscription_providers_are_labelled_not_unpriced() {
        let u = usage(10, 10, 0);
        assert_eq!(
            price_call(ProviderClass::of_name("ollama"), "gemma4:e4b", &u).source,
            CostSource::Free
        );
        assert_eq!(
            price_call(ProviderClass::of_name("copilot"), "gpt-5.4", &u).source,
            CostSource::Flat
        );
    }

    #[test]
    fn a_subscription_run_still_counts_at_list_price_for_a_session_cap() {
        let u = usage(1_000_000, 0, 0);
        let flat = ProviderClass::Flat;
        assert_eq!(price_call(flat, KNOWN, &u).usd, 0.0);
        assert!(price_call(flat.for_session_budget(), KNOWN, &u).usd > 0.0);
    }

    #[test]
    fn a_call_with_no_usage_is_unknown_only_when_it_could_have_cost_money() {
        let sources = |class, failed| price_outcome(class, KNOWN, None, failed).source;
        assert_eq!(sources(ProviderClass::Local, false), CostSource::Free);
        assert_eq!(sources(ProviderClass::Flat, false), CostSource::Flat);
        assert_eq!(sources(ProviderClass::Metered, false), CostSource::Unpriced);
        assert_eq!(sources(ProviderClass::Metered, true), CostSource::NotBilled);
    }

    #[test]
    fn tokens_counted_before_a_failure_are_still_billed() {
        let p = price_outcome(
            ProviderClass::Metered,
            KNOWN,
            Some(&usage(1000, 10, 0)),
            true,
        );
        assert_eq!(p.source, CostSource::Table);
        assert!(p.usd > 0.0);
    }

    #[test]
    fn a_loopback_endpoint_is_local_and_a_hosted_one_is_metered() {
        assert_eq!(
            ProviderClass::of_endpoint(Some("http://localhost:11434/v1")),
            ProviderClass::Local
        );
        assert_eq!(
            ProviderClass::of_endpoint(Some("https://openrouter.ai/api/v1")),
            ProviderClass::Metered
        );
        assert_eq!(ProviderClass::of_endpoint(None), ProviderClass::Metered);
        assert_eq!(
            ProviderClass::of_endpoint(Some("http://[::1]:8080/v1")),
            ProviderClass::Local
        );
    }

    #[test]
    fn a_provider_billed_figure_wins_over_the_table() {
        let billed = Usage {
            billed_usd: Some(0.5),
            ..usage(10, 10, 0)
        };
        let p = price_call(ProviderClass::Metered, KNOWN, &billed);
        assert_eq!((p.usd, p.source), (0.5, CostSource::Provider));
    }
}
