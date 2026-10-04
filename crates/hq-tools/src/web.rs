//! Web tools — search the web and fetch page content.
//!
//! Search tries a self-hosted SearxNG instance first when `searxng_url` is
//! configured and reachable (free, no API key, see `scripts/setup-searxng.sh`),
//! falling back to the paid Brave Search API when SearxNG is unset, failing,
//! cooling down after recent failures, or returns nothing. Neither backend is
//! guaranteed to exist on a given host. The machine profile reports each one
//! as configured or reachable without querying it; [`probe_search_backends`]
//! (used by `hq doctor`) sends one real query per backend. Every response
//! here names the backend that actually answered and each fallback attempt.
//!
//! The whole chain runs under one deadline (`SEARCH_DEADLINE`, 20s): SearxNG
//! gets at most 5s, Brave at most 12s, each capped by whatever is left. Every
//! transport, HTTP, JSON and shape failure puts that backend into exponential
//! cooldown, keyed by its endpoint.
//!
//! Filters are optional and provider-dependent. Backend differences:
//!
//! | option     | SearxNG                           | Brave                         |
//! |------------|-----------------------------------|-------------------------------|
//! | freshness  | `time_range` (engine-dependent)   | `freshness` pd/pw/pm/py       |
//! | language   | `language`                        | `search_lang`                 |
//! | country    | only with language (`en-US`)      | `country`                     |
//! | category   | general, news, science            | general, news                 |
//! | domains    | `site:` operators + post-filter   | `site:` operators + post-filter |
//! | page       | `pageno`, unbounded               | `offset`, pages 1..=10        |
//!
//! An option the answering backend can't honor is listed in
//! `unsupported_filters` rather than silently dropped. Domain filters are
//! always enforced by a post-filter on the result host, so a page can come
//! back with fewer than `max_results` hits.
//!
//! Fetching uses reqwest plus html2text for HTML, `hq-convert` for PDFs (text
//! layer first, OCR for scanned files), and an automatic Jina Reader
//! (`r.jina.ai`, a third-party service that receives only the URL) fallback
//! for JS-rendered pages that come back as an empty shell. Every redirect hop
//! is re-checked against the private-network rules. Pages that need a logged-in
//! browser, interaction, or anything Jina can't render stay unsupported.

use anyhow::{Result, bail};
use async_trait::async_trait;
use reqwest::{Client, RequestBuilder};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::future::Future;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tracing::debug;

use crate::registry::HqTool;
use hq_core::types::ValidationResult;

#[cfg(test)]
mod fetch_tests;
mod health;
mod ssrf;

pub use health::probe_search_backends;
#[cfg(test)]
use ssrf::is_non_public_ip;
use ssrf::{GuardedResolver, validate_url};

const FETCH_TIMEOUT: Duration = Duration::from_secs(30);
const JINA_TIMEOUT: Duration = Duration::from_secs(25);
// Fetch + text layer + OCR (30 + 15 + 45s) stays under hq-agent's 95s outer tool timeout.
const PDF_TEXT_TIMEOUT: Duration = Duration::from_secs(15);
const PDF_OCR_TIMEOUT: Duration = Duration::from_secs(45);
const MAX_BODY_BYTES: usize = 10 * 1024 * 1024; // 10 MB before extraction
const MAX_REDIRECTS: usize = 10;
const DEFAULT_MAX_OUTPUT_CHARS: usize = 100_000; // 100KB, matches Claude Code
const DEFAULT_MAX_RESULTS: usize = 5;
pub const MAX_RESULTS_CAP: usize = 20;
const MAX_DOMAIN_FILTERS: usize = 10;
const HTML_TEXT_WIDTH: usize = 80;
const USER_AGENT: &str = "Mozilla/5.0 (compatible; HQ-Agent/0.7)";
const CACHE_TTL: Duration = Duration::from_secs(15 * 60); // 15 min cache
const SEARCH_DEADLINE: Duration = Duration::from_secs(20);
const SEARXNG_TIMEOUT: Duration = Duration::from_secs(5);
const BRAVE_TIMEOUT: Duration = Duration::from_secs(12);
const BRAVE_ENDPOINT: &str = "https://api.search.brave.com/res/v1/web/search";
/// Brave's `offset` tops out at 9, so page 10 is the last one it can serve.
const BRAVE_MAX_PAGE: u32 = 10;
/// Below this many characters a PDF's text layer is treated as missing, as in the Telegram relay.
const MIN_PDF_TEXT_CHARS: usize = 50;

// ─── Types ──────────────────────────────────────────────────────

/// A single search result. Fields past `snippet` are provider metadata and
/// are absent when the provider didn't supply them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
    /// 1-based rank within the page the provider returned, before domain filtering.
    #[serde(default)]
    pub position: usize,
    /// Backend that produced this result: `searxng` or `brave`.
    #[serde(default)]
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    /// Publication or last-update date as the provider reported it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published: Option<String>,
    /// Upstream engines SearxNG aggregated this result from.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub engines: Vec<String>,
}

/// One backend's part in a search: answered, failed (and why), or skipped.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderAttempt {
    pub provider: String,
    pub outcome: String,
}

/// Collection of search results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSearchResults {
    pub query: String,
    pub results: Vec<SearchResult>,
    /// Backend whose results these are.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,
    #[serde(default = "first_page")]
    pub page: u32,
    /// Page to request next, when the backend says there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_page: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attempts: Vec<ProviderAttempt>,
    /// Requested filters the answering backend could not apply.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unsupported_filters: Vec<String>,
}

fn first_page() -> u32 {
    1
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    Day,
    Week,
    Month,
    Year,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    General,
    News,
    Science,
}

/// Optional search controls. `Default` reproduces the unfiltered behaviour.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchOptions {
    pub max_results: usize,
    /// 1-based page number.
    pub page: u32,
    pub freshness: Option<Freshness>,
    /// ISO 639-1 code, lowercase (`en`).
    pub language: Option<String>,
    /// ISO 3166-1 alpha-2 code, uppercase (`US`).
    pub country: Option<String>,
    pub category: Option<Category>,
    pub include_domains: Vec<String>,
    pub exclude_domains: Vec<String>,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            max_results: DEFAULT_MAX_RESULTS,
            page: 1,
            freshness: None,
            language: None,
            country: None,
            category: None,
            include_domains: Vec::new(),
            exclude_domains: Vec::new(),
        }
    }
}

impl SearchOptions {
    /// Parse the tool arguments shared by the MCP and agent `web_search` tools.
    /// Bad values are rejected with a message naming the field, never ignored.
    pub fn from_args(args: &Value) -> Result<Self, String> {
        let mut opts = Self::default();
        if let Some(v) = args.get("max_results").filter(|v| !v.is_null()) {
            let n = v.as_u64().ok_or("max_results must be a positive integer")?;
            opts.max_results = (n as usize).clamp(1, MAX_RESULTS_CAP);
        }
        if let Some(v) = args.get("page").filter(|v| !v.is_null()) {
            let n = v
                .as_u64()
                .filter(|n| *n >= 1)
                .ok_or("page must be an integer >= 1")?;
            opts.page = u32::try_from(n).map_err(|_| "page is too large")?;
        }
        if let Some(s) = str_arg(args, "freshness") {
            opts.freshness = Some(match s.as_str() {
                "day" => Freshness::Day,
                "week" => Freshness::Week,
                "month" => Freshness::Month,
                "year" => Freshness::Year,
                _ => {
                    return Err(format!(
                        "freshness must be one of day, week, month, year (got {s:?})"
                    ));
                }
            });
        }
        if let Some(s) = str_arg(args, "language") {
            if s.len() != 2 || !s.chars().all(|c| c.is_ascii_alphabetic()) {
                return Err(format!(
                    "language must be a 2-letter ISO 639-1 code like \"en\" (got {s:?})"
                ));
            }
            opts.language = Some(s.to_lowercase());
        }
        if let Some(s) = str_arg(args, "country") {
            if s.len() != 2 || !s.chars().all(|c| c.is_ascii_alphabetic()) {
                return Err(format!(
                    "country must be a 2-letter ISO 3166 code like \"US\" (got {s:?})"
                ));
            }
            opts.country = Some(s.to_uppercase());
        }
        if let Some(s) = str_arg(args, "category") {
            opts.category = Some(match s.as_str() {
                "general" => Category::General,
                "news" => Category::News,
                "science" => Category::Science,
                _ => {
                    return Err(format!(
                        "category must be one of general, news, science (got {s:?})"
                    ));
                }
            });
        }
        opts.include_domains = domain_list_arg(args, "include_domains")?;
        opts.exclude_domains = domain_list_arg(args, "exclude_domains")?;
        Ok(opts)
    }
}

fn str_arg(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
}

fn domain_list_arg(args: &Value, key: &str) -> Result<Vec<String>, String> {
    let Some(v) = args.get(key).filter(|v| !v.is_null()) else {
        return Ok(Vec::new());
    };
    let items = v
        .as_array()
        .ok_or(format!("{key} must be an array of domains"))?;
    if items.len() > MAX_DOMAIN_FILTERS {
        return Err(format!("{key} takes at most {MAX_DOMAIN_FILTERS} domains"));
    }
    items
        .iter()
        .map(|d| {
            d.as_str()
                .ok_or(format!("{key} entries must be strings"))
                .and_then(normalize_domain)
        })
        .collect()
}

/// `https://www.Example.com/docs` becomes `example.com`.
fn normalize_domain(raw: &str) -> Result<String, String> {
    let lower = raw.trim().to_lowercase();
    let no_scheme = lower
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let host = no_scheme.split('/').next().unwrap_or_default();
    let host = host.trim_start_matches("www.").trim_end_matches('.');
    let valid = host.contains('.')
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
    if valid {
        Ok(host.to_string())
    } else {
        Err(format!("invalid domain {raw:?}"))
    }
}

fn domain_of(url: &str) -> Option<String> {
    let host = reqwest::Url::parse(url).ok()?.host_str()?.to_lowercase();
    Some(host.trim_start_matches("www.").to_string())
}

fn host_matches(url: &str, domain: &str) -> bool {
    domain_of(url).is_some_and(|h| h == domain || h.ends_with(&format!(".{domain}")))
}

fn domain_allowed(url: &str, opts: &SearchOptions) -> bool {
    let included = opts.include_domains.is_empty()
        || opts.include_domains.iter().any(|d| host_matches(url, d));
    included && !opts.exclude_domains.iter().any(|d| host_matches(url, d))
}

/// The query as sent to a backend: domain filters become `site:` operators.
fn effective_query(query: &str, opts: &SearchOptions) -> String {
    let mut q = query.trim().to_string();
    match opts.include_domains.as_slice() {
        [] => {}
        [one] => q.push_str(&format!(" site:{one}")),
        many => {
            let sites: Vec<String> = many.iter().map(|d| format!("site:{d}")).collect();
            q.push_str(&format!(" ({})", sites.join(" OR ")));
        }
    }
    for d in &opts.exclude_domains {
        q.push_str(&format!(" -site:{d}"));
    }
    q
}

// ─── URL Cache ──────────────────────────────────────────────────

struct CacheEntry {
    page: FetchedPage,
    fetched_at: Instant,
}

/// Simple in-process URL cache with TTL eviction.
/// Avoids re-fetching the same page within a session (15 min TTL).
static FETCH_CACHE: std::sync::LazyLock<Mutex<HashMap<String, CacheEntry>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

fn cache_get(url: &str) -> Option<FetchedPage> {
    let cache = FETCH_CACHE.lock().ok()?;
    let entry = cache.get(url)?;
    (entry.fetched_at.elapsed() < CACHE_TTL).then(|| entry.page.clone())
}

fn cache_put(url: &str, page: &FetchedPage) {
    if let Ok(mut cache) = FETCH_CACHE.lock() {
        // Evict stale entries if cache is getting large (>100 entries)
        if cache.len() > 100 {
            cache.retain(|_, v| v.fetched_at.elapsed() < CACHE_TTL);
        }
        cache.insert(
            url.to_string(),
            CacheEntry {
                page: page.clone(),
                fetched_at: Instant::now(),
            },
        );
    }
}

// ─── Shared HTTP Clients ───────────────────────────────────────

/// For search backends and Jina. Per-request timeouts are set at each call site.
static HTTP_CLIENT: std::sync::LazyLock<Client> = std::sync::LazyLock::new(|| {
    Client::builder()
        .user_agent(USER_AGENT)
        .redirect(reqwest::redirect::Policy::limited(MAX_REDIRECTS))
        .build()
        .expect("failed to build reqwest::Client")
});

/// For model-chosen URLs: each redirect hop must pass the same SSRF rules as
/// the original URL, so a public page can't bounce the fetch to 127.0.0.1.
/// Hostnames are resolved by `GuardedResolver`, which refuses non-public
/// answers and hands the connector the validated addresses, so DNS rebinding
/// cannot change where the socket goes.
static FETCH_CLIENT: std::sync::LazyLock<Client> =
    std::sync::LazyLock::new(|| fetch_client(FETCH_TIMEOUT, validate_url));

/// A fetch client whose every redirect hop must pass `allow`. Production
/// passes `validate_url`; tests wrap it to admit only their local server.
fn fetch_client(
    timeout: Duration,
    allow: impl Fn(&str) -> Result<()> + Send + Sync + 'static,
) -> Client {
    fetch_client_with_resolver(timeout, allow, GuardedResolver::system())
}

fn fetch_client_with_resolver(
    timeout: Duration,
    allow: impl Fn(&str) -> Result<()> + Send + Sync + 'static,
    resolver: GuardedResolver,
) -> Client {
    proxy_policy(Client::builder(), use_proxy_from_env())
        .timeout(timeout)
        .dns_resolver(std::sync::Arc::new(resolver))
        .user_agent(USER_AGENT)
        .redirect(reqwest::redirect::Policy::custom(move |attempt| {
            // `previous()` includes the initial URL, which is not a redirect.
            if attempt.previous().len() > MAX_REDIRECTS {
                return attempt.error("too many redirects");
            }
            match allow(attempt.url().as_str()) {
                Ok(()) => attempt.follow(),
                Err(e) => attempt.error(format!("redirect blocked: {e}")),
            }
        }))
        .build()
        .expect("failed to build reqwest::Client")
}

/// Opt-in for fetching through `HTTP_PROXY`/`HTTPS_PROXY`/`ALL_PROXY` or the system proxy.
const USE_PROXY_ENV: &str = "HQ_WEB_FETCH_USE_PROXY";

/// A proxy resolves names itself, so the pinned resolver and its address checks never
/// run for requests sent through one. Default to no proxy; warn once if it is opted into.
fn use_proxy_from_env() -> bool {
    static WARN: std::sync::Once = std::sync::Once::new();
    let on = std::env::var(USE_PROXY_ENV).is_ok_and(|v| v == "1");
    if on {
        WARN.call_once(|| {
            tracing::warn!(
                "{USE_PROXY_ENV}=1: web_fetch goes through the configured proxy, so SSRF address \
                 pinning does not apply; the proxy must enforce its own egress policy"
            );
        });
    }
    on
}

fn proxy_policy(builder: reqwest::ClientBuilder, use_proxy: bool) -> reqwest::ClientBuilder {
    if use_proxy { builder } else { builder.no_proxy() }
}

/// The SSRF-guarded client for other tools that fetch a URL they did not choose
/// (every redirect hop and DNS answer is checked). Pair with [`check_public_url`].
pub fn guarded_client() -> &'static Client {
    &FETCH_CLIENT
}

/// Syntactic SSRF check for a URL about to go through [`guarded_client`].
pub fn check_public_url(url: &str) -> Result<()> {
    validate_url(url)
}

fn get_client() -> &'static Client {
    &HTTP_CLIENT
}

// ─── Backend Health (circuit breaker) ──────────────────────────
//
// A single hard failure (e.g. SearxNG not running, Brave 429) used to cost
// every subsequent call the same timeout/error again. This remembers the
// last failure per backend endpoint and skips straight to the next backend
// until a cooldown expires, instead of re-probing one already known to be down.

const SEARXNG_BACKOFF_BASE: Duration = Duration::from_secs(10);
const SEARXNG_BACKOFF_CAP: Duration = Duration::from_secs(5 * 60);
const BRAVE_RATE_LIMIT_BACKOFF_BASE: Duration = Duration::from_secs(30);
const BRAVE_RATE_LIMIT_BACKOFF_CAP: Duration = Duration::from_secs(30 * 60);
const BRAVE_ERROR_BACKOFF_BASE: Duration = Duration::from_secs(15);
const BRAVE_ERROR_BACKOFF_CAP: Duration = Duration::from_secs(5 * 60);

#[derive(Default)]
struct BackendHealth {
    unreachable_until: Option<Instant>,
    consecutive_failures: u32,
}

impl BackendHealth {
    fn is_cooling_down(&self) -> bool {
        self.unreachable_until.is_some_and(|t| Instant::now() < t)
    }

    fn record_success(&mut self) {
        self.consecutive_failures = 0;
        self.unreachable_until = None;
    }

    /// Exponential backoff from `base`, doubling per consecutive failure, capped at `cap`.
    fn record_failure(&mut self, base: Duration, cap: Duration) {
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        let shift = self.consecutive_failures.min(16);
        let secs = base.as_secs().saturating_mul(1u64 << shift);
        self.unreachable_until = Some(Instant::now() + Duration::from_secs(secs).min(cap));
    }
}

/// Keyed by endpoint, so two SearxNG instances (or a test server) never share a cooldown.
static HEALTH: std::sync::LazyLock<Mutex<HashMap<String, BackendHealth>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

fn with_health<T>(key: &str, f: impl FnOnce(&mut BackendHealth) -> T) -> Option<T> {
    let mut map = HEALTH.lock().ok()?;
    Some(f(map.entry(key.to_string()).or_default()))
}

// ─── Search Backends ────────────────────────────────────────────

/// Why a backend call failed. `rate_limited` picks the longer backoff.
struct ProviderError {
    reason: String,
    rate_limited: bool,
}

impl ProviderError {
    fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
            rate_limited: false,
        }
    }
}

/// One page as a backend returned it, before domain filtering.
struct Page {
    results: Vec<SearchResult>,
    next_page: Option<u32>,
}

/// Send a backend request and decode JSON, classifying every failure.
/// Credentials live in headers, never in the reason text.
async fn get_json(request: RequestBuilder) -> Result<Value, ProviderError> {
    let resp = request.send().await.map_err(|e| {
        let kind = if e.is_timeout() {
            "request timed out"
        } else if e.is_connect() {
            "connection failed"
        } else {
            "request failed"
        };
        ProviderError::new(format!("{kind}: {}", e.without_url()))
    })?;
    decode_json(resp).await
}

/// Classify a backend response that did arrive: status first, then JSON.
async fn decode_json(resp: reqwest::Response) -> Result<Value, ProviderError> {
    let status = resp.status();
    if status.as_u16() == 429 {
        return Err(ProviderError {
            reason: "rate limited (HTTP 429)".into(),
            rate_limited: true,
        });
    }
    if status.as_u16() == 401 || status.as_u16() == 403 {
        return Err(ProviderError::new(format!(
            "invalid credentials or access denied (HTTP {status})"
        )));
    }
    if !status.is_success() {
        return Err(ProviderError::new(format!("HTTP {status}")));
    }
    let body = resp
        .text()
        .await
        .map_err(|e| ProviderError::new(format!("failed reading body: {}", e.without_url())))?;
    serde_json::from_str(&body).map_err(|e| ProviderError::new(format!("malformed JSON: {e}")))
}

fn searxng_params(query: &str, opts: &SearchOptions) -> Vec<(&'static str, String)> {
    let mut params = vec![
        ("q", effective_query(query, opts)),
        ("format", "json".to_string()),
        ("pageno", opts.page.to_string()),
    ];
    if let Some(f) = opts.freshness {
        let range = match f {
            Freshness::Day => "day",
            Freshness::Week => "week",
            Freshness::Month => "month",
            Freshness::Year => "year",
        };
        params.push(("time_range", range.to_string()));
    }
    match (&opts.language, &opts.country) {
        (Some(l), Some(c)) => params.push(("language", format!("{l}-{c}"))),
        (Some(l), None) => params.push(("language", l.clone())),
        _ => {}
    }
    if let Some(c) = opts.category {
        let name = match c {
            Category::General => "general",
            Category::News => "news",
            Category::Science => "science",
        };
        params.push(("categories", name.to_string()));
    }
    params
}

fn searxng_unsupported(opts: &SearchOptions) -> Vec<String> {
    if opts.country.is_some() && opts.language.is_none() {
        return vec!["country (SearxNG only takes a locale: pass language too, e.g. language=en with country=US)".into()];
    }
    Vec::new()
}

/// Search a self-hosted SearxNG instance's JSON API. This is an
/// operator-configured local backend, not a model-supplied URL, so it
/// deliberately bypasses `validate_url`/`upgrade_url` (SSRF policing for
/// fetch targets a model chooses — a different trust boundary — and
/// `upgrade_url` would break plain-http localhost instances anyway).
/// Requires `search.formats: [html, json]` in the instance's settings.yml
/// (see `scripts/searxng/settings.yml`), since SearxNG 403s the JSON format
/// by default.
async fn searxng_request(
    base_url: &str,
    query: &str,
    opts: &SearchOptions,
) -> Result<Value, ProviderError> {
    let url = format!("{}/search", base_url.trim_end_matches('/'));
    get_json(get_client().get(url).query(&searxng_params(query, opts))).await
}

/// Parse a SearxNG `/search?format=json` body. A body without a `results`
/// array is a broken backend, not an empty result set.
fn parse_searxng_results(json: &Value, page: u32) -> Result<Page, String> {
    let items = json
        .get("results")
        .and_then(Value::as_array)
        .ok_or("response has no `results` array")?;
    let results: Vec<SearchResult> = items
        .iter()
        .filter(|r| r["url"].as_str().is_some_and(|u| !u.is_empty()))
        .enumerate()
        .map(|(i, r)| {
            let url = r["url"].as_str().unwrap_or_default().to_string();
            let engines = match r["engines"].as_array() {
                Some(list) => list
                    .iter()
                    .filter_map(Value::as_str)
                    .map(String::from)
                    .collect(),
                None => r["engine"].as_str().map(String::from).into_iter().collect(),
            };
            SearchResult {
                title: r["title"].as_str().unwrap_or_default().to_string(),
                snippet: r["content"].as_str().unwrap_or_default().to_string(),
                position: i + 1,
                provider: "searxng".into(),
                domain: domain_of(&url),
                published: non_empty_str(&r["publishedDate"]),
                engines,
                url,
            }
        })
        .collect();
    // SearxNG has no "more results" flag; an empty page is the end.
    let next_page = (!results.is_empty()).then_some(page + 1);
    Ok(Page { results, next_page })
}

fn non_empty_str(v: &Value) -> Option<String> {
    v.as_str().filter(|s| !s.is_empty()).map(String::from)
}

fn brave_section(opts: &SearchOptions) -> &'static str {
    if opts.category == Some(Category::News) {
        "news"
    } else {
        "web"
    }
}

fn brave_params(query: &str, opts: &SearchOptions) -> Vec<(&'static str, String)> {
    let mut params = vec![
        ("q", effective_query(query, opts)),
        ("count", opts.max_results.to_string()),
        ("offset", opts.page.saturating_sub(1).to_string()),
        ("result_filter", brave_section(opts).to_string()),
    ];
    if let Some(f) = opts.freshness {
        let code = match f {
            Freshness::Day => "pd",
            Freshness::Week => "pw",
            Freshness::Month => "pm",
            Freshness::Year => "py",
        };
        params.push(("freshness", code.to_string()));
    }
    if let Some(c) = &opts.country {
        params.push(("country", c.clone()));
    }
    if let Some(l) = &opts.language {
        params.push(("search_lang", l.clone()));
    }
    params
}

fn brave_unsupported(opts: &SearchOptions) -> Vec<String> {
    if opts.category == Some(Category::Science) {
        return vec!["category=science (Brave supports general and news)".into()];
    }
    Vec::new()
}

async fn brave_request(
    endpoint: &str,
    api_key: &str,
    query: &str,
    opts: &SearchOptions,
) -> Result<Value, ProviderError> {
    let request = get_client()
        .get(endpoint)
        .header("Accept", "application/json")
        .header("X-Subscription-Token", api_key)
        .query(&brave_params(query, opts));
    get_json(request).await
}

/// Parse a Brave web-search body. Brave omits the section entirely when it
/// has no hits, so a well-formed `"type": "search"` body without it is empty.
fn parse_brave_results(json: &Value, opts: &SearchOptions) -> Result<Page, String> {
    let section = brave_section(opts);
    let items = match json
        .get(section)
        .and_then(|s| s.get("results"))
        .and_then(Value::as_array)
    {
        Some(items) => items.as_slice(),
        None if json.get("type").and_then(Value::as_str) == Some("search") => &[],
        None => return Err(format!("response has no `{section}.results` array")),
    };
    let results: Vec<SearchResult> = items
        .iter()
        .filter(|r| r["url"].as_str().is_some_and(|u| !u.is_empty()))
        .enumerate()
        .map(|(i, r)| {
            let url = r["url"].as_str().unwrap_or_default().to_string();
            SearchResult {
                title: r["title"].as_str().unwrap_or_default().to_string(),
                snippet: r["description"].as_str().unwrap_or_default().to_string(),
                position: i + 1,
                provider: "brave".into(),
                domain: domain_of(&url),
                published: non_empty_str(&r["page_age"]).or_else(|| non_empty_str(&r["age"])),
                engines: Vec::new(),
                url,
            }
        })
        .collect();
    let more = json["query"]["more_results_available"]
        .as_bool()
        .unwrap_or(false);
    let next_page =
        (more && !results.is_empty() && opts.page < BRAVE_MAX_PAGE).then_some(opts.page + 1);
    Ok(Page { results, next_page })
}

/// Per-backend identity, time budget and cooldown policy.
struct Backend<'a> {
    key: &'a str,
    label: &'static str,
    budget: Duration,
    /// Backoff (base, cap) for a failure; the flag is "rate limited".
    backoff: fn(bool) -> (Duration, Duration),
}

fn searxng_backoff(_rate_limited: bool) -> (Duration, Duration) {
    (SEARXNG_BACKOFF_BASE, SEARXNG_BACKOFF_CAP)
}

fn brave_backoff(rate_limited: bool) -> (Duration, Duration) {
    if rate_limited {
        (BRAVE_RATE_LIMIT_BACKOFF_BASE, BRAVE_RATE_LIMIT_BACKOFF_CAP)
    } else {
        (BRAVE_ERROR_BACKOFF_BASE, BRAVE_ERROR_BACKOFF_CAP)
    }
}

/// Run one backend under its budget (capped by the overall deadline), record
/// the outcome in its health entry and in `attempts`, and return its page.
async fn try_backend<Fut>(
    backend: &Backend<'_>,
    deadline: Instant,
    attempts: &mut Vec<ProviderAttempt>,
    request: impl FnOnce() -> Fut,
    parse: impl FnOnce(&Value) -> Result<Page, String>,
) -> Option<Page>
where
    Fut: Future<Output = Result<Value, ProviderError>>,
{
    let mut note = |outcome: String| {
        attempts.push(ProviderAttempt {
            provider: backend.label.into(),
            outcome,
        })
    };
    if with_health(backend.key, |h| h.is_cooling_down()).unwrap_or(false) {
        note("skipped: cooling down after recent failures".into());
        return None;
    }
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        note("skipped: overall search deadline reached".into());
        return None;
    }
    let budget = backend.budget.min(remaining);
    let failure = match tokio::time::timeout(budget, request()).await {
        Err(_) => ProviderError::new(format!("timed out after {:.1}s", budget.as_secs_f32())),
        Ok(Err(e)) => e,
        Ok(Ok(json)) => match parse(&json) {
            Ok(page) => {
                with_health(backend.key, BackendHealth::record_success);
                note(format!("ok, {} results", page.results.len()));
                return Some(page);
            }
            Err(reason) => ProviderError::new(format!("invalid response: {reason}")),
        },
    };
    let (base, cap) = (backend.backoff)(failure.rate_limited);
    with_health(backend.key, |h| h.record_failure(base, cap));
    debug!(backend = backend.label, reason = %failure.reason, "web search backend failed");
    note(failure.reason);
    None
}

/// Time budgets for the fallback chain; only tests shrink them.
struct Budgets {
    total: Duration,
    searxng: Duration,
    brave: Duration,
}

const DEFAULT_BUDGETS: Budgets = Budgets {
    total: SEARCH_DEADLINE,
    searxng: SEARXNG_TIMEOUT,
    brave: BRAVE_TIMEOUT,
};

/// Search the web: tries a self-hosted SearxNG instance first (free, no API
/// key), falling back to the Brave Search API on failure or empty results if
/// `brave_api_key` is set. Errors with remediation guidance if neither
/// backend is configured, and with every attempt's reason if all fail.
pub async fn web_search(
    query: &str,
    opts: &SearchOptions,
    searxng_url: Option<&str>,
    brave_api_key: Option<&str>,
) -> Result<WebSearchResults> {
    let searxng_url = searxng_url.map(str::trim).filter(|s| !s.is_empty());
    let brave = brave_api_key
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|key| (BRAVE_ENDPOINT, key));
    search_chain(query, opts, searxng_url, brave, &DEFAULT_BUDGETS).await
}

async fn search_chain(
    query: &str,
    opts: &SearchOptions,
    searxng_url: Option<&str>,
    brave: Option<(&str, &str)>,
    budgets: &Budgets,
) -> Result<WebSearchResults> {
    if searxng_url.is_none() && brave.is_none() {
        return Err(no_backend_error());
    }
    let deadline = Instant::now() + budgets.total;
    let mut attempts = Vec::new();
    let mut answered: Option<(&'static str, Page, Vec<String>)> = None;

    if let Some(base) = searxng_url {
        let backend = Backend {
            key: base,
            label: "searxng",
            budget: budgets.searxng,
            backoff: searxng_backoff,
        };
        debug!(query = %query, base, "searching SearxNG");
        let page = try_backend(
            &backend,
            deadline,
            &mut attempts,
            || searxng_request(base, query, opts),
            |json| parse_searxng_results(json, opts.page),
        )
        .await;
        if let Some(page) = page {
            if !page.results.is_empty() || brave.is_none() {
                return Ok(finish(
                    query,
                    opts,
                    "searxng",
                    page,
                    searxng_unsupported(opts),
                    attempts,
                ));
            }
            answered = Some(("searxng", page, searxng_unsupported(opts)));
        }
    }

    if let Some((endpoint, api_key)) = brave {
        if opts.page > BRAVE_MAX_PAGE {
            attempts.push(ProviderAttempt {
                provider: "brave".into(),
                outcome: format!("skipped: Brave serves pages 1..={BRAVE_MAX_PAGE} only"),
            });
        } else {
            let backend = Backend {
                key: endpoint,
                label: "brave",
                budget: budgets.brave,
                backoff: brave_backoff,
            };
            debug!(query = %query, "searching Brave Search");
            let page = try_backend(
                &backend,
                deadline,
                &mut attempts,
                || brave_request(endpoint, api_key, query, opts),
                |json| parse_brave_results(json, opts),
            )
            .await;
            if let Some(page) = page {
                answered = Some(("brave", page, brave_unsupported(opts)));
            }
        }
    }

    match answered {
        Some((backend, page, unsupported)) => {
            Ok(finish(query, opts, backend, page, unsupported, attempts))
        }
        None => {
            let summary: Vec<String> = attempts
                .iter()
                .map(|a| format!("{}: {}", a.provider, a.outcome))
                .collect();
            bail!("All web search backends failed ({})", summary.join("; "))
        }
    }
}

fn finish(
    query: &str,
    opts: &SearchOptions,
    backend: &str,
    page: Page,
    unsupported_filters: Vec<String>,
    attempts: Vec<ProviderAttempt>,
) -> WebSearchResults {
    let results = page
        .results
        .into_iter()
        .filter(|r| domain_allowed(&r.url, opts))
        .take(opts.max_results)
        .collect();
    WebSearchResults {
        query: query.to_string(),
        results,
        backend: Some(backend.to_string()),
        page: opts.page,
        next_page: page.next_page,
        attempts,
        unsupported_filters,
    }
}

fn no_backend_error() -> anyhow::Error {
    // Only name the setup script if this host actually has a checkout to run
    // it from — naming a path that doesn't exist here is exactly FR-005's
    // complaint (the message was unactionable on a checkout-less VPS).
    if hq_core::machine::agent_hq_checkout_path().is_some() {
        return anyhow::anyhow!(
            "No web search backend available. Run `scripts/setup-searxng.sh` for a free \
             self-hosted search backend, or set `brave_api_key` in ~/.hq/config.yaml \
             (or HQ_BRAVE_API_KEY) for the paid Brave Search fallback."
        );
    }
    anyhow::anyhow!(
        "No web search backend available, and this host has no source checkout to run a \
         setup script from. Ask the operator to provision a backend directly: point \
         `searxng_url` at a reachable SearxNG instance, or set `brave_api_key` \
         (or HQ_BRAVE_API_KEY) for the paid Brave Search fallback."
    )
}

/// JSON schema shared by the MCP and agent `web_search` tools.
pub fn web_search_parameters() -> Value {
    json!({
        "type": "object",
        "required": ["query"],
        "properties": {
            "query": {
                "type": "string",
                "description": "The search query (min 2 characters)"
            },
            "max_results": {
                "type": "integer",
                "description": "Maximum number of results (default 5, max 20). Domain filters can return fewer."
            },
            "page": {
                "type": "integer",
                "description": "1-based results page (default 1). Use next_page from a previous response. Brave serves pages 1-10."
            },
            "freshness": {
                "type": "string",
                "enum": ["day", "week", "month", "year"],
                "description": "Only results from the last day/week/month/year"
            },
            "language": {
                "type": "string",
                "description": "2-letter language code, e.g. \"en\""
            },
            "country": {
                "type": "string",
                "description": "2-letter country code, e.g. \"US\". SearxNG needs language as well."
            },
            "category": {
                "type": "string",
                "enum": ["general", "news", "science"],
                "description": "Result category (Brave supports general and news)"
            },
            "include_domains": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Only results from these domains (and their subdomains), e.g. [\"docs.rs\"]. Max 10."
            },
            "exclude_domains": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Drop results from these domains. Max 10."
            }
        }
    })
}

// ─── Web Fetch ──────────────────────────────────────────────────

const JINA_READER_BASE: &str = "https://r.jina.ai/";
// A genuinely empty extraction (nav labels only, no real copy) vs. a real
// page that just happens to be short, like example.com's ~150-char body —
// verified against example.com live: 150ish chars extracted from 559 bytes
// of HTML, which must NOT trip this on length alone.
const SPA_NEAR_EMPTY_TEXT_CHARS: usize = 40;
const SPA_TEXT_TO_HTML_RATIO: f64 = 0.03;
const SPA_MIN_HTML_BYTES_FOR_RATIO_CHECK: usize = 500;

const METHOD_HTML: &str = "html-to-text";
const METHOD_TEXT: &str = "plain-text";
const METHOD_PDF_TEXT: &str = "pdf-text-layer";
const METHOD_PDF_OCR: &str = "pdf-ocr";
const METHOD_JINA: &str = "jina-reader";

/// A fetched page with its provenance. `content` may be truncated; the cache
/// keeps the full text.
#[derive(Debug, Clone, Serialize)]
pub struct FetchedPage {
    /// The requested URL after the https upgrade.
    pub url: String,
    /// Where the content actually came from after redirects.
    pub final_url: String,
    pub content_type: String,
    /// `html-to-text`, `plain-text`, `pdf-text-layer`, `pdf-ocr` or `jina-reader`.
    pub method: &'static str,
    pub content: String,
    pub total_chars: usize,
    pub truncated: bool,
    /// Partial or degraded extraction, e.g. OCR limits or a third-party render.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

impl FetchedPage {
    fn truncate_to(mut self, max_chars: usize) -> Self {
        if self.content.len() > max_chars {
            let boundary = self.content.floor_char_boundary(max_chars);
            self.content.truncate(boundary);
            self.truncated = true;
        }
        self
    }

    /// Text for agent consumption: a provenance line, the content, and a truncation marker.
    pub fn to_text(&self) -> String {
        let mut out = format!(
            "[Fetched {} | {} | extracted via {}]\n",
            self.final_url,
            if self.content_type.is_empty() {
                "unknown type"
            } else {
                &self.content_type
            },
            self.method
        );
        for note in &self.notes {
            out.push_str(&format!("[Note: {note}]\n"));
        }
        out.push('\n');
        out.push_str(&self.content);
        if self.truncated {
            out.push_str(&format!(
                "\n\n[Content truncated at {} chars ({} total)]",
                self.content.len(),
                self.total_chars
            ));
        }
        out
    }
}

/// Heuristic for "this HTML page's extracted text looks like an empty
/// client-rendered shell rather than real content" — e.g. a Next.js/React
/// SPA that only fills its DOM after JS runs, which a plain GET + HTML-to-text
/// pass never executes. Two independent signals: near-zero text regardless of
/// page size, or (only once the page is big enough for the ratio to be
/// meaningful) too little text relative to a large raw HTML payload — the
/// signature of a big JS bundle with almost no server-rendered copy. A short
/// but real page (small HTML, modest text, healthy ratio) must trip neither.
fn looks_like_empty_spa_shell(extracted_text: &str, raw_html_len: usize) -> bool {
    let trimmed_len = extracted_text.trim().chars().count();
    if trimmed_len < SPA_NEAR_EMPTY_TEXT_CHARS {
        return true;
    }
    if raw_html_len > SPA_MIN_HTML_BYTES_FOR_RATIO_CHECK {
        let ratio = extracted_text.trim().len() as f64 / raw_html_len as f64;
        if ratio < SPA_TEXT_TO_HTML_RATIO {
            return true;
        }
    }
    false
}

/// Render a URL through Jina Reader, which executes JS server-side and
/// returns markdown. No API key; only the URL is sent.
async fn fetch_via_jina(jina_base: &str, url: &str) -> Result<String> {
    let resp = get_client()
        .get(format!("{jina_base}{url}"))
        .timeout(JINA_TIMEOUT)
        .send()
        .await?;
    if !resp.status().is_success() {
        bail!("Jina Reader returned HTTP {}", resp.status());
    }
    let bytes = read_limited(resp, MAX_BODY_BYTES).await?;
    Ok(String::from_utf8_lossy(&bytes).to_string())
}

#[derive(Debug, PartialEq, Eq)]
enum BodyKind {
    Html,
    Pdf,
    Text,
    Binary,
}

/// Media types rejected from headers alone, before downloading the body.
fn is_binary_media_type(content_type: &str) -> bool {
    ["image/", "audio/", "video/", "application/zip", "font/"]
        .iter()
        .any(|t| content_type.contains(t))
}

/// PDFs are recognised by magic bytes as well as by type, since many servers
/// label them `application/octet-stream`.
fn classify_body(content_type: &str, body: &[u8]) -> BodyKind {
    if body.starts_with(b"%PDF-") || content_type.contains("application/pdf") {
        return BodyKind::Pdf;
    }
    if content_type.contains("text/html") || content_type.contains("application/xhtml") {
        return BodyKind::Html;
    }
    const SNIFF_BYTES: usize = 1024;
    let looks_binary = body.iter().take(SNIFF_BYTES).any(|b| *b == 0);
    if is_binary_media_type(content_type)
        || content_type.contains("application/octet-stream")
        || looks_binary
    {
        return BodyKind::Binary;
    }
    BodyKind::Text
}

/// Read a body without ever buffering more than `limit` bytes.
async fn read_limited(mut resp: reqwest::Response, limit: usize) -> Result<Vec<u8>> {
    if let Some(len) = resp.content_length()
        && len as usize > limit
    {
        bail!("Response body too large: {len} bytes (limit: {limit} bytes)");
    }
    let mut buf = Vec::new();
    while let Some(chunk) = resp.chunk().await? {
        if buf.len() + chunk.len() > limit {
            bail!("Response body too large: over {limit} bytes");
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(buf)
}

/// Extract text from PDF bytes via `hq-convert`: the text layer first, OCR
/// when that comes back empty (a scanned PDF).
async fn extract_pdf(bytes: &[u8]) -> Result<(String, &'static str)> {
    let path = std::env::temp_dir().join(format!("hq-web-fetch-{}.pdf", uuid::Uuid::new_v4()));
    tokio::fs::write(&path, bytes).await?;
    let result = extract_pdf_file(&path).await;
    let _ = tokio::fs::remove_file(&path).await;
    result
}

async fn extract_pdf_file(path: &Path) -> Result<(String, &'static str)> {
    use hq_convert::{InboundConverter, OcrEngine};

    let text_layer = match InboundConverter::new() {
        Ok(converter) => tokio::time::timeout(PDF_TEXT_TIMEOUT, converter.convert(path))
            .await
            .ok()
            .and_then(Result::ok),
        Err(_) => None,
    };
    if let Some(text) = text_layer.filter(|t| t.trim().chars().count() >= MIN_PDF_TEXT_CHARS) {
        return Ok((text, METHOD_PDF_TEXT));
    }
    match tokio::time::timeout(PDF_OCR_TIMEOUT, OcrEngine::extract_pdf(path)).await {
        Err(_) => bail!(
            "PDF has no text layer and OCR timed out after {}s",
            PDF_OCR_TIMEOUT.as_secs()
        ),
        Ok(Ok(text)) if !text.trim().is_empty() => Ok((text, METHOD_PDF_OCR)),
        Ok(Ok(_)) => bail!("PDF has no extractable text, even after OCR"),
        Ok(Err(e)) => bail!("PDF has no text layer and OCR is unavailable on this host: {e}"),
    }
}

/// Fetch a URL and return its text with provenance.
/// Caches results for 15 minutes. Auto-upgrades HTTP to HTTPS.
pub async fn web_fetch(url: &str, max_chars: usize) -> Result<FetchedPage> {
    let url = upgrade_url(url);
    validate_url(&url)?;

    if let Some(cached) = cache_get(&url) {
        debug!(url = %url, "web_fetch cache hit");
        return Ok(cached.truncate_to(max_chars));
    }

    let fetcher = Fetcher {
        client: &FETCH_CLIENT,
        timeout: FETCH_TIMEOUT,
        jina_base: JINA_READER_BASE,
    };
    let page = fetcher.fetch(&url).await?;
    cache_put(&url, &page);
    Ok(page.truncate_to(max_chars))
}

/// Where an uncached fetch goes. Tests point it at local servers.
struct Fetcher<'a> {
    client: &'a Client,
    /// Must match the timeout `client` was built with; only used in error text.
    timeout: Duration,
    jina_base: &'a str,
}

/// reqwest's `Display` for a redirect error drops the policy's reason
/// ("redirect blocked: ..."), which only survives in the source chain.
fn error_chain(e: &reqwest::Error) -> String {
    let mut text = e.to_string();
    let mut source = std::error::Error::source(e);
    while let Some(cause) = source {
        text.push_str(&format!(": {cause}"));
        source = cause.source();
    }
    text
}

impl Fetcher<'_> {
    async fn fetch(&self, url: &str) -> Result<FetchedPage> {
        debug!(url = %url, "fetching web page");
        let response = self.client.get(url).send().await.map_err(|e| {
            if e.is_timeout() {
                anyhow::anyhow!(
                    "Timed out after {}s fetching {url}",
                    self.timeout.as_secs_f32()
                )
            } else {
                anyhow::anyhow!("Fetch failed for {url}: {}", error_chain(&e))
            }
        })?;
        let status = response.status();
        if !status.is_success() {
            bail!("HTTP {} for {}", status, url);
        }
        let final_url = response.url().to_string();
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_lowercase();
        if is_binary_media_type(&content_type) {
            bail!(
                "Cannot extract text from binary content type: {}. Use bash + curl for binary downloads.",
                content_type
            );
        }

        let bytes = read_limited(response, MAX_BODY_BYTES).await?;
        let mut notes = Vec::new();
        let (text, method) = self
            .extract(url, &final_url, &content_type, &bytes, &mut notes)
            .await?;
        if text.trim().is_empty() {
            notes.push("extraction produced no text".into());
        }
        Ok(FetchedPage {
            url: url.to_string(),
            final_url,
            content_type,
            method,
            total_chars: text.len(),
            content: text,
            truncated: false,
            notes,
        })
    }

    async fn extract(
        &self,
        url: &str,
        final_url: &str,
        content_type: &str,
        bytes: &[u8],
        notes: &mut Vec<String>,
    ) -> Result<(String, &'static str)> {
        match classify_body(content_type, bytes) {
            BodyKind::Binary => bail!(
                "Cannot extract text from binary content ({}). Use bash + curl for binary downloads.",
                if content_type.is_empty() {
                    "no content type"
                } else {
                    content_type
                }
            ),
            BodyKind::Pdf => {
                let (text, method) = extract_pdf(bytes).await?;
                if method == METHOD_PDF_OCR {
                    notes.push("scanned PDF read with OCR; only the first pages are covered and text may contain recognition errors".into());
                }
                Ok((text, method))
            }
            BodyKind::Text => Ok((String::from_utf8_lossy(bytes).to_string(), METHOD_TEXT)),
            BodyKind::Html => Ok(self.extract_html(url, final_url, bytes, notes).await),
        }
    }

    /// JS-heavy SPAs often render a near-empty shell server-side, since this
    /// is a plain GET with no JS execution. Retry through Jina Reader when the
    /// text looks like that shell; keep the original extraction if Jina also
    /// comes up empty.
    async fn extract_html(
        &self,
        url: &str,
        final_url: &str,
        bytes: &[u8],
        notes: &mut Vec<String>,
    ) -> (String, &'static str) {
        let text = html_to_text(&String::from_utf8_lossy(bytes), HTML_TEXT_WIDTH);
        if !looks_like_empty_spa_shell(&text, bytes.len()) {
            return (text, METHOD_HTML);
        }
        debug!(url = %url, "web_fetch looks like an empty SPA shell, trying Jina Reader");
        match fetch_via_jina(self.jina_base, final_url).await {
            Ok(jina) if !jina.trim().is_empty() => {
                notes.push("page rendered by the third-party Jina Reader (r.jina.ai), which received only the URL".into());
                (jina, METHOD_JINA)
            }
            Ok(_) | Err(_) => {
                notes.push("page looks client-rendered and Jina Reader could not render it; text may be incomplete".into());
                (text, METHOD_HTML)
            }
        }
    }
}

/// Auto-upgrade http:// to https:// for safety.
fn upgrade_url(url: &str) -> String {
    if let Some(stripped) = url.strip_prefix("http://") {
        format!("https://{}", stripped)
    } else {
        url.to_string()
    }
}

/// Convert HTML to readable plain text.
pub fn html_to_text(html: &str, width: usize) -> String {
    html2text::from_read(html.as_bytes(), width).unwrap_or_else(|_| html.to_string())
}

// ─── Formatting ─────────────────────────────────────────────────

/// Format search results as a readable text block for agent consumption.
pub fn format_search_results(results: &WebSearchResults) -> String {
    let mut out = format!("## Search Results for: \"{}\"", results.query);
    if results.page > 1 {
        out.push_str(&format!(" (page {})", results.page));
    }
    if let Some(backend) = &results.backend {
        out.push_str(&format!(" via {backend}"));
    }
    out.push_str("\n\n");

    if !results.unsupported_filters.is_empty() {
        out.push_str(&format!(
            "Not applied by this backend: {}\n\n",
            results.unsupported_filters.join("; ")
        ));
    }

    if results.results.is_empty() {
        out.push_str("No results found.\n");
    }

    for (i, result) in results.results.iter().enumerate() {
        out.push_str(&format!("### {}. {}\n", i + 1, result.title));
        out.push_str(&format!("URL: {}\n", result.url));
        let mut meta: Vec<String> = result.domain.iter().cloned().collect();
        meta.extend(result.published.iter().cloned());
        if !result.engines.is_empty() {
            meta.push(format!("engines: {}", result.engines.join(", ")));
        }
        if !meta.is_empty() {
            out.push_str(&format!("Source: {}\n", meta.join(" | ")));
        }
        if !result.snippet.is_empty() {
            out.push_str(&result.snippet);
            out.push('\n');
        }
        out.push('\n');
    }

    if let Some(next) = results.next_page {
        out.push_str(&format!("More results: request page {next}.\n"));
    }
    let degraded = results
        .attempts
        .iter()
        .any(|a| !a.outcome.starts_with("ok"));
    if degraded {
        let lines: Vec<String> = results
            .attempts
            .iter()
            .map(|a| format!("{}: {}", a.provider, a.outcome))
            .collect();
        out.push_str(&format!("Backends tried: {}\n", lines.join("; ")));
    }

    out
}

// ─── HqTool: WebSearchHqTool ───────────────────────────────────

/// Tool-level deadlines, shared with hq-agent's own web tool wrappers.
pub const SEARCH_TOOL_TIMEOUT_MS: u64 = 30_000;
pub const FETCH_TOOL_TIMEOUT_MS: u64 = 95_000;

/// Web search tool exposed via MCP gateway. SearxNG primary, Brave fallback.
pub struct WebSearchHqTool {
    searxng_url: Option<String>,
    brave_api_key: Option<String>,
}

impl WebSearchHqTool {
    pub fn new(searxng_url: Option<String>, brave_api_key: Option<String>) -> Self {
        Self {
            searxng_url,
            brave_api_key,
        }
    }
}

#[async_trait]
impl HqTool for WebSearchHqTool {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        "Search the web for current information. Uses a self-hosted SearxNG instance when one is configured and reachable, falling back to Brave Search — check this host's MACHINE.md/session context for which backend is actually provisioned. Optional filters: freshness (day/week/month/year), language, country, category (general/news/science), include_domains/exclude_domains, and page for later results. Returns titles, URLs, snippets, source metadata, the answering backend, next_page, and any filter that backend could not apply. Example: {\"query\": \"tokio release notes\", \"freshness\": \"month\", \"include_domains\": [\"github.com\"]}."
    }

    fn parameters(&self) -> Value {
        web_search_parameters()
    }

    fn category(&self) -> &str {
        "web"
    }

    fn is_read_only(&self) -> bool {
        true
    }

    /// Outer safety net above `web_search`'s own 20s deadline for the whole
    /// SearxNG and Brave chain.
    fn timeout_ms(&self) -> Option<u64> {
        Some(SEARCH_TOOL_TIMEOUT_MS)
    }

    fn search_hint(&self) -> Option<&str> {
        Some("search the web for current information")
    }

    fn behavioral_prompt(&self) -> Option<&str> {
        Some(
            "After answering a question using web search results, include a 'Sources:' section \
             with markdown links to the pages you referenced. Use the current year in time-sensitive queries.",
        )
    }

    async fn validate(&self, args: &Value) -> ValidationResult {
        let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
        if query.len() < 2 {
            return ValidationResult::block("query must be at least 2 characters", 400);
        }
        if let Err(e) = SearchOptions::from_args(args) {
            return ValidationResult::block(e, 400);
        }
        ValidationResult::ok()
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: query"))?;
        let opts = SearchOptions::from_args(&args).map_err(anyhow::Error::msg)?;

        let results = web_search(
            query,
            &opts,
            self.searxng_url.as_deref(),
            self.brave_api_key.as_deref(),
        )
        .await?;

        Ok(json!({
            "text": format_search_results(&results),
            "result_count": results.results.len(),
            "results": results.results,
            "backend": results.backend,
            "page": results.page,
            "next_page": results.next_page,
            "attempts": results.attempts,
            "unsupported_filters": results.unsupported_filters,
        }))
    }
}

// ─── HqTool: WebFetchHqTool ────────────────────────────────────

/// Fetch a web page and extract clean text content. No API key required.
pub struct WebFetchHqTool;

#[async_trait]
impl HqTool for WebFetchHqTool {
    fn name(&self) -> &str {
        "web_fetch"
    }

    fn description(&self) -> &str {
        "Fetch a URL and extract clean text content. HTML is converted to text, PDFs are read from their text layer (OCR for scanned PDFs where the host has it), and pages that render client-side fall back to the third-party Jina Reader, which receives only the URL. Returns the content with final_url, content_type, method, truncation and notes on partial extraction. Images, audio, video and archives are rejected. No API key required."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "required": ["url"],
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to fetch (http or https only)"
                },
                "max_chars": {
                    "type": "integer",
                    "description": "Maximum characters to return (default 100000)"
                }
            }
        })
    }

    fn category(&self) -> &str {
        "web"
    }

    fn search_hint(&self) -> Option<&str> {
        Some("fetch web page or PDF and extract text content")
    }

    fn is_read_only(&self) -> bool {
        true
    }

    /// Outer safety net above `web_fetch`'s own limits: a 30s fetch plus
    /// either a 25s Jina render or a PDF's 15s text layer plus 45s of OCR.
    fn timeout_ms(&self) -> Option<u64> {
        Some(FETCH_TOOL_TIMEOUT_MS)
    }

    fn behavioral_prompt(&self) -> Option<&str> {
        Some(
            "Use 125-character max for direct quotes from fetched pages. \
             Use quotation marks for exact language. Never reproduce song lyrics or copyrighted content verbatim.",
        )
    }

    async fn validate(&self, args: &Value) -> ValidationResult {
        let url = args.get("url").and_then(|v| v.as_str()).unwrap_or("");
        if url.is_empty() {
            return ValidationResult::block("url is required", 400);
        }
        if let Err(e) = validate_url(url) {
            return ValidationResult::block(format!("{}", e), 400);
        }
        ValidationResult::ok()
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let url = args
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: url"))?;

        let max_chars = args
            .get("max_chars")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(DEFAULT_MAX_OUTPUT_CHARS);

        let page = web_fetch(url, max_chars).await?;

        Ok(json!({
            "url": page.url,
            "final_url": page.final_url,
            "content_type": page.content_type,
            "method": page.method,
            "chars": page.content.len(),
            "total_chars": page.total_chars,
            "truncated": page.truncated,
            "notes": page.notes,
            "content": page.content,
        }))
    }
}

// ─── Tests ──────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn test_html_to_text_basic() {
        let html = "<h1>Hello</h1><p>World</p>";
        let text = html_to_text(html, 80);
        assert!(text.contains("Hello"));
        assert!(text.contains("World"));
    }

    #[test]
    fn test_html_to_text_strips_scripts() {
        let html = "<p>Visible</p><script>alert('hidden')</script><p>Also visible</p>";
        let text = html_to_text(html, 80);
        assert!(text.contains("Visible"));
        assert!(text.contains("Also visible"));
        assert!(!text.contains("alert"));
    }

    #[test]
    fn spa_shell_heuristic_does_not_flag_a_short_but_real_page() {
        // Regression: example.com's real body is short (~150 chars from 559
        // bytes of HTML) and was incorrectly flagged as an empty SPA shell
        // by an earlier version of this heuristic that used an absolute
        // 300-char floor — caught live when web_fetch("https://example.com")
        // routed through Jina Reader even though it's a plain, fully
        // server-rendered page.
        let text = "Example Domain\n\nThis domain is for use in documentation examples \
                     without needing permission. You may use this domain in literature \
                     without prior coordination or asking for permission.\n\nMore information...";
        assert!(!looks_like_empty_spa_shell(text, 559));
    }

    #[test]
    fn spa_shell_heuristic_flags_a_large_page_with_almost_no_text() {
        // A big JS-bundle SPA shell: lots of HTML, almost no extracted text.
        let text = "Home Investment Partnerships Projects Team Contact";
        assert!(looks_like_empty_spa_shell(text, 280_785));
    }

    #[test]
    fn spa_shell_heuristic_flags_genuinely_empty_text_regardless_of_size() {
        assert!(looks_like_empty_spa_shell("", 100));
        assert!(looks_like_empty_spa_shell("   ", 10_000));
    }

    #[tokio::test]
    async fn no_backend_error_only_names_a_path_that_exists() {
        // Regression for FR-005: the error must not point at
        // scripts/setup-searxng.sh unless this host actually has a checkout
        // to run it from. This crate's own tests always run from a checkout
        // (CARGO_MANIFEST_DIR), so the message should name the script here.
        let err = web_search("test query", &SearchOptions::default(), None, None)
            .await
            .unwrap_err();
        let msg = err.to_string();
        if hq_core::machine::agent_hq_checkout_path().is_some() {
            assert!(msg.contains("scripts/setup-searxng.sh"), "{msg}");
        } else {
            assert!(!msg.contains("scripts/setup-searxng.sh"), "{msg}");
        }
        assert!(msg.contains("brave_api_key"), "{msg}");
    }

    #[test]
    fn test_validate_url_allows_https() {
        assert!(validate_url("https://example.com").is_ok());
        assert!(validate_url("http://example.com/page").is_ok());
    }

    #[test]
    fn test_validate_url_blocks_file_scheme() {
        assert!(validate_url("file:///etc/passwd").is_err());
    }

    #[test]
    fn test_validate_url_blocks_localhost() {
        assert!(validate_url("http://localhost/admin").is_err());
        assert!(validate_url("http://127.0.0.1/admin").is_err());
        assert!(validate_url("http://0.0.0.0/").is_err());
        assert!(validate_url("http://[::1]/").is_err());
    }

    #[test]
    fn private_ipv6_ranges_are_blocked() {
        for ip in ["fd12::1", "fe80::1", "::ffff:10.0.0.1", "::ffff:127.0.0.1"] {
            assert!(is_non_public_ip(&ip.parse().unwrap()), "{ip} should be private");
        }
        assert!(!is_non_public_ip(&"2606:4700::1111".parse().unwrap()));
        assert!(!is_non_public_ip(&"::ffff:8.8.8.8".parse().unwrap()));
    }

    #[test]
    fn test_validate_url_blocks_private_ips() {
        assert!(validate_url("http://192.168.1.1/").is_err());
        assert!(validate_url("http://10.0.0.1/").is_err());
        assert!(validate_url("http://172.16.0.1/").is_err());
    }

    fn result(url: &str) -> SearchResult {
        SearchResult {
            title: "Rust Language".to_string(),
            url: url.to_string(),
            snippet: "A systems programming language".to_string(),
            position: 1,
            provider: "searxng".into(),
            domain: domain_of(url),
            published: Some("2026-09-01".into()),
            engines: vec!["google".into(), "bing".into()],
        }
    }

    fn results(items: Vec<SearchResult>) -> WebSearchResults {
        WebSearchResults {
            query: "rust lang".to_string(),
            results: items,
            backend: Some("searxng".into()),
            page: 1,
            next_page: None,
            attempts: vec![],
            unsupported_filters: vec![],
        }
    }

    #[test]
    fn test_format_search_results_empty() {
        let text = format_search_results(&results(vec![]));
        assert!(text.contains("No results found"));
    }

    #[test]
    fn test_format_search_results_with_items() {
        let mut r = results(vec![result("https://www.rust-lang.org/learn")]);
        r.next_page = Some(2);
        r.unsupported_filters = vec!["country".into()];
        r.attempts = vec![
            ProviderAttempt {
                provider: "searxng".into(),
                outcome: "timed out after 5.0s".into(),
            },
            ProviderAttempt {
                provider: "brave".into(),
                outcome: "ok, 1 results".into(),
            },
        ];
        let text = format_search_results(&r);
        assert!(text.contains("Rust Language"));
        assert!(
            text.contains("Source: rust-lang.org | 2026-09-01 | engines: google, bing"),
            "{text}"
        );
        assert!(text.contains("request page 2"));
        assert!(text.contains("Not applied by this backend: country"));
        assert!(text.contains("searxng: timed out"));
    }

    #[test]
    fn old_serialized_results_still_deserialize() {
        let old = json!({"query": "q", "results": [{"title": "t", "url": "https://a.com", "snippet": "s"}]});
        let parsed: WebSearchResults = serde_json::from_value(old).unwrap();
        assert_eq!(parsed.page, 1);
        assert_eq!(parsed.results[0].position, 0);
    }

    #[test]
    fn test_parse_searxng_results_happy_path() {
        let json = json!({
            "results": [
                {
                    "title": "Rust Language",
                    "url": "https://rust-lang.org",
                    "content": "A systems programming language",
                    "engines": ["google", "duckduckgo"],
                    "publishedDate": "2026-09-20T00:00:00"
                },
                { "title": "No date", "url": "https://b.com", "content": "", "engine": "bing", "publishedDate": null }
            ]
        });
        let page = parse_searxng_results(&json, 1).unwrap();
        assert_eq!(page.results.len(), 2);
        let first = &page.results[0];
        assert_eq!(first.title, "Rust Language");
        assert_eq!(first.snippet, "A systems programming language");
        assert_eq!(first.position, 1);
        assert_eq!(first.provider, "searxng");
        assert_eq!(first.domain.as_deref(), Some("rust-lang.org"));
        assert_eq!(first.published.as_deref(), Some("2026-09-20T00:00:00"));
        assert_eq!(first.engines, vec!["google", "duckduckgo"]);
        assert_eq!(page.results[1].published, None);
        assert_eq!(page.results[1].engines, vec!["bing"]);
        assert_eq!(page.next_page, Some(2));
    }

    #[test]
    fn test_parse_searxng_empty_page_ends_pagination() {
        let page = parse_searxng_results(&json!({ "results": [] }), 3).unwrap();
        assert!(page.results.is_empty());
        assert_eq!(page.next_page, None);
    }

    #[test]
    fn test_parse_searxng_rejects_structurally_invalid_body() {
        assert!(parse_searxng_results(&json!({ "error": "nope" }), 1).is_err());
        assert!(parse_searxng_results(&json!({ "results": "x" }), 1).is_err());
    }

    #[test]
    fn test_parse_brave_results_web_news_and_empty() {
        let opts = SearchOptions::default();
        let body = json!({
            "type": "search",
            "query": { "more_results_available": true },
            "web": { "results": [{ "title": "T", "url": "https://docs.rs/x", "description": "D", "page_age": "2026-09-01T00:00:00" }] }
        });
        let page = parse_brave_results(&body, &opts).unwrap();
        assert_eq!(page.results[0].provider, "brave");
        assert_eq!(
            page.results[0].published.as_deref(),
            Some("2026-09-01T00:00:00")
        );
        assert_eq!(page.next_page, Some(2));

        let news = SearchOptions {
            category: Some(Category::News),
            ..Default::default()
        };
        let body = json!({ "type": "search", "news": { "results": [{ "title": "N", "url": "https://bbc.co.uk/n", "age": "2 hours ago" }] } });
        let page = parse_brave_results(&body, &news).unwrap();
        assert_eq!(page.results[0].published.as_deref(), Some("2 hours ago"));
        assert_eq!(page.next_page, None);

        let empty = parse_brave_results(&json!({ "type": "search" }), &opts).unwrap();
        assert!(empty.results.is_empty());
        assert!(parse_brave_results(&json!({ "message": "?" }), &opts).is_err());

        let last = SearchOptions {
            page: BRAVE_MAX_PAGE,
            ..Default::default()
        };
        let body = json!({ "type": "search", "query": { "more_results_available": true }, "web": { "results": [{ "title": "T", "url": "https://a.com" }] } });
        assert_eq!(parse_brave_results(&body, &last).unwrap().next_page, None);
    }

    #[test]
    fn options_default_when_no_filters() {
        let opts = SearchOptions::from_args(&json!({ "query": "x" })).unwrap();
        assert_eq!(opts, SearchOptions::default());
        let params = searxng_params("rust", &opts);
        assert_eq!(
            params,
            vec![
                ("q", "rust".into()),
                ("format", "json".into()),
                ("pageno", "1".into())
            ]
        );
        assert!(searxng_unsupported(&opts).is_empty() && brave_unsupported(&opts).is_empty());
    }

    #[test]
    fn options_parse_and_normalise() {
        let opts = SearchOptions::from_args(&json!({
            "max_results": 50, "page": 2, "freshness": "Week", "language": "EN", "country": "us",
            "category": "news", "include_domains": ["https://www.Docs.rs/tokio"], "exclude_domains": ["pinterest.com"]
        }))
        .unwrap();
        assert_eq!(opts.max_results, MAX_RESULTS_CAP);
        assert_eq!(opts.page, 2);
        assert_eq!(opts.freshness, Some(Freshness::Week));
        assert_eq!(opts.language.as_deref(), Some("en"));
        assert_eq!(opts.country.as_deref(), Some("US"));
        assert_eq!(opts.include_domains, vec!["docs.rs"]);
    }

    #[test]
    fn options_reject_bad_values() {
        for bad in [
            json!({ "freshness": "hour" }),
            json!({ "language": "english" }),
            json!({ "country": "USA" }),
            json!({ "category": "images" }),
            json!({ "page": 0 }),
            json!({ "include_domains": "docs.rs" }),
            json!({ "include_domains": ["not a domain"] }),
            json!({ "exclude_domains": [1] }),
        ] {
            assert!(SearchOptions::from_args(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn provider_param_mapping() {
        let opts = SearchOptions {
            max_results: 7,
            page: 3,
            freshness: Some(Freshness::Month),
            language: Some("en".into()),
            country: Some("GB".into()),
            category: Some(Category::News),
            include_domains: vec!["bbc.co.uk".into(), "reuters.com".into()],
            exclude_domains: vec!["spam.com".into()],
        };
        let q = "(site:bbc.co.uk OR site:reuters.com) -site:spam.com";
        let sx = searxng_params("uk rates", &opts);
        assert!(sx.contains(&("q", format!("uk rates {q}"))));
        assert!(sx.contains(&("pageno", "3".into())));
        assert!(sx.contains(&("time_range", "month".into())));
        assert!(sx.contains(&("language", "en-GB".into())));
        assert!(sx.contains(&("categories", "news".into())));
        let br = brave_params("uk rates", &opts);
        assert!(br.contains(&("count", "7".into())));
        assert!(br.contains(&("offset", "2".into())));
        assert!(br.contains(&("freshness", "pm".into())));
        assert!(br.contains(&("country", "GB".into())));
        assert!(br.contains(&("search_lang", "en".into())));
        assert!(br.contains(&("result_filter", "news".into())));
    }

    #[test]
    fn unsupported_filters_are_reported() {
        let country_only = SearchOptions {
            country: Some("US".into()),
            ..Default::default()
        };
        assert_eq!(searxng_unsupported(&country_only).len(), 1);
        assert!(brave_unsupported(&country_only).is_empty());
        let science = SearchOptions {
            category: Some(Category::Science),
            ..Default::default()
        };
        assert_eq!(brave_unsupported(&science).len(), 1);
        assert!(searxng_unsupported(&science).is_empty());
    }

    #[test]
    fn domain_post_filter() {
        let opts = SearchOptions {
            include_domains: vec!["rust-lang.org".into()],
            exclude_domains: vec!["blog.rust-lang.org".into()],
            ..Default::default()
        };
        assert!(domain_allowed("https://doc.rust-lang.org/std", &opts));
        assert!(domain_allowed("https://www.rust-lang.org/", &opts));
        assert!(!domain_allowed("https://blog.rust-lang.org/post", &opts));
        assert!(!domain_allowed("https://notrust-lang.org/", &opts));
    }

    #[test]
    fn body_classification() {
        assert_eq!(
            classify_body("text/html; charset=utf-8", b"<html>"),
            BodyKind::Html
        );
        assert_eq!(classify_body("application/pdf", b"%PDF-1.7"), BodyKind::Pdf);
        assert_eq!(
            classify_body("application/octet-stream", b"%PDF-1.4\n"),
            BodyKind::Pdf
        );
        assert_eq!(
            classify_body("application/octet-stream", b"PK\x03\x04"),
            BodyKind::Binary
        );
        assert_eq!(classify_body("", b"ab\0cd"), BodyKind::Binary);
        assert_eq!(classify_body("application/json", b"{}"), BodyKind::Text);
        assert!(is_binary_media_type("image/png"));
        assert!(!is_binary_media_type("application/pdf"));
    }

    #[test]
    fn fetched_page_reports_provenance_and_truncation() {
        let page = FetchedPage {
            url: "https://a.com/x.pdf".into(),
            final_url: "https://cdn.a.com/x.pdf".into(),
            content_type: "application/pdf".into(),
            method: METHOD_PDF_OCR,
            content: "héllo world".into(),
            total_chars: 12,
            truncated: false,
            notes: vec!["scanned PDF".into()],
        }
        .truncate_to(2);
        assert!(page.truncated);
        let text = page.to_text();
        assert!(text.starts_with(
            "[Fetched https://cdn.a.com/x.pdf | application/pdf | extracted via pdf-ocr]"
        ));
        assert!(text.contains("[Note: scanned PDF]"));
        assert!(
            text.contains("[Content truncated at 1 chars (12 total)]"),
            "{text}"
        );
    }

    /// A one-page PDF with a real text layer, offsets computed so the xref is valid.
    fn tiny_pdf(text: &str) -> Vec<u8> {
        let stream = format!("BT /F1 12 Tf 72 720 Td ({text}) Tj ET");
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_string(),
            format!("<< /Length {} >>\nstream\n{stream}\nendstream", stream.len()),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        ];
        let mut pdf = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
        }
        let xref = pdf.len();
        pdf.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
        );
        for off in offsets {
            pdf.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );
        pdf
    }

    #[tokio::test]
    async fn pdf_text_layer_is_extracted() {
        let sentence = "Quarterly research findings show search coverage improved across every region we measured";
        let (text, method) = extract_pdf(&tiny_pdf(sentence)).await.unwrap();
        assert_eq!(method, METHOD_PDF_TEXT);
        assert!(text.contains("Quarterly research findings"), "{text}");
    }

    #[tokio::test]
    async fn garbage_pdf_fails_with_a_clear_reason() {
        let err = extract_pdf(b"%PDF-1.4\nnot really a pdf")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("PDF"), "{err}");
    }

    const FAST: Budgets = Budgets {
        total: Duration::from_secs(5),
        searxng: Duration::from_millis(400),
        brave: Duration::from_millis(400),
    };

    fn searxng_body(urls: &[&str]) -> Value {
        json!({ "results": urls.iter().map(|u| json!({ "title": u, "url": u, "content": "" })).collect::<Vec<_>>() })
    }

    fn brave_body(urls: &[&str]) -> Value {
        json!({ "type": "search", "web": { "results": urls.iter().map(|u| json!({ "title": u, "url": u })).collect::<Vec<_>>() } })
    }

    async fn mount(server: &MockServer, route: &str, response: ResponseTemplate) {
        Mock::given(method("GET"))
            .and(path(route))
            .respond_with(response)
            .mount(server)
            .await;
    }

    /// wiremock pools servers, so a reused address would inherit an earlier test's cooldown.
    async fn fresh_server() -> MockServer {
        let server = MockServer::start().await;
        let uri = server.uri();
        HEALTH.lock().unwrap().retain(|key, _| !key.starts_with(&uri));
        server
    }

    fn brave_endpoint(server: &MockServer) -> String {
        format!("{}/res/v1/web/search", server.uri())
    }

    #[tokio::test]
    async fn searxng_answers_with_filters_passed_through() {
        let sx = fresh_server().await;
        Mock::given(method("GET"))
            .and(path("/search"))
            .and(query_param("time_range", "week"))
            .and(query_param("pageno", "2"))
            .and(query_param("language", "en-US"))
            .and(query_param("categories", "news"))
            .and(query_param("q", "rates site:bbc.co.uk"))
            .respond_with(ResponseTemplate::new(200).set_body_json(searxng_body(&[
                "https://www.bbc.co.uk/news/1",
                "https://evil.example/2",
            ])))
            .mount(&sx)
            .await;
        let opts = SearchOptions::from_args(&json!({
            "page": 2, "freshness": "week", "language": "en", "country": "US",
            "category": "news", "include_domains": ["bbc.co.uk"]
        }))
        .unwrap();
        let r = search_chain("rates", &opts, Some(&sx.uri()), None, &FAST)
            .await
            .unwrap();
        assert_eq!(r.backend.as_deref(), Some("searxng"));
        assert_eq!(r.results.len(), 1, "post-filter drops off-domain hits");
        assert_eq!(r.page, 2);
        assert_eq!(r.next_page, Some(3));
    }

    #[tokio::test]
    async fn brave_receives_mapped_params_and_key_header() {
        let br = fresh_server().await;
        Mock::given(method("GET"))
            .and(path("/res/v1/web/search"))
            .and(header("X-Subscription-Token", "k"))
            .and(query_param("freshness", "pd"))
            .and(query_param("offset", "1"))
            .and(query_param("country", "US"))
            .and(query_param("search_lang", "en"))
            .and(query_param("result_filter", "web"))
            .respond_with(ResponseTemplate::new(200).set_body_json(brave_body(&["https://a.com"])))
            .mount(&br)
            .await;
        let opts = SearchOptions::from_args(
            &json!({ "page": 2, "freshness": "day", "language": "en", "country": "US" }),
        )
        .unwrap();
        let endpoint = brave_endpoint(&br);
        let r = search_chain("q", &opts, None, Some((&endpoint, "k")), &FAST)
            .await
            .unwrap();
        assert_eq!(r.backend.as_deref(), Some("brave"));
        assert_eq!(r.results.len(), 1);
    }

    #[tokio::test]
    async fn malformed_json_falls_back_and_cools_the_primary_down() {
        let sx = fresh_server().await;
        mount(
            &sx,
            "/search",
            ResponseTemplate::new(200).set_body_string("{not json"),
        )
        .await;
        let br = fresh_server().await;
        mount(
            &br,
            "/res/v1/web/search",
            ResponseTemplate::new(200).set_body_json(brave_body(&["https://a.com"])),
        )
        .await;
        let endpoint = brave_endpoint(&br);
        let opts = SearchOptions::default();

        let r = search_chain("q", &opts, Some(&sx.uri()), Some((&endpoint, "k")), &FAST)
            .await
            .unwrap();
        assert_eq!(r.backend.as_deref(), Some("brave"));
        assert!(
            r.attempts[0].outcome.contains("malformed JSON"),
            "{:?}",
            r.attempts
        );

        let again = search_chain("q", &opts, Some(&sx.uri()), Some((&endpoint, "k")), &FAST)
            .await
            .unwrap();
        assert!(
            again.attempts[0].outcome.contains("cooling down"),
            "{:?}",
            again.attempts
        );
    }

    #[tokio::test]
    async fn invalid_shape_counts_as_a_failure() {
        let sx = fresh_server().await;
        mount(
            &sx,
            "/search",
            ResponseTemplate::new(200).set_body_json(json!({ "unexpected": true })),
        )
        .await;
        let err = search_chain("q", &SearchOptions::default(), Some(&sx.uri()), None, &FAST)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("invalid response"), "{err}");
        assert!(with_health(&sx.uri(), |h| h.is_cooling_down()).unwrap());
    }

    #[tokio::test]
    async fn timeout_is_bounded_and_falls_back() {
        let sx = fresh_server().await;
        mount(
            &sx,
            "/search",
            ResponseTemplate::new(200)
                .set_delay(Duration::from_secs(3))
                .set_body_json(searxng_body(&["https://a.com"])),
        )
        .await;
        let br = fresh_server().await;
        mount(
            &br,
            "/res/v1/web/search",
            ResponseTemplate::new(200).set_body_json(brave_body(&["https://b.com"])),
        )
        .await;
        let endpoint = brave_endpoint(&br);
        let started = Instant::now();
        let r = search_chain(
            "q",
            &SearchOptions::default(),
            Some(&sx.uri()),
            Some((&endpoint, "k")),
            &FAST,
        )
        .await
        .unwrap();
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(
            r.attempts[0].outcome.starts_with("timed out"),
            "{:?}",
            r.attempts
        );
        assert_eq!(r.results[0].url, "https://b.com");
    }

    #[tokio::test]
    async fn overall_deadline_skips_the_fallback() {
        let sx = fresh_server().await;
        mount(
            &sx,
            "/search",
            ResponseTemplate::new(200).set_delay(Duration::from_secs(3)),
        )
        .await;
        let br = fresh_server().await;
        let endpoint = brave_endpoint(&br);
        let tight = Budgets {
            total: Duration::from_millis(300),
            searxng: Duration::from_secs(5),
            brave: Duration::from_secs(5),
        };
        let err = search_chain(
            "q",
            &SearchOptions::default(),
            Some(&sx.uri()),
            Some((&endpoint, "k")),
            &tight,
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(err.contains("searxng: timed out"), "{err}");
        assert!(
            err.contains("brave: skipped: overall search deadline reached"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn rate_limit_then_cooldown_and_credentials_not_leaked() {
        let br = fresh_server().await;
        mount(&br, "/res/v1/web/search", ResponseTemplate::new(429)).await;
        let endpoint = brave_endpoint(&br);
        let opts = SearchOptions::default();
        let err = search_chain("q", &opts, None, Some((&endpoint, "secret-key")), &FAST)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("rate limited"), "{err}");
        assert!(!err.contains("secret-key"));
        let err = search_chain("q", &opts, None, Some((&endpoint, "secret-key")), &FAST)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("cooling down"), "{err}");
    }

    #[tokio::test]
    async fn invalid_credentials_and_all_provider_failure() {
        let sx = fresh_server().await;
        mount(&sx, "/search", ResponseTemplate::new(503)).await;
        let br = fresh_server().await;
        mount(&br, "/res/v1/web/search", ResponseTemplate::new(401)).await;
        let endpoint = brave_endpoint(&br);
        let err = search_chain(
            "q",
            &SearchOptions::default(),
            Some(&sx.uri()),
            Some((&endpoint, "bad")),
            &FAST,
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(err.contains("searxng: HTTP 503"), "{err}");
        assert!(err.contains("brave: invalid credentials"), "{err}");
        assert!(!err.contains("bad"), "{err}");
    }

    #[tokio::test]
    async fn empty_results_are_an_answer_not_a_missing_backend() {
        let sx = fresh_server().await;
        mount(
            &sx,
            "/search",
            ResponseTemplate::new(200).set_body_json(searxng_body(&[])),
        )
        .await;
        let r = search_chain("q", &SearchOptions::default(), Some(&sx.uri()), None, &FAST)
            .await
            .unwrap();
        assert!(r.results.is_empty());
        assert_eq!(r.next_page, None);
        assert!(with_health(&sx.uri(), |h| !h.is_cooling_down()).unwrap());
    }

    #[tokio::test]
    async fn brave_page_limit_is_reported() {
        let br = fresh_server().await;
        let endpoint = brave_endpoint(&br);
        let opts = SearchOptions {
            page: 11,
            ..Default::default()
        };
        let err = search_chain("q", &opts, None, Some((&endpoint, "k")), &FAST)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("pages 1..=10"), "{err}");
    }

    /// Live probe against real backends. Run with
    /// `HQ_TEST_SEARXNG_URL=http://localhost:8888 HQ_TEST_BRAVE_API_KEY=... cargo test -p hq-tools live_filter_probe -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore = "needs a live SearxNG and/or Brave key; see doc comment"]
    async fn live_filter_probe() {
        let searxng = std::env::var("HQ_TEST_SEARXNG_URL").ok();
        let brave = std::env::var("HQ_TEST_BRAVE_API_KEY").ok();
        let opts = SearchOptions::from_args(&json!({
            "freshness": "year", "language": "en", "country": "US", "include_domains": ["github.com"]
        }))
        .unwrap();
        for (label, sx, key) in [
            ("searxng", searxng.as_deref(), None),
            ("brave", None, brave.as_deref()),
        ] {
            if sx.is_none() && key.is_none() {
                continue;
            }
            let r = web_search("tokio release", &opts, sx, key).await.unwrap();
            println!("{label}: {}", format_search_results(&r));
            assert!(r.results.iter().all(|x| x.url.contains("github.com")));
        }
    }
}
