//! Active `web_search` health check: one real query per configured backend.

use super::{
    BRAVE_ENDPOINT, Page, SearchOptions, brave_params, decode_json, error_chain, get_client,
    parse_brave_results, parse_searxng_results, searxng_params,
};
use hq_core::machine::WebSearchBackendStatus;
use reqwest::RequestBuilder;
use serde_json::Value;
use std::time::Duration;

const PROBE_TIMEOUT: Duration = Duration::from_secs(8);
const PROBE_QUERY: &str = "wikipedia";
const PROBE_MAX_RESULTS: usize = 1;

/// Send one small query to each configured backend, concurrently and each
/// bounded by its own timeout. Brave spends one request of its quota. The
/// cooldown state `web_search` keeps is left untouched.
pub async fn probe_search_backends(
    searxng_url: Option<&str>,
    brave_api_key: Option<&str>,
) -> Vec<WebSearchBackendStatus> {
    let brave = non_empty(brave_api_key).map(|key| (BRAVE_ENDPOINT, key));
    probe_endpoints(non_empty(searxng_url), brave, PROBE_TIMEOUT).await
}

fn non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|s| !s.is_empty())
}

pub(super) async fn probe_endpoints(
    searxng_url: Option<&str>,
    brave: Option<(&str, &str)>,
    timeout: Duration,
) -> Vec<WebSearchBackendStatus> {
    let opts = SearchOptions {
        max_results: PROBE_MAX_RESULTS,
        ..SearchOptions::default()
    };
    let searxng = async {
        let Some(base) = searxng_url else {
            return not_configured("searxng");
        };
        let request = get_client()
            .get(format!("{}/search", base.trim_end_matches('/')))
            .query(&searxng_params(PROBE_QUERY, &opts));
        let label = format!("searxng ({base})");
        probe_one("searxng", &label, request, timeout, |json| {
            parse_searxng_results(json, opts.page)
        })
        .await
    };
    let brave = async {
        let Some((endpoint, api_key)) = brave else {
            return not_configured("brave");
        };
        let request = get_client()
            .get(endpoint)
            .header("Accept", "application/json")
            .header("X-Subscription-Token", api_key)
            .query(&brave_params(PROBE_QUERY, &opts));
        probe_one("brave", "brave", request, timeout, |json| {
            parse_brave_results(json, &opts)
        })
        .await
    };
    let (searxng, brave) = tokio::join!(searxng, brave);
    vec![searxng, brave]
}

fn not_configured(provider: &str) -> WebSearchBackendStatus {
    status(
        provider,
        false,
        None,
        None,
        format!("{provider}: not configured"),
    )
}

fn status(
    provider: &str,
    configured: bool,
    reachable: Option<bool>,
    answered: Option<bool>,
    detail: String,
) -> WebSearchBackendStatus {
    WebSearchBackendStatus {
        provider: provider.into(),
        configured,
        reachable,
        answered,
        detail,
    }
}

/// "Reachable" means a response arrived; "answered" means it parsed as a
/// search result. Auth and rate-limit failures are reachable but unanswered.
async fn probe_one(
    provider: &str,
    label: &str,
    request: RequestBuilder,
    timeout: Duration,
    parse: impl FnOnce(&Value) -> Result<Page, String>,
) -> WebSearchBackendStatus {
    let response = match request.timeout(timeout).send().await {
        Err(e) if e.is_timeout() => {
            let detail = format!(
                "{label} unreachable: no response within {:.1}s",
                timeout.as_secs_f32()
            );
            return status(provider, true, Some(false), Some(false), detail);
        }
        Err(e) => {
            let detail = format!("{label} unreachable: {}", error_chain(&e.without_url()));
            return status(provider, true, Some(false), Some(false), detail);
        }
        Ok(response) => response,
    };
    let page = match decode_json(response).await {
        Err(e) => Err(e.reason),
        Ok(json) => parse(&json).map_err(|reason| format!("invalid response: {reason}")),
    };
    match page {
        Err(reason) => {
            let detail = format!("{label} reachable, test query failed: {reason}");
            status(provider, true, Some(true), Some(false), detail)
        }
        Ok(page) => {
            let detail = format!(
                "{label} answered a test query ({} results)",
                page.results.len()
            );
            status(provider, true, Some(true), Some(true), detail)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const TEST_TIMEOUT: Duration = Duration::from_millis(500);
    const TEST_KEY: &str = "probe-secret-key";

    async fn brave_server(response: ResponseTemplate) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/brave"))
            .and(header("X-Subscription-Token", TEST_KEY))
            .and(query_param("count", "1"))
            .respond_with(response)
            .mount(&server)
            .await;
        server
    }

    fn searxng_ok() -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_json(json!({
            "results": [{"url": "https://example.org/", "title": "t", "content": "c"}]
        }))
    }

    #[tokio::test]
    async fn both_backends_answering_are_reported_separately() {
        let searxng = MockServer::start().await;
        Mock::given(path("/search"))
            .and(query_param("format", "json"))
            .respond_with(searxng_ok())
            .mount(&searxng)
            .await;
        let brave = brave_server(ResponseTemplate::new(200).set_body_json(json!({
            "type": "search", "web": {"results": [{"url": "https://example.org/", "title": "t"}]}
        })))
        .await;
        let endpoint = format!("{}/brave", brave.uri());

        let got = probe_endpoints(
            Some(&searxng.uri()),
            Some((&endpoint, TEST_KEY)),
            TEST_TIMEOUT,
        )
        .await;

        assert_eq!(got.len(), 2);
        for s in &got {
            assert_eq!(
                (s.configured, s.reachable, s.answered),
                (true, Some(true), Some(true)),
                "{s:?}"
            );
            assert!(
                s.detail.contains("answered a test query (1 results)"),
                "{s:?}"
            );
        }
    }

    #[tokio::test]
    async fn invalid_credentials_are_reachable_but_unanswered_and_never_leak_the_key() {
        let brave = brave_server(ResponseTemplate::new(401)).await;
        let endpoint = format!("{}/brave", brave.uri());

        let got = probe_endpoints(None, Some((&endpoint, TEST_KEY)), TEST_TIMEOUT).await;

        assert_eq!(got[0], not_configured("searxng"));
        let b = &got[1];
        assert_eq!((b.reachable, b.answered), (Some(true), Some(false)));
        assert!(b.detail.contains("invalid credentials"), "{}", b.detail);
        assert!(!format!("{got:?}").contains(TEST_KEY));
    }

    #[tokio::test]
    async fn malformed_json_and_rate_limits_count_as_unanswered() {
        let searxng = MockServer::start().await;
        Mock::given(path("/search"))
            .respond_with(ResponseTemplate::new(200).set_body_string("<html>not json"))
            .mount(&searxng)
            .await;
        let brave = brave_server(ResponseTemplate::new(429)).await;
        let endpoint = format!("{}/brave", brave.uri());

        let got = probe_endpoints(
            Some(&searxng.uri()),
            Some((&endpoint, TEST_KEY)),
            TEST_TIMEOUT,
        )
        .await;

        assert_eq!(
            (got[0].reachable, got[0].answered),
            (Some(true), Some(false))
        );
        assert!(
            got[0].detail.contains("malformed JSON"),
            "{}",
            got[0].detail
        );
        assert_eq!(
            (got[1].reachable, got[1].answered),
            (Some(true), Some(false))
        );
        assert!(got[1].detail.contains("rate limited"), "{}", got[1].detail);
    }

    #[tokio::test]
    async fn a_structurally_invalid_body_is_not_an_answer() {
        let searxng = MockServer::start().await;
        Mock::given(path("/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"unexpected": true})))
            .mount(&searxng)
            .await;

        let got = probe_endpoints(Some(&searxng.uri()), None, TEST_TIMEOUT).await;

        assert_eq!(got[0].answered, Some(false));
        assert!(
            got[0].detail.contains("invalid response"),
            "{}",
            got[0].detail
        );
    }

    #[tokio::test]
    async fn a_hanging_backend_is_bounded_by_the_probe_timeout() {
        let searxng = MockServer::start().await;
        Mock::given(path("/search"))
            .respond_with(searxng_ok().set_delay(Duration::from_secs(5)))
            .mount(&searxng)
            .await;

        let start = std::time::Instant::now();
        let got = probe_endpoints(Some(&searxng.uri()), None, TEST_TIMEOUT).await;

        assert!(
            start.elapsed() < Duration::from_secs(3),
            "{:?}",
            start.elapsed()
        );
        assert_eq!(
            (got[0].reachable, got[0].answered),
            (Some(false), Some(false))
        );
        assert!(
            got[0].detail.contains("no response within"),
            "{}",
            got[0].detail
        );
    }

    #[tokio::test]
    async fn a_closed_port_is_unreachable() {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let url = format!("http://127.0.0.1:{port}");

        let got = probe_endpoints(Some(&url), None, TEST_TIMEOUT).await;

        assert_eq!(
            (got[0].reachable, got[0].answered),
            (Some(false), Some(false))
        );
        assert!(!got[0].usable());
    }
}
