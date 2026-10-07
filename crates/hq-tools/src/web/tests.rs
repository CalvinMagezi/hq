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
    // to run it from. With the built-in engines off and nothing else set, the
    // error names every way to enable one and never a setup script path.
    let err = web_search("test query", &SearchOptions::default(), None, None, false)
        .await
        .unwrap_err();
    let msg = err.to_string();
    assert!(!msg.contains("scripts/setup-searxng.sh"), "{msg}");
    assert!(msg.contains("web_search_native"), "{msg}");
    assert!(msg.contains("searxng_url"), "{msg}");
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
        json!({ "category": "videos" }),
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
    native_engine: Duration::from_millis(400),
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
    let r = search_chain("rates", &opts, Some(&sx.uri()), None, None, &FAST)
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
    let r = search_chain("q", &opts, None, Some((&endpoint, "k")), None, &FAST)
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

    let r = search_chain("q", &opts, Some(&sx.uri()), Some((&endpoint, "k")), None, &FAST)
        .await
        .unwrap();
    assert_eq!(r.backend.as_deref(), Some("brave"));
    assert!(
        r.attempts[0].outcome.contains("malformed JSON"),
        "{:?}",
        r.attempts
    );

    let again = search_chain("q", &opts, Some(&sx.uri()), Some((&endpoint, "k")), None, &FAST)
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
    let err = search_chain("q", &SearchOptions::default(), Some(&sx.uri()), None, None, &FAST)
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
        None, &FAST,
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
        native_engine: Duration::from_secs(5),
        brave: Duration::from_secs(5),
    };
    let err = search_chain(
        "q",
        &SearchOptions::default(),
        Some(&sx.uri()),
        Some((&endpoint, "k")),
        None,
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
    let err = search_chain("q", &opts, None, Some((&endpoint, "secret-key")), None, &FAST)
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("rate limited"), "{err}");
    assert!(!err.contains("secret-key"));
    let err = search_chain("q", &opts, None, Some((&endpoint, "secret-key")), None, &FAST)
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
        None, &FAST,
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
    let r = search_chain("q", &SearchOptions::default(), Some(&sx.uri()), None, None, &FAST)
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
    let err = search_chain("q", &opts, None, Some((&endpoint, "k")), None, &FAST)
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
        let r = web_search("tokio release", &opts, sx, key, false).await.unwrap();
        println!("{label}: {}", format_search_results(&r));
        assert!(r.results.iter().all(|x| x.url.contains("github.com")));
    }
}
