use super::super::*;
use super::markup::{attr_value, blocks, collapse_ws, html_text, sel, tag_text};

/// Browsers get scraped HTML; API engines get the descriptive agent string.
const BROWSER_USER_AGENT: &str =
    "Mozilla/5.0 (X11; Linux x86_64; rv:128.0) Gecko/20100101 Firefox/128.0";
const ACCEPT_LANGUAGE: &str = "en-US,en;q=0.9";
const SNIPPET_MAX_CHARS: usize = 400;
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
    DuckDuckGo,
    Brave,
    Wikipedia,
    BingNews,
    HackerNews,
    Arxiv,
    OpenAlex,
}

const GENERAL: &[Engine] = &[Engine::DuckDuckGo, Engine::Brave, Engine::Wikipedia];
const NEWS: &[Engine] = &[Engine::BingNews, Engine::HackerNews];
const SCIENCE: &[Engine] = &[Engine::Arxiv, Engine::OpenAlex];

impl Engine {
    pub(in crate::web) fn for_category(category: Option<Category>) -> &'static [Engine] {
        match category {
            None | Some(Category::General) => GENERAL,
            Some(Category::News) => NEWS,
            Some(Category::Science) => SCIENCE,
        }
    }

    pub(in crate::web) fn name(self) -> &'static str {
        match self {
            Engine::DuckDuckGo => "duckduckgo",
            Engine::Brave => "brave",
            Engine::Wikipedia => "wikipedia",
            Engine::BingNews => "bing news",
            Engine::HackerNews => "hacker news",
            Engine::Arxiv => "arxiv",
            Engine::OpenAlex => "openalex",
        }
    }

    /// How much a hit from this engine counts in the merged ranking. Wikipedia
    /// answers every query with encyclopedia articles and Hacker News with
    /// discussions, so they supplement the web engines rather than lead.
    pub(in crate::web) fn weight(self) -> f32 {
        match self {
            Engine::Wikipedia => WEIGHT_SUPPLEMENTARY,
            Engine::HackerNews => WEIGHT_SUPPLEMENTARY,
            _ => 1.0,
        }
    }

    /// DuckDuckGo's HTML endpoint pages only with a session token, and Bing's
    /// news feed has no documented offset, so both serve page 1 only.
    pub(in crate::web) fn supports_paging(self) -> bool {
        !matches!(self, Engine::DuckDuckGo | Engine::BingNews)
    }

    pub(in crate::web) fn default_base(self, opts: &SearchOptions) -> String {
        match self {
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
        let scraped = |req: RequestBuilder| {
            req.header("User-Agent", BROWSER_USER_AGENT)
                .header("Accept-Language", ACCEPT_LANGUAGE)
        };
        match self {
            Engine::DuckDuckGo => {
                let mut form = vec![("q", q), ("b", String::new())];
                if let (Some(l), Some(c)) = (&opts.language, &opts.country) {
                    form.push(("kl", format!("{}-{l}", c.to_lowercase())));
                }
                if let Some(f) = opts.freshness {
                    form.push(("df", freshness_letter(f).into()));
                }
                let req = client.post(format!("{base}/html/")).form(&form);
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
                get_text(scraped(client.get(format!("{base}/search")).query(&params))).await
            }
            Engine::BingNews => {
                let params = [("q", q), ("format", "rss".to_string())];
                get_text(scraped(
                    client.get(format!("{base}/news/search")).query(&params),
                ))
                .await
            }
            Engine::Wikipedia => {
                let params = [
                    ("action", "query".to_string()),
                    ("list", "search".into()),
                    ("srsearch", query.trim().to_string()),
                    ("srlimit", per_page.to_string()),
                    ("sroffset", (page_index as usize * per_page).to_string()),
                    ("format", "json".into()),
                    ("utf8", "1".into()),
                ];
                get_json(client.get(format!("{base}/w/api.php")).query(&params)).await
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
    pub(in crate::web) fn parse(
        self,
        body: &Value,
        base: &str,
    ) -> Result<Vec<SearchResult>, String> {
        let results = match self {
            Engine::DuckDuckGo => parse_duckduckgo(text_of(body)?),
            Engine::Brave => parse_brave_html(text_of(body)?),
            Engine::BingNews => parse_bing_news(text_of(body)?),
            Engine::Arxiv => parse_arxiv(text_of(body)?),
            Engine::Wikipedia => parse_wikipedia(body, base)?,
            Engine::HackerNews => parse_hacker_news(body)?,
            Engine::OpenAlex => parse_openalex(body)?,
        };
        if results.is_empty() && is_html_engine(self) && looks_like_challenge(text_of(body)?) {
            return Err("bot challenge page instead of results".into());
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

fn freshness_secs(f: Freshness) -> i64 {
    const DAY: i64 = 86_400;
    match f {
        Freshness::Day => DAY,
        Freshness::Week => 7 * DAY,
        Freshness::Month => 30 * DAY,
        Freshness::Year => 365 * DAY,
    }
}

fn result(title: String, url: String, snippet: String) -> SearchResult {
    SearchResult {
        domain: domain_of(&url),
        title,
        url,
        snippet: truncate_chars(&snippet, SNIPPET_MAX_CHARS),
        position: 0,
        provider: String::new(),
        published: None,
        engines: Vec::new(),
    }
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max).collect();
    format!("{}…", cut.trim_end())
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

fn parse_wikipedia(body: &Value, base: &str) -> Result<Vec<SearchResult>, String> {
    let hits = body["query"]["search"]
        .as_array()
        .ok_or("response has no `query.search` array")?;
    let root = reqwest::Url::parse(base).map_err(|e| format!("bad Wikipedia base URL: {e}"))?;
    Ok(hits
        .iter()
        .filter_map(|h| {
            let title = h["title"].as_str()?;
            let mut url = root.clone();
            url.path_segments_mut()
                .ok()?
                .pop_if_empty()
                .extend(["wiki", &title.replace(' ', "_")]);
            let snippet = collapse_ws(&html_text(h["snippet"].as_str().unwrap_or_default()));
            let mut r = result(title.to_string(), url.to_string(), snippet);
            r.published = non_empty_str(&h["timestamp"]);
            Some(r)
        })
        .collect())
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
            let url = non_empty_str(&h["url"]).unwrap_or_else(|| discussion.clone());
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
                .or_else(|| non_empty_str(&w["id"]))?;
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
