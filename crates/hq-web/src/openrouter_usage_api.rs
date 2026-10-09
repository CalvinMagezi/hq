//! `GET /api/openrouter-usage`: spend and balance for the OpenRouter key HQ runs on.

use axum::{Json, extract::State, response::IntoResponse};
use hq_core::config::{HqConfig, openrouter_key, openrouter_primary};
use hq_llm::openrouter_usage::{OpenRouterUsage, fetch_usage};
use hq_llm::provider::LlmError;
use serde_json::{Value, json};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

use crate::WsState;
use crate::error::ApiError;

/// A request burst shares one OpenRouter read.
const LIVE_CACHE_TTL: Duration = Duration::from_secs(60);
const DEFAULT_BASE: &str = "https://openrouter.ai/api/v1";

/// What OpenRouter does not tell us, said once so the panel never fills the gap with a guess.
pub(crate) const NO_RATE_NOTE: &str = "OpenRouter reports spend for today, this week and this month and publishes no burn rate. HQ projects one from its own record of calls at /api/usage/forecast.";
const NO_KEY_NOTE: &str = "The OpenRouter key is not set, so spend cannot be read.";
const REFUSED_NOTE: &str =
    "OpenRouter refused the key when asked for usage, so no spend is shown. Chat is not affected.";

type Live = Result<OpenRouterUsage, (bool, String)>;

/// A cached reading and the inputs it answers: the endpoint and an in-process fingerprint of the
/// key (never stored or sent anywhere), so changing either drops the entry at once.
struct Cached {
    at: Instant,
    source: (String, u64),
    live: Live,
}

static LIVE_CACHE: Mutex<Option<Cached>> = Mutex::const_new(None);

fn fingerprint(base: &str, key: &str) -> (String, u64) {
    let mut h = DefaultHasher::new();
    key.hash(&mut h);
    (base.to_string(), h.finish())
}

async fn cached_live(base: &str, key: &str) -> Live {
    let source = fingerprint(base, key);
    let mut guard = LIVE_CACHE.lock().await;
    if let Some(c) = guard.as_ref()
        && c.source == source
        && c.at.elapsed() < LIVE_CACHE_TTL
    {
        return c.live.clone();
    }
    // The lock is held across the read so a burst shares one call; each request is bounded by
    // the 15 s timeout in `fetch_usage`.
    let live = fetch_usage(base, key)
        .await
        .map_err(|e| (matches!(e, LlmError::Auth { .. }), e.to_string()));
    *guard = Some(Cached {
        at: Instant::now(),
        source,
        live: live.clone(),
    });
    live
}

fn usage_json(model: Option<&str>, live: Live) -> Value {
    match live {
        Ok(u) => json!({
            "active": true, "provider": "openrouter", "model": model,
            "usage": u, "credits_left": u.credits_left(), "note": NO_RATE_NOTE,
        }),
        Err((true, _)) => json!({ "active": true, "unavailable": true, "note": REFUSED_NOTE }),
        Err((false, reason)) => json!({
            "active": true, "error": format!("Could not read OpenRouter usage: {reason}"),
        }),
    }
}

pub(crate) async fn openrouter_usage_handler(
    State(state): State<Arc<WsState>>,
) -> Result<impl IntoResponse, ApiError> {
    let config = match state.hq_config.as_deref() {
        Some(c) => c.clone(),
        None => HqConfig::load().unwrap_or_default(),
    };
    let Some(backend) = openrouter_primary(&config) else {
        return Ok(Json(json!({ "active": false })));
    };
    let Some(key) = openrouter_key(&config, backend) else {
        return Ok(Json(
            json!({ "active": true, "unavailable": true, "note": NO_KEY_NOTE }),
        ));
    };
    let base = backend
        .resolved_endpoint()
        .unwrap_or_else(|| DEFAULT_BASE.into());
    let live = cached_live(&base, &key).await;
    Ok(Json(usage_json(backend.model.as_deref(), live)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_llm::openrouter_usage::parse_usage;

    #[test]
    fn the_cache_source_changes_with_the_endpoint_or_the_key() {
        let a = fingerprint("https://a.example/v1", "k1");
        assert_eq!(a, fingerprint("https://a.example/v1", "k1"));
        assert_ne!(a, fingerprint("https://b.example/v1", "k1"));
        assert_ne!(a, fingerprint("https://a.example/v1", "k2"));
    }

    #[test]
    fn shapes_a_reading_a_refusal_and_a_failure() {
        let body = json!({"data": {"usage": 12.5, "usage_daily": 0.75, "limit": null}});
        let usage = parse_usage(&body, None).unwrap();
        let ok = usage_json(Some("m"), Ok(usage));
        assert_eq!(ok["usage"]["usage_daily"], 0.75);
        assert!(ok["usage"]["limit"].is_null() && ok["credits_left"].is_null());
        let refused = usage_json(None, Err((true, "auth error (403)".into())));
        assert_eq!(refused["unavailable"], true);
        assert!(refused.get("error").is_none());
        let failed = usage_json(None, Err((false, "boom".into())));
        assert!(failed["error"].as_str().unwrap().contains("boom"));
    }
}
