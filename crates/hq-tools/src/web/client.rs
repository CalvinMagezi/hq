use super::*;

pub(super) struct CacheEntry {
    pub(super) page: FetchedPage,
    pub(super) fetched_at: Instant,
}

/// Simple in-process URL cache with TTL eviction.
/// Avoids re-fetching the same page within a session (15 min TTL).
pub(super) static FETCH_CACHE: std::sync::LazyLock<Mutex<HashMap<String, CacheEntry>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

pub(super) fn cache_get(url: &str) -> Option<FetchedPage> {
    let cache = FETCH_CACHE.lock().ok()?;
    let entry = cache.get(url)?;
    (entry.fetched_at.elapsed() < CACHE_TTL).then(|| entry.page.clone())
}

pub(super) fn cache_put(url: &str, page: &FetchedPage) {
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


/// For search backends and Jina. Per-request timeouts are set at each call site.
pub(super) static HTTP_CLIENT: std::sync::LazyLock<Client> = std::sync::LazyLock::new(|| {
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
pub(super) static FETCH_CLIENT: std::sync::LazyLock<Client> =
    std::sync::LazyLock::new(|| fetch_client(FETCH_TIMEOUT, validate_url));

/// A fetch client whose every redirect hop must pass `allow`. Production
/// passes `validate_url`; tests wrap it to admit only their local server.
pub(super) fn fetch_client(
    timeout: Duration,
    allow: impl Fn(&str) -> Result<()> + Send + Sync + 'static,
) -> Client {
    fetch_client_with_resolver(timeout, allow, GuardedResolver::system())
}

pub(super) fn fetch_client_with_resolver(
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
pub(super) const USE_PROXY_ENV: &str = "HQ_WEB_FETCH_USE_PROXY";

/// A proxy resolves names itself, so the pinned resolver and its address checks never
/// run for requests sent through one. Default to no proxy; warn once if it is opted into.
pub(super) fn use_proxy_from_env() -> bool {
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

pub(super) fn proxy_policy(builder: reqwest::ClientBuilder, use_proxy: bool) -> reqwest::ClientBuilder {
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

pub(super) fn get_client() -> &'static Client {
    &HTTP_CLIENT
}
