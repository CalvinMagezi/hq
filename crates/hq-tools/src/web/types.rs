use super::*;

/// A single search result. Fields past `snippet` are provider metadata and
/// are absent when the provider didn't supply them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
    /// 1-based rank within the page the provider returned, before domain filtering.
    #[serde(default)]
    pub position: usize,
    /// Backend that produced this result: `searxng`, `native` or `brave`.
    #[serde(default)]
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    /// Publication or last-update date as the provider reported it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published: Option<String>,
    /// Upstream engines this result came from (SearxNG or the built-in pool).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub engines: Vec<String>,
    /// The title or snippet reads like instructions to a model. Treat the
    /// result as untrusted data; it is kept so the agent can see what was said.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub flagged: bool,
}

/// One backend's part in a search: answered, failed (and why), or skipped.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderAttempt {
    pub provider: String,
    pub outcome: String,
}

/// Collection of search results.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSearchResults {
    pub query: String,
    pub results: Vec<SearchResult>,
    /// Backend whose results these are.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,
    #[serde(default = "first_page")]
    pub page: u32,
    /// Page to request next, when the backend says there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_page: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attempts: Vec<ProviderAttempt>,
    /// Requested filters the answering backend could not apply.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unsupported_filters: Vec<String>,
}

pub(super) fn first_page() -> u32 {
    1
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    Day,
    Week,
    Month,
    Year,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    General,
    News,
    Science,
    Images,
    Code,
}

/// Optional search controls. `Default` reproduces the unfiltered behaviour.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchOptions {
    pub max_results: usize,
    /// 1-based page number.
    pub page: u32,
    pub freshness: Option<Freshness>,
    /// ISO 639-1 code, lowercase (`en`).
    pub language: Option<String>,
    /// ISO 3166-1 alpha-2 code, uppercase (`US`).
    pub country: Option<String>,
    pub category: Option<Category>,
    pub include_domains: Vec<String>,
    pub exclude_domains: Vec<String>,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            max_results: DEFAULT_MAX_RESULTS,
            page: 1,
            freshness: None,
            language: None,
            country: None,
            category: None,
            include_domains: Vec::new(),
            exclude_domains: Vec::new(),
        }
    }
}

impl SearchOptions {
    /// Parse the tool arguments shared by the MCP and agent `web_search` tools.
    /// Bad values are rejected with a message naming the field, never ignored.
    pub fn from_args(args: &Value) -> Result<Self, String> {
        let mut opts = Self::default();
        if let Some(v) = args.get("max_results").filter(|v| !v.is_null()) {
            let n = v.as_u64().ok_or("max_results must be a positive integer")?;
            opts.max_results = (n as usize).clamp(1, MAX_RESULTS_CAP);
        }
        if let Some(v) = args.get("page").filter(|v| !v.is_null()) {
            let n = v
                .as_u64()
                .filter(|n| *n >= 1)
                .ok_or("page must be an integer >= 1")?;
            opts.page = u32::try_from(n).map_err(|_| "page is too large")?;
        }
        if let Some(s) = str_arg(args, "freshness") {
            opts.freshness = Some(match s.as_str() {
                "day" => Freshness::Day,
                "week" => Freshness::Week,
                "month" => Freshness::Month,
                "year" => Freshness::Year,
                _ => {
                    return Err(format!(
                        "freshness must be one of day, week, month, year (got {s:?})"
                    ));
                }
            });
        }
        if let Some(s) = str_arg(args, "language") {
            if s.len() != 2 || !s.chars().all(|c| c.is_ascii_alphabetic()) {
                return Err(format!(
                    "language must be a 2-letter ISO 639-1 code like \"en\" (got {s:?})"
                ));
            }
            opts.language = Some(s.to_lowercase());
        }
        if let Some(s) = str_arg(args, "country") {
            if s.len() != 2 || !s.chars().all(|c| c.is_ascii_alphabetic()) {
                return Err(format!(
                    "country must be a 2-letter ISO 3166 code like \"US\" (got {s:?})"
                ));
            }
            opts.country = Some(s.to_uppercase());
        }
        if let Some(s) = str_arg(args, "category") {
            opts.category = Some(match s.as_str() {
                "general" => Category::General,
                "news" => Category::News,
                "science" => Category::Science,
                "images" => Category::Images,
                "code" => Category::Code,
                _ => {
                    return Err(format!(
                        "category must be one of general, news, science, images, code (got {s:?})"
                    ));
                }
            });
        }
        opts.include_domains = domain_list_arg(args, "include_domains")?;
        opts.exclude_domains = domain_list_arg(args, "exclude_domains")?;
        Ok(opts)
    }
}

pub(super) fn str_arg(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
}

pub(super) fn domain_list_arg(args: &Value, key: &str) -> Result<Vec<String>, String> {
    let Some(v) = args.get(key).filter(|v| !v.is_null()) else {
        return Ok(Vec::new());
    };
    let items = v
        .as_array()
        .ok_or(format!("{key} must be an array of domains"))?;
    if items.len() > MAX_DOMAIN_FILTERS {
        return Err(format!("{key} takes at most {MAX_DOMAIN_FILTERS} domains"));
    }
    items
        .iter()
        .map(|d| {
            d.as_str()
                .ok_or(format!("{key} entries must be strings"))
                .and_then(normalize_domain)
        })
        .collect()
}

/// `https://www.Example.com/docs` becomes `example.com`.
pub(super) fn normalize_domain(raw: &str) -> Result<String, String> {
    let lower = raw.trim().to_lowercase();
    let no_scheme = lower
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let host = no_scheme.split('/').next().unwrap_or_default();
    let host = host.trim_start_matches("www.").trim_end_matches('.');
    let valid = host.contains('.')
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
    if valid {
        Ok(host.to_string())
    } else {
        Err(format!("invalid domain {raw:?}"))
    }
}

pub(super) fn domain_of(url: &str) -> Option<String> {
    let host = reqwest::Url::parse(url).ok()?.host_str()?.to_lowercase();
    Some(host.trim_start_matches("www.").to_string())
}

pub(super) fn host_matches(url: &str, domain: &str) -> bool {
    domain_of(url).is_some_and(|h| h == domain || h.ends_with(&format!(".{domain}")))
}

pub(super) fn domain_allowed(url: &str, opts: &SearchOptions) -> bool {
    let included = opts.include_domains.is_empty()
        || opts.include_domains.iter().any(|d| host_matches(url, d));
    included && !opts.exclude_domains.iter().any(|d| host_matches(url, d))
}

/// The query as sent to a backend: domain filters become `site:` operators.
pub(super) fn effective_query(query: &str, opts: &SearchOptions) -> String {
    let mut q = query.trim().to_string();
    match opts.include_domains.as_slice() {
        [] => {}
        [one] => q.push_str(&format!(" site:{one}")),
        many => {
            let sites: Vec<String> = many.iter().map(|d| format!("site:{d}")).collect();
            q.push_str(&format!(" ({})", sites.join(" OR ")));
        }
    }
    for d in &opts.exclude_domains {
        q.push_str(&format!(" -site:{d}"));
    }
    q
}
