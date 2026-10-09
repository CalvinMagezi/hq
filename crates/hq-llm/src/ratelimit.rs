//! The remaining quota a provider reports in the headers of an ordinary completion. It costs no extra
//! request and no admin key, and it is the only live quota signal OpenAI, Groq and Anthropic give on
//! the inference key. The latest reading per provider host is kept in memory.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use reqwest::header::HeaderMap;
use serde::Serialize;

/// What one response said about the quota. Reset values are kept as the provider sent them: OpenAI and
/// Groq send durations ("6m0s"), Anthropic sends an RFC 3339 time.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct RateLimitReading {
    pub requests_limit: Option<u64>,
    pub requests_remaining: Option<u64>,
    pub requests_reset: Option<String>,
    pub tokens_limit: Option<u64>,
    pub tokens_remaining: Option<u64>,
    pub tokens_reset: Option<String>,
    /// Unix seconds when the response arrived.
    pub captured_at: u64,
}

impl RateLimitReading {
    fn is_empty(&self) -> bool {
        self.requests_limit.is_none()
            && self.requests_remaining.is_none()
            && self.tokens_limit.is_none()
            && self.tokens_remaining.is_none()
    }
}

fn text(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().chars().take(64).collect::<String>())
        .filter(|s| !s.is_empty())
}

fn number(headers: &HeaderMap, name: &str) -> Option<u64> {
    text(headers, name)?.parse().ok()
}

/// Read a reading from the headers of a response, or `None` when the provider sent none. Both the
/// `x-ratelimit-*` family (OpenAI, Groq) and `anthropic-ratelimit-*` are understood.
pub fn parse(headers: &HeaderMap, now_secs: u64) -> Option<RateLimitReading> {
    let reading = RateLimitReading {
        requests_limit: number(headers, "x-ratelimit-limit-requests")
            .or_else(|| number(headers, "anthropic-ratelimit-requests-limit")),
        requests_remaining: number(headers, "x-ratelimit-remaining-requests")
            .or_else(|| number(headers, "anthropic-ratelimit-requests-remaining")),
        requests_reset: text(headers, "x-ratelimit-reset-requests")
            .or_else(|| text(headers, "anthropic-ratelimit-requests-reset")),
        tokens_limit: number(headers, "x-ratelimit-limit-tokens")
            .or_else(|| number(headers, "anthropic-ratelimit-tokens-limit")),
        tokens_remaining: number(headers, "x-ratelimit-remaining-tokens")
            .or_else(|| number(headers, "anthropic-ratelimit-tokens-remaining")),
        tokens_reset: text(headers, "x-ratelimit-reset-tokens")
            .or_else(|| text(headers, "anthropic-ratelimit-tokens-reset")),
        captured_at: now_secs,
    };
    (!reading.is_empty()).then_some(reading)
}

static LATEST: LazyLock<Mutex<HashMap<String, RateLimitReading>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn host_of(base: &str) -> String {
    base.split("://")
        .nth(1)
        .and_then(|rest| rest.split(['/', ':']).next())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

/// Remember what a response from `api_base` said about its quota, if anything.
pub fn observe(api_base: &str, headers: &HeaderMap) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    if let Some(reading) = parse(headers, now) {
        LATEST
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(host_of(api_base), reading);
    }
}

/// The latest reading from the provider at `endpoint`, if a response has carried one.
pub fn latest_for(endpoint: &str) -> Option<RateLimitReading> {
    LATEST
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&host_of(endpoint))
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::{HeaderName, HeaderValue};

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        pairs
            .iter()
            .map(|(k, v)| (HeaderName::from_bytes(k.as_bytes()).unwrap(), HeaderValue::from_str(v).unwrap()))
            .collect()
    }

    #[test]
    fn openai_and_groq_headers_are_read() {
        let h = headers(&[
            ("x-ratelimit-limit-requests", "500"),
            ("x-ratelimit-remaining-requests", "499"),
            ("x-ratelimit-reset-requests", "120ms"),
            ("x-ratelimit-limit-tokens", "30000"),
            ("x-ratelimit-remaining-tokens", "29500"),
            ("x-ratelimit-reset-tokens", "1s"),
        ]);
        let r = parse(&h, 7).unwrap();
        assert_eq!((r.requests_remaining, r.tokens_remaining, r.captured_at), (Some(499), Some(29500), 7));
        assert_eq!(r.requests_reset.as_deref(), Some("120ms"));
    }

    #[test]
    fn anthropic_headers_are_read() {
        let h = headers(&[
            ("anthropic-ratelimit-requests-limit", "50"),
            ("anthropic-ratelimit-requests-remaining", "49"),
            ("anthropic-ratelimit-tokens-remaining", "39000"),
            ("anthropic-ratelimit-tokens-reset", "2026-10-09T05:00:00Z"),
        ]);
        let r = parse(&h, 0).unwrap();
        assert_eq!((r.requests_limit, r.tokens_remaining), (Some(50), Some(39000)));
        assert_eq!(r.tokens_reset.as_deref(), Some("2026-10-09T05:00:00Z"));
    }

    #[test]
    fn no_headers_or_junk_numbers_give_no_reading() {
        assert!(parse(&HeaderMap::new(), 0).is_none());
        assert!(parse(&headers(&[("x-ratelimit-remaining-requests", "soon")]), 0).is_none());
    }

    #[test]
    fn the_latest_reading_is_kept_per_host_and_found_from_an_endpoint() {
        let h = headers(&[("x-ratelimit-remaining-requests", "7")]);
        observe("https://api.groq.test/openai/v1", &h);
        assert_eq!(latest_for("https://api.groq.test/other/path").unwrap().requests_remaining, Some(7));
        assert!(latest_for("https://elsewhere.test/v1").is_none());
    }
}
