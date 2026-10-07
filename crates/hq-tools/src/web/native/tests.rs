use super::engines::{Engine, arxiv_terms};
use super::*;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const DDG_HTML: &str = r#"<html><body>
<div class="result results_links result--ad"><h2 class="result__title"><a class="result__a" href="https://ads.example.com/x">Sponsored</a></h2></div>
<div class="result results_links web-result"><div class="result__body">
  <h2 class="result__title"><a class="result__a" href="https://docs.rs/tokio/latest/tokio/runtime/">tokio::runtime - Rust</a></h2>
  <a class="result__snippet" href="https://docs.rs/tokio/latest/tokio/runtime/">The <b>Tokio</b> runtime. Unlike other <b>Rust</b> programs.</a>
</div></div>
<div class="result results_links web-result"><div class="result__body">
  <h2 class="result__title"><a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Ftokio.rs%2F&amp;rut=abc">Tokio</a></h2>
  <a class="result__snippet" href="x">An asynchronous Rust runtime.</a>
</div></div></body></html>"#;

const BRAVE_HTML: &str = r#"<html><body>
<div class="snippet svelte-x" data-pos="0" data-type="web"><div class="result-content"><a href="https://docs.rs/tokio/latest/tokio/runtime" class="l1"><cite class="snippet-url">docs.rs</cite><div class="title" title="tokio::runtime">tokio::runtime - Rust</div></a><div class="generic-snippet"><div class="content">The Tokio runtime.</div></div></div></div>
<div class="snippet svelte-x" id="discussions"><a href="https://reddit.com/r/rust">not a web result</a></div>
</body></html>"#;

const BING_NEWS_RSS: &str = r#"<?xml version="1.0"?><rss><channel><title>x - BingNews</title>
<item><title>Rust 2026 edition lands</title><link>http://www.bing.com/news/apiclick.aspx?ref=FexRss&amp;aid=&amp;url=https%3a%2f%2fexample.com%2fnews%2frust&amp;c=1</link><description>The edition &amp; its changes.</description><pubDate>Fri, 25 Sep 2026 15:20:00 GMT</pubDate></item>
</channel></rss>"#;

const ARXIV_ATOM: &str = r#"<?xml version='1.0'?><feed xmlns="http://www.w3.org/2005/Atom"><title>arXiv Query</title>
<entry><id>http://arxiv.org/abs/2201.00978v1</id><title>PyramidTNT:
  Improved Transformer</title><link href="https://arxiv.org/abs/2201.00978v1" rel="alternate" type="text/html"/><link href="https://arxiv.org/pdf/2201.00978v1" rel="related" type="application/pdf" title="pdf"/><summary>Transformer networks have achieved great progress.</summary><published>2022-01-04T04:56:57Z</published></entry></feed>"#;

fn hit(url: &str, snippet: &str, engine: &str) -> SearchResult {
    SearchResult {
        title: url.into(),
        url: url.into(),
        snippet: snippet.into(),
        position: 0,
        provider: "native".into(),
        domain: None,
        published: None,
        engines: vec![engine.into()],
    }
}

fn parsed(engine: Engine, body: &str, base: &str) -> Vec<SearchResult> {
    engine.parse(&Value::String(body.into()), base).unwrap()
}

#[test]
fn duckduckgo_skips_ads_and_unwraps_redirect_links() {
    let r = parsed(Engine::DuckDuckGo, DDG_HTML, "");
    let urls: Vec<&str> = r.iter().map(|h| h.url.as_str()).collect();
    assert_eq!(
        urls,
        [
            "https://docs.rs/tokio/latest/tokio/runtime/",
            "https://tokio.rs/"
        ]
    );
    assert_eq!(
        r[0].snippet,
        "The Tokio runtime. Unlike other Rust programs."
    );
    assert_eq!(r[0].engines, ["duckduckgo"]);
    assert_eq!(r[0].provider, "native");
}

#[test]
fn brave_html_keeps_only_web_results() {
    let r = parsed(Engine::Brave, BRAVE_HTML, "");
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].title, "tokio::runtime - Rust");
    assert_eq!(r[0].snippet, "The Tokio runtime.");
}

#[test]
fn bing_news_extracts_target_url_and_date() {
    let r = parsed(Engine::BingNews, BING_NEWS_RSS, "");
    assert_eq!(r[0].url, "https://example.com/news/rust");
    assert_eq!(r[0].snippet, "The edition & its changes.");
    assert_eq!(
        r[0].published.as_deref(),
        Some("Fri, 25 Sep 2026 15:20:00 GMT")
    );
}

#[test]
fn arxiv_prefers_the_alternate_link_and_flattens_titles() {
    let r = parsed(Engine::Arxiv, ARXIV_ATOM, "");
    assert_eq!(r[0].url, "https://arxiv.org/abs/2201.00978v1");
    assert_eq!(r[0].title, "PyramidTNT: Improved Transformer");
    assert_eq!(r[0].published.as_deref(), Some("2022-01-04T04:56:57Z"));
}

#[test]
fn wikipedia_builds_article_urls_and_strips_markup() {
    let body = json!({"query": {"search": [
        {"title": "Tokio (software)", "snippet": "a <span class=\"searchmatch\">Rust</span> library", "timestamp": "2026-01-01T00:00:00Z"}
    ]}});
    let r = Engine::Wikipedia
        .parse(&body, "https://en.wikipedia.org")
        .unwrap();
    assert_eq!(r[0].url, "https://en.wikipedia.org/wiki/Tokio_(software)");
    assert_eq!(r[0].snippet, "a Rust library");
}

#[test]
fn hacker_news_falls_back_to_the_discussion_link() {
    let body = json!({"hits": [
        {"title": "Ask HN: Rust?", "objectID": "42", "points": 7, "num_comments": 3, "created_at": "2026-01-01T00:00:00Z"},
        {"title": "Linked story", "objectID": "43", "url": "https://example.com/a"}
    ]});
    let r = Engine::HackerNews.parse(&body, "").unwrap();
    assert_eq!(r[0].url, "https://news.ycombinator.com/item?id=42");
    assert_eq!(r[1].url, "https://example.com/a");
}

#[test]
fn openalex_rebuilds_the_abstract_in_word_order() {
    let body = json!({"results": [{
        "display_name": "Attention", "publication_date": "2017-06-12",
        "primary_location": {"landing_page_url": "https://arxiv.org/abs/1706.03762"},
        "abstract_inverted_index": {"is": [1], "Attention": [0], "all": [3], "you": [4], "need": [5], "what": [2]}
    }]});
    let r = Engine::OpenAlex.parse(&body, "").unwrap();
    assert_eq!(r[0].snippet, "Attention is what all you need");
    assert_eq!(r[0].url, "https://arxiv.org/abs/1706.03762");
}

#[test]
fn a_challenge_page_is_a_failure_but_an_empty_result_page_is_not() {
    let challenge = Engine::Brave.parse(
        &Value::String("<html>Please solve this CAPTCHA</html>".into()),
        "",
    );
    assert!(challenge.unwrap_err().contains("challenge"));
    let empty = Engine::Brave.parse(
        &Value::String("<html><p>No results found for zzzz</p></html>".into()),
        "",
    );
    assert!(empty.unwrap().is_empty());
}

#[test]
fn merge_ranks_agreement_first_and_dedupes_tracking_variants() {
    let a = vec![
        hit("https://a.com/one?utm_source=x", "short", "duckduckgo"),
        hit("https://b.com/two", "b", "duckduckgo"),
    ];
    let b = vec![
        hit("https://www.b.com/two/", "b with a longer snippet", "bing"),
        hit("https://c.com/three", "c", "bing"),
    ];
    let merged = merge(vec![(1.0, a), (1.0, b)]);
    let urls: Vec<&str> = merged.iter().map(|r| r.url.as_str()).collect();
    assert_eq!(
        urls,
        [
            "https://b.com/two",
            "https://a.com/one?utm_source=x",
            "https://c.com/three"
        ]
    );
    assert_eq!(merged[0].engines, ["duckduckgo", "bing"]);
    assert_eq!(merged[0].snippet, "b with a longer snippet");
    assert_eq!(
        merged.iter().map(|r| r.position).collect::<Vec<_>>(),
        [1, 2, 3]
    );
}

#[test]
fn urls_that_differ_in_real_parameters_stay_distinct() {
    assert_ne!(
        normalize_url("https://a.com/p?id=1"),
        normalize_url("https://a.com/p?id=2")
    );
    assert_eq!(
        normalize_url("http://www.a.com/p/#top"),
        normalize_url("https://a.com/p")
    );
}

async fn native_server() -> MockServer {
    let server = MockServer::start().await;
    let uri = server.uri();
    HEALTH
        .lock()
        .unwrap()
        .retain(|key, _| !key.starts_with(&uri));
    RESULT_CACHE
        .lock()
        .unwrap()
        .retain(|key, _| !key.ends_with(&uri));
    server
}

async fn mount_general(server: &MockServer, ddg_status: u16) {
    Mock::given(method("POST"))
        .and(path("/html/"))
        .respond_with(ResponseTemplate::new(ddg_status).set_body_string(DDG_HTML))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/search"))
        .and(query_param("source", "web"))
        .respond_with(ResponseTemplate::new(200).set_body_string(BRAVE_HTML))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/w/api.php"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"query": {"search": []}})))
        .mount(server)
        .await;
}

fn env(base: &str) -> NativeEnv<'_> {
    NativeEnv {
        client: get_client(),
        base_override: Some(base),
    }
}

async fn run(server: &MockServer, query: &str, opts: &SearchOptions) -> Result<WebSearchResults> {
    let base = server.uri();
    let budgets = Budgets {
        total: Duration::from_secs(5),
        searxng: Duration::from_millis(400),
        native_engine: Duration::from_secs(2),
        brave: Duration::from_millis(400),
    };
    search_chain(query, opts, None, None, Some(&env(&base)), &budgets).await
}

#[tokio::test]
async fn pool_merges_engines_and_reports_each_attempt() {
    let server = native_server().await;
    mount_general(&server, 200).await;
    let r = run(&server, "tokio runtime", &SearchOptions::default())
        .await
        .unwrap();
    assert_eq!(r.backend.as_deref(), Some("native"));
    assert_eq!(
        r.results[0].url,
        "https://docs.rs/tokio/latest/tokio/runtime/"
    );
    assert_eq!(r.results[0].engines, ["duckduckgo", "brave"]);
    let ok: Vec<&str> = r
        .attempts
        .iter()
        .filter(|a| a.outcome.starts_with("ok"))
        .map(|a| a.provider.as_str())
        .collect();
    assert_eq!(ok, ["duckduckgo", "brave", "wikipedia"], "{:?}", r.attempts);
}

#[tokio::test]
async fn one_failing_engine_does_not_stop_the_others() {
    let server = native_server().await;
    mount_general(&server, 500).await;
    let r = run(&server, "failing engine query", &SearchOptions::default())
        .await
        .unwrap();
    assert_eq!(r.backend.as_deref(), Some("native"));
    assert!(
        r.attempts
            .iter()
            .any(|a| a.provider == "duckduckgo" && a.outcome.contains("500")),
        "{:?}",
        r.attempts
    );
    assert!(!r.results.is_empty());
}

#[tokio::test]
async fn second_identical_search_is_served_from_cache() {
    let server = native_server().await;
    mount_general(&server, 200).await;
    run(&server, "cached query", &SearchOptions::default())
        .await
        .unwrap();
    let before = server.received_requests().await.unwrap().len();
    let again = run(&server, "cached query", &SearchOptions::default())
        .await
        .unwrap();
    assert_eq!(server.received_requests().await.unwrap().len(), before);
    assert!(
        again.attempts[0].outcome.contains("cached"),
        "{:?}",
        again.attempts
    );
}

#[tokio::test]
async fn all_engines_failing_reports_every_reason() {
    let server = native_server().await;
    Mock::given(wiremock::matchers::any())
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    let err = run(&server, "everything down", &SearchOptions::default())
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("All web search backends failed"), "{err}");
    assert!(
        err.contains("duckduckgo") && err.contains("wikipedia"),
        "{err}"
    );
}

#[tokio::test]
async fn science_category_queries_arxiv_and_openalex() {
    let server = native_server().await;
    Mock::given(method("GET"))
        .and(path("/api/query"))
        .and(query_param(
            "search_query",
            "all:attention AND all:you AND all:need",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_string(ARXIV_ATOM))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/works"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"results": []})))
        .mount(&server)
        .await;
    let opts = SearchOptions {
        category: Some(Category::Science),
        ..SearchOptions::default()
    };
    let r = run(&server, "attention is all you need", &opts)
        .await
        .unwrap();
    assert_eq!(r.results[0].url, "https://arxiv.org/abs/2201.00978v1");
}

#[tokio::test]
async fn later_pages_skip_engines_that_cannot_page() {
    let server = native_server().await;
    mount_general(&server, 200).await;
    let opts = SearchOptions {
        page: 2,
        ..SearchOptions::default()
    };
    let r = run(&server, "paged query", &opts).await.unwrap();
    assert!(
        r.attempts
            .iter()
            .any(|a| a.provider == "duckduckgo" && a.outcome.contains("page 1 only")),
        "{:?}",
        r.attempts
    );
    assert_eq!(r.page, 2);
}

#[tokio::test]
async fn empty_pool_falls_through_to_brave() {
    let server = native_server().await;
    Mock::given(wiremock::matchers::any())
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    let brave = fresh_server_for_brave().await;
    let endpoint = format!("{}/res/v1/web/search", brave.uri());
    let base = server.uri();
    let budgets = Budgets {
        total: Duration::from_secs(5),
        searxng: Duration::from_millis(400),
        native_engine: Duration::from_secs(2),
        brave: Duration::from_secs(2),
    };
    let r = search_chain(
        "brave fallback",
        &SearchOptions::default(),
        None,
        Some((&endpoint, "k")),
        Some(&env(&base)),
        &budgets,
    )
    .await
    .unwrap();
    assert_eq!(r.backend.as_deref(), Some("brave"));
}

async fn fresh_server_for_brave() -> MockServer {
    let brave = MockServer::start().await;
    HEALTH
        .lock()
        .unwrap()
        .retain(|key, _| !key.starts_with(&brave.uri()));
    Mock::given(method("GET"))
        .and(path("/res/v1/web/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "type": "search",
            "web": {"results": [{"title": "t", "url": "https://example.com/", "description": "d"}]},
            "query": {"more_results_available": false}
        })))
        .mount(&brave)
        .await;
    brave
}

/// Diagnostic and weekly canary against the real engines:
/// `cargo test -p hq-tools live_native -- --ignored --nocapture`. Every engine
/// in `required` must have answered, so one broken parser cannot hide behind
/// the others. For general queries the scraped web engines are rotated out by
/// IP reputation (a cloud runner gets a DuckDuckGo challenge or a Brave 429, a
/// home connection often does not), so one of them answering is enough.
#[tokio::test]
#[ignore = "needs the network and live engines"]
async fn live_native_pool_answers_each_category() {
    type Case = (
        &'static str,
        Option<Category>,
        &'static [&'static str],
        &'static [&'static str],
    );
    let cases: [Case; 5] = [
        (
            "tokio runtime",
            None,
            &["wikipedia"],
            &["brave", "duckduckgo"],
        ),
        (
            "rust language",
            Some(Category::News),
            &["bing news", "hacker news"],
            &[],
        ),
        (
            "attention is all you need",
            Some(Category::Science),
            &["arxiv", "openalex"],
            &[],
        ),
        (
            "red panda",
            Some(Category::Images),
            &["wikimedia commons", "openverse"],
            &[],
        ),
        (
            "tokio runtime",
            Some(Category::Code),
            &["github", "stack overflow", "crates.io", "npm"],
            &[],
        ),
    ];
    let mut broken = Vec::new();
    for (query, category, required, any_of) in cases {
        let opts = SearchOptions {
            category,
            max_results: 8,
            ..SearchOptions::default()
        };
        let r = web_search(query, &opts, None, None, true).await.unwrap();
        println!("{query} [{category:?}] via {:?}", r.backend);
        for a in &r.attempts {
            println!("  {}: {}", a.provider, a.outcome);
        }
        for h in &r.results {
            println!("  {}. {} <{}> {:?}", h.position, h.title, h.url, h.engines);
        }
        let answered = |engine: &str| {
            r.attempts.iter().any(|a| {
                a.provider == engine
                    && a.outcome.starts_with("ok")
                    && !a.outcome.starts_with("ok, 0 ")
            })
        };
        broken.extend(
            required
                .iter()
                .filter(|e| !answered(e))
                .map(|e| format!("{e} ({query})")),
        );
        if !any_of.is_empty() && !any_of.iter().any(|e| answered(e)) {
            broken.push(format!("none of {any_of:?} ({query})"));
        }
    }
    assert!(broken.is_empty(), "engines that did not answer: {broken:?}");
}

#[test]
fn arxiv_terms_drop_stop_words_and_query_syntax() {
    assert_eq!(
        arxiv_terms("attention is all you need"),
        ["attention", "you", "need"]
    );
    assert_eq!(
        arxiv_terms("cat:cs.LG (graph) \"nets\""),
        ["catcsLG", "graph", "nets"]
    );
    assert_eq!(
        arxiv_terms("the of"),
        ["the", "of"],
        "all stop words keeps the query"
    );
}

#[test]
fn arxiv_error_feed_is_not_a_result() {
    let feed = r#"<feed><entry><id>http://arxiv.org/api/errors#incorrect_id_format</id><title>Error</title><summary>bad</summary></entry></feed>"#;
    assert!(parsed(Engine::Arxiv, feed, "").is_empty());
}

#[test]
fn news_freshness_is_reported_as_partly_unsupported_and_general_is_not() {
    let news = SearchOptions {
        category: Some(Category::News),
        freshness: Some(Freshness::Week),
        ..SearchOptions::default()
    };
    assert!(native_unsupported(&news)[0].contains("Bing News ignores it"));
    let general = SearchOptions {
        freshness: Some(Freshness::Year),
        ..SearchOptions::default()
    };
    assert!(native_unsupported(&general).is_empty());
}

async fn mount_only_wikipedia(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/w/api.php"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"query": {"search": [{"title": "Rust", "snippet": "a language"}]}}),
        ))
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/html/"))
        .respond_with(ResponseTemplate::new(503))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/search"))
        .respond_with(ResponseTemplate::new(503))
        .mount(server)
        .await;
}

#[tokio::test]
async fn a_pool_with_only_wikipedia_answering_yields_to_the_brave_api() {
    let server = native_server().await;
    mount_only_wikipedia(&server).await;
    let brave = fresh_server_for_brave().await;
    let endpoint = format!("{}/res/v1/web/search", brave.uri());
    let base = server.uri();
    let budgets = Budgets {
        total: Duration::from_secs(5),
        searxng: Duration::from_millis(400),
        native_engine: Duration::from_secs(2),
        brave: Duration::from_secs(2),
    };
    let r = search_chain(
        "degraded pool",
        &SearchOptions::default(),
        None,
        Some((&endpoint, "k")),
        Some(&env(&base)),
        &budgets,
    )
    .await
    .unwrap();
    assert_eq!(r.backend.as_deref(), Some("brave"));
}

#[tokio::test]
async fn a_degraded_pool_is_still_returned_when_nothing_else_is_configured_and_is_not_cached() {
    let server = native_server().await;
    mount_only_wikipedia(&server).await;
    let r = run(&server, "wikipedia only", &SearchOptions::default())
        .await
        .unwrap();
    assert_eq!(r.backend.as_deref(), Some("native"));
    assert_eq!(r.results[0].engines, ["wikipedia"]);
    let key = cache_key(
        "wikipedia only",
        &SearchOptions::default(),
        Some(&server.uri()),
    );
    assert!(
        cached_page(&key).is_none(),
        "a degraded answer must not be cached"
    );
}

#[tokio::test]
async fn an_empty_searxng_answer_falls_through_to_the_pool() {
    let server = native_server().await;
    mount_general(&server, 200).await;
    let sx = native_server().await;
    Mock::given(method("GET"))
        .and(path("/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"results": []})))
        .mount(&sx)
        .await;
    let budgets = Budgets {
        total: Duration::from_secs(5),
        searxng: Duration::from_secs(2),
        native_engine: Duration::from_secs(2),
        brave: Duration::from_millis(400),
    };
    let base = server.uri();
    let r = search_chain(
        "searxng empty",
        &SearchOptions::default(),
        Some(&sx.uri()),
        None,
        Some(&env(&base)),
        &budgets,
    )
    .await
    .unwrap();
    assert_eq!(r.backend.as_deref(), Some("native"));
    assert_eq!(r.attempts[0].provider, "searxng");
}

#[test]
fn the_result_cache_never_exceeds_its_cap() {
    let page = Page {
        results: vec![hit("https://a.com/", "s", "brave")],
        next_page: None,
    };
    for i in 0..(CACHE_MAX_ENTRIES + 50) {
        store_page(format!("cap-test-{i}"), &page, Duration::from_secs(600));
    }
    assert!(RESULT_CACHE.lock().unwrap().len() <= CACHE_MAX_ENTRIES);
}

#[test]
fn commons_results_lead_with_the_direct_image_url_and_carry_license_and_author() {
    let body = json!({"query": {"pages": {
        "2": {"index": 2, "title": "File:Second.jpg", "imageinfo": [{"mime": "image/jpeg", "url": "https://upload.example/2.jpg", "descriptionurl": "https://commons.example/File:Second.jpg", "width": 10, "height": 20, "extmetadata": {}}]},
        "1": {"index": 1, "title": "File:Red Panda.jpg", "imageinfo": [{"mime": "image/jpeg", "url": "https://upload.example/1.jpg", "descriptionurl": "https://commons.example/File:Red_Panda.jpg", "width": 3900, "height": 2583,
              "extmetadata": {"LicenseShortName": {"value": "CC0"}, "Artist": {"value": "<a href=\"x\">Jane Doe</a>"}, "ImageDescription": {"value": "A <b>red panda</b> in a tree"}}}]}
    }}});
    let r = Engine::CommonsImages.parse(&body, "").unwrap();
    assert_eq!(r[0].title, "Red Panda.jpg");
    assert_eq!(r[0].url, "https://commons.example/File:Red_Panda.jpg");
    assert_eq!(
        r[0].snippet,
        "Image: https://upload.example/1.jpg | 3900x2583 | CC0 | Jane Doe. A red panda in a tree"
    );
    assert_eq!(
        r[1].title, "Second.jpg",
        "ordered by the API's search index"
    );
    assert!(
        Engine::CommonsImages
            .parse(&json!({"batchcomplete": ""}), "")
            .unwrap()
            .is_empty()
    );
}

#[test]
fn commons_skips_video_and_documents_and_non_web_urls() {
    let page = |index: u64, title: &str, mime: &str, url: &str| json!({"index": index, "title": title, "imageinfo": [{"mime": mime, "url": url, "descriptionurl": "https://commons.example/x", "width": 1, "height": 1, "extmetadata": {}}]});
    let body = json!({"query": {"pages": {
        "1": page(1, "File:clip.webm", "video/webm", "https://upload.example/clip.webm"),
        "2": page(2, "File:doc.pdf", "application/pdf", "https://upload.example/doc.pdf"),
        "3": page(3, "File:evil.jpg", "image/jpeg", "javascript:alert(1)"),
        "4": page(4, "File:ok.jpg", "image/jpeg", "https://upload.example/ok.jpg"),
    }}});
    let r = Engine::CommonsImages.parse(&body, "").unwrap();
    assert_eq!(
        r.iter().map(|h| h.title.as_str()).collect::<Vec<_>>(),
        ["ok.jpg"]
    );
}

#[test]
fn openverse_formats_licenses_and_falls_back_to_the_image_url() {
    let body = json!({"results": [
        {"title": "Red Panda", "url": "https://img.example/a.jpg", "foreign_landing_url": "https://flickr.example/a", "license": "by-nd", "license_version": "2.0", "creator": "Chester Zoo", "width": 1024, "height": 685},
        {"url": "https://img.example/b.jpg"}
    ]});
    let r = Engine::Openverse.parse(&body, "").unwrap();
    assert_eq!(
        r[0].snippet,
        "Image: https://img.example/a.jpg | 1024x685 | CC BY-ND 2.0 | Chester Zoo"
    );
    assert_eq!(r[1].url, "https://img.example/b.jpg");
}

#[test]
fn openverse_labels_public_domain_licenses_without_a_cc_prefix_and_drops_non_web_urls() {
    let body = json!({"results": [
        {"title": "A", "url": "https://img.example/a.jpg", "license": "cc0", "license_version": "1.0"},
        {"title": "B", "url": "data:image/png;base64,AAAA"},
        {"title": "C", "url": "ftp://img.example/c.jpg"}
    ]});
    let r = Engine::Openverse.parse(&body, "").unwrap();
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].snippet, "Image: https://img.example/a.jpg | CC0");
}

#[test]
fn github_and_stack_overflow_results_summarise_stars_and_votes() {
    let gh = json!({"items": [{"full_name": "tokio-rs/tokio", "html_url": "https://github.com/tokio-rs/tokio", "description": "A runtime", "language": "Rust", "stargazers_count": 33351, "pushed_at": "2026-10-04T14:24:49Z"}]});
    let r = Engine::GitHub.parse(&gh, "").unwrap();
    assert_eq!(r[0].snippet, "A runtime (Rust, 33351 stars)");
    assert_eq!(r[0].published.as_deref(), Some("2026-10-04T14:24:49Z"));

    let so = json!({"items": [{"title": "Can&#39;t nest runtimes", "link": "https://stackoverflow.com/q/1", "score": 32, "is_answered": true, "answer_count": 2, "tags": ["rust", "rust-tokio"], "creation_date": 1592920926}]});
    let r = Engine::StackOverflow.parse(&so, "").unwrap();
    assert_eq!(r[0].title, "Can't nest runtimes");
    assert_eq!(
        r[0].snippet,
        "32 votes, 2 answers (answered); tags: rust, rust-tokio"
    );
    assert!(r[0].published.as_deref().unwrap().starts_with("2020-06-23"));
}

#[test]
fn package_registries_name_the_registry_and_rank_below_the_main_code_engines() {
    let crates = json!({"crates": [{"name": "tokio", "description": "An event-driven\n platform", "max_version": "1.53.2", "downloads": 1035898722u64, "updated_at": "2026-10-03T11:18:32Z"}]});
    let r = Engine::Crates.parse(&crates, "").unwrap();
    assert_eq!(
        (r[0].title.as_str(), r[0].url.as_str()),
        ("tokio (crates.io)", "https://crates.io/crates/tokio")
    );
    assert_eq!(
        r[0].snippet,
        "An event-driven platform (v1.53.2, 1035898722 downloads)"
    );

    let npm = json!({"objects": [{"package": {"name": "tokio", "description": "Scraping", "version": "0.1.2", "date": "2018-05-14T01:00:06Z", "links": {"npm": "https://www.npmjs.com/package/tokio"}}}]});
    assert_eq!(Engine::Npm.parse(&npm, "").unwrap()[0].title, "tokio (npm)");
    assert!(
        Engine::Crates.weight() < Engine::GitHub.weight()
            && Engine::Npm.weight() < Engine::StackOverflow.weight()
    );
}

#[tokio::test]
async fn code_category_queries_each_code_engine_without_site_operators() {
    let server = native_server().await;
    Mock::given(method("GET")).and(path("/search/repositories")).and(query_param("q", "tokio runtime"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"items": [{"full_name": "tokio-rs/tokio", "html_url": "https://github.com/tokio-rs/tokio", "description": "rt", "stargazers_count": 1}]})))
        .mount(&server).await;
    Mock::given(method("GET"))
        .and(path("/2.3/search/advanced"))
        .and(query_param("site", "stackoverflow"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"items": []})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/crates"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"crates": []})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/-/v1/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"objects": []})))
        .mount(&server)
        .await;
    let opts = SearchOptions {
        category: Some(Category::Code),
        include_domains: vec!["github.com".into()],
        ..SearchOptions::default()
    };
    let r = run(&server, "tokio runtime", &opts).await.unwrap();
    assert_eq!(r.results[0].url, "https://github.com/tokio-rs/tokio");
    assert_eq!(r.attempts.len(), 4, "{:?}", r.attempts);
    assert!(
        r.attempts.iter().all(|a| a.outcome.starts_with("ok")),
        "{:?}",
        r.attempts
    );
}

#[tokio::test]
async fn code_engines_get_their_paging_and_freshness_parameters() {
    let server = native_server().await;
    Mock::given(wiremock::matchers::any())
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"items": [], "crates": [], "objects": []})),
        )
        .mount(&server)
        .await;
    let opts = SearchOptions {
        category: Some(Category::Code),
        page: 3,
        max_results: 10,
        freshness: Some(Freshness::Week),
        ..SearchOptions::default()
    };
    run(&server, "paging params", &opts).await.unwrap();
    let urls: Vec<String> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| r.url.to_string())
        .collect();
    let find = |needle: &str| {
        urls.iter()
            .find(|u| u.contains(needle))
            .unwrap_or_else(|| panic!("no {needle} request in {urls:?}"))
            .clone()
    };
    let gh = find("/search/repositories");
    assert!(
        gh.contains("page=3") && gh.contains("per_page=10") && gh.contains("pushed%3A%3E"),
        "{gh}"
    );
    let so = find("/2.3/search/advanced");
    assert!(
        so.contains("site=stackoverflow") && so.contains("page=3") && so.contains("fromdate="),
        "{so}"
    );
    assert!(find("/-/v1/search").contains("from=20"));
    assert!(find("/api/v1/crates").contains("page=3"));
}

#[tokio::test]
async fn a_github_quota_403_and_a_stack_exchange_throttle_400_are_rate_limits() {
    let server = native_server().await;
    Mock::given(method("GET"))
        .and(path("/search/repositories"))
        .respond_with(ResponseTemplate::new(403).insert_header("x-ratelimit-remaining", "0"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/2.3/search/advanced"))
        .respond_with(
            ResponseTemplate::new(400)
                .set_body_json(json!({"error_id": 502, "error_name": "throttle_violation"})),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/crates"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/-/v1/search"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"objects": []})))
        .mount(&server)
        .await;
    let opts = SearchOptions {
        category: Some(Category::Code),
        ..SearchOptions::default()
    };
    let r = run(&server, "rate limited engines", &opts).await.unwrap();
    let outcome = |engine: &str| {
        r.attempts
            .iter()
            .find(|a| a.provider == engine)
            .unwrap()
            .outcome
            .clone()
    };
    assert!(
        outcome("github").contains("rate limited"),
        "{:?}",
        r.attempts
    );
    assert!(
        outcome("stack overflow").contains("rate limited"),
        "{:?}",
        r.attempts
    );
    assert!(
        outcome("crates.io").contains("access denied"),
        "a plain 403 stays an access error: {:?}",
        r.attempts
    );
}

#[test]
fn the_new_categories_parse_and_brave_reports_them_unsupported() {
    for (word, cat) in [("images", Category::Images), ("code", Category::Code)] {
        let opts = SearchOptions::from_args(&json!({"category": word})).unwrap();
        assert_eq!(opts.category, Some(cat));
        assert_eq!(
            brave_unsupported(&opts),
            [format!("category={word} (Brave supports general and news)")]
        );
    }
}
