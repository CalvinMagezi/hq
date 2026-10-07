//! Built-in keyless meta-search: fans one query out to several free public
//! engines in parallel, merges and ranks the answers, and caches the result.
//! Each engine has its own cooldown, so one that blocks or changes layout
//! drops out alone while the rest keep answering.

use super::*;
use tokio::task::JoinSet;

mod engines;
mod google;
mod images_code;
mod markup;
#[cfg(test)]
mod tests;

pub(super) use engines::Engine;

/// Once a primary engine has answered, slower engines get this long before the
/// pool returns without them (and caches the answer as partial).
const STRAGGLER_GRACE: Duration = Duration::from_millis(1500);
const NATIVE_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const RESULT_CACHE_TTL: Duration = Duration::from_secs(10 * 60);
/// A pool with some engines down is cached briefly, so a retry soon after picks them up again.
const PARTIAL_RESULT_CACHE_TTL: Duration = Duration::from_secs(60);
/// An answer of "nothing found" from a healthy pool, kept briefly so a rare
/// query asked again does not hit every engine again.
const NEGATIVE_RESULT_CACHE_TTL: Duration = Duration::from_secs(30);
const CACHE_MAX_ENTRIES: usize = 200;
/// Distinct searches that may run at once. Each runs in its own task and holds
/// connections to every engine for up to the pool deadline, so a burst of
/// parallel sub-agents queues here instead of opening hundreds of requests.
const MAX_CONCURRENT_POOL_RUNS: usize = 12;
/// Engines at or above this weight are the real web indexes.
const PRIMARY_WEIGHT: f32 = 1.0;
/// Query parameters that only track clicks; they never change which page a URL is.
const TRACKING_PARAMS: &[&str] = &["fbclid", "gclid", "msclkid"];

/// The SSRF-guarded client: engine hosts are fixed, but redirects are not.
pub(super) static NATIVE_CLIENT: std::sync::LazyLock<Client> =
    std::sync::LazyLock::new(|| fetch_client(NATIVE_REQUEST_TIMEOUT, validate_url));

/// Where and how engines are reached. Tests point `base_override` at a mock server.
pub(super) struct NativeEnv<'a> {
    pub(super) client: &'a Client,
    pub(super) base_override: Option<&'a str>,
    /// How long slower engines get once a primary one has answered; `None` waits for every engine.
    pub(super) straggler_grace: Option<Duration>,
}

impl NativeEnv<'static> {
    pub(super) fn production() -> Self {
        Self {
            client: &NATIVE_CLIENT,
            base_override: None,
            straggler_grace: Some(STRAGGLER_GRACE),
        }
    }

    /// For `hq doctor`, which must hear from every engine.
    pub(super) fn diagnostic() -> Self {
        Self {
            straggler_grace: None,
            ..Self::production()
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

/// Queries that differ only in case or spacing are the same search, and the
/// options that change what the engines are asked are spelled out so a new
/// field cannot silently change or fragment the key.
fn cache_key(query: &str, opts: &SearchOptions, base_override: Option<&str>) -> String {
    let query = query
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    let sorted = |domains: &[String]| {
        let mut d = domains.to_vec();
        d.sort();
        d.join(",")
    };
    format!(
        "{query}\u{1}p{}|n{}|f{:?}|l{}|c{}|k{:?}|i{}|x{}\u{1}{}",
        opts.page,
        opts.max_results,
        opts.freshness,
        opts.language.as_deref().unwrap_or_default(),
        opts.country.as_deref().unwrap_or_default(),
        opts.category,
        sorted(&opts.include_domains),
        sorted(&opts.exclude_domains),
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

struct EngineRun {
    order: usize,
    weight: f32,
    attempts: Vec<ProviderAttempt>,
    results: Option<Vec<SearchResult>>,
}

/// What the pool produced for one search.
#[derive(Clone)]
pub(super) struct PoolAnswer {
    /// `None` when no engine answered (the attempts say why); `Some` with no
    /// results means engines answered and found nothing.
    pub(super) page: Option<Page>,
    /// No full-weight web engine answered, so the page is only what the
    /// supplementary engines (Wikipedia, Hacker News) had.
    pub(super) degraded: bool,
    pub(super) attempts: Vec<ProviderAttempt>,
}

type SharedOutcome = Option<std::sync::Arc<(PoolAnswer, bool)>>;

/// Searches in progress, by cache key. A second identical search waits for the
/// first instead of asking every engine again; twenty parallel sub-agents that
/// ask the same thing cost one pool run.
static INFLIGHT: std::sync::LazyLock<
    Mutex<HashMap<String, tokio::sync::watch::Receiver<SharedOutcome>>>,
> = std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

static POOL_RUNS: std::sync::LazyLock<std::sync::Arc<tokio::sync::Semaphore>> =
    std::sync::LazyLock::new(|| {
        std::sync::Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_POOL_RUNS))
    });

/// Removes the in-flight entry however the search task ends, panics included.
struct InflightGuard(String);

impl Drop for InflightGuard {
    fn drop(&mut self) {
        if let Ok(mut map) = INFLIGHT.lock() {
            map.remove(&self.0);
        }
    }
}

fn store_outcome(key: String, answer: &PoolAnswer, complete: bool) {
    let Some(page) = answer.page.as_ref() else {
        return;
    };
    let ttl = if !page.results.is_empty() {
        if complete {
            Some(RESULT_CACHE_TTL)
        } else {
            (!answer.degraded).then_some(PARTIAL_RESULT_CACHE_TTL)
        }
    } else {
        (!answer.degraded).then_some(NEGATIVE_RESULT_CACHE_TTL)
    };
    if let Some(ttl) = ttl {
        store_page(key, page, ttl);
    }
}

/// Cached, single-flight front for [`search_pool`]. A complete answer is kept
/// for 10 minutes, one from a pool with engines down for a minute, an empty
/// one from a healthy pool for 30 seconds, a degraded one not at all. The
/// search runs in its own task, so a caller that gives up (a tool timeout, a
/// cancelled turn) still leaves the cache warm for the retry.
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
    let Some(mut rx) = join_or_start(&key, query, opts, env, deadline, budget) else {
        // Sharing is unavailable (a poisoned lock): run this search on its own.
        return search_pool(query, opts, env, deadline, budget).await.0;
    };
    let wait = deadline.saturating_duration_since(Instant::now());
    let failure = |outcome: &str| PoolAnswer {
        page: None,
        degraded: true,
        attempts: vec![ProviderAttempt {
            provider: "native".into(),
            outcome: outcome.into(),
        }],
    };
    match tokio::time::timeout(wait, rx.wait_for(Option::is_some)).await {
        Ok(Ok(outcome)) => outcome.as_ref().map_or_else(
            || failure("the shared search produced nothing"),
            |o| o.0.clone(),
        ),
        Ok(Err(_)) => failure("the shared search for this query failed"),
        Err(_) => failure("timed out waiting for the search already running for this query"),
    }
}

fn join_or_start(
    key: &str,
    query: &str,
    opts: &SearchOptions,
    env: &NativeEnv<'_>,
    deadline: Instant,
    budget: Duration,
) -> Option<tokio::sync::watch::Receiver<SharedOutcome>> {
    let mut map = INFLIGHT.lock().ok()?;
    if let Some(rx) = map.get(key) {
        return Some(rx.clone());
    }
    let (tx, rx) = tokio::sync::watch::channel(None);
    map.insert(key.to_string(), rx.clone());
    drop(map);
    let (key, query, opts) = (key.to_string(), query.to_string(), opts.clone());
    let client = env.client.clone();
    let base = env.base_override.map(str::to_string);
    let straggler_grace = env.straggler_grace;
    tokio::spawn(async move {
        let _guard = InflightGuard(key.clone());
        let queue = tokio::time::timeout_at(
            tokio::time::Instant::now() + deadline.saturating_duration_since(Instant::now()),
            POOL_RUNS.clone().acquire_owned(),
        )
        .await;
        let Ok(Ok(_permit)) = queue else {
            let busy = PoolAnswer {
                page: None,
                degraded: true,
                attempts: vec![ProviderAttempt {
                    provider: "native".into(),
                    outcome: "skipped: too many searches already running".into(),
                }],
            };
            let _ = tx.send(Some(std::sync::Arc::new((busy, false))));
            return;
        };
        let env = NativeEnv {
            client: &client,
            base_override: base.as_deref(),
            straggler_grace,
        };
        let (answer, complete) = search_pool(&query, &opts, &env, deadline, budget).await;
        store_outcome(key, &answer, complete);
        let _ = tx.send(Some(std::sync::Arc::new((answer, complete))));
    });
    Some(rx)
}

/// Query every engine for the category in parallel and merge the answers. The
/// flag says every engine that was asked answered. No caching here, so
/// `hq doctor` can use it to test the engines for real.
// ponytail: only Google is paced (see `google::pace`); the other engines rely on
// the result cache and cooldowns to cap their request rate. Add jitter if one starts blocking.
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
    let mut cut_short = false;
    let mut grace_ends: Option<Instant> = None;
    loop {
        let next = match grace_ends {
            Some(end) => {
                match tokio::time::timeout_at(end.into(), set.join_next()).await {
                    Ok(next) => next,
                    Err(_) => {
                        cut_short = true;
                        set.abort_all();
                        break;
                    }
                }
            }
            None => set.join_next().await,
        };
        let Some(joined) = next else { break };
        match joined {
            Ok(run) => {
                let answered_well = run.weight >= PRIMARY_WEIGHT
                    && run.results.as_ref().is_some_and(|r| !r.is_empty());
                if let (true, None, Some(grace)) = (answered_well, grace_ends, env.straggler_grace) {
                    grace_ends = Some(Instant::now() + grace);
                }
                runs.push(run);
            }
            Err(e) => attempts.push(ProviderAttempt {
                provider: "native".into(),
                outcome: format!("engine task failed: {e}"),
            }),
        }
    }
    runs.sort_by_key(|r| r.order);
    attempts.extend(runs.iter().flat_map(|r| r.attempts.clone()));
    let complete = !cut_short && !runs.is_empty() && runs.iter().all(|r| r.results.is_some());
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
    let results = merge(lists, query);
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
    let beyond_pages = opts.page > engine.max_page();
    if opts.page > 1 && (!engine.supports_paging() || beyond_pages) {
        let outcome = if beyond_pages {
            format!("skipped: serves pages up to {} only", engine.max_page())
        } else {
            "skipped: serves page 1 only".to_string()
        };
        attempts.push(ProviderAttempt {
            provider: engine.name().into(),
            outcome,
        });
        return EngineRun {
            order,
            weight: engine.weight(),
            attempts,
            results: None,
        };
    }
    let key = format!("{base}#native:{}", engine.name());
    if engine.paced() && !with_health(&key, |h| h.is_cooling_down()).unwrap_or(false) {
        // Queueing behind other searches happens before the engine's own budget
        // starts, so a long queue is not mistaken for a failing engine.
        let remaining = deadline.saturating_duration_since(Instant::now());
        if tokio::time::timeout(remaining, google::pace())
            .await
            .is_err()
        {
            attempts.push(ProviderAttempt {
                provider: engine.name().into(),
                outcome: "skipped: queued behind other searches past the deadline".into(),
            });
            return EngineRun {
                order,
                weight: engine.weight(),
                attempts,
                results: None,
            };
        }
    }
    let backend = Backend {
        key: &key,
        label: engine.name(),
        budget,
        backoff: FailureClass::suspension,
    };
    let page = try_backend(
        &backend,
        deadline,
        &mut attempts,
        || engine.fetch(client, base, query, opts),
        |body| {
            engine.parse(body).map(|results| Page {
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

/// SearxNG's ranking. A result's score is the product of the weights of the
/// engines that found it, times the number of positions it was found at, times
/// the sum of `1 / position` over those positions (position is 1-based within
/// each engine's own list). Agreement between engines therefore counts
/// heavily, and a top-ranked hit counts far more than a tenth-ranked one. Ties
/// keep the order engines were queried in.
fn merge(lists: Vec<(f32, Vec<SearchResult>)>, query: &str) -> Vec<SearchResult> {
    struct Slot {
        weight: f32,
        positions: Vec<usize>,
        seq: usize,
        result: SearchResult,
    }
    let mut slots: HashMap<String, Slot> = HashMap::new();
    let mut seq = 0;
    for (weight, list) in lists {
        for (rank, r) in list.into_iter().enumerate() {
            let position = rank + 1;
            match slots.get_mut(&normalize_url(&r.url)) {
                Some(slot) => {
                    // An engine's weight counts once per result, however often it lists it.
                    if r.engines.iter().any(|e| !slot.result.engines.contains(e)) {
                        slot.weight *= weight;
                    }
                    slot.positions.push(position);
                    absorb(&mut slot.result, r);
                }
                None => {
                    slots.insert(
                        normalize_url(&r.url),
                        Slot {
                            weight,
                            positions: vec![position],
                            seq,
                            result: r,
                        },
                    );
                    seq += 1;
                }
            }
        }
    }
    let terms = query_terms(query);
    let score = |s: &Slot| {
        let sum: f32 = s.positions.iter().map(|p| 1.0 / *p as f32).sum();
        s.weight * s.positions.len() as f32 * sum * relevance(&terms, &s.result)
    };
    let mut ranked: Vec<Slot> = slots.into_values().collect();
    ranked.sort_by(|a, b| score(b).total_cmp(&score(a)).then(a.seq.cmp(&b.seq)));
    ranked
        .into_iter()
        .enumerate()
        .map(|(i, mut s)| {
            s.result.position = i + 1;
            s.result
        })
        .collect()
}

const STOPWORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "by", "for", "how", "in", "is", "it", "of", "on",
    "or", "the", "to", "vs", "what", "with",
];
/// Weight a result keeps when none of the query's terms appear in it.
const MIN_RELEVANCE: f32 = 0.2;

fn query_terms(query: &str) -> Vec<String> {
    let mut terms: Vec<String> = query
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty() && !STOPWORDS.contains(t))
        .map(str::to_string)
        .collect();
    terms.dedup();
    terms
}

/// Share of the query's terms found in the result's title, snippet or URL,
/// scaled into `MIN_RELEVANCE..=1`. Narrow engines return their closest hit
/// even when it has nothing to do with the query, which this demotes.
fn relevance(terms: &[String], r: &SearchResult) -> f32 {
    if terms.is_empty() {
        return 1.0;
    }
    let haystack = format!("{} {} {}", r.title, r.snippet, r.url).to_lowercase();
    let hits = terms
        .iter()
        .filter(|t| haystack.contains(t.as_str()))
        .count();
    MIN_RELEVANCE + (1.0 - MIN_RELEVANCE) * hits as f32 / terms.len() as f32
}

/// Fold a duplicate hit into the kept one as SearxNG does: the longer title
/// and content win, a missing date is filled, an `https` link replaces an
/// `http` one, and every engine is kept.
fn absorb(kept: &mut SearchResult, dup: SearchResult) {
    if dup.title.len() > kept.title.len() {
        kept.title = dup.title;
    }
    if dup.snippet.len() > kept.snippet.len() {
        kept.snippet = dup.snippet;
    }
    if kept.published.is_none() {
        kept.published = dup.published;
    }
    if kept.url.starts_with("http://") && dup.url.starts_with("https://") {
        kept.url = dup.url;
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
