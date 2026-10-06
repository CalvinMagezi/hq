//! Web tools — search the web and fetch page content.
//!
//! Search tries a self-hosted SearxNG instance first when `searxng_url` is
//! configured and reachable (free, no API key, see `scripts/setup-searxng.sh`),
//! falling back to the paid Brave Search API when SearxNG is unset, failing,
//! cooling down after recent failures, or returns nothing. Neither backend is
//! guaranteed to exist on a given host. The machine profile reports each one
//! as configured or reachable without querying it; [`probe_search_backends`]
//! (used by `hq doctor`) sends one real query per backend. Every response
//! here names the backend that actually answered and each fallback attempt.
//!
//! The whole chain runs under one deadline (`SEARCH_DEADLINE`, 20s): SearxNG
//! gets at most 5s, Brave at most 12s, each capped by whatever is left. Every
//! transport, HTTP, JSON and shape failure puts that backend into exponential
//! cooldown, keyed by its endpoint.
//!
//! Filters are optional and provider-dependent. Backend differences:
//!
//! | option     | SearxNG                           | Brave                         |
//! |------------|-----------------------------------|-------------------------------|
//! | freshness  | `time_range` (engine-dependent)   | `freshness` pd/pw/pm/py       |
//! | language   | `language`                        | `search_lang`                 |
//! | country    | only with language (`en-US`)      | `country`                     |
//! | category   | general, news, science            | general, news                 |
//! | domains    | `site:` operators + post-filter   | `site:` operators + post-filter |
//! | page       | `pageno`, unbounded               | `offset`, pages 1..=10        |
//!
//! An option the answering backend can't honor is listed in
//! `unsupported_filters` rather than silently dropped. Domain filters are
//! always enforced by a post-filter on the result host, so a page can come
//! back with fewer than `max_results` hits.
//!
//! Fetching uses reqwest plus html2text for HTML, `hq-convert` for PDFs (text
//! layer first, OCR for scanned files), and an automatic Jina Reader
//! (`r.jina.ai`, a third-party service that receives only the URL) fallback
//! for JS-rendered pages that come back as an empty shell. Every redirect hop
//! is re-checked against the private-network rules. Pages that need a logged-in
//! browser, interaction, or anything Jina can't render stay unsupported.

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
use types::*;
