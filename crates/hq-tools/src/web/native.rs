//! Built-in keyless meta-search: fans one query out to several free public
//! engines in parallel, merges and ranks the answers, and caches the result.
//! Each engine has its own cooldown, so one that blocks or changes layout
//! drops out alone while the rest keep answering.

use super::*;
use tokio::task::JoinSet;

mod engines;
mod markup;
#[cfg(test)]
mod tests;

pub(super) use engines::Engine;

const NATIVE_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const RESULT_CACHE_TTL: Duration = Duration::from_secs(10 * 60);
/// A pool with some engines down is cached briefly, so a retry soon after picks them up again.
const PARTIAL_RESULT_CACHE_TTL: Duration = Duration::from_secs(60);
const CACHE_MAX_ENTRIES: usize = 200;
/// Damping constant from the reciprocal-rank-fusion paper.
const RRF_K: f32 = 60.0;
/// Engines at or above this weight are the real web indexes.
const PRIMARY_WEIGHT: f32 = 1.0;
const ENGINE_BACKOFF_BASE: Duration = Duration::from_secs(20);
const ENGINE_BACKOFF_CAP: Duration = Duration::from_secs(10 * 60);
const ENGINE_RATE_LIMIT_BACKOFF_BASE: Duration = Duration::from_secs(60);
const ENGINE_RATE_LIMIT_BACKOFF_CAP: Duration = Duration::from_secs(30 * 60);
/// Query parameters that only track clicks; they never change which page a URL is.
const TRACKING_PARAMS: &[&str] = &["fbclid", "gclid", "msclkid"];

/// The SSRF-guarded client: engine hosts are fixed, but redirects are not.
pub(super) static NATIVE_CLIENT: std::sync::LazyLock<Client> =
    std::sync::LazyLock::new(|| fetch_client(NATIVE_REQUEST_TIMEOUT, validate_url));

/// Where and how engines are reached. Tests point `base_override` at a mock server.
pub(super) struct NativeEnv<'a> {
    pub(super) client: &'a Client,
    pub(super) base_override: Option<&'a str>,
}

impl NativeEnv<'static> {
    pub(super) fn production() -> Self {
        Self {
            client: &NATIVE_CLIENT,
            base_override: None,
        }
    }
}

struct CacheEntry {
    results: Vec<SearchResult>,
    next_page: Option<u32>,
    stored_at: Instant,
    ttl: Duration,
}

static RESULT_CACHE: std::sync::LazyLock<Mutex<HashMap<String, CacheEntry>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

fn cache_key(query: &str, opts: &SearchOptions, base_override: Option<&str>) -> String {
    format!(
        "{}\u{1}{opts:?}\u{1}{}",
        query.trim().to_lowercase(),
        base_override.unwrap_or_default()
    )
}

fn cached_page(key: &str) -> Option<Page> {
    let cache = RESULT_CACHE.lock().ok()?;
    let entry = cache.get(key).filter(|e| e.stored_at.elapsed() < e.ttl)?;
    Some(Page {
        results: entry.results.clone(),
        next_page: entry.next_page,
    })
}

fn store_page(key: String, page: &Page, ttl: Duration) {
    let Ok(mut cache) = RESULT_CACHE.lock() else {
        return;
    };
    if cache.len() >= CACHE_MAX_ENTRIES {
        cache.retain(|_, e| e.stored_at.elapsed() < e.ttl);
    }
    if cache.len() >= CACHE_MAX_ENTRIES {
        let oldest = cache
            .iter()
            .min_by_key(|(_, e)| e.stored_at)
            .map(|(k, _)| k.clone());
        if let Some(oldest) = oldest {
            cache.remove(&oldest);
        }
    }
    cache.insert(
        key,
        CacheEntry {
            results: page.results.clone(),
            next_page: page.next_page,
            stored_at: Instant::now(),
            ttl,
        },
    );
}

fn engine_backoff(rate_limited: bool) -> (Duration, Duration) {
    if rate_limited {
        (
            ENGINE_RATE_LIMIT_BACKOFF_BASE,
            ENGINE_RATE_LIMIT_BACKOFF_CAP,
        )
    } else {
        (ENGINE_BACKOFF_BASE, ENGINE_BACKOFF_CAP)
    }
}

struct EngineRun {
    order: usize,
    weight: f32,
    attempts: Vec<ProviderAttempt>,
    results: Option<Vec<SearchResult>>,
}

/// What the pool produced for one search.
pub(super) struct PoolAnswer {
    /// `None` when no engine answered (the attempts say why); `Some` with no
    /// results means engines answered and found nothing.
    pub(super) page: Option<Page>,
    /// No full-weight web engine answered, so the page is only what the
    /// supplementary engines (Wikipedia, Hacker News) had.
    pub(super) degraded: bool,
    pub(super) attempts: Vec<ProviderAttempt>,
}

/// Cached front for [`search_pool`]. A complete answer is kept for 10 minutes,
/// one from a pool with engines down for a minute, a degraded one not at all.
pub(super) async fn native_search(
    query: &str,
    opts: &SearchOptions,
    env: &NativeEnv<'_>,
    deadline: Instant,
    budget: Duration,
) -> PoolAnswer {
    let key = cache_key(query, opts, env.base_override);
    if let Some(page) = cached_page(&key) {
        let note = ProviderAttempt {
            provider: "native".into(),
            outcome: format!("ok, {} results (cached)", page.results.len()),
        };
        return PoolAnswer {
            page: Some(page),
            degraded: false,
            attempts: vec![note],
        };
    }
    let (answer, complete) = search_pool(query, opts, env, deadline, budget).await;
    if let Some(page) = answer.page.as_ref().filter(|p| !p.results.is_empty()) {
        let ttl = if complete {
            Some(RESULT_CACHE_TTL)
        } else {
            (!answer.degraded).then_some(PARTIAL_RESULT_CACHE_TTL)
        };
        if let Some(ttl) = ttl {
            store_page(key, page, ttl);
        }
    }
    answer
}

/// Query every engine for the category in parallel and merge the answers. The
/// flag says every engine that was asked answered. No caching here, so
/// `hq doctor` can use it to test the engines for real.
// ponytail: engines are asked once per query with no per-engine pacing; the
// result cache and cooldowns cap the request rate. Add jitter if one starts blocking.
pub(super) async fn search_pool(
    query: &str,
    opts: &SearchOptions,
    env: &NativeEnv<'_>,
    deadline: Instant,
    budget: Duration,
) -> (PoolAnswer, bool) {
    let mut set = JoinSet::new();
    for (order, engine) in Engine::for_category(opts.category)
        .iter()
        .copied()
        .enumerate()
    {
        let client = env.client.clone();
        let (query, opts) = (query.to_string(), opts.clone());
        let base = env
            .base_override
            .map(str::to_string)
            .unwrap_or_else(|| engine.default_base(&opts));
        set.spawn(async move {
            run_engine(
                order, engine, &client, &base, &query, &opts, deadline, budget,
            )
            .await
        });
    }
    let mut runs = Vec::new();
    let mut attempts = Vec::new();
    while let Some(joined) = set.join_next().await {
        match joined {
            Ok(run) => runs.push(run),
            Err(e) => attempts.push(ProviderAttempt {
                provider: "native".into(),
                outcome: format!("engine task failed: {e}"),
            }),
        }
    }
    runs.sort_by_key(|r| r.order);
    attempts.extend(runs.iter().flat_map(|r| r.attempts.clone()));
    let complete = !runs.is_empty() && runs.iter().all(|r| r.results.is_some());
    let degraded = !runs
        .iter()
        .any(|r| r.results.is_some() && r.weight >= PRIMARY_WEIGHT);
    let lists: Vec<(f32, Vec<SearchResult>)> = runs
        .into_iter()
        .filter_map(|r| r.results.map(|results| (r.weight, results)))
        .collect();
    if lists.is_empty() {
        let answer = PoolAnswer {
            page: None,
            degraded: true,
            attempts,
        };
        return (answer, false);
    }
    let results = merge(lists);
    let next_page = (!results.is_empty()).then(|| opts.page.saturating_add(1));
    let answer = PoolAnswer {
        page: Some(Page { results, next_page }),
        degraded,
        attempts,
    };
    (answer, complete)
}

#[allow(clippy::too_many_arguments)]
async fn run_engine(
    order: usize,
    engine: Engine,
    client: &Client,
    base: &str,
    query: &str,
    opts: &SearchOptions,
    deadline: Instant,
    budget: Duration,
) -> EngineRun {
    let mut attempts = Vec::new();
    if opts.page > 1 && !engine.supports_paging() {
        attempts.push(ProviderAttempt {
            provider: engine.name().into(),
            outcome: "skipped: serves page 1 only".into(),
        });
        return EngineRun {
            order,
            weight: engine.weight(),
            attempts,
            results: None,
        };
    }
    let key = format!("{base}#native:{}", engine.name());
    let backend = Backend {
        key: &key,
        label: engine.name(),
        budget,
        backoff: engine_backoff,
    };
    let page = try_backend(
        &backend,
        deadline,
        &mut attempts,
        || engine.fetch(client, base, query, opts),
        |body| {
            engine.parse(body, base).map(|results| Page {
                results,
                next_page: None,
            })
        },
    )
    .await;
    EngineRun {
        order,
        weight: engine.weight(),
        attempts,
        results: page.map(|p| p.results),
    }
}

/// Reciprocal-rank fusion: a result several engines agree on outranks one that
/// a single engine put first. Ties keep the order engines were queried in.
fn merge(lists: Vec<(f32, Vec<SearchResult>)>) -> Vec<SearchResult> {
    struct Slot {
        score: f32,
        seq: usize,
        result: SearchResult,
    }
    let mut slots: HashMap<String, Slot> = HashMap::new();
    let mut seq = 0;
    for (weight, list) in lists {
        for (rank, r) in list.into_iter().enumerate() {
            let score = weight / (RRF_K + rank as f32 + 1.0);
            match slots.get_mut(&normalize_url(&r.url)) {
                Some(slot) => {
                    slot.score += score;
                    absorb(&mut slot.result, r);
                }
                None => {
                    slots.insert(
                        normalize_url(&r.url),
                        Slot {
                            score,
                            seq,
                            result: r,
                        },
                    );
                    seq += 1;
                }
            }
        }
    }
    let mut ranked: Vec<Slot> = slots.into_values().collect();
    ranked.sort_by(|a, b| b.score.total_cmp(&a.score).then(a.seq.cmp(&b.seq)));
    ranked
        .into_iter()
        .enumerate()
        .map(|(i, mut s)| {
            s.result.position = i + 1;
            s.result
        })
        .collect()
}

/// Fold a duplicate hit into the kept one: richer snippet, date if missing, every engine.
fn absorb(kept: &mut SearchResult, dup: SearchResult) {
    if dup.snippet.len() > kept.snippet.len() {
        kept.snippet = dup.snippet;
    }
    if kept.published.is_none() {
        kept.published = dup.published;
    }
    for e in dup.engines {
        if !kept.engines.contains(&e) {
            kept.engines.push(e);
        }
    }
}

/// Identity of a page for deduplication: scheme, `www.`, fragment, trailing
/// slash and click-tracking parameters do not make a different result.
fn normalize_url(raw: &str) -> String {
    let Ok(url) = reqwest::Url::parse(raw) else {
        return raw.to_string();
    };
    let host = url
        .host_str()
        .unwrap_or_default()
        .trim_start_matches("www.")
        .to_lowercase();
    let host = match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host,
    };
    let mut query: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(k, _)| !k.starts_with("utm_") && !TRACKING_PARAMS.contains(&k.as_ref()))
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    query.sort();
    let query: Vec<String> = query.into_iter().map(|(k, v)| format!("{k}={v}")).collect();
    format!(
        "{host}{}?{}",
        url.path().trim_end_matches('/'),
        query.join("&")
    )
}
