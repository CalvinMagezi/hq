//! GitHub Copilot provider (Claude/Gemini models via the Copilot subscription).
//!
//! Speaks the Anthropic Messages wire format at `<api base>/v1/messages`.
//! Sending the raw GitHub OAuth token (from `gh auth token`, or
//! `COPILOT_GITHUB_TOKEN`/`GH_TOKEN`/`GITHUB_TOKEN`) straight through as
//! `Authorization: Bearer <token>` *looks* like it works, but live-verified
//! side by side against Hermes (which exchanges first): identical requests
//! with the raw token get randomly bucketed under a narrower `copilot-4-cli`
//! integrator and 400 with `model_not_available_for_integrator` (~45% of
//! calls). GitHub's edge proxy expects the token to first be exchanged for a
//! short-lived session token (`tid=...;exp=...`) via `GET
//! https://api.github.com/copilot_internal/v2/token`, which also returns the
//! correct account-scoped API base (`api.githubcopilot.com` for individual
//! seats, `api.business.githubcopilot.com` for business/enterprise seats).
//! This module does that exchange and caches the result; `openai_compat`'s
//! generic provider reuses [`cached_session_token_for`] for the same reason
//! when it targets a Copilot host over `/chat/completions` instead — a
//! supported backend shape (`OpenaiCompatible` pointed at a Copilot
//! endpoint, see `backends.rs`'s `model_routed_through_copilot`) and the one
//! the 45%-failure report was measured against (see its
//! `copilot_aware_credential`). Reuses `crate::anthropic`'s pure
//! request/response/SSE helpers so the wire protocol is implemented exactly
//! once.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use anyhow::Result;
use async_trait::async_trait;
use futures::StreamExt;
use tokio::sync::RwLock;
use tokio_stream::Stream;
use tokio_stream::wrappers::ReceiverStream;

use crate::anthropic::{
    AnthropicStreamState, DEFAULT_MAX_TOKENS, build_messages_body, classify_anthropic_error,
    drain_sse, parse_messages_response, parse_usage,
};
use crate::provider::{ChatRequest, ChatResponse, LlmError, LlmProvider, StreamChunk};

/// Base URL for the GitHub Copilot API.
pub const COPILOT_BASE_URL: &str = "https://api.githubcopilot.com";

/// `anthropic-version` header Copilot's Claude-model routing expects.
const COPILOT_ANTHROPIC_VERSION: &str = "2023-06-01";

/// Editor attribution headers. Verified live: not strictly required by the
/// API today, but costs nothing to send and matches the proven-working
/// header set other Copilot-integrated tools use, in case GitHub tightens
/// this later.
/// `pub(crate)`: also attached by `openai_compat`'s generic provider when its
/// endpoint is a Copilot host (see `attach_copilot_headers`) — same account,
/// same required headers, one definition.
pub(crate) const EDITOR_VERSION: &str = "vscode/1.104.1";
pub(crate) const COPILOT_INTEGRATION_ID: &str = "vscode-chat";
pub(crate) const OPENAI_INTENT: &str = "conversation-edits";

/// Env vars checked for a raw GitHub token, in priority order.
const TOKEN_ENV_VARS: [&str; 3] = ["COPILOT_GITHUB_TOKEN", "GH_TOKEN", "GITHUB_TOKEN"];

/// Name of the first env var in [`TOKEN_ENV_VARS`] that holds a non-empty
/// token. Returns the variable name only, never its value, so diagnostics can
/// say where the credential comes from without printing it.
pub fn env_token_source() -> Option<&'static str> {
    env_token_source_in(|name| std::env::var(name).ok())
}

fn env_token_source_in(lookup: impl Fn(&str) -> Option<String>) -> Option<&'static str> {
    TOKEN_ENV_VARS
        .into_iter()
        .find(|name| lookup(name).is_some_and(|v| !v.trim().is_empty()))
}

/// Optional `gh` account to read the token from. `gh auth token` without
/// `--user` returns whichever account is active, which may not be the one
/// holding the Copilot seat when several accounts are logged in.
const COPILOT_GH_USER_ENV: &str = "COPILOT_GH_USER";

/// GitHub's internal token-exchange endpoint: trades the raw OAuth token for
/// a short-lived Copilot session token scoped to the correct integrator and
/// account tier, plus the account's real API base URL. Mirrors Hermes's own
/// pre-request exchange (`hermes_cli`), which sees none of the integrator
/// misclassification failures the raw token hits.
const TOKEN_EXCHANGE_URL: &str = "https://api.github.com/copilot_internal/v2/token";

/// `User-Agent` the exchange endpoint expects (matches VS Code's Copilot
/// Chat extension, which mints the exchangeable tokens in the first place).
const EXCHANGE_USER_AGENT: &str = "GitHubCopilotChat/0.26.7";

/// Fallback session TTL when the exchange response has neither `refresh_in`
/// nor `expires_at` (shouldn't happen against the real API, but a missing
/// field must not mean "cache forever").
const TOKEN_REFRESH_INTERVAL: Duration = Duration::from_secs(600);

/// Re-exchange this long before the session token's actual expiry so a
/// request never races a token that expires mid-flight.
const SESSION_REFRESH_SAFETY_MARGIN: Duration = Duration::from_secs(60);

const MIN_SESSION_TTL: Duration = Duration::from_secs(30);

/// Cache TTL for the live model catalog fetch — matches Hermes's own
/// `_COPILOT_CONTEXT_CACHE_TTL`. GitHub's live catalog of models Copilot
/// currently serves includes its own per-model prompt-token cap, which can
/// differ sharply from a model's native context window (Gemini 3.8 Flash: 1M
/// native vs Copilot's 200K, live-verified). Mirrors Hermes's
/// `fetch_github_model_catalog` / `get_copilot_model_context`
/// (`hermes_cli/models.py`), same cadence.
const CONTEXT_CACHE_TTL: Duration = Duration::from_secs(3600);

/// Process-wide cache of `max_prompt_tokens` per model id from the live
/// catalog. Global rather than per-`CopilotProvider` instance: the registry
/// reconstructs providers frequently (every backend-chain rebuild), but the
/// catalog itself changes rarely and belongs to the account, not the
/// instance.
type ContextCache = RwLock<Option<(Instant, HashMap<String, u32>)>>;
static CONTEXT_CACHE: OnceLock<ContextCache> = OnceLock::new();

fn context_cache() -> &'static ContextCache {
    CONTEXT_CACHE.get_or_init(|| RwLock::new(None))
}

/// `max_prompt_tokens` for `model` from Copilot's live `/models` catalog, or
/// `None` if the model isn't listed there or the fetch/auth fails — callers
/// fall back to their own static table in that case, same as Hermes's own
/// `get_copilot_model_context` returning `None`.
pub async fn live_context_window(model: &str) -> Option<u32> {
    {
        let cache = context_cache().read().await;
        if let Some((fetched_at, limits)) = cache.as_ref()
            && fetched_at.elapsed() < CONTEXT_CACHE_TTL
        {
            return limits.get(model).copied();
        }
    }
    let session = cached_session_token(false).await.ok()?;
    let limits = fetch_context_limits(&session).await?;
    let value = limits.get(model).copied();
    let mut cache = context_cache().write().await;
    *cache = Some((Instant::now(), limits));
    value
}

/// One-shot `GET /models`, parsed into `{model id -> max_prompt_tokens}`.
/// Entries missing either field are skipped rather than failing the whole
/// fetch — a partial catalog is still useful.
async fn fetch_context_limits(session: &SessionToken) -> Option<HashMap<String, u32>> {
    let api_base = session.api_base.as_deref().unwrap_or(COPILOT_BASE_URL);
    let resp = crate::http::SHARED_HTTP_CLIENT
        .get(format!("{api_base}/models"))
        .header("Authorization", format!("Bearer {}", session.jwt))
        .header("Editor-Version", EDITOR_VERSION)
        .header("Copilot-Integration-Id", COPILOT_INTEGRATION_ID)
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let json: serde_json::Value = resp.json().await.ok()?;
    Some(parse_context_limits(&json))
}

/// Pure parse of a `GET /models` response body into `{model id ->
/// max_prompt_tokens}`. Entries missing either field are skipped rather than
/// failing the whole catalog — a partial catalog is still useful. Split out
/// from [`fetch_context_limits`] so the parsing logic is testable without a
/// live request.
fn parse_context_limits(json: &serde_json::Value) -> HashMap<String, u32> {
    let mut out = HashMap::new();
    let Some(items) = json.get("data").and_then(|d| d.as_array()) else {
        return out;
    };
    for item in items {
        let Some(id) = item.get("id").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(max_prompt) = item
            .get("capabilities")
            .and_then(|c| c.get("limits"))
            .and_then(|l| l.get("max_prompt_tokens"))
            .and_then(|v| v.as_u64())
        else {
            continue;
        };
        out.insert(id.to_string(), max_prompt as u32);
    }
    out
}

/// Pick the first non-empty value, matching the Copilot CLI's own env-var
/// priority. Pure so it's directly testable without touching real process
/// environment.
fn select_env_token(values: [Option<&str>; 3]) -> Option<String> {
    values
        .into_iter()
        .find_map(|v| v.map(str::trim).filter(|s| !s.is_empty()))
        .map(str::to_string)
}

#[cfg(test)]
mod token_selection_tests {
    use super::{env_token_source_in, select_env_token};

    #[test]
    fn token_source_names_first_non_empty_var_without_value() {
        let lookup = |n: &str| match n {
            "COPILOT_GITHUB_TOKEN" => Some("  ".to_string()),
            "GH_TOKEN" => Some("secret".to_string()),
            _ => None,
        };
        assert_eq!(env_token_source_in(lookup), Some("GH_TOKEN"));
        assert_eq!(env_token_source_in(|_| None), None);
    }

    #[test]
    fn prefers_first_set_value() {
        assert_eq!(
            select_env_token([Some("a"), Some("b"), Some("c")]),
            Some("a".to_string())
        );
    }

    #[test]
    fn skips_empty_and_falls_through() {
        assert_eq!(
            select_env_token([Some(""), None, Some("c")]),
            Some("c".to_string())
        );
    }

    #[test]
    fn returns_none_when_all_unset() {
        assert_eq!(select_env_token([None, None, None]), None);
    }

    #[test]
    fn trims_whitespace() {
        assert_eq!(
            select_env_token([Some("  padded  "), None, None]),
            Some("padded".to_string())
        );
    }
}

#[cfg(test)]
mod context_catalog_tests {
    use super::parse_context_limits;

    /// Trimmed but structurally exact live catalog entry (captured 2026-09-17
    /// against a real Copilot work-seat subscription's `GET /models`):
    /// `max_prompt_tokens=200000` for `gemini-3.8-flash`, which is what the
    /// session-reset banner and the session builder's context-window
    /// derivation both need out of this response.
    fn live_catalog_fixture() -> serde_json::Value {
        serde_json::json!({
            "object": "list",
            "data": [
                {
                    "id": "gemini-3.8-flash",
                    "object": "model",
                    "capabilities": {
                        "limits": {
                            "max_context_window_tokens": 265536,
                            "max_output_tokens": 65536,
                            "max_prompt_tokens": 200000
                        }
                    }
                },
                {
                    "id": "claude-haiku-4.5",
                    "object": "model",
                    "capabilities": {
                        "limits": {
                            "max_context_window_tokens": 200000,
                            "max_output_tokens": 16384,
                            "max_prompt_tokens": 176000
                        }
                    }
                },
                { "id": "copilot-acp", "object": "model" }
            ]
        })
    }

    #[test]
    fn extracts_max_prompt_tokens_per_model_from_the_live_shape() {
        let limits = parse_context_limits(&live_catalog_fixture());
        assert_eq!(limits.get("gemini-3.8-flash"), Some(&200_000));
        assert_eq!(limits.get("claude-haiku-4.5"), Some(&176_000));
    }

    #[test]
    fn skips_entries_missing_capabilities_or_limits() {
        let limits = parse_context_limits(&live_catalog_fixture());
        assert!(
            !limits.contains_key("copilot-acp"),
            "an entry with no capabilities.limits must be skipped, not panic or fabricate a value"
        );
    }

    #[test]
    fn missing_data_array_yields_an_empty_map_not_an_error() {
        let limits = parse_context_limits(&serde_json::json!({"object": "list"}));
        assert!(limits.is_empty());
    }
}

#[cfg(test)]
mod session_exchange_tests {
    use super::{parse_session_token, session_ttl};

    /// Structurally exact business-seat exchange response shape (the case
    /// the flaky-integrator bug was traced to): `endpoints.api` points at
    /// the business host, distinct from the individual-seat default.
    fn business_seat_fixture() -> serde_json::Value {
        serde_json::json!({
            "token": "tid=abc123;exp=1999999999;sku=business",
            "expires_at": 1_999_999_999,
            "refresh_in": 1500,
            "endpoints": {
                "api": "https://api.business.githubcopilot.com",
                "telemetry": "https://telemetry.githubcopilot.com"
            }
        })
    }

    #[test]
    fn parses_jwt_and_business_api_base_from_the_live_shape() {
        let session = parse_session_token(&business_seat_fixture()).unwrap();
        assert_eq!(session.jwt, "tid=abc123;exp=1999999999;sku=business");
        assert_eq!(
            session.api_base.as_deref(),
            Some("https://api.business.githubcopilot.com")
        );
    }

    #[test]
    fn leaves_api_base_none_when_endpoints_and_proxy_ep_are_both_absent() {
        // Individual accounts: caller keeps its own default rather than this
        // module silently redirecting it to one.
        let session = parse_session_token(&serde_json::json!({"token": "tid=x;exp=1"})).unwrap();
        assert_eq!(session.api_base, None);
    }

    #[test]
    fn derives_api_base_from_proxy_ep_when_endpoints_api_is_absent() {
        let session = parse_session_token(&serde_json::json!({
            "token": "tid=x;exp=1;proxy-ep=proxy.enterprise.githubcopilot.com"
        }))
        .unwrap();
        assert_eq!(
            session.api_base.as_deref(),
            Some("https://api.enterprise.githubcopilot.com")
        );
    }

    #[test]
    fn does_not_double_prefix_a_proxy_ep_that_already_starts_with_api() {
        let session = parse_session_token(&serde_json::json!({
            "token": "tid=x;exp=1;proxy-ep=api.enterprise.githubcopilot.com"
        }))
        .unwrap();
        assert_eq!(
            session.api_base.as_deref(),
            Some("https://api.enterprise.githubcopilot.com")
        );
    }

    #[test]
    fn rejects_a_response_with_no_token() {
        let err = parse_session_token(&serde_json::json!({"endpoints": {"api": "https://x"}}));
        assert!(err.is_err(), "a response with no `token` must not parse ok");
    }

    #[test]
    fn refresh_in_wins_over_expires_at_and_applies_the_safety_margin() {
        let ttl = session_ttl(&serde_json::json!({"refresh_in": 1500, "expires_at": 1}));
        assert_eq!(ttl, std::time::Duration::from_secs(1500 - 60));
    }

    #[test]
    fn falls_back_to_expires_at_when_refresh_in_is_absent() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let ttl = session_ttl(&serde_json::json!({"expires_at": now + 600}));
        // Within a few seconds of 540s (600 - 60s safety margin); avoid an
        // exact-equality race against the wall clock ticking between here
        // and inside `session_ttl`.
        assert!(ttl.as_secs() <= 540 && ttl.as_secs() >= 535);
    }

    #[test]
    fn never_returns_a_ttl_below_the_floor_even_for_an_already_expired_token() {
        let ttl = session_ttl(&serde_json::json!({"refresh_in": 5}));
        assert_eq!(ttl, super::MIN_SESSION_TTL);
    }
}

/// A GitHub Copilot session credential: the exchanged short-lived token plus
/// the account-scoped API base it came with, when the exchange determined
/// one (business/enterprise/proxied seats return `endpoints.api` or a
/// `proxy-ep` on the token; plain individual seats return neither). `None`
/// means "use whatever default the caller already had" — a business-seat
/// backend entry configured with its own endpoint must not get silently
/// redirected to the individual-seat default just because one exchange
/// attempt failed or genuinely found no account-specific host.
#[derive(Clone)]
pub(crate) struct SessionToken {
    /// The `tid=...;exp=...;...` value sent as `Authorization: Bearer <jwt>`.
    pub(crate) jwt: String,
    pub(crate) api_base: Option<String>,
    expires_at: Instant,
}

/// Process-wide session-token cache, keyed by the raw token it was exchanged
/// from (a backend entry can supply a different credential than `gh auth
/// token`'s default, e.g. a custom `credential_env`). Global rather than
/// per-provider-instance for the same reason as [`CONTEXT_CACHE`]: the
/// registry reconstructs providers frequently (every backend-chain rebuild),
/// but the exchanged session is good for up to roughly an hour and belongs
/// to the account, not the instance.
static SESSION_CACHE: OnceLock<RwLock<HashMap<String, SessionToken>>> = OnceLock::new();

fn session_cache() -> &'static RwLock<HashMap<String, SessionToken>> {
    SESSION_CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

/// TTL for the raw-token fallback used when the exchange itself fails (e.g.
/// this account's token type doesn't support exchange, or a transient
/// network/GitHub-API hiccup). Short, so it retries the exchange again soon
/// rather than being stuck on the pre-exchange flaky behavior for a full
/// session lifetime.
const EXCHANGE_FAILURE_FALLBACK_TTL: Duration = Duration::from_secs(60);

/// Resolve a usable Copilot session token for the raw token from env vars /
/// `gh auth token`. Used by [`CopilotProvider`].
pub(crate) async fn cached_session_token(force: bool) -> Result<SessionToken, LlmError> {
    let raw = CopilotProvider::resolve_raw_token().await?;
    cached_session_token_for(&raw, force).await
}

/// Resolve a usable Copilot session token for an already-resolved raw token:
/// the cached one when still fresh, otherwise a fresh exchange. `force`
/// skips the cache read (used after a 401, in case the exchanged token
/// itself was revoked).
///
/// `pub(crate)`: also used by `openai_compat`'s generic provider when its
/// endpoint is a Copilot host — same integrator-misclassification bug as the
/// native Messages path this module exists for (see the module doc
/// comment), just reached over `/chat/completions` instead of `/v1/messages`.
///
/// Never fails outright: an exchange that errors (network hiccup, or an
/// account/token type the exchange doesn't support — some GitHub OAuth apps
/// mint tokens exchange 404s on) falls back to sending the raw token
/// directly against the default API base, matching this module's pre-fix
/// behavior, so a broken exchange degrades to "as flaky as before" rather
/// than "broken outright".
pub(crate) async fn cached_session_token_for(
    raw_token: &str,
    force: bool,
) -> Result<SessionToken, LlmError> {
    if !force {
        let cache = session_cache().read().await;
        if let Some(session) = cache.get(raw_token)
            && session.expires_at > Instant::now()
        {
            return Ok(session.clone());
        }
    }
    let session = match exchange_session_token(raw_token).await {
        Ok(session) => session,
        Err(e) => {
            tracing::warn!(
                error = %e,
                "Copilot token exchange failed, falling back to the raw token"
            );
            SessionToken {
                jwt: raw_token.to_string(),
                api_base: None,
                expires_at: Instant::now() + EXCHANGE_FAILURE_FALLBACK_TTL,
            }
        }
    };
    let mut cache = session_cache().write().await;
    cache.insert(raw_token.to_string(), session.clone());
    Ok(session)
}

/// `GET /copilot_internal/v2/token`: trade the raw GitHub OAuth token for a
/// short-lived Copilot session token and the account's real API base.
/// Live-verified header shape (matches VS Code / the Copilot CLI): the
/// `token` scheme here, not `Bearer` — unlike every other Copilot request in
/// this module, which the exchanged token itself *is* sent with as Bearer.
async fn exchange_session_token(raw_token: &str) -> Result<SessionToken, LlmError> {
    let resp = crate::http::SHARED_HTTP_CLIENT
        .get(TOKEN_EXCHANGE_URL)
        .header("Authorization", format!("token {raw_token}"))
        .header("Editor-Version", EDITOR_VERSION)
        .header("User-Agent", EXCHANGE_USER_AGENT)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| LlmError::from_request_error(&e))?;

    let status = resp.status().as_u16();
    let bytes = resp.bytes().await.map_err(|e| {
        LlmError::Other(anyhow::anyhow!(
            "failed to read Copilot token-exchange response: {e}"
        ))
    })?;
    if !(200..300).contains(&status) {
        return Err(LlmError::Auth {
            status,
            message: format!(
                "Copilot token exchange failed: {}",
                String::from_utf8_lossy(&bytes).trim()
            ),
        });
    }
    let json: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| {
        LlmError::Other(anyhow::anyhow!(
            "failed to parse Copilot token-exchange response: {e}"
        ))
    })?;
    parse_session_token(&json)
}

/// Copilot API base URL derived from the exchanged token's own
/// `proxy-ep=proxy.<host>` field, for accounts where the exchange response's
/// `endpoints.api` is absent but the token still isn't served by the default
/// host (proxied/enterprise accounts). `None` when the token carries no such
/// field (individual accounts: the default `COPILOT_BASE_URL` applies).
fn derive_base_url_from_proxy_ep(token: &str) -> Option<String> {
    let rest = token.split("proxy-ep=").nth(1)?;
    let proxy_ep = rest.split(';').next().unwrap_or(rest).trim();
    if proxy_ep.is_empty() {
        return None;
    }
    let host = proxy_ep
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/');
    // Substitute a leading `proxy.` for `api.`; a host that's already
    // `api.<something>` (no `proxy.` prefix) must not get a second `api.`
    // prepended on top of it.
    let host = match host.strip_prefix("proxy.") {
        Some(rest) => format!("api.{rest}"),
        None => host.to_string(),
    };
    Some(format!("https://{host}"))
}

/// Pure parse of a `GET /copilot_internal/v2/token` response body. Split out
/// from [`exchange_session_token`] so it's testable without a live request.
fn parse_session_token(json: &serde_json::Value) -> Result<SessionToken, LlmError> {
    let jwt = json
        .get("token")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| LlmError::Auth {
            status: 401,
            message: "Copilot token exchange response missing `token`".to_string(),
        })?
        .to_string();
    let api_base = json
        .get("endpoints")
        .and_then(|e| e.get("api"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.trim_end_matches('/').to_string())
        .or_else(|| derive_base_url_from_proxy_ep(&jwt));
    Ok(SessionToken {
        jwt,
        api_base,
        expires_at: Instant::now() + session_ttl(json),
    })
}

/// How long to trust an exchanged session token before re-exchanging,
/// derived from whichever of `refresh_in` (seconds) / `expires_at` (unix
/// seconds) the response carries, minus a safety margin.
fn session_ttl(json: &serde_json::Value) -> Duration {
    if let Some(refresh_in) = json.get("refresh_in").and_then(|v| v.as_u64()) {
        return Duration::from_secs(refresh_in)
            .saturating_sub(SESSION_REFRESH_SAFETY_MARGIN)
            .max(MIN_SESSION_TTL);
    }
    if let Some(expires_at) = json.get("expires_at").and_then(|v| v.as_i64()) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let remaining = (expires_at - now).max(0) as u64;
        return Duration::from_secs(remaining)
            .saturating_sub(SESSION_REFRESH_SAFETY_MARGIN)
            .max(MIN_SESSION_TTL);
    }
    TOKEN_REFRESH_INTERVAL
}

/// GitHub Copilot provider. Speaks the Anthropic Messages wire format.
pub struct CopilotProvider {
    http: reqwest::Client,
    /// Set only by the test constructor, bypassing the process-wide cache
    /// and the real token exchange entirely so tests stay isolated from each
    /// other and from real credentials.
    session_override: Option<SessionToken>,
}

impl CopilotProvider {
    pub fn new() -> Self {
        Self {
            http: crate::http::SHARED_HTTP_CLIENT.clone(),
            session_override: None,
        }
    }

    /// Test-only constructor: points at a local mock server with an
    /// already-"exchanged" session token so tests never touch real env vars,
    /// spawn `gh`, or hit the real token-exchange endpoint.
    #[cfg(test)]
    fn new_with_base_and_token(base_url: &str, token: &str) -> Self {
        Self {
            http: crate::http::SHARED_HTTP_CLIENT.clone(),
            session_override: Some(SessionToken {
                jwt: token.to_string(),
                api_base: Some(base_url.to_string()),
                expires_at: Instant::now() + Duration::from_secs(3600),
            }),
        }
    }

    async fn get_session_token(&self, force: bool) -> Result<SessionToken, LlmError> {
        if let Some(session) = &self.session_override {
            return Ok(session.clone());
        }
        cached_session_token(force).await
    }

    /// Resolve a raw GitHub token: env vars first, else `gh auth token`.
    ///
    /// `pub` (not `pub(crate)`): also used by [`live_context_window`]'s
    /// cross-crate callers (e.g. `hq-agent`'s session builder) to authenticate
    /// the live catalog fetch without constructing a full provider instance.
    pub async fn resolve_raw_token() -> Result<String, LlmError> {
        let env_values = [
            std::env::var(TOKEN_ENV_VARS[0]).ok(),
            std::env::var(TOKEN_ENV_VARS[1]).ok(),
            std::env::var(TOKEN_ENV_VARS[2]).ok(),
        ];
        if let Some(token) = select_env_token([
            env_values[0].as_deref(),
            env_values[1].as_deref(),
            env_values[2].as_deref(),
        ]) {
            return Ok(token);
        }

        let mut gh = tokio::process::Command::new("gh");
        gh.args(["auth", "token"]);
        if let Some(user) = std::env::var(COPILOT_GH_USER_ENV)
            .ok()
            .filter(|u| !u.is_empty())
        {
            gh.args(["--user", &user]);
        }
        let output = gh.output().await.map_err(|e| LlmError::Auth {
            status: 401,
            message: format!(
                "no Copilot credential: env vars unset and `gh auth token` failed to spawn: {e}"
            ),
        })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(LlmError::Auth {
                status: 401,
                message: format!("`gh auth token` failed: {}", stderr.trim()),
            });
        }

        let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if token.is_empty() {
            return Err(LlmError::Auth {
                status: 401,
                message: "`gh auth token` returned an empty token".to_string(),
            });
        }
        Ok(token)
    }

    fn request_headers(builder: reqwest::RequestBuilder, token: &str) -> reqwest::RequestBuilder {
        builder
            .header("Authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .header("anthropic-version", COPILOT_ANTHROPIC_VERSION)
            .header("Editor-Version", EDITOR_VERSION)
            .header("Copilot-Integration-Id", COPILOT_INTEGRATION_ID)
            .header("Openai-Intent", OPENAI_INTENT)
            .header("x-initiator", "agent")
    }

    /// Get a session token and send the request with automatic 401 retry on
    /// a forced re-exchange. The `streaming` flag adds the SSE accept header
    /// when true.
    async fn send_with_retry(
        &self,
        body: &serde_json::Value,
        streaming: bool,
    ) -> Result<reqwest::Response, LlmError> {
        let mut session = self.get_session_token(false).await?;

        let build = |session: &SessionToken| {
            let api_base = session.api_base.as_deref().unwrap_or(COPILOT_BASE_URL);
            let mut b = Self::request_headers(
                self.http.post(format!("{api_base}/v1/messages")),
                &session.jwt,
            )
            .json(body);
            if streaming {
                b = b.header("accept", "text/event-stream");
            }
            b
        };

        let mut resp = build(&session)
            .send()
            .await
            .map_err(|e| LlmError::from_request_error(&e))?;

        // A 401 may mean a rotated/revoked or expired session token — force
        // a fresh exchange once and retry.
        if resp.status().as_u16() == 401 {
            session = self.get_session_token(true).await?;
            resp = build(&session)
                .send()
                .await
                .map_err(|e| LlmError::from_request_error(&e))?;
        }

        Ok(resp)
    }
}

impl Default for CopilotProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl LlmProvider for CopilotProvider {
    fn name(&self) -> &str {
        "copilot"
    }

    async fn chat(&self, request: &ChatRequest) -> Result<ChatResponse> {
        let body = build_messages_body(request, DEFAULT_MAX_TOKENS, false);
        let resp = self.send_with_retry(&body, false).await?;

        let status = resp.status().as_u16();
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| LlmError::Other(anyhow::anyhow!("failed to read response: {e}")))?;

        if !(200..300).contains(&status) {
            return Err(classify_anthropic_error(status, &bytes).into());
        }

        let json: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| {
            LlmError::Other(anyhow::anyhow!("failed to parse Copilot response: {e}"))
        })?;

        let message = parse_messages_response(&json)?;
        let model = json
            .get("model")
            .and_then(|m| m.as_str())
            .unwrap_or(&request.model)
            .to_string();
        let (input_tokens, output_tokens, cache_read_tokens, cache_write_tokens) =
            parse_usage(json.get("usage"));

        Ok(ChatResponse {
            message,
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_write_tokens,
            reasoning_tokens: 0,
            provider_cost_usd: None,
            model,
        })
    }

    async fn chat_stream(
        &self,
        request: &ChatRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>> {
        let body = build_messages_body(request, DEFAULT_MAX_TOKENS, true);
        let resp = self.send_with_retry(&body, true).await?;

        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            let bytes = resp.bytes().await.unwrap_or_default();
            return Err(classify_anthropic_error(status, &bytes).into());
        }

        let (tx, rx) = tokio::sync::mpsc::channel::<Result<StreamChunk>>(64);
        tokio::spawn(async move {
            let mut byte_stream = resp.bytes_stream();
            let mut buf: Vec<u8> = Vec::new();
            let mut state = AnthropicStreamState::default();

            loop {
                let next = tokio::select! {
                    biased;
                    _ = tx.closed() => return,
                    next = byte_stream.next() => next,
                };

                match next {
                    Some(Ok(bytes)) => {
                        buf.extend_from_slice(&bytes);
                        for chunk in drain_sse(&mut buf, &mut state) {
                            let is_err = chunk.is_err();
                            if tx.send(chunk).await.is_err() {
                                return;
                            }
                            if is_err {
                                return;
                            }
                        }
                        if state.is_done() {
                            return;
                        }
                    }
                    Some(Err(e)) => {
                        let _ = tx.send(Err(LlmError::from_request_error(&e).into())).await;
                        return;
                    }
                    None => {
                        if !state.is_done() {
                            let _ = tx
                                .send(Err(LlmError::Network(
                                    "Copilot stream closed before message_stop (truncated response)"
                                        .to_string(),
                                )
                                .into()))
                                .await;
                        }
                        return;
                    }
                }
            }
        });

        Ok(Box::pin(ReceiverStream::new(rx)))
    }
}

#[cfg(test)]
mod http_tests {
    use super::*;
    use hq_core::types::{ChatMessage, MessageRole};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    fn user_request(model: &str) -> ChatRequest {
        ChatRequest {
            model: model.to_string(),
            messages: vec![ChatMessage {
                image_parts: Vec::new(),
                role: MessageRole::User,
                content: "hi".to_string(),
                tool_calls: vec![],
                tool_call_id: None,
                reasoning_content: None,
            }],
            tools: vec![],
            temperature: None,
            max_tokens: None,
        }
    }

    #[tokio::test]
    async fn chat_sends_bearer_auth_and_parses_the_response() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 8192];
            let n = sock.read(&mut buf).await.unwrap();
            let request = String::from_utf8_lossy(&buf[..n]).to_ascii_lowercase();

            let body = r#"{"id":"msg_1","model":"claude-haiku-4-5-20251001","role":"assistant","content":[{"type":"text","text":"Hi"}],"usage":{"input_tokens":5,"output_tokens":2}}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            sock.write_all(response.as_bytes()).await.unwrap();
            sock.flush().await.unwrap();
            request
        });

        let provider =
            CopilotProvider::new_with_base_and_token(&format!("http://{addr}"), "gho_testtoken");
        let response = provider
            .chat(&user_request("claude-haiku-4.5"))
            .await
            .unwrap();

        assert_eq!(response.message.content, "Hi");
        assert_eq!(response.model, "claude-haiku-4-5-20251001");
        assert_eq!(response.input_tokens, 5);
        assert_eq!(response.output_tokens, 2);

        let sent = server.await.unwrap();
        assert!(sent.contains("post /v1/messages"));
        assert!(sent.contains("authorization: bearer gho_testtoken"));
        assert!(sent.contains("anthropic-version: 2023-06-01"));
    }

    #[tokio::test]
    async fn chat_stream_parses_copilot_sse_frames_including_trailing_done_marker() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut scratch = [0u8; 4096];
            let _ = sock.read(&mut scratch).await;

            // Exact frame shape captured from a live Copilot API response,
            // including the trailing bare `data: [DONE]` line.
            let body = "event: message_start\n\
                 data: {\"type\":\"message_start\",\"message\":{\"model\":\"claude-haiku-4-5-20251001\",\"usage\":{\"input_tokens\":11,\"output_tokens\":1}}}\n\n\
                 event: content_block_start\n\
                 data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n\
                 event: content_block_delta\n\
                 data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hi\"}}\n\n\
                 event: content_block_stop\n\
                 data: {\"type\":\"content_block_stop\",\"index\":0}\n\n\
                 event: message_delta\n\
                 data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":2}}\n\n\
                 event: message_stop\n\
                 data: {\"type\":\"message_stop\"}\n\n\
                 data: [DONE]\n\n";
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n{body}"
            );
            sock.write_all(response.as_bytes()).await.unwrap();
            sock.flush().await.unwrap();
        });

        let provider =
            CopilotProvider::new_with_base_and_token(&format!("http://{addr}"), "gho_testtoken");
        let mut stream = provider
            .chat_stream(&user_request("claude-haiku-4.5"))
            .await
            .unwrap();

        let mut text = String::new();
        let mut saw_done = false;
        while let Some(item) = stream.next().await {
            match item.unwrap() {
                StreamChunk::Text(t) => text.push_str(&t),
                StreamChunk::Done => saw_done = true,
                _ => {}
            }
        }
        server.await.unwrap();

        assert_eq!(text, "Hi");
        assert!(saw_done, "expected a terminal Done after message_stop");
    }
}
