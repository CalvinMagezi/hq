//! `web_fetch` against local servers: redirects, timeout, Jina fallback, empty extraction.

use super::*;
use wiremock::matchers::{method, path, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TEST_TIMEOUT: Duration = Duration::from_millis(300);
const SLOW_RESPONSE: Duration = Duration::from_secs(3);
const HTML: &str = "text/html";
const SPA_SHELL: &str = r#"<html><head><title></title></head><body><div id="root"></div><script src="/app.js"></script></body></html>"#;

/// The production redirect policy, admitting only the mock server's origin
/// so every other hop still meets the real `validate_url`.
fn test_client(server: &MockServer) -> Client {
    test_client_with(server, GuardedResolver::system())
}

fn test_client_with(server: &MockServer, resolver: GuardedResolver) -> Client {
    let origin = format!("{}/", server.uri());
    fetch_client_with_resolver(
        TEST_TIMEOUT,
        move |url| {
            if url.starts_with(&origin) {
                return Ok(());
            }
            validate_url(url)
        },
        resolver,
    )
}

/// A resolver whose DNS says every name lives on loopback, where the mock server is.
fn rebinding_resolver() -> GuardedResolver {
    GuardedResolver::with_lookup(std::sync::Arc::new(|_host| {
        Box::pin(async { Ok(vec!["127.0.0.1".parse().unwrap()]) })
    }))
}

async fn fetch(server: &MockServer, route: &str) -> Result<FetchedPage> {
    let client = test_client(server);
    let jina_base = format!("{}/jina/", server.uri());
    let fetcher = Fetcher {
        client: &client,
        timeout: TEST_TIMEOUT,
        jina_base: &jina_base,
    };
    fetcher.fetch(&format!("{}{route}", server.uri())).await
}

fn redirect_to(location: &str) -> ResponseTemplate {
    ResponseTemplate::new(302).insert_header("Location", location)
}

async fn mount(server: &MockServer, route: &str, response: ResponseTemplate) {
    Mock::given(method("GET"))
        .and(path(route))
        .respond_with(response)
        .mount(server)
        .await;
}

#[tokio::test]
async fn a_redirect_within_the_allowed_origin_is_followed() {
    let server = MockServer::start().await;
    mount(&server, "/old", redirect_to("/new")).await;
    mount(
        &server,
        "/new",
        ResponseTemplate::new(200)
            .insert_header("content-type", "text/plain")
            .set_body_string("moved content"),
    )
    .await;

    let page = fetch(&server, "/old").await.unwrap();

    assert!(page.url.ends_with("/old"));
    assert!(page.final_url.ends_with("/new"), "{}", page.final_url);
    assert_eq!(page.content, "moved content");
}

#[tokio::test]
async fn a_redirect_to_a_private_address_is_blocked_with_its_reason() {
    let server = MockServer::start().await;
    mount(
        &server,
        "/to-private",
        redirect_to("http://10.255.255.1/admin"),
    )
    .await;
    mount(
        &server,
        "/to-metadata",
        redirect_to("http://169.254.169.254/latest"),
    )
    .await;

    for route in ["/to-private", "/to-metadata"] {
        let err = fetch(&server, route).await.unwrap_err().to_string();
        assert!(err.contains("redirect blocked"), "{route}: {err}");
        assert!(err.contains("SSRF protection"), "{route}: {err}");
    }
}

#[tokio::test]
async fn a_redirect_to_loopback_is_blocked() {
    let server = MockServer::start().await;
    let port = server.address().port();
    mount(
        &server,
        "/to-localhost",
        redirect_to(&format!("http://localhost:{port}/x")),
    )
    .await;

    let err = fetch(&server, "/to-localhost")
        .await
        .unwrap_err()
        .to_string();

    assert!(err.contains("redirect blocked"), "{err}");
    assert!(err.contains("localhost"), "{err}");
}

#[tokio::test]
async fn a_redirect_loop_stops_at_the_limit() {
    let server = MockServer::start().await;
    mount(&server, "/loop", redirect_to("/loop")).await;

    let err = fetch(&server, "/loop").await.unwrap_err().to_string();

    assert!(err.contains("too many redirects"), "{err}");
    let hops = server.received_requests().await.unwrap().len();
    // Regression: the cap once counted the initial request, allowing one redirect fewer.
    assert_eq!(hops, MAX_REDIRECTS + 1, "the initial request plus every allowed redirect");
}

#[tokio::test]
async fn a_slow_page_times_out_with_the_configured_budget() {
    let server = MockServer::start().await;
    mount(
        &server,
        "/slow",
        ResponseTemplate::new(200)
            .set_body_string("late")
            .set_delay(SLOW_RESPONSE),
    )
    .await;

    let start = Instant::now();
    let err = fetch(&server, "/slow").await.unwrap_err().to_string();

    assert!(start.elapsed() < SLOW_RESPONSE, "{:?}", start.elapsed());
    assert!(err.contains("Timed out after 0.3s"), "{err}");
}

#[tokio::test]
async fn a_js_shell_falls_back_to_jina_and_discloses_it() {
    let server = MockServer::start().await;
    mount(
        &server,
        "/app",
        ResponseTemplate::new(200).set_body_raw(SPA_SHELL, HTML),
    )
    .await;
    Mock::given(method("GET"))
        .and(path_regex("^/jina/"))
        .respond_with(ResponseTemplate::new(200).set_body_string("# Rendered by the reader"))
        .expect(1)
        .mount(&server)
        .await;

    let page = fetch(&server, "/app").await.unwrap();

    assert_eq!(page.method, METHOD_JINA);
    assert_eq!(page.content, "# Rendered by the reader");
    assert!(
        page.notes.iter().any(|n| n.contains("Jina Reader")),
        "{:?}",
        page.notes
    );
    let jina_hit = server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.url.path().starts_with("/jina/"))
        .unwrap();
    assert!(
        jina_hit.url.path().ends_with("/app"),
        "Jina gets the page URL only"
    );
}

#[tokio::test]
async fn a_js_shell_jina_cannot_render_is_reported_as_empty() {
    let server = MockServer::start().await;
    mount(
        &server,
        "/app",
        ResponseTemplate::new(200).set_body_raw(SPA_SHELL, HTML),
    )
    .await;
    Mock::given(path_regex("^/jina/"))
        .respond_with(ResponseTemplate::new(200).set_body_string("   "))
        .mount(&server)
        .await;

    let page = fetch(&server, "/app").await.unwrap();

    assert_eq!(page.method, METHOD_HTML);
    assert!(page.content.trim().is_empty(), "{:?}", page.content);
    assert!(
        page.notes.iter().any(|n| n.contains("could not render")),
        "{:?}",
        page.notes
    );
    assert!(
        page.notes.iter().any(|n| n.contains("no text")),
        "{:?}",
        page.notes
    );
}

#[tokio::test]
async fn an_empty_text_body_is_an_explicit_empty_extraction() {
    let server = MockServer::start().await;
    mount(
        &server,
        "/empty.txt",
        ResponseTemplate::new(200).insert_header("content-type", "text/plain"),
    )
    .await;

    let page = fetch(&server, "/empty.txt").await.unwrap();

    assert_eq!(page.method, METHOD_TEXT);
    assert_eq!(page.total_chars, 0);
    assert!(
        page.notes.iter().any(|n| n.contains("no text")),
        "{:?}",
        page.notes
    );
    assert!(
        page.to_text().contains("no text"),
        "the note reaches the tool output"
    );
}

#[tokio::test]
async fn a_public_looking_hostname_that_resolves_to_loopback_is_refused() {
    let server = MockServer::start().await;
    let port = server.address().port();
    mount(&server, "/secret", ResponseTemplate::new(200).set_body_string("internal")).await;
    let client = test_client_with(&server, rebinding_resolver());
    let fetcher = Fetcher { client: &client, timeout: TEST_TIMEOUT, jina_base: "http://unused/" };

    let err = fetcher
        .fetch(&format!("http://rebind.example:{port}/secret"))
        .await
        .unwrap_err()
        .to_string();

    assert!(err.contains("SSRF protection"), "{err}");
    assert!(server.received_requests().await.unwrap().is_empty(), "no request may reach the server");
}

#[tokio::test]
async fn a_redirect_to_a_hostname_that_resolves_to_loopback_is_refused() {
    let server = MockServer::start().await;
    let port = server.address().port();
    mount(&server, "/hop", redirect_to(&format!("http://rebind.example:{port}/secret"))).await;
    mount(&server, "/secret", ResponseTemplate::new(200).set_body_string("internal")).await;
    let client = test_client_with(&server, rebinding_resolver());
    let fetcher = Fetcher { client: &client, timeout: TEST_TIMEOUT, jina_base: "http://unused/" };

    let err = fetcher.fetch(&format!("{}/hop", server.uri())).await.unwrap_err();

    assert!(error_text(&err).contains("SSRF protection"), "{err:?}");
    let hit_secret = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .any(|r| r.url.path() == "/secret");
    assert!(!hit_secret, "the redirect target must never be requested");
}

#[tokio::test]
async fn a_dot_localhost_name_is_refused_before_any_lookup() {
    let server = MockServer::start().await;
    let port = server.address().port();
    let client = test_client_with(&server, rebinding_resolver());
    let fetcher = Fetcher { client: &client, timeout: TEST_TIMEOUT, jina_base: "http://unused/" };

    let err = fetcher.fetch(&format!("http://api.localhost:{port}/")).await.unwrap_err();

    assert!(error_text(&err).contains("SSRF protection"), "{err:?}");
}

fn error_text(e: &anyhow::Error) -> String {
    format!("{e:#}")
}

#[tokio::test]
async fn a_configured_proxy_is_not_used_unless_opted_in() {
    let proxy = MockServer::start().await;
    let target = MockServer::start().await;
    mount(&proxy, "/ok", ResponseTemplate::new(200).set_body_string("via proxy")).await;
    mount(&target, "/ok", ResponseTemplate::new(200).set_body_string("direct")).await;
    let build = |use_proxy: bool| {
        let builder = Client::builder().proxy(reqwest::Proxy::all(proxy.uri()).unwrap());
        proxy_policy(builder, use_proxy).build().unwrap()
    };
    let url = format!("{}/ok", target.uri());

    let direct = build(false).get(&url).send().await.unwrap().text().await.unwrap();
    assert_eq!(direct, "direct");
    assert!(proxy.received_requests().await.unwrap().is_empty());

    let proxied = build(true).get(&url).send().await.unwrap().text().await.unwrap();
    assert_eq!(proxied, "via proxy", "the opt-in does route through the proxy");
}

const ARTICLE_HTML: &str = r#"<html><head><title>Post</title></head><body>
<nav><a href="/x">Navigation link text</a></nav>
<article><h1>A real post</h1>
<p>This post has enough ordinary prose in it to pass the minimum size that makes the extractor trust the article it found on the page.</p>
<p>A second paragraph keeps going so the content is clearly the point of the page and not the surrounding chrome of the site.</p>
</article><footer>Footer legal text</footer></body></html>"#;

const LD_JSON_SHELL: &str = r#"<html><head><script type="application/ld+json">{"@type":"Article","headline":"Hidden headline","articleBody":"The full text of this article is only present in the structured data of the page, which is how many client rendered sites still serve search engines, and it is long enough to count. A second sentence adds more detail about the topic so that the whole text is comfortably above the trust threshold."}</script></head><body><div id="root"></div></body></html>"#;

#[tokio::test]
async fn an_article_page_is_extracted_without_its_chrome() {
    let server = MockServer::start().await;
    mount(&server, "/post", ResponseTemplate::new(200).set_body_raw(ARTICLE_HTML, HTML)).await;

    let page = fetch(&server, "/post").await.unwrap();

    assert_eq!(page.method, METHOD_ARTICLE);
    assert!(page.content.contains("ordinary prose"), "{}", page.content);
    assert!(!page.content.contains("Navigation link text"), "{}", page.content);
    assert!(!page.content.contains("Footer legal text"), "{}", page.content);
}

#[tokio::test]
async fn a_shell_with_embedded_data_is_recovered_without_calling_jina() {
    let server = MockServer::start().await;
    mount(&server, "/app", ResponseTemplate::new(200).set_body_raw(LD_JSON_SHELL, HTML)).await;
    Mock::given(method("GET"))
        .and(path_regex("^/jina/"))
        .respond_with(ResponseTemplate::new(200).set_body_string("should not be used"))
        .expect(0)
        .mount(&server)
        .await;

    let page = fetch(&server, "/app").await.unwrap();

    assert_eq!(page.method, METHOD_EMBEDDED);
    assert!(page.content.contains("only present in the structured data"), "{}", page.content);
    assert!(page.notes.iter().any(|n| n.contains("embedded in its HTML")), "{:?}", page.notes);
}

#[tokio::test]
async fn a_shell_stays_unrendered_and_says_so_when_jina_is_disabled() {
    let server = MockServer::start().await;
    mount(&server, "/app", ResponseTemplate::new(200).set_body_raw(SPA_SHELL, HTML)).await;
    let client = test_client(&server);
    let fetcher = Fetcher { client: &client, timeout: TEST_TIMEOUT, jina_base: "" };

    let page = fetcher.fetch(&format!("{}/app", server.uri())).await.unwrap();

    assert_eq!(page.method, METHOD_HTML);
    assert!(page.notes.iter().any(|n| n.contains("fallback is disabled")), "{:?}", page.notes);
    assert_eq!(server.received_requests().await.unwrap().len(), 1, "nothing but the page was requested");
}

#[tokio::test]
async fn a_large_bundle_page_with_a_little_server_copy_still_goes_to_recovery() {
    let server = MockServer::start().await;
    let filler = "<script>var a=1;</script>".repeat(4_000);
    let html = format!(
        "<html><head><script type=\"application/ld+json\">{{\"articleBody\":\"{}\"}}</script></head><body><p>A short intro line that is real copy but not the article itself, plus a cookie notice.</p>{filler}</body></html>",
        "The full article only exists in the embedded data, written long enough to be trusted as content. ".repeat(3)
    );
    mount(&server, "/big", ResponseTemplate::new(200).set_body_raw(html, HTML)).await;

    let page = fetch(&server, "/big").await.unwrap();

    assert_eq!(page.method, METHOD_EMBEDDED, "{:?}", page.notes);
}

#[tokio::test]
async fn invisible_characters_in_a_page_are_removed_from_the_fetched_text() {
    let server = MockServer::start().await;
    let html = "<html><body><p>Visible text \u{200B}here\u{202E}.\u{E0049}\u{E0047}</p></body></html>";
    mount(&server, "/p", ResponseTemplate::new(200).set_body_raw(html, HTML)).await;

    let page = fetch(&server, "/p").await.unwrap();

    assert!(page.content.contains("Visible text here."), "{:?}", page.content);
    assert!(!page.content.chars().any(|c| matches!(c, '\u{200B}' | '\u{202E}' | '\u{E0049}')), "{:?}", page.content);
}

/// Browser Stage 0 (`docs/plans/agent-browser.md`): how often does the native
/// `web_fetch` path come back empty where a rendering reader would not?
/// `cargo test -p hq-tools bench_web_fetch_vs_jina -- --ignored --nocapture`
/// Output file: `HQ_FETCH_BENCH_OUT` (JSON). Uses the network and r.jina.ai.
#[tokio::test]
#[ignore = "needs the network and r.jina.ai"]
async fn bench_web_fetch_vs_jina() {
    const MIN_USEFUL_CHARS: usize = 500;
    const PEEK_CHARS: usize = 110;
    let corpus = include_str!("fixtures/browser-corpus.txt");
    let fetcher = Fetcher {
        client: &FETCH_CLIENT,
        timeout: FETCH_TIMEOUT,
        jina_base: "",
    };
    let (mut rows, mut render_gap, mut both_fail, mut native_ok) = (Vec::new(), 0, 0, 0);
    for url in corpus.lines().filter(|l| l.starts_with("http")) {
        let native = fetcher.fetch(url).await;
        let (n_chars, n_method, n_peek) = match &native {
            Ok(p) => (
                p.total_chars,
                p.method,
                p.content.chars().take(PEEK_CHARS).collect::<String>().replace('\n', " "),
            ),
            Err(e) => (0, "error", format!("{e}").chars().take(PEEK_CHARS).collect()),
        };
        let jina = fetch_via_jina(JINA_READER_BASE, url).await;
        let j_chars = jina.as_ref().map_or(0, |t| t.chars().count());
        let verdict = match (n_chars >= MIN_USEFUL_CHARS, j_chars >= MIN_USEFUL_CHARS) {
            (true, _) => {
                native_ok += 1;
                "native-ok"
            }
            (false, true) => {
                render_gap += 1;
                "needs-render"
            }
            (false, false) => {
                both_fail += 1;
                "both-fail"
            }
        };
        println!("{verdict:<12} native {n_chars:>6} {n_method:<14} jina {j_chars:>6}  {url}  | {n_peek}");
        rows.push(json!({"url": url, "verdict": verdict, "native_chars": n_chars,
            "native_method": n_method, "jina_chars": j_chars, "native_peek": n_peek}));
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
    let total = rows.len();
    println!(
        "\n{total} pages: native ok {native_ok}, needs render {render_gap} ({:.0}%), both fail {both_fail}",
        100.0 * render_gap as f64 / total as f64
    );
    if let Ok(out) = std::env::var("HQ_FETCH_BENCH_OUT") {
        std::fs::write(out, serde_json::to_string_pretty(&rows).unwrap()).unwrap();
    }
}
