use super::*;

/// Format search results as a readable text block for agent consumption.
pub fn format_search_results(results: &WebSearchResults) -> String {
    let mut out = format!("## Search Results for: \"{}\"", results.query);
    if results.page > 1 {
        out.push_str(&format!(" (page {})", results.page));
    }
    if let Some(backend) = &results.backend {
        out.push_str(&format!(" via {backend}"));
    }
    out.push_str("\n\n");

    if !results.unsupported_filters.is_empty() {
        out.push_str(&format!(
            "Not applied by this backend: {}\n\n",
            results.unsupported_filters.join("; ")
        ));
    }

    if results.results.is_empty() {
        out.push_str("No results found.\n");
    }

    for (i, result) in results.results.iter().enumerate() {
        out.push_str(&format!("### {}. {}\n", i + 1, result.title));
        out.push_str(&format!("URL: {}\n", result.url));
        let mut meta: Vec<String> = result.domain.iter().cloned().collect();
        meta.extend(result.published.iter().cloned());
        if !result.engines.is_empty() {
            meta.push(format!("engines: {}", result.engines.join(", ")));
        }
        if !meta.is_empty() {
            out.push_str(&format!("Source: {}\n", meta.join(" | ")));
        }
        if !result.snippet.is_empty() {
            out.push_str(&result.snippet);
            out.push('\n');
        }
        out.push('\n');
    }

    if let Some(next) = results.next_page {
        out.push_str(&format!("More results: request page {next}.\n"));
    }
    let degraded = results
        .attempts
        .iter()
        .any(|a| !a.outcome.starts_with("ok"));
    if degraded {
        let lines: Vec<String> = results
            .attempts
            .iter()
            .map(|a| format!("{}: {}", a.provider, a.outcome))
            .collect();
        out.push_str(&format!("Backends tried: {}\n", lines.join("; ")));
    }

    out
}


/// Tool-level deadlines, shared with hq-agent's own web tool wrappers.
pub const SEARCH_TOOL_TIMEOUT_MS: u64 = 30_000;
pub const FETCH_TOOL_TIMEOUT_MS: u64 = 95_000;

/// Web search tool exposed via MCP gateway. SearxNG when configured, then the built-in engines, then Brave.
pub struct WebSearchHqTool {
    searxng_url: Option<String>,
    brave_api_key: Option<String>,
    native: bool,
}

impl WebSearchHqTool {
    pub fn new(searxng_url: Option<String>, brave_api_key: Option<String>, native: bool) -> Self {
        Self {
            searxng_url,
            brave_api_key,
            native,
        }
    }
}

#[async_trait]
impl HqTool for WebSearchHqTool {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        "Search the web for current information. Works out of the box through a built-in keyless engine pool (Google, DuckDuckGo, Brave, Wikipedia; Bing News and Hacker News for news; arXiv, OpenAlex and Europe PMC for science; Google Images, Wikimedia Commons and Openverse for images; GitHub, Stack Overflow, Ask Ubuntu, Super User, MDN, crates.io and npm for code), merged and de-duplicated. A configured SearxNG instance is tried first and the paid Brave Search API last. Optional filters: freshness (day/week/month/year), language, country, category (general/news/science/images/code), include_domains/exclude_domains, and page for later results. Returns titles, URLs, snippets, source metadata, the answering backend, next_page, and any filter that backend could not apply. Example: {\"query\": \"tokio release notes\", \"freshness\": \"month\", \"include_domains\": [\"github.com\"]}."
    }

    fn parameters(&self) -> Value {
        web_search_parameters()
    }

    fn category(&self) -> &str {
        "web"
    }

    fn is_read_only(&self) -> bool {
        true
    }

    /// Outer safety net above `web_search`'s own 20s deadline for the whole
    /// SearxNG, built-in engine and Brave chain.
    fn timeout_ms(&self) -> Option<u64> {
        Some(SEARCH_TOOL_TIMEOUT_MS)
    }

    fn search_hint(&self) -> Option<&str> {
        Some("search the web for current information")
    }

    fn behavioral_prompt(&self) -> Option<&str> {
        Some(
            "After answering a question using web search results, include a 'Sources:' section \
             with markdown links to the pages you referenced. Use the current year in time-sensitive queries.",
        )
    }

    async fn validate(&self, args: &Value) -> ValidationResult {
        let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
        if query.len() < 2 {
            return ValidationResult::block("query must be at least 2 characters", 400);
        }
        if let Err(e) = SearchOptions::from_args(args) {
            return ValidationResult::block(e, 400);
        }
        ValidationResult::ok()
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: query"))?;
        let opts = SearchOptions::from_args(&args).map_err(anyhow::Error::msg)?;

        let results = web_search(
            query,
            &opts,
            self.searxng_url.as_deref(),
            self.brave_api_key.as_deref(),
            self.native,
        )
        .await?;

        Ok(json!({
            "text": format_search_results(&results),
            "result_count": results.results.len(),
            "results": results.results,
            "backend": results.backend,
            "page": results.page,
            "next_page": results.next_page,
            "attempts": results.attempts,
            "unsupported_filters": results.unsupported_filters,
        }))
    }
}


/// Fetch a web page and extract clean text content. No API key required.
pub struct WebFetchHqTool;

#[async_trait]
impl HqTool for WebFetchHqTool {
    fn name(&self) -> &str {
        "web_fetch"
    }

    fn description(&self) -> &str {
        "Fetch a URL and extract clean text content. HTML pages are reduced to their main content (title, byline, date, body, links) without navigation and footers, falling back to a plain text conversion. Pages that render client-side are recovered from their embedded JSON-LD or framework data, then from the third-party Jina Reader (receives only the URL; `HQ_WEB_FETCH_JINA=0` disables it). PDFs are read from their text layer, with OCR for scanned PDFs where the host has pdftoppm and tesseract. Returns the content with final_url, content_type, method, truncation and notes on partial extraction. Images, audio, video and archives are rejected. No API key required."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "required": ["url"],
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to fetch (http or https only)"
                },
                "max_chars": {
                    "type": "integer",
                    "description": "Maximum characters to return (default 100000)"
                }
            }
        })
    }

    fn category(&self) -> &str {
        "web"
    }

    fn search_hint(&self) -> Option<&str> {
        Some("fetch web page or PDF and extract text content")
    }

    fn is_read_only(&self) -> bool {
        true
    }

    /// Outer safety net above `web_fetch`'s own limits: a 30s fetch plus
    /// either a 25s Jina render or a PDF's 15s text layer plus 45s of OCR.
    fn timeout_ms(&self) -> Option<u64> {
        Some(FETCH_TOOL_TIMEOUT_MS)
    }

    fn behavioral_prompt(&self) -> Option<&str> {
        Some(
            "Use 125-character max for direct quotes from fetched pages. \
             Use quotation marks for exact language. Never reproduce song lyrics or copyrighted content verbatim.",
        )
    }

    async fn validate(&self, args: &Value) -> ValidationResult {
        let url = args.get("url").and_then(|v| v.as_str()).unwrap_or("");
        if url.is_empty() {
            return ValidationResult::block("url is required", 400);
        }
        if let Err(e) = validate_url(url) {
            return ValidationResult::block(format!("{}", e), 400);
        }
        ValidationResult::ok()
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let url = args
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: url"))?;

        let max_chars = args
            .get("max_chars")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(DEFAULT_MAX_OUTPUT_CHARS);

        let page = web_fetch(url, max_chars).await?;

        Ok(json!({
            "url": page.url,
            "final_url": page.final_url,
            "content_type": page.content_type,
            "method": page.method,
            "chars": page.content.len(),
            "total_chars": page.total_chars,
            "truncated": page.truncated,
            "notes": page.notes,
            "content": page.content,
        }))
    }
}
