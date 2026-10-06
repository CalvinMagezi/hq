//! Web tools — search and fetch for agent sessions.
//!
//! Search works with no setup through a built-in engine pool; a configured
//! SearxNG instance is tried first and the paid Brave Search API last (see
//! `hq_tools::web`). Fetching uses reqwest +
//! html2text for page content, with an automatic Jina Reader fallback for
//! JS-rendered pages; no API key required for either.

use anyhow::Result;
use async_trait::async_trait;
use hq_core::types::ToolResult;
use hq_tools::web;
use serde_json::{Value, json};
use tracing::debug;

use crate::coding::text_result;
use crate::tools::AgentTool;

// ─── WebSearchTool ──────────────────────────────────────────────

/// Search the web. SearxNG when configured, then the built-in engines, then Brave Search.
pub struct WebSearchTool {
    searxng_url: Option<String>,
    brave_api_key: Option<String>,
    native: bool,
}

impl WebSearchTool {
    pub fn new(searxng_url: Option<String>, brave_api_key: Option<String>, native: bool) -> Self {
        Self {
            searxng_url,
            brave_api_key,
            native,
        }
    }
}

#[async_trait]
impl AgentTool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }

    fn behavioral_prompt(&self) -> Option<&str> {
        Some(
            "Cite the sources you used. Include the current year when the answer is time-sensitive, since the model's training data lags. Search before answering anything about current events, prices, or releases.",
        )
    }

    fn description(&self) -> &str {
        concat!(
            "Search the web for current information. Works out of the box through a ",
            "built-in keyless engine pool (DuckDuckGo, Brave, Wikipedia; Bing News ",
            "and Hacker News for news; arXiv and OpenAlex for science), merged and ",
            "de-duplicated. A configured SearxNG instance is tried first and the paid ",
            "Brave Search API last.\n\n",
            "Use this tool when you need:\n",
            "- Current documentation or API references\n",
            "- Error message lookups and troubleshooting\n",
            "- Package/library information and versions\n",
            "- General knowledge that may have changed since training\n\n",
            "Parameters:\n",
            "- query (required): The search query, at least 2 characters\n",
            "- max_results (optional): Number of results, default 5, max 20\n",
            "- page (optional): 1-based page, use next_page from a previous search\n",
            "- freshness (optional): day, week, month or year\n",
            "- language / country (optional): 2-letter codes, e.g. en / US\n",
            "- category (optional): general, news or science\n",
            "- include_domains / exclude_domains (optional): up to 10 domains each\n\n",
            "Filters and paging depend on the backend; any filter it could not apply is ",
            "listed in the output, as is the backend that answered.\n\n",
            "Returns formatted search results with titles, URLs, snippets and source metadata.",
        )
    }

    fn parameters(&self) -> Value {
        web::web_search_parameters()
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn timeout_ms(&self) -> Option<u64> {
        Some(web::SEARCH_TOOL_TIMEOUT_MS)
    }

    async fn execute(&self, _id: &str, args: Value) -> Result<ToolResult> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: query"))?;

        if query.len() < 2 {
            return Ok(text_result("Error: query must be at least 2 characters"));
        }

        let opts = match web::SearchOptions::from_args(&args) {
            Ok(opts) => opts,
            Err(e) => return Ok(text_result(format!("Error: {e}"))),
        };

        debug!(query = %query, ?opts, "agent web_search");

        match web::web_search(
            query,
            &opts,
            self.searxng_url.as_deref(),
            self.brave_api_key.as_deref(),
            self.native,
        )
        .await
        {
            Ok(results) => Ok(text_result(web::format_search_results(&results))),
            Err(e) => Ok(text_result(format!("Search failed: {}", e))),
        }
    }
}

// ─── WebFetchTool ───────────────────────────────────────────────

/// Fetch a URL and extract clean text content. No API key required.
pub struct WebFetchTool;

#[async_trait]
impl AgentTool for WebFetchTool {
    fn name(&self) -> &str {
        "web_fetch"
    }

    fn behavioral_prompt(&self) -> Option<&str> {
        Some(
            "Keep max_chars at or below 8000 — larger fetches crowd out the rest of the context. Quote at most 125 characters from any single source in your answer.",
        )
    }

    fn description(&self) -> &str {
        concat!(
            "Fetch a URL and extract clean text content. ",
            "Automatically converts HTML to readable plain text and reads PDFs ",
            "(text layer, or OCR for scanned PDFs where the host supports it).\n\n",
            "Use this tool when you need to:\n",
            "- Read documentation pages or articles\n",
            "- Download API responses (JSON, text)\n",
            "- Extract content from web pages and PDFs found via web_search\n\n",
            "Parameters:\n",
            "- url (required): The URL to fetch (http/https only)\n",
            "- max_chars (optional): Max characters to return (default 50000)\n\n",
            "Output starts with a provenance line: final URL, content type and extraction ",
            "method. Client-rendered pages fall back to the third-party Jina Reader, which ",
            "receives only the URL; that is noted in the output.\n\n",
            "Security: Blocks file://, localhost, and private IP addresses, including via redirects.\n",
            "Images, audio, video and archives are rejected. Use bash + curl for those.",
        )
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
                    "description": "Maximum characters to return (default 50000)"
                }
            }
        })
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn timeout_ms(&self) -> Option<u64> {
        Some(web::FETCH_TOOL_TIMEOUT_MS)
    }

    async fn execute(&self, _id: &str, args: Value) -> Result<ToolResult> {
        let url = args
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: url"))?;

        let max_chars = args
            .get("max_chars")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(50_000);

        debug!(url = %url, max_chars, "agent web_fetch");

        match web::web_fetch(url, max_chars).await {
            Ok(page) => Ok(text_result(page.to_text())),
            Err(e) => Ok(text_result(format!("Fetch failed: {}", e))),
        }
    }
}
