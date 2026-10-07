use super::super::*;
use super::markup::{attr_value, blocks, collapse_ws, html_text, sel, tag_text};
use super::{google, images_code};

/// Browsers get scraped HTML; API engines get the descriptive agent string.
pub(super) const BROWSER_USER_AGENT: &str =
    "Mozilla/5.0 (X11; Linux x86_64; rv:128.0) Gecko/20100101 Firefox/128.0";
const DEFAULT_ACCEPT_LANGUAGE: &str = "en-US,en;q=0.9";
const DDG_REFERER: &str = "https://html.duckduckgo.com/";
/// Wikimedia throttles browser-like generic agents and asks API clients to say what they are.
const WIKIMEDIA_USER_AGENT: &str = concat!(
    "HQ-Agent/",
    env!("CARGO_PKG_VERSION"),
    " (open-source agent hub; web_search)"
);
/// Same limits SearxNG applies to every result before merging.
const TITLE_MAX_CHARS: usize = 200;
const SNIPPET_MAX_CHARS: usize = 1200;
const WEIGHT_SUPPLEMENTARY: f32 = 0.4;
const CHALLENGE_MARKERS: &[&str] = &[
    "captcha",
    "unusual traffic",
    "are you a robot",
    "anomaly",
    "verify you are human",
    "enable javascript",
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::web) enum Engine {
    GoogleCse,
    GoogleCseImages,
    DuckDuckGo,
    Brave,
    Wikipedia,
    BingNews,
    HackerNews,
    Arxiv,
    OpenAlex,
    CommonsImages,
    Openverse,
    GitHub,
    StackOverflow,
    Crates,
    Npm,
    Mdn,
    AskUbuntu,
    SuperUser,
    EuropePmc,
}

const GENERAL: &[Engine] = &[
    Engine::GoogleCse,
    Engine::DuckDuckGo,
    Engine::Brave,
    Engine::Wikipedia,
];
const NEWS: &[Engine] = &[Engine::BingNews, Engine::HackerNews];
const SCIENCE: &[Engine] = &[Engine::Arxiv, Engine::OpenAlex, Engine::EuropePmc];
const IMAGES: &[Engine] = &[
    Engine::GoogleCseImages,
    Engine::CommonsImages,
    Engine::Openverse,
];
const CODE: &[Engine] = &[
    Engine::GitHub,
    Engine::StackOverflow,
    Engine::AskUbuntu,
    Engine::SuperUser,
    Engine::Mdn,
    Engine::Crates,
    Engine::Npm,
];

impl Engine {
    pub(in crate::web) fn for_category(category: Option<Category>) -> &'static [Engine] {
        match category {
            None | Some(Category::General) => GENERAL,
            Some(Category::News) => NEWS,
            Some(Category::Science) => SCIENCE,
            Some(Category::Images) => IMAGES,
            Some(Category::Code) => CODE,
        }
    }

    pub(in crate::web) fn name(self) -> &'static str {
        match self {
            Engine::GoogleCse => "google cse",
            Engine::GoogleCseImages => "google cse images",
            Engine::DuckDuckGo => "duckduckgo",
            Engine::Brave => "brave",
            Engine::Wikipedia => "wikipedia",
            Engine::BingNews => "bing news",
            Engine::HackerNews => "hacker news",
            Engine::Arxiv => "arxiv",
            Engine::OpenAlex => "openalex",
            Engine::CommonsImages => "wikimedia commons",
            Engine::Openverse => "openverse",
            Engine::GitHub => "github",
            Engine::StackOverflow => "stack overflow",
            Engine::Crates => "crates.io",
            Engine::Npm => "npm",
            Engine::Mdn => "mdn",
            Engine::AskUbuntu => "askubuntu",
            Engine::SuperUser => "superuser",
            Engine::EuropePmc => "europe pmc",
        }
    }

    /// How much a hit from this engine counts in the merged ranking. Engines
    /// that only know one corner of the web (encyclopedia articles, Hacker News
    /// discussions, package names, MDN's web platform docs, biomedical papers,
    /// Linux Q&A) fill their list for any query, relevant or not, so they
    /// supplement the main engines rather than lead. Where several agree on a
    /// page, the product of weights in the ranking still lifts it.
    pub(in crate::web) fn weight(self) -> f32 {
        match self {
            Engine::Wikipedia
            | Engine::HackerNews
            | Engine::Crates
            | Engine::Npm
            | Engine::Mdn
            | Engine::EuropePmc
            | Engine::AskUbuntu
            | Engine::SuperUser => WEIGHT_SUPPLEMENTARY,
            _ => 1.0,
        }
    }

    /// Whether searches to this engine are spaced out (Google throttles bursts).
    pub(in crate::web) fn paced(self) -> bool {
        matches!(self, Engine::GoogleCse | Engine::GoogleCseImages)
    }

    /// Highest results page the engine serves; Google's element endpoint stops at 5.
    pub(in crate::web) fn max_page(self) -> u32 {
        match self {
            Engine::GoogleCse | Engine::GoogleCseImages => google::MAX_PAGE,
            _ => u32::MAX,
        }
    }

    /// DuckDuckGo's HTML endpoint pages only with a session token, and Bing's
    /// news feed has no documented offset, so both serve page 1 only.
    pub(in crate::web) fn supports_paging(self) -> bool {
        !matches!(self, Engine::DuckDuckGo | Engine::BingNews)
    }

    pub(in crate::web) fn default_base(self, opts: &SearchOptions) -> String {
        match self {
            Engine::GoogleCse | Engine::GoogleCseImages => "https://cse.google.com".into(),
            Engine::DuckDuckGo => "https://html.duckduckgo.com".into(),
            Engine::Brave => "https://search.brave.com".into(),
            Engine::BingNews => "https://www.bing.com".into(),
            Engine::Wikipedia => {
                let lang = opts.language.as_deref().unwrap_or("en");
                format!("https://{lang}.wikipedia.org")
            }
            Engine::HackerNews => "https://hn.algolia.com".into(),
            Engine::Arxiv => "https://export.arxiv.org".into(),
            Engine::OpenAlex => "https://api.openalex.org".into(),
            Engine::CommonsImages => "https://commons.wikimedia.org".into(),
            Engine::Openverse => "https://api.openverse.org".into(),
            Engine::GitHub => "https://api.github.com".into(),
            Engine::StackOverflow => "https://api.stackexchange.com".into(),
            Engine::Crates => "https://crates.io".into(),
            Engine::Npm => "https://registry.npmjs.org".into(),
            Engine::Mdn => "https://developer.mozilla.org".into(),
            Engine::AskUbuntu | Engine::SuperUser => "https://api.stackexchange.com".into(),
            Engine::EuropePmc => "https://www.ebi.ac.uk".into(),
        }
    }

    pub(in crate::web) async fn fetch(
        self,
        client: &Client,
        base: &str,
        query: &str,
        opts: &SearchOptions,
    ) -> Result<Value, ProviderError> {
        let base = base.trim_end_matches('/');
        let q = effective_query(query, opts);
        let page_index = opts.page.saturating_sub(1);
        let per_page = opts.max_results.clamp(1, MAX_RESULTS_CAP);
        let lang = accept_language(opts);
        let scraped = |req: RequestBuilder| {
            req.header("User-Agent", BROWSER_USER_AGENT)
                .header("Accept-Language", &lang)
        };
        match self {
            Engine::GoogleCse => google::fetch(client, base, &q, opts, false).await,
            Engine::GoogleCseImages => google::fetch(client, base, &q, opts, true).await,
            Engine::CommonsImages
            | Engine::Openverse
            | Engine::GitHub
            | Engine::StackOverflow
            | Engine::Crates
            | Engine::Npm
            | Engine::Mdn
            | Engine::AskUbuntu
            | Engine::SuperUser
            | Engine::EuropePmc => images_code::fetch(self, client, base, query, opts).await,
            Engine::DuckDuckGo => {
                let mut form = vec![("q", q), ("b", String::new())];
                if let (Some(l), Some(c)) = (&opts.language, &opts.country) {
                    form.push(("kl", format!("{}-{l}", c.to_lowercase())));
                }
                if let Some(f) = opts.freshness {
                    form.push(("df", freshness_letter(f).into()));
                }
                // The headers a browser sends for a form submission from the page itself.
                let req = client
                    .post(format!("{base}/html/"))
                    .header("Referer", DDG_REFERER)
                    .header("Sec-Fetch-Dest", "document")
                    .header("Sec-Fetch-Mode", "navigate")
                    .header("Sec-Fetch-Site", "same-origin")
                    .header("Sec-Fetch-User", "?1")
                    .form(&form);
                get_text(scraped(req)).await
            }
            Engine::Brave => {
                let mut params = vec![
                    ("q", q),
                    ("source", "web".into()),
                    ("offset", page_index.to_string()),
                ];
                if let Some(f) = opts.freshness {
                    params.push(("tf", format!("p{}", freshness_letter(f))));
                }
                let request = client
                    .get(format!("{base}/search"))
                    .header("Cookie", brave_cookies(opts))
                    .query(&params);
                get_text(scraped(request)).await
            }
            Engine::BingNews => {
                let params = [("q", q), ("format", "rss".to_string())];
                get_text(scraped(
                    client.get(format!("{base}/news/search")).query(&params),
                ))
                .await
            }
            Engine::Wikipedia => {
                // A title lookup, as SearxNG does: a full-text search answers every
                // query with loosely related articles, which only adds noise.
                let mut url = reqwest::Url::parse(&format!("{base}/api/rest_v1/page/summary/"))
                    .map_err(|e| format!("bad Wikipedia base URL: {e}"))?;
                url.path_segments_mut()
                    .map_err(|_| "Wikipedia base URL cannot take a path")?
                    .pop_if_empty()
                    .push(&wikipedia_title(query));
                let resp = client
                    .get(url)
                    .header("User-Agent", WIKIMEDIA_USER_AGENT)
                    .header("Accept-Language", &lang)
                    .send()
                    .await
                    .map_err(|e| {
                        ProviderError::new(format!("request failed: {}", e.without_url()))
                    })?;
                let status = resp.status().as_u16();
                if status == 404 {
                    return Ok(Value::Null);
                }
                // A title with characters Wikipedia cannot hold (`#`, `[`, `|`) is not an outage.
                if status == 400 {
                    let body = resp.text().await.unwrap_or_default();
                    if body.contains("title-invalid-characters") {
                        return Ok(Value::Null);
                    }
                    return Err(ProviderError::new("HTTP 400 Bad Request"));
                }
                decode_json(resp).await
            }
            Engine::HackerNews => {
                let mut params = vec![
                    ("query", query.trim().to_string()),
                    ("tags", "story".into()),
                    ("hitsPerPage", per_page.to_string()),
                    ("page", page_index.to_string()),
                ];
                if let Some(f) = opts.freshness {
                    let since = chrono::Utc::now().timestamp() - freshness_secs(f);
                    params.push(("numericFilters", format!("created_at_i>{since}")));
                }
                get_json(client.get(format!("{base}/api/v1/search")).query(&params)).await
            }
            Engine::Arxiv => {
                let terms: Vec<String> = arxiv_terms(query)
                    .into_iter()
                    .map(|t| format!("all:{t}"))
                    .collect();
                let params = [
                    ("search_query", terms.join(" AND ")),
                    ("start", (page_index as usize * per_page).to_string()),
                    ("max_results", per_page.to_string()),
                ];
                get_text(client.get(format!("{base}/api/query")).query(&params)).await
            }
            Engine::OpenAlex => {
                let mut params = vec![
                    ("search", query.trim().to_string()),
                    ("per-page", per_page.to_string()),
                    ("page", opts.page.to_string()),
                ];
                if let Some(f) = opts.freshness {
                    let since = chrono::Utc::now() - chrono::Duration::seconds(freshness_secs(f));
                    let filter = format!("from_publication_date:{}", since.format("%Y-%m-%d"));
                    params.push(("filter", filter));
                }
                get_json(client.get(format!("{base}/works")).query(&params)).await
            }
        }
    }

    /// Parse a body returned by [`Engine::fetch`]. An empty list is a valid
    /// answer unless the page looks like a bot challenge.
    pub(in crate::web) fn parse(self, body: &Value) -> Result<Vec<SearchResult>, ProviderError> {
        let results = match self {
            Engine::GoogleCse | Engine::GoogleCseImages => google::parse(self, body)?,
            Engine::DuckDuckGo => parse_duckduckgo(text_of(body)?),
            Engine::Brave => parse_brave_html(text_of(body)?),
            Engine::BingNews => parse_bing_news(text_of(body)?),
            Engine::Arxiv => parse_arxiv(text_of(body)?),
            Engine::Wikipedia => parse_wikipedia(body)?,
            Engine::HackerNews => parse_hacker_news(body)?,
            Engine::OpenAlex => parse_openalex(body)?,
            Engine::CommonsImages
            | Engine::Openverse
            | Engine::GitHub
            | Engine::StackOverflow
            | Engine::Crates
            | Engine::Npm
            | Engine::Mdn
            | Engine::AskUbuntu
            | Engine::SuperUser
            | Engine::EuropePmc => images_code::parse(self, body)?,
        };
        if results.is_empty() && is_html_engine(self) && looks_like_challenge(text_of(body)?) {
            return Err(ProviderError::of(
                FailureClass::Captcha,
                "bot challenge page instead of results",
            ));
        }
        Ok(results
            .into_iter()
            .map(|mut r| {
                r.provider = "native".into();
                r.engines = vec![self.name().into()];
                r
            })
            .collect())
    }
}

/// arXiv's index drops common words, so one in an AND query matches nothing.
const ARXIV_STOP_WORDS: &[&str] = &[
    "a", "an", "the", "is", "are", "of", "in", "on", "to", "for", "and", "or", "all", "with", "by",
    "at", "as", "it", "be",
];

/// Words only: arXiv's query syntax gives `:`, `(` and quotes a meaning of their own.
pub(super) fn arxiv_terms(query: &str) -> Vec<String> {
    let all: Vec<String> = query
        .split_whitespace()
        .map(|t| {
            t.chars()
                .filter(|c| c.is_alphanumeric() || *c == '-')
                .collect::<String>()
        })
        .filter(|t| !t.is_empty())
        .collect();
    let kept: Vec<String> = all
        .iter()
        .filter(|t| !ARXIV_STOP_WORDS.contains(&t.to_lowercase().as_str()))
        .cloned()
        .collect();
    if kept.is_empty() { all } else { kept }
}

/// `Accept-Language` the way SearxNG builds it: the language with its region,
/// then English as a fallback, and a plain US English default.
pub(super) fn accept_language(opts: &SearchOptions) -> String {
    match (&opts.language, &opts.country) {
        (Some(l), Some(c)) => format!("{l},{l}-{c};q=0.7,en;q=0.3"),
        (Some(l), None) => format!("{l},{l}-{l};q=0.7,en;q=0.3"),
        _ => DEFAULT_ACCEPT_LANGUAGE.to_string(),
    }
}

/// The preference cookies Brave's own search page carries, as SearxNG sends them.
fn brave_cookies(opts: &SearchOptions) -> String {
    let country = opts
        .country
        .as_deref()
        .map_or("all".to_string(), str::to_lowercase);
    let ui_lang = match (&opts.language, &opts.country) {
        (Some(l), Some(c)) => format!("{l}-{}", c.to_lowercase()),
        (Some(l), None) => format!("{l}-{l}"),
        _ => "en-us".to_string(),
    };
    format!("safesearch=off; useLocation=0; summarizer=0; country={country}; ui_lang={ui_lang}")
}

fn is_html_engine(engine: Engine) -> bool {
    matches!(engine, Engine::DuckDuckGo | Engine::Brave)
}

fn text_of(body: &Value) -> Result<&str, String> {
    body.as_str()
        .ok_or_else(|| "expected a text body".to_string())
}

fn looks_like_challenge(body: &str) -> bool {
    let lower = body.to_lowercase();
    CHALLENGE_MARKERS.iter().any(|m| lower.contains(m))
}

fn freshness_letter(f: Freshness) -> &'static str {
    match f {
        Freshness::Day => "d",
        Freshness::Week => "w",
        Freshness::Month => "m",
        Freshness::Year => "y",
    }
}

pub(super) fn freshness_secs(f: Freshness) -> i64 {
    const DAY: i64 = 86_400;
    match f {
        Freshness::Day => DAY,
        Freshness::Week => 7 * DAY,
        Freshness::Month => 30 * DAY,
        Freshness::Year => 365 * DAY,
    }
}

/// A result as SearxNG stores it before merging: whitespace collapsed, the
/// title cut at 200 characters and the content at 1200, each at a word
/// boundary, and a content that only repeats the title dropped.
pub(super) fn result(title: String, url: String, snippet: String) -> SearchResult {
    let title = truncate_words(&collapse_ws(&title), TITLE_MAX_CHARS);
    let snippet = truncate_words(&collapse_ws(&snippet), SNIPPET_MAX_CHARS);
    let snippet = if snippet == title {
        String::new()
    } else {
        snippet
    };
    SearchResult {
        domain: domain_of(&url),
        title,
        url,
        snippet,
        position: 0,
        provider: String::new(),
        published: None,
        engines: Vec::new(),
    }
}

/// Cut to at most `max` characters at the last word boundary, marking the cut.
fn truncate_words(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max).collect();
    let at_word = cut
        .rfind(char::is_whitespace)
        .map_or(cut.as_str(), |i| &cut[..i]);
    format!("{} …", at_word.trim_end())
}

fn element_text(el: scraper::ElementRef<'_>) -> String {
    collapse_ws(&el.text().collect::<String>())
}

fn parse_duckduckgo(html: &str) -> Vec<SearchResult> {
    let doc = scraper::Html::parse_document(html);
    let (block, link, snippet) = (
        sel("div.result"),
        sel("a.result__a"),
        sel(".result__snippet"),
    );
    doc.select(&block)
        .filter(|b| !b.value().classes().any(|c| c == "result--ad"))
        .filter_map(|b| {
            let a = b.select(&link).next()?;
            let url = resolve_duckduckgo_href(a.value().attr("href")?)?;
            let snip = b
                .select(&snippet)
                .next()
                .map(element_text)
                .unwrap_or_default();
            Some(result(element_text(a), url, snip))
        })
        .collect()
}

/// Result links are sometimes wrapped as `//duckduckgo.com/l/?uddg=<encoded>`.
fn resolve_duckduckgo_href(href: &str) -> Option<String> {
    let absolute = if href.starts_with("//") {
        format!("https:{href}")
    } else {
        href.to_string()
    };
    let parsed = reqwest::Url::parse(&absolute).ok()?;
    if parsed.host_str() == Some("duckduckgo.com") && parsed.path() == "/l/" {
        return parsed
            .query_pairs()
            .find(|(k, _)| k == "uddg")
            .map(|(_, v)| v.into_owned());
    }
    matches!(parsed.scheme(), "http" | "https").then_some(absolute)
}

fn parse_brave_html(html: &str) -> Vec<SearchResult> {
    let doc = scraper::Html::parse_document(html);
    let (block, anchor, title, content) = (
        sel("div.snippet[data-type=\"web\"]"),
        sel("a[href]"),
        sel(".title"),
        sel(".generic-snippet .content, .snippet-description"),
    );
    doc.select(&block)
        .filter_map(|b| {
            let url = b
                .select(&anchor)
                .filter_map(|a| a.value().attr("href"))
                .find(|h| h.starts_with("http"))?
                .to_string();
            let name = b.select(&title).next().map(element_text)?;
            let snip = b
                .select(&content)
                .next()
                .map(element_text)
                .unwrap_or_default();
            Some(result(name, url, snip))
        })
        .collect()
}

fn parse_bing_news(xml: &str) -> Vec<SearchResult> {
    blocks(xml, "item")
        .into_iter()
        .filter_map(|item| {
            let title = html_text(&tag_text(item, "title")?);
            let link = html_text(&tag_text(item, "link")?);
            let url = decode_bing_news_href(&link);
            let snip = tag_text(item, "description")
                .map(|d| html_text(&d))
                .unwrap_or_default();
            let mut r = result(title, url, snip);
            r.published = tag_text(item, "pubDate");
            Some(r)
        })
        .collect()
}

/// News links are `bing.com/news/apiclick.aspx?...&url=<percent-encoded target>`.
fn decode_bing_news_href(link: &str) -> String {
    reqwest::Url::parse(link)
        .ok()
        .and_then(|u| {
            u.query_pairs()
                .find(|(k, _)| k == "url")
                .map(|(_, v)| v.into_owned())
        })
        .filter(|t| t.starts_with("http"))
        .unwrap_or_else(|| link.to_string())
}

fn parse_arxiv(xml: &str) -> Vec<SearchResult> {
    blocks(xml, "entry")
        .into_iter()
        .filter_map(|entry| {
            let title = collapse_ws(&html_text(&tag_text(entry, "title")?));
            let id = tag_text(entry, "id")?;
            // Errors come back as a one-entry feed that must not read as a hit.
            if id.contains("/api/errors") {
                return None;
            }
            let url = attr_value(entry, "link", "rel=\"alternate\"", "href")
                .unwrap_or(id)
                .replacen("http://", "https://", 1);
            let summary = collapse_ws(&html_text(&tag_text(entry, "summary").unwrap_or_default()));
            let mut r = result(title, url, summary);
            r.published = tag_text(entry, "published");
            Some(r)
        })
        .collect()
}

/// A lowercase query is title-cased the way SearxNG does before a title lookup.
pub(super) fn wikipedia_title(query: &str) -> String {
    let query = query.trim();
    if query != query.to_lowercase() {
        return query.to_string();
    }
    let mut out = String::with_capacity(query.len());
    let mut at_word_start = true;
    for c in query.chars() {
        if c.is_alphabetic() {
            out.extend(if at_word_start {
                c.to_uppercase().collect::<Vec<_>>()
            } else {
                vec![c]
            });
            at_word_start = false;
        } else {
            out.push(c);
            at_word_start = true;
        }
    }
    out
}

/// Only an ordinary article answers. SearxNG shows it as an infobox beside the
/// results; here it is a ranked hit, and disambiguation pages are left out.
fn parse_wikipedia(body: &Value) -> Result<Vec<SearchResult>, String> {
    if body["type"].as_str() != Some("standard") {
        return Ok(Vec::new());
    }
    let url = non_empty_str(&body["content_urls"]["desktop"]["page"])
        .filter(|u| u.starts_with("https://") || u.starts_with("http://"));
    let title = non_empty_str(&body["titles"]["display"])
        .map(|t| html_text(&t))
        .or_else(|| non_empty_str(&body["title"]));
    let (Some(url), Some(title)) = (url, title) else {
        return Ok(Vec::new());
    };
    let mut r = result(
        title,
        url,
        body["extract"].as_str().unwrap_or_default().to_string(),
    );
    r.published = non_empty_str(&body["timestamp"]);
    Ok(vec![r])
}

fn parse_hacker_news(body: &Value) -> Result<Vec<SearchResult>, String> {
    let hits = body["hits"]
        .as_array()
        .ok_or("response has no `hits` array")?;
    Ok(hits
        .iter()
        .filter_map(|h| {
            let title = h["title"].as_str().filter(|t| !t.is_empty())?;
            let id = h["objectID"].as_str()?;
            let discussion = format!("https://news.ycombinator.com/item?id={id}");
            let url = non_empty_str(&h["url"])
                .filter(|u| u.starts_with("https://") || u.starts_with("http://"))
                .unwrap_or_else(|| discussion.clone());
            let snippet = format!(
                "{} points, {} comments on Hacker News ({discussion})",
                h["points"].as_u64().unwrap_or(0),
                h["num_comments"].as_u64().unwrap_or(0)
            );
            let mut r = result(title.to_string(), url, snippet);
            r.published = non_empty_str(&h["created_at"]);
            Some(r)
        })
        .collect())
}

fn parse_openalex(body: &Value) -> Result<Vec<SearchResult>, String> {
    let works = body["results"]
        .as_array()
        .ok_or("response has no `results` array")?;
    Ok(works
        .iter()
        .filter_map(|w| {
            let title = w["display_name"].as_str().filter(|t| !t.is_empty())?;
            let url = non_empty_str(&w["primary_location"]["landing_page_url"])
                .or_else(|| non_empty_str(&w["doi"]))
                .or_else(|| non_empty_str(&w["id"]))
                .filter(|u| u.starts_with("https://") || u.starts_with("http://"))?;
            let mut r = result(
                title.to_string(),
                url,
                openalex_abstract(&w["abstract_inverted_index"]),
            );
            r.published = non_empty_str(&w["publication_date"]);
            Some(r)
        })
        .collect())
}

/// OpenAlex ships abstracts as `{word: [positions]}`; put the words back in order.
fn openalex_abstract(index: &Value) -> String {
    let Some(map) = index.as_object() else {
        return String::new();
    };
    let mut words: Vec<(u64, &str)> = map
        .iter()
        .flat_map(|(word, positions)| {
            positions
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_u64)
                .map(move |p| (p, word.as_str()))
        })
        .collect();
    words.sort_unstable_by_key(|(p, _)| *p);
    words
        .into_iter()
        .map(|(_, w)| w)
        .collect::<Vec<_>>()
        .join(" ")
}
