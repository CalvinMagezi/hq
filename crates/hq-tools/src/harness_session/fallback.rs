//! Falling back to another lane when a start fails because the lane is unavailable.
//!
//! A profile lists `fallback` lanes. A start that fails for a reason a different lane would not
//! share (a login that ran out, a quota or rate limit, the agent cap) is tried again on the next
//! lane, and the reply says which lane failed and why. Any other failure (a bad folder, a refused
//! policy, a crash) stops the chain: another lane would fail the same way.

use hq_core::config::HqConfig;

/// Words in a start error that mean the lane, not the request, is the problem.
const UNAVAILABLE_MARKERS: &[&str] = &[
    "authentication",
    "not logged in",
    "log in",
    "login",
    "subscription",
    "quota",
    "billing",
    "rate limit",
    "credit",
    "too many agents",
    "429",
];

/// Most lanes one start may fall through, so a long or looping list cannot turn into many starts.
pub(super) const MAX_FALLBACKS: usize = 4;

/// Whether `error` says the lane is unavailable rather than the request being wrong.
pub(super) fn lane_unavailable(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    UNAVAILABLE_MARKERS.iter().any(|m| error.contains(m))
}

/// The lanes to try after `lane`, in order, without repeats of it or of each other.
pub(super) fn fallbacks_of(cfg: &HqConfig, lane: &str) -> Vec<String> {
    let Some(profile) = cfg.agent_host.harness_profiles.get(lane) else {
        return Vec::new();
    };
    let mut seen = vec![lane.to_string()];
    for next in &profile.fallback {
        if !seen.contains(next) {
            seen.push(next.clone());
        }
    }
    seen.into_iter().skip(1).take(MAX_FALLBACKS).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_core::config::HarnessProfileConfig;

    fn with_fallback(lane: &str, fallback: &[&str]) -> HqConfig {
        let mut cfg = HqConfig::default();
        cfg.agent_host.harness_profiles.insert(
            lane.to_string(),
            HarnessProfileConfig {
                base: "opencode".into(),
                fallback: fallback.iter().map(|s| s.to_string()).collect(),
                ..Default::default()
            },
        );
        cfg
    }

    #[test]
    fn only_a_lane_problem_is_worth_another_lane() {
        assert!(lane_unavailable("Authentication required"));
        assert!(lane_unavailable("hit the rate limit for this plan"));
        assert!(lane_unavailable("host: too many agents (128)"));
        assert!(!lane_unavailable("folder /x does not exist"));
        assert!(!lane_unavailable("the policy refused this start"));
    }

    #[test]
    fn the_list_drops_the_lane_itself_and_repeats_and_is_capped() {
        let cfg = with_fallback("a", &["a", "b", "b", "c", "d", "e", "f"]);
        assert_eq!(fallbacks_of(&cfg, "a"), ["b", "c", "d", "e"]);
        assert!(fallbacks_of(&cfg, "unknown").is_empty());
    }
}
