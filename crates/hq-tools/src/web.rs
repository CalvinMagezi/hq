//! Web tools — search the web and fetch page content.
//!
//! Search works with no setup. The chain is a self-hosted SearxNG instance when
//! `searxng_url` is set, then the built-in engine pool (`native`: Google (the
//! keyless Programmable Search element endpoint SearxNG uses), DuckDuckGo,
//! Brave and a Wikipedia summary for general queries, Bing News and Hacker News for
//! news, arXiv, OpenAlex and Europe PMC for science, Google Images, Wikimedia Commons and Openverse for
//! images, GitHub, Stack Overflow, Ask Ubuntu, Super User, MDN, crates.io and npm for code, queried in
//! parallel, merged with SearxNG's ranking (engine weights times positions times
//! the sum of 1 over position) and cached for 10 minutes), then the paid Brave Search
//! API when `brave_api_key` is set. A later backend runs when an earlier one
//! fails, cools down after recent failures, or returns nothing. The machine
//! profile reports each backend as configured or reachable without querying it;
//! [`probe_search_backends`] (used by `hq doctor`) sends one real query per
//! backend. Every response here names the backend that actually answered and
//! each fallback attempt, including every engine in the built-in pool.
//!
//! The whole chain runs under one deadline (`SEARCH_DEADLINE`, 20s): SearxNG
//! gets at most 5s, each built-in engine 8s (in parallel), Brave 12s, each
//! capped by whatever is left. Every transport, HTTP, parse and challenge-page
//! failure puts that backend, or that single engine, into exponential cooldown.
//!
//! Filters are optional and provider-dependent. Backend differences:
//!
//! | option     | SearxNG                           | Brave API                     | built-in pool                          |
//! |------------|-----------------------------------|-------------------------------|----------------------------------------|
//! | freshness  | `time_range` (engine-dependent)   | `freshness` pd/pw/pm/py       | per engine, see `unsupported_filters`  |
//! | language   | `language`                        | `search_lang`                 | Wikipedia host, DuckDuckGo with country |
//! | country    | only with language (`en-US`)      | `country`                     | DuckDuckGo with language               |
//! | category   | general, news, science, images, it | general, news                | general, news, science, images, code   |
//! | domains    | `site:` operators + post-filter   | `site:` operators + post-filter | `site:` operators + post-filter      |
//! | page       | `pageno`, unbounded               | `offset`, pages 1..=10        | page 1 only on DuckDuckGo and Bing News |
//!
//! An option the answering backend can't honor is listed in
//! `unsupported_filters` rather than silently dropped. Domain filters are
//! always enforced by a post-filter on the result host, so a page can come
//! back with fewer than `max_results` hits.
//!
//! Fetching uses reqwest. HTML goes through `readable` (main-content extraction
//! with title, byline, date and links), falling back to html2text when no
//! confident container is found. A page that comes back as an empty
//! client-rendered shell is recovered from its embedded JSON-LD or framework
//! data, then from Jina Reader (`r.jina.ai`, a third-party service that
//! receives only the URL; `HQ_WEB_FETCH_JINA=0` turns it off). PDFs use
//! `hq-convert` (text layer first, OCR for scanned files, which needs
//! `pdftoppm` and `tesseract` off macOS). Every redirect hop is re-checked
//! against the private-network rules. Pages that need a logged-in browser,
//! interaction, or anything none of those can recover stay unsupported.

use anyhow::{Result, bail};
use async_trait::async_trait;
use reqwest::{Client, RequestBuilder};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::future::Future;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tracing::debug;

use crate::registry::HqTool;
use hq_core::types::ValidationResult;

mod backends;
mod chain;
mod client;
mod fetch;
#[cfg(test)]
mod fetch_tests;
mod health;
mod native;
mod readable;
mod ssrf;
#[cfg(test)]
mod tests;
mod tools;
mod types;

pub use health::probe_search_backends;
#[cfg(test)]
use ssrf::is_non_public_ip;
use ssrf::{GuardedResolver, validate_url};

const FETCH_TIMEOUT: Duration = Duration::from_secs(30);
const JINA_TIMEOUT: Duration = Duration::from_secs(25);
// Fetch + text layer + OCR (30 + 15 + 45s) stays under hq-agent's 95s outer tool timeout.
const PDF_TEXT_TIMEOUT: Duration = Duration::from_secs(15);
const PDF_OCR_TIMEOUT: Duration = Duration::from_secs(45);
const MAX_BODY_BYTES: usize = 10 * 1024 * 1024; // 10 MB before extraction
const MAX_REDIRECTS: usize = 10;
const DEFAULT_MAX_OUTPUT_CHARS: usize = 100_000; // 100KB, matches Claude Code
const DEFAULT_MAX_RESULTS: usize = 5;
pub const MAX_RESULTS_CAP: usize = 20;
const MAX_DOMAIN_FILTERS: usize = 10;
const HTML_TEXT_WIDTH: usize = 80;
const USER_AGENT: &str = "Mozilla/5.0 (compatible; HQ-Agent/0.7)";
const CACHE_TTL: Duration = Duration::from_secs(15 * 60); // 15 min cache
const SEARCH_DEADLINE: Duration = Duration::from_secs(20);
const SEARXNG_TIMEOUT: Duration = Duration::from_secs(5);
const NATIVE_ENGINE_TIMEOUT: Duration = Duration::from_secs(8);
const BRAVE_TIMEOUT: Duration = Duration::from_secs(12);
const BRAVE_ENDPOINT: &str = "https://api.search.brave.com/res/v1/web/search";
/// Brave's `offset` tops out at 9, so page 10 is the last one it can serve.
const BRAVE_MAX_PAGE: u32 = 10;
/// Below this many characters a PDF's text layer is treated as missing, as in the Telegram relay.
const MIN_PDF_TEXT_CHARS: usize = 50;

pub use chain::{web_search, web_search_parameters};
pub use client::{check_public_url, guarded_client};
pub use fetch::{FetchedPage, html_to_text, web_fetch};
pub use tools::{
    FETCH_TOOL_TIMEOUT_MS, SEARCH_TOOL_TIMEOUT_MS, WebFetchHqTool, WebSearchHqTool,
    format_search_results,
};
pub use types::{Category, Freshness, ProviderAttempt, SearchOptions, SearchResult, WebSearchResults};

use backends::*;
#[cfg(test)]
use chain::*;
use client::*;
use fetch::*;
use native::*;
use readable::{embedded_text, extract_article};
use types::*;
