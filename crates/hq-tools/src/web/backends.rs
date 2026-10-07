use super::*;

//
// A single hard failure (e.g. SearxNG not running, Brave 429) used to cost
// every subsequent call the same timeout/error again. This remembers the
// last failure per backend endpoint and skips straight to the next backend
// until a cooldown expires, instead of re-probing one already known to be down.

pub(super) const SEARXNG_BACKOFF_BASE: Duration = Duration::from_secs(10);
pub(super) const SEARXNG_BACKOFF_CAP: Duration = Duration::from_secs(5 * 60);
pub(super) const BRAVE_RATE_LIMIT_BACKOFF_BASE: Duration = Duration::from_secs(30);
pub(super) const BRAVE_RATE_LIMIT_BACKOFF_CAP: Duration = Duration::from_secs(30 * 60);
pub(super) const BRAVE_ERROR_BACKOFF_BASE: Duration = Duration::from_secs(15);
pub(super) const BRAVE_ERROR_BACKOFF_CAP: Duration = Duration::from_secs(5 * 60);

#[derive(Default)]
pub(super) struct BackendHealth {
    pub(super) unreachable_until: Option<Instant>,
    pub(super) consecutive_failures: u32,
}

impl BackendHealth {
    pub(super) fn is_cooling_down(&self) -> bool {
        self.unreachable_until.is_some_and(|t| Instant::now() < t)
    }

    fn record_success(&mut self) {
        self.consecutive_failures = 0;
        self.unreachable_until = None;
    }

    /// Backoff of `base` for the first failure, doubling per consecutive one, capped at `cap`.
    fn record_failure(&mut self, base: Duration, cap: Duration) {
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        let shift = self.consecutive_failures.saturating_sub(1).min(16);
        let secs = base.as_secs().saturating_mul(1u64 << shift);
        self.unreachable_until = Some(Instant::now() + Duration::from_secs(secs).min(cap));
    }
}

/// Keyed by endpoint, so two SearxNG instances (or a test server) never share a cooldown.
pub(super) static HEALTH: std::sync::LazyLock<Mutex<HashMap<String, BackendHealth>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

pub(super) fn with_health<T>(key: &str, f: impl FnOnce(&mut BackendHealth) -> T) -> Option<T> {
    let mut map = HEALTH.lock().ok()?;
    Some(f(map.entry(key.to_string()).or_default()))
}


/// What kind of failure a backend reported, which sets how long it is
/// suspended. The classes and their flat durations are SearxNG's shipped
/// `suspended_times`: a captcha an hour, a rate limit or an access denial three
/// minutes, anything else five seconds. A success clears the suspension.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(super) enum FailureClass {
    #[default]
    Generic,
    RateLimited,
    AccessDenied,
    Captcha,
}

pub(super) const CAPTCHA_SUSPENSION: Duration = Duration::from_secs(3600);
pub(super) const REFUSAL_SUSPENSION: Duration = Duration::from_secs(180);
pub(super) const REFUSAL_SUSPENSION_CAP: Duration = Duration::from_secs(180);
pub(super) const GENERIC_SUSPENSION: Duration = Duration::from_secs(5);
pub(super) const GENERIC_SUSPENSION_CAP: Duration = Duration::from_secs(5);

impl FailureClass {
    /// (base, cap) of the suspension. Base and cap are equal, so repeated failures
    /// do not lengthen it, as in SearxNG.
    pub(super) fn suspension(self) -> (Duration, Duration) {
        match self {
            FailureClass::Captcha => (CAPTCHA_SUSPENSION, CAPTCHA_SUSPENSION),
            FailureClass::RateLimited | FailureClass::AccessDenied => {
                (REFUSAL_SUSPENSION, REFUSAL_SUSPENSION_CAP)
            }
            FailureClass::Generic => (GENERIC_SUSPENSION, GENERIC_SUSPENSION_CAP),
        }
    }
}

/// Why a backend call failed, and how to suspend it.
#[derive(Debug)]
pub(super) struct ProviderError {
    pub(super) reason: String,
    pub(super) class: FailureClass,
}

impl ProviderError {
    pub(super) fn new(reason: impl Into<String>) -> Self {
        Self::of(FailureClass::Generic, reason)
    }

    pub(super) fn of(class: FailureClass, reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
            class,
        }
    }
}

impl From<String> for ProviderError {
    fn from(reason: String) -> Self {
        Self::new(reason)
    }
}

impl From<&str> for ProviderError {
    fn from(reason: &str) -> Self {
        Self::new(reason)
    }
}

/// One page as a backend returned it, before domain filtering.
#[derive(Clone)]
pub(super) struct Page {
    pub(super) results: Vec<SearchResult>,
    pub(super) next_page: Option<u32>,
}

/// Send a backend request and decode JSON, classifying every failure.
/// Credentials live in headers, never in the reason text.
pub(super) async fn get_json(request: RequestBuilder) -> Result<Value, ProviderError> {
    let body = get_body(request).await?;
    serde_json::from_str(&body).map_err(|e| ProviderError::new(format!("malformed JSON: {e}")))
}

/// Like [`get_json`] for HTML and XML engines: the body travels as a string
/// value so it can run through the same `try_backend` runner.
pub(super) async fn get_text(request: RequestBuilder) -> Result<Value, ProviderError> {
    get_body(request).await.map(Value::String)
}

pub(super) async fn get_body(request: RequestBuilder) -> Result<String, ProviderError> {
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
    read_body(resp).await
}

/// Classify a backend response that did arrive: status first, then the body.
pub(super) async fn read_body(resp: reqwest::Response) -> Result<String, ProviderError> {
    let status = resp.status();
    // GitHub answers an exhausted quota with 403 and these headers.
    let quota_exhausted = resp.headers().contains_key("retry-after")
        || resp
            .headers()
            .get("x-ratelimit-remaining")
            .is_some_and(|v| v == "0");
    if status.as_u16() == 429 || (status.as_u16() == 403 && quota_exhausted) {
        return Err(ProviderError::of(
            FailureClass::RateLimited,
            format!("rate limited (HTTP {})", status.as_u16()),
        ));
    }
    // Stack Exchange reports throttling as a 400 with an error name in the body.
    if status.as_u16() == 400 {
        let body = resp.text().await.unwrap_or_default();
        if body.contains("throttle_violation") || body.contains("too_many_requests") {
            return Err(ProviderError::of(
                FailureClass::RateLimited,
                "rate limited (throttled)",
            ));
        }
        return Err(ProviderError::new("HTTP 400 Bad Request"));
    }
    // Cloudflare and reCAPTCHA challenge pages arrive as 403 or 503.
    if matches!(status.as_u16(), 403 | 503) {
        let body = resp.text().await.unwrap_or_default();
        if CHALLENGE_BODY_MARKERS.iter().any(|m| body.contains(m)) {
            return Err(ProviderError::of(
                FailureClass::Captcha,
                format!("challenge page (HTTP {status})"),
            ));
        }
        if status.as_u16() == 403 {
            return Err(ProviderError::of(
                FailureClass::AccessDenied,
                format!("invalid credentials or access denied (HTTP {status})"),
            ));
        }
        return Err(ProviderError::new(format!("HTTP {status}")));
    }
    if status.as_u16() == 401 {
        return Err(ProviderError::of(
            FailureClass::AccessDenied,
            format!("invalid credentials or access denied (HTTP {status})"),
        ));
    }
    if !status.is_success() {
        return Err(ProviderError::new(format!("HTTP {status}")));
    }
    resp.text()
        .await
        .map_err(|e| ProviderError::new(format!("failed reading body: {}", e.without_url())))
}

const CHALLENGE_BODY_MARKERS: &[&str] = &[
    "__cf_chl_",
    "/cdn-cgi/challenge-platform/",
    "cf-error-code\">1020",
    "https://www.google.com/recaptcha/",
];

pub(super) async fn decode_json(resp: reqwest::Response) -> Result<Value, ProviderError> {
    let body = read_body(resp).await?;
    serde_json::from_str(&body).map_err(|e| ProviderError::new(format!("malformed JSON: {e}")))
}

pub(super) fn searxng_params(query: &str, opts: &SearchOptions) -> Vec<(&'static str, String)> {
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
            Category::Images => "images",
            Category::Code => "it",
        };
        params.push(("categories", name.to_string()));
    }
    params
}

pub(super) fn searxng_unsupported(opts: &SearchOptions) -> Vec<String> {
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
pub(super) async fn searxng_request(
    base_url: &str,
    query: &str,
    opts: &SearchOptions,
) -> Result<Value, ProviderError> {
    let url = format!("{}/search", base_url.trim_end_matches('/'));
    get_json(get_client().get(url).query(&searxng_params(query, opts))).await
}

/// Parse a SearxNG `/search?format=json` body. A body without a `results`
/// array is a broken backend, not an empty result set.
pub(super) fn parse_searxng_results(json: &Value, page: u32) -> Result<Page, String> {
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
                flagged: false,
                url,
            }
        })
        .collect();
    // SearxNG has no "more results" flag; an empty page is the end.
    let next_page = (!results.is_empty()).then_some(page + 1);
    Ok(Page { results, next_page })
}

pub(super) fn non_empty_str(v: &Value) -> Option<String> {
    v.as_str().filter(|s| !s.is_empty()).map(String::from)
}

pub(super) fn brave_section(opts: &SearchOptions) -> &'static str {
    if opts.category == Some(Category::News) {
        "news"
    } else {
        "web"
    }
}

pub(super) fn brave_params(query: &str, opts: &SearchOptions) -> Vec<(&'static str, String)> {
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

pub(super) fn brave_unsupported(opts: &SearchOptions) -> Vec<String> {
    let name = match opts.category {
        Some(Category::Science) => "science",
        Some(Category::Images) => "images",
        Some(Category::Code) => "code",
        _ => return Vec::new(),
    };
    vec![format!("category={name} (Brave supports general and news)")]
}

pub(super) async fn brave_request(
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
pub(super) fn parse_brave_results(json: &Value, opts: &SearchOptions) -> Result<Page, String> {
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
                flagged: false,
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
pub(super) struct Backend<'a> {
    pub(super) key: &'a str,
    pub(super) label: &'static str,
    pub(super) budget: Duration,
    /// Backoff (base, cap) for a failure of the given class.
    pub(super) backoff: fn(FailureClass) -> (Duration, Duration),
}

pub(super) fn searxng_backoff(class: FailureClass) -> (Duration, Duration) {
    match class {
        FailureClass::Generic => (SEARXNG_BACKOFF_BASE, SEARXNG_BACKOFF_CAP),
        other => other.suspension(),
    }
}

pub(super) fn brave_backoff(class: FailureClass) -> (Duration, Duration) {
    match class {
        FailureClass::RateLimited => (BRAVE_RATE_LIMIT_BACKOFF_BASE, BRAVE_RATE_LIMIT_BACKOFF_CAP),
        FailureClass::Generic => (BRAVE_ERROR_BACKOFF_BASE, BRAVE_ERROR_BACKOFF_CAP),
        other => other.suspension(),
    }
}

/// Run one backend under its budget (capped by the overall deadline), record
/// the outcome in its health entry and in `attempts`, and return its page.
pub(super) async fn try_backend<Fut>(
    backend: &Backend<'_>,
    deadline: Instant,
    attempts: &mut Vec<ProviderAttempt>,
    request: impl FnOnce() -> Fut,
    parse: impl FnOnce(&Value) -> Result<Page, ProviderError>,
) -> Option<Page>
where
    Fut: Future<Output = Result<Value, ProviderError>>,
{
    let mut note = |outcome: String| {
        attempts.push(ProviderAttempt {
            provider: backend.label.into(),
            outcome: sanitize_note(&outcome),
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
            Err(e) => ProviderError::of(e.class, format!("invalid response: {}", e.reason)),
        },
    };
    let (base, cap) = (backend.backoff)(failure.class);
    with_health(backend.key, |h| h.record_failure(base, cap));
    debug!(backend = backend.label, reason = %failure.reason, "web search backend failed");
    note(failure.reason);
    None
}
