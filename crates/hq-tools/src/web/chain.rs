use super::*;

/// Time budgets for the fallback chain; only tests shrink them.
pub(super) struct Budgets {
    pub(super) total: Duration,
    pub(super) searxng: Duration,
    pub(super) native_engine: Duration,
    pub(super) brave: Duration,
}

pub(super) const DEFAULT_BUDGETS: Budgets = Budgets {
    total: SEARCH_DEADLINE,
    searxng: SEARXNG_TIMEOUT,
    native_engine: NATIVE_ENGINE_TIMEOUT,
    brave: BRAVE_TIMEOUT,
};

/// Search the web: a configured SearxNG instance first, then the built-in
/// keyless engine pool (`native`), then the Brave Search API if
/// `brave_api_key` is set. Each later backend is tried on failure or empty
/// results. Errors with remediation guidance if none is enabled, and with
/// every attempt's reason if all fail.
pub async fn web_search(
    query: &str,
    opts: &SearchOptions,
    searxng_url: Option<&str>,
    brave_api_key: Option<&str>,
    native: bool,
) -> Result<WebSearchResults> {
    // A query is an outbound channel: a credential pasted into it would be sent
    // to every engine, so it is refused before anything leaves the process.
    let query = &strip_invisible(query);
    if hq_core::redact::looks_like_credential(query) {
        bail!(
            "The search query looks like it contains a credential (API key, token or private key), \
             so it was not sent. Remove the secret and search again."
        );
    }
    let searxng_url = searxng_url.map(str::trim).filter(|s| !s.is_empty());
    let brave = brave_api_key
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|key| (BRAVE_ENDPOINT, key));
    let env = native.then(NativeEnv::production);
    search_chain(query, opts, searxng_url, brave, env.as_ref(), &DEFAULT_BUDGETS).await
}

pub(super) async fn search_chain(
    query: &str,
    opts: &SearchOptions,
    searxng_url: Option<&str>,
    brave: Option<(&str, &str)>,
    native: Option<&NativeEnv<'_>>,
    budgets: &Budgets,
) -> Result<WebSearchResults> {
    let peer = peer::configured();
    search_chain_via(query, opts, searxng_url, brave, native, peer.as_ref(), budgets).await
}

/// `search_chain` with the peer given explicitly, so tests need no global.
pub(super) async fn search_chain_via(
    query: &str,
    opts: &SearchOptions,
    searxng_url: Option<&str>,
    brave: Option<(&str, &str)>,
    native: Option<&NativeEnv<'_>>,
    peer: Option<&hq_core::config::RemoteMcpServer>,
    budgets: &Budgets,
) -> Result<WebSearchResults> {
    let peer = peer.filter(|_| !opts.peer_hop);
    if searxng_url.is_none() && brave.is_none() && native.is_none() && peer.is_none() {
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
            |json| parse_searxng_results(json, opts.page).map_err(ProviderError::from),
        )
        .await;
        if let Some(page) = page {
            if !page.results.is_empty() || (brave.is_none() && native.is_none()) {
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

    if let Some(env) = native {
        debug!(query = %query, "searching built-in engines");
        let pool = native_search(query, opts, env, deadline, budgets.native_engine).await;
        attempts.extend(pool.attempts);
        // No hits from a pool whose real web engines are all out is a failure to
        // report, not an empty answer: the attempts say which engines were blocked.
        let blocked = |p: &Page| p.results.is_empty() && pool.degraded;
        if let Some(page) = pool.page.filter(|p| !blocked(p)) {
            // A pool with only Wikipedia or Hacker News answering is a last resort,
            // so a configured Brave key still gets its turn.
            let good_enough = !page.results.is_empty() && !pool.degraded;
            if good_enough || brave.is_none() {
                return Ok(finish(
                    query,
                    opts,
                    "native",
                    page,
                    native_unsupported(opts),
                    attempts,
                ));
            }
            answered = Some(("native", page, native_unsupported(opts)));
        }
    }

    if let Some(server) = peer {
        let key = format!("peer:{}", server.name);
        let backend = Backend {
            key: &key,
            label: "peer",
            budget: peer::PEER_TIMEOUT,
            backoff: searxng_backoff,
        };
        debug!(query = %query, peer = %server.name, "searching through a peer HQ");
        let page = try_backend(
            &backend,
            deadline,
            &mut attempts,
            || peer::request(server, query, opts),
            |json| peer::parse(json).map_err(ProviderError::from),
        )
        .await;
        if let Some(page) = page.filter(|p| !p.results.is_empty()) {
            answered = Some(("peer", page, Vec::new()));
        }
    }

    // A peer's answer is a full one, so a paid Brave call is kept for when it fails.
    if !matches!(answered, Some(("peer", ..)))
        && let Some((endpoint, api_key)) = brave
    {
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
                |json| parse_brave_results(json, opts).map_err(ProviderError::from),
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

/// Filters the built-in pool cannot apply on every engine it queries.
pub(super) fn native_unsupported(opts: &SearchOptions) -> Vec<String> {
    let mut notes = Vec::new();
    if opts.freshness.is_some() && opts.category == Some(Category::Science) {
        notes.push("freshness (arXiv and Europe PMC ignore it; OpenAlex applies it)".to_string());
    }
    if opts.freshness.is_some() && opts.category == Some(Category::Images) {
        notes.push("freshness (Google Images applies it; Wikimedia Commons and Openverse ignore it)".to_string());
    }
    if opts.freshness.is_some() && opts.category == Some(Category::Code) {
        notes.push(
            "freshness (GitHub, Stack Overflow, Ask Ubuntu and Super User apply it; MDN, crates.io and npm ignore it)"
                .to_string(),
        );
    }
    if opts.freshness.is_some() && opts.category == Some(Category::News) {
        notes.push("freshness (Bing News ignores it; Hacker News applies it)".to_string());
    }
    notes
}

pub(super) fn finish(
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
        .map(|mut r| {
            sanitize_result(&mut r);
            r
        })
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

pub(super) fn no_backend_error() -> anyhow::Error {
    anyhow::anyhow!(
        "No web search backend is enabled. Set `web_search_native: true` (the default) in \
         ~/.hq/config.yaml for the built-in engine, point `searxng_url` at a SearxNG \
         instance, or set `brave_api_key` (or HQ_BRAVE_API_KEY) for the paid Brave Search API."
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
                "description": "1-based results page (default 1). Use next_page from a previous response. Some engines serve only page 1, and Brave serves pages 1-10."
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
                "description": "2-letter country code, e.g. \"US\". SearxNG and DuckDuckGo need language as well."
            },
            "category": {
                "type": "string",
                "enum": ["general", "news", "science", "images", "code"],
                "description": "Result category. images lists freely licensed images with their direct URL; code searches GitHub, Stack Overflow and package registries. Brave supports general and news only."
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
