use super::*;

/// Time budgets for the fallback chain; only tests shrink them.
pub(super) struct Budgets {
    pub(super) total: Duration,
    pub(super) searxng: Duration,
    pub(super) brave: Duration,
}

pub(super) const DEFAULT_BUDGETS: Budgets = Budgets {
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

pub(super) async fn search_chain(
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
