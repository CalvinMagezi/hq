//! `GET /api/usage/providers`: what each configured backend's provider says about its spend and
//! balance, next to what HQ's own ledger recorded for it. Every row says where a number came from
//! (`provider` or `ledger`), so an estimate is never shown as a provider fact.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::{Json, extract::State, response::IntoResponse};
use chrono::{DateTime, Utc};
use futures::future::join_all;
use hq_core::config::{BackendEntry, HqConfig};
use hq_db::usage_ledger::{LedgerWindows, ledger_windows};
use hq_llm::backend_chain::{ConfigCredentials, resolve_credential};
use hq_llm::provider::LlmError;
use hq_llm::provider_usage::{
    Adapter, ProviderReading, adapter_for, read_anthropic_admin, read_deepseek, read_moonshot,
    read_openrouter,
};
use serde::Serialize;
use serde_json::json;
use tokio::sync::Mutex;

use crate::WsState;
use crate::error::ApiError;

/// A burst of requests shares one read per backend.
const CACHE_TTL: Duration = Duration::from_secs(60);
const DEFAULT_OPENROUTER_BASE: &str = "https://openrouter.ai/api/v1";
const DEFAULT_ANTHROPIC_BASE: &str = "https://api.anthropic.com/v1";

type Fetched = Result<ProviderReading, Failure>;

#[derive(Clone)]
struct Failure {
    refused: bool,
    reason: String,
}

static CACHE: Mutex<Option<HashMap<String, (Instant, Fetched)>>> = Mutex::const_new(None);

#[derive(Serialize)]
struct Row {
    backend: String,
    adapter: Adapter,
    /// `provider` when the provider answered, `ledger` when only HQ's own record is shown.
    source: &'static str,
    status: &'static str,
    provider: Option<ProviderReading>,
    ledger: LedgerWindows,
    /// What the latest ordinary response said about the remaining quota, with no extra request.
    rate_limit: Option<hq_llm::ratelimit::RateLimitReading>,
    note: Option<String>,
}

fn note_for(adapter: Adapter) -> Option<&'static str> {
    match adapter {
        Adapter::LedgerOnly => Some(
            "This provider documents no balance or spend endpoint. The figures are HQ's own record of calls it made, so spend from other tools is not included.",
        ),
        Adapter::Local => Some("Runs on this machine. Tokens are tracked and nothing is billed."),
        Adapter::Subscription => Some(
            "Billed through a subscription quota, not per token. See /api/copilot-usage for credits.",
        ),
        Adapter::AnthropicAdmin => {
            Some("Spend comes from the Anthropic organization cost report (UTC days).")
        }
        _ => None,
    }
}

fn row(
    entry: &BackendEntry,
    adapter: Adapter,
    fetched: Option<Fetched>,
    ledger: LedgerWindows,
) -> Row {
    let (status, provider, note) = match (adapter, fetched) {
        (_, Some(Ok(reading))) => ("ok", Some(reading), note_for(adapter).map(str::to_string)),
        (_, Some(Err(f))) if f.refused => (
            "refused",
            None,
            Some("The provider refused the key when asked for usage. Chat is not affected.".into()),
        ),
        (_, Some(Err(f))) => (
            "error",
            None,
            Some(format!("Could not read usage: {}", f.reason)),
        ),
        (Adapter::Local, None) => ("local", None, note_for(adapter).map(str::to_string)),
        (Adapter::Subscription, None) => {
            ("subscription", None, note_for(adapter).map(str::to_string))
        }
        (Adapter::LedgerOnly, None) => ("ledger_only", None, note_for(adapter).map(str::to_string)),
        (_, None) => (
            "no_key",
            None,
            Some("No key is set for this backend, so the provider cannot be asked.".into()),
        ),
    };
    Row {
        rate_limit: entry
            .resolved_endpoint()
            .and_then(|e| hq_llm::ratelimit::latest_for(&e)),
        backend: entry.name.clone(),
        adapter,
        source: if provider.is_some() {
            "provider"
        } else {
            "ledger"
        },
        status,
        provider,
        ledger,
        note,
    }
}

async fn fetch_for(
    adapter: Adapter,
    entry: &BackendEntry,
    key: &str,
    now: DateTime<Utc>,
) -> Result<ProviderReading, LlmError> {
    let endpoint = entry.resolved_endpoint();
    match adapter {
        Adapter::OpenRouter => {
            read_openrouter(endpoint.as_deref().unwrap_or(DEFAULT_OPENROUTER_BASE), key).await
        }
        Adapter::Deepseek => read_deepseek(endpoint.as_deref().unwrap_or_default(), key).await,
        Adapter::Moonshot => read_moonshot(endpoint.as_deref().unwrap_or_default(), key).await,
        Adapter::AnthropicAdmin => {
            let base = endpoint.as_deref().unwrap_or(DEFAULT_ANTHROPIC_BASE);
            read_anthropic_admin(base, key, now).await
        }
        Adapter::Subscription | Adapter::Local | Adapter::LedgerOnly => {
            Ok(ProviderReading::default())
        }
    }
}

async fn cached_fetch(
    entry: &BackendEntry,
    adapter: Adapter,
    key: &str,
    now: DateTime<Utc>,
) -> Fetched {
    // The cache key carries the endpoint and an in-process hash of the key, never the key itself,
    // so changing either drops the entry at once.
    let id = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        key.hash(&mut h);
        format!(
            "{}|{:?}|{:x}",
            entry.name,
            entry.resolved_endpoint(),
            h.finish()
        )
    };
    let mut guard = CACHE.lock().await;
    let map = guard.get_or_insert_with(HashMap::new);
    if let Some((at, fetched)) = map.get(&id)
        && at.elapsed() < CACHE_TTL
    {
        return fetched.clone();
    }
    let fetched = fetch_for(adapter, entry, key, now)
        .await
        .map_err(|e| Failure {
            refused: matches!(e, LlmError::Auth { .. }),
            reason: e.to_string(),
        });
    map.insert(id, (Instant::now(), fetched.clone()));
    fetched
}

fn admin_key(config: &HqConfig) -> Option<String> {
    config
        .usage_ledger
        .anthropic_admin_key_env
        .as_deref()
        .and_then(|name| std::env::var(name).ok())
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
}

async fn row_for(
    entry: &BackendEntry,
    config: &HqConfig,
    creds: &ConfigCredentials,
    ledger: LedgerWindows,
    now: DateTime<Utc>,
) -> Row {
    let admin = admin_key(config);
    let adapter = adapter_for(
        entry.kind,
        entry.resolved_endpoint().as_deref(),
        admin.is_some(),
    );
    let needs_key = !matches!(
        adapter,
        Adapter::Subscription | Adapter::Local | Adapter::LedgerOnly
    );
    if !needs_key {
        return row(entry, adapter, None, ledger);
    }
    let key = match adapter {
        Adapter::AnthropicAdmin => admin,
        _ => resolve_credential(entry, creds),
    };
    let Some(key) = key else {
        return row(entry, adapter, None, ledger);
    };
    let fetched = cached_fetch(entry, adapter, &key, now).await;
    row(entry, adapter, Some(fetched), ledger)
}

pub(crate) async fn usage_providers_handler(
    State(state): State<Arc<WsState>>,
) -> Result<impl IntoResponse, ApiError> {
    let config = match state.hq_config.as_deref() {
        Some(c) => c.clone(),
        None => HqConfig::load().unwrap_or_default(),
    };
    let creds = ConfigCredentials::from_config(&config);
    let now = Utc::now();
    let backends: Vec<&BackendEntry> = config
        .backends
        .backends
        .iter()
        .filter(|b| b.enabled)
        .collect();
    let mut pending = Vec::with_capacity(backends.len());
    for entry in backends {
        let ledger = state
            .db
            .with_conn(|conn| ledger_windows(conn, &entry.name, now.timestamp()))
            .unwrap_or_default();
        pending.push(row_for(entry, &config, &creds, ledger, now));
    }
    let rows = join_all(pending).await;
    Ok(Json(json!({ "generated_at": now, "backends": rows })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str) -> BackendEntry {
        serde_yaml::from_str(&format!(
            "name: {name}\nkind: openai-compatible\nendpoint: https://api.groq.com/openai/v1\n"
        ))
        .unwrap()
    }

    fn ledger(today: f64) -> LedgerWindows {
        LedgerWindows {
            today,
            ..LedgerWindows::default()
        }
    }

    #[test]
    fn a_provider_with_no_usage_endpoint_is_shown_from_the_ledger_and_says_so() {
        let r = row(&entry("groq"), Adapter::LedgerOnly, None, ledger(1.5));
        assert_eq!((r.status, r.source), ("ledger_only", "ledger"));
        assert_eq!(r.ledger.today, 1.5);
        assert!(r.note.unwrap().contains("HQ's own record"));
    }

    #[test]
    fn a_refused_key_is_reported_as_refused_without_leaking_the_reason() {
        let failure = Failure {
            refused: true,
            reason: "secret detail".into(),
        };
        let r = row(
            &entry("a"),
            Adapter::Deepseek,
            Some(Err(failure)),
            ledger(0.0),
        );
        assert_eq!(r.status, "refused");
        assert!(!r.note.unwrap().contains("secret detail"));
    }

    #[test]
    fn a_missing_key_is_a_status_not_an_error() {
        let r = row(&entry("a"), Adapter::Deepseek, None, ledger(0.0));
        assert_eq!((r.status, r.source), ("no_key", "ledger"));
    }

    #[test]
    fn an_answered_read_is_marked_as_the_providers_own_figure() {
        let reading = ProviderReading::default();
        let r = row(
            &entry("a"),
            Adapter::OpenRouter,
            Some(Ok(reading)),
            ledger(0.0),
        );
        assert_eq!((r.status, r.source), ("ok", "provider"));
    }
}
