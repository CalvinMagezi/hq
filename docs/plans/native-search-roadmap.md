# Native search: research and roadmap

Status: research, 2026-10-07. Nothing here is built yet. It records what four
parallel investigations and a set of live probes found about how to make HQ's
built-in `web_search` more stable and more useful to agents than SearxNG is,
using what only an in-process Rust implementation inside HQ can do.

## Where we are

Native search recreates SearxNG's keyless pipeline (`crates/hq-tools/src/web/native*`):
parallel fan-out, SearxNG's ranking formula, flat per-class suspensions, cleaned
and deduplicated results. Measured against a captured SearxNG baseline it
reaches overlap@10 0.60 and top-3 recall 0.80, which is close to the ceiling
(the same Google endpoint overlaps itself only 0.5 to 0.8 across two networks).

What carries the quality is one engine: Google's Programmable Search element
endpoint (`google cse`), which is keyless and undocumented. DuckDuckGo and Brave
HTML are blocked from datacenter IPs most of the time. So the open problems are
resilience (one engine, one IP-throttled quota), coverage beyond Google, and
quality for an agent reader rather than a human.

## What the research changed

1. **Fingerprint impersonation is not the lever.** Same-IP tests of curl,
   python-requests and `curl_cffi` (chrome, firefox, chrome99_android) against
   Google `/wml`, DuckDuckGo, Brave, Startpage, Qwant and Mojeek showed the
   blocks are IP reputation or request rate, not TLS/HTTP2 fingerprint. Google
   `/wml` returned 403 to every client including SearxNG's own recipe. DuckDuckGo
   challenged four of five clients after the first request and passed plain
   requests again after resting 90 seconds. Brave was the only discriminating
   target and not in the direction impersonation predicts (curl and
   reqwest/native-tls pass, python-requests and wreq's Chrome/Firefox emulation
   get 429). wreq (the only live Rust option) costs about 3.4 MB of binary,
   cmake and clang on CI, and a BoringSSL symbol clash risk with the vendored
   OpenSSL. **No-go** unless a datacenter block is later proven to be
   fingerprint-gated.
2. **Startpage and Yahoo do not work from the VPS.** Both work from a home
   connection (Startpage after an Anubis proof of work at difficulty 6, not 4 as
   in SearxNG's code; Yahoo after two manual redirect hops). From the VPS,
   Startpage serves a hard "Startpage Blocked" page and Yahoo answers HTTP 500.
   The proof-of-work solver would have been a Rust advantage, but it cannot
   rescue an IP-level block.
3. **Mojeek works from the VPS.** It has its own independent index, its
   Altcha challenge is PBKDF2-SHA256 (cost 8000), solved in 1.3 s in Python from
   the VPS (a few milliseconds in Rust), and the search page returned 10 results
   with no captcha. This is the one proof-of-work engine worth building.
4. **The undocumented Google endpoint is the single biggest risk.** The
   official routes are closing: Google's Custom Search JSON API is closed to new
   customers and ends 2027-01-01, the Bing Search API was retired in 2025, and
   Brave's free tier became a $5 monthly credit that needs a card. Diversifying
   away from one undocumented endpoint is the main reason to add engines.
5. **Egress, not engines, is the lever for the VPS.** A blocked datacenter IP
   can borrow a residential one: HQ already runs on several hosts joined by a
   tailnet and already speaks MCP between them.

## Ranked opportunities

Impact is on agent outcomes and reliability. "Rust" marks work that an in-process
implementation does cheaply and a Python service cannot.

| # | Item | Impact | Effort | Risk | Rust |
|---|---|---|---|---|---|
| 1 | Snippet and error-text sanitiser (control, zero-width, bidi, Unicode tag characters, length caps, instruction-text flag) plus a secret scan on outbound queries | High (safety) | S | Low | |
| 2 | Single-flight (identical concurrent queries share one pool run), normalised cache key, negative cache | High under parallel sub-agents | S-M | Low | yes |
| 3 | Early return once the primary engines answered, late results merged into the cache; today latency is the slowest engine, up to 8 s | High (p50 latency) | M | Medium | yes |
| 4 | Lexical rerank (BM25 of query vs title and snippet, term coverage, proximity) blended with engine order, same-domain cap, near-duplicate collapse, MMR | High | S | Low-medium | yes |
| 5 | Evidence mode: `read: true` fetches the top-k pages in parallel through the existing extractor and returns query-relevant passages with source offsets and a token budget | Very high | M | Medium | yes |
| 6 | Mojeek engine with a native Altcha solver (new `pow.rs`) | High (second independent index) | 1-1.5 d | Low-medium | yes |
| 7 | Peer search: the VPS calls the home HQ's `web_search` over the tailnet through `remote_mcp` | High for the VPS | 0.5-1 d | Low | |
| 8 | `EgressPool`: per-engine proxy routes (SOCKS5/HTTP), cooldowns keyed by (engine, egress), engine-host allowlist | High (targets IP reputation) | 3-4 d | Low-medium | yes |
| 9 | Cross-process engine health in a separate SQLite file: restart-safe breaker with jittered half-open probes, learned token buckets, shared Google slot reservation | High | M-L | Medium | yes |
| 10 | Stack Exchange term-relaxation ladder (the endpoint needs every term to match, so five-word queries return nothing) | Medium | S | Low | |
| 11 | More clean engines: Stack Exchange network sites, Crossref, Wikidata, Wikipedia REST search; opt-in Brave API key engine | Medium | 1-2 d | Very low | |
| 12 | Deterministic answerers (time zones, arithmetic, units, semver; later currency and weather) returned as `answers` with provenance | Medium-high | M | Low | yes |
| 13 | Eval harness: recorded engine fixtures replayed through the ranker, graded labels, nDCG/MRR, optional LLM judge; log which results an agent later opened or cited | Enabler | M | Medium (privacy) | |
| 14 | Intent routing that adds code, news or science engines on strong signals, capped at two extras | Medium | S | Medium (request volume) | |
| 15 | Persistent result cache with stale-while-revalidate, served when engines are blocked | Medium | M | Low-medium | |
| 16 | Operator surfaces: `hq web-search status` (note that `hq search` is already the vault search), doctor, settings page, MACHINE.md hints | Medium | M | Low | |
| 17 | Bounded, auditable adaptive weights from agent feedback (bandit style, 0.7 to 1.3 clamp), default off until there is data | Medium | M | Medium-high | |
| 18 | Optional embedding rerank through the existing Ollama or OpenRouter path | Medium | S | Medium | |

## Proposals in more detail

### Coverage

- **Mojeek.** `GENERAL`, weight about 0.8. Flow: GET `/captcha/challenge`, scan
  the 4-byte big-endian counter until `PBKDF2-SHA256(nonce || counter, salt,
  cost, 32 bytes)` starts with `keyPrefix`, POST the base64 solution as a
  multipart `altcha` field to `/captcha/verify`, keep the `chllg` cookie, then
  GET `/search?q=` (omit `s` on page 1; `s=10*(page-1)` after). Parse
  `ul.results-standard > li`, `a.ob`, `h2 a.title`, `p.s`. Cache the cookie. It
  needs a small stateful session helper like the one `google.rs` uses for its
  token.
- **Do not build** Startpage or Yahoo for the VPS (blocked there). They could be
  a home-IP-only option once egress routing exists. Skip Qwant (DataDome 403),
  Ecosia, Presearch, Reddit JSON, Yandex, Swisscows (client-rendered) and
  Marginalia (interstitial, key gated).
- **Clean additions** at supplementary weight: Stack Exchange `serverfault` and
  `unix`, Crossref (science), Wikidata search, Wikipedia REST search. An opt-in
  Brave API key engine gives a supported route.
- **SearxNG federation:** one of eight sampled public instances returned JSON
  (the rest 429, 403, 418 or a bot page), and searx.space health data did not
  predict it. Not recommended; self-hosting SearxNG
  (`scripts/setup-searxng.sh`) is better.

### Egress

- **Peer search (item 7).** Register the home HQ as a `remote_mcp` server on the
  VPS and let the VPS call its `web_search` when local engines are blocked. It
  reuses MCP auth and the tailnet-only bind, and the peer applies its own SSRF
  guard. It needs a loop guard and tolerates the peer being asleep.
- **`EgressPool` (item 8).** New `web/egress.rs`. Config shape
  `web_search.egress: [{name, proxy | kind: hq_peer, engines?, fallback_only?}]`
  with `direct` implicit and first. `client_for(engine)` picks the first eligible
  route that is not cooling down. Cooldowns are keyed by (engine, egress), so a
  VPS block does not mute the engine on the home route. Proxied clients are
  built for engine requests only, never `web_fetch`, because a proxy resolves DNS
  itself and the pinned resolver does not run: the redirect policy must reject any
  hop outside the engine host allowlist.
- **Recipes, not code:** `ssh -D` with a dedicated key restricted by
  `permitopen` to the engine hosts (do not reuse the host key, whose gate only
  allows the host subcommands) and a userspace `tailscaled --socks5-server` with an
  exit node. Tor and WARP are not worth advertising.

### Request path and state

- **Single-flight, early return, cache (items 2, 3, 15).** `search_pool` should
  detach: the first caller returns when the primary engines have answered, the
  rest finish in the background and update the cache. Key by a normalised query
  and options, not a Debug string; cache empty and blocked answers briefly;
  serve stale results while blocked.
- **Backpressure.** Twenty parallel sub-agents queue behind the 1 s Google
  pacing lock and can spend the whole 20 s deadline waiting. Add early shedding
  and a global concurrency limit; across processes, reserve Google slots in
  shared state.
- **Shared engine health (item 9).** A separate `search_state.db` next to the
  vault database (a shared-database checkpoint stall once caused a crash loop),
  WAL, a short busy timeout. The search path reads an in-process mirror and
  writes through one bounded writer thread per process, batched. Tables:
  `engine_state` (breaker state, fail streak, wall-clock `open_until`, probe
  lease, last class and reason, EWMA latency and success, token bucket),
  `engine_event` (capped ring), `kv_state`. SearxNG's flat durations stay as
  the first-failure value, then escalate with jitter; a conditional UPDATE
  grants exactly one process the half-open probe. If the store fails, behaviour
  falls back to today's in-memory state. Block state stays machine-local: a Mac
  block says nothing about the VPS IP.

### Quality for an agent reader

- **Lexical rerank and diversity (item 4).** The pool never looks at the query
  after merging; the only relevance lever is engine weight. A small BM25 and
  term-coverage score blended with engine order by reciprocal rank, a
  same-domain cap, near-duplicate collapse on title and snippet hashes, and MMR
  are cheap, deterministic and need no model. Default on.
- **Evidence mode (item 5).** Today an agent searches, then makes two to five
  `web_fetch` calls of up to 100k characters each. A single call that fetches
  the top-k in parallel (semaphore of 4, one per host; 8 s per URL, 12 s overall,
  2 MB cap; no OCR; no Jina fallback because it sends the URL to a third party),
  chunks each page into 120 to 200 word windows with their heading, ranks them
  with pooled BM25 and MMR (at most three per source), and returns about 3k
  tokens of cited passages with `char_start`/`char_end` offsets and a failure
  list would replace that. The new tool name must be added to
  `UNTRUSTED_SOURCE_TOOLS` in `governance/taint.rs`, or its output is not
  tainted. Pages can be keyword-stuffed so injected text ranks as a passage;
  strip hidden text and cap passage length.
- **Query understanding (items 10, 14).** Relax Stack Exchange queries in steps
  (stopwords, top four terms, top three, then a tag). Route to extra categories
  only on strong signals and cap the extras at two.
- **Optional heavier rerank (items 18, and a cross-encoder).** The vault
  embedding path calls one input per request, so reranking 20 results costs 21
  sequential calls: opt-in only. A cross-encoder (MiniLM-L6, 23 MB int8, Apache-2.0)
  through `ort` in load-dynamic mode is realistic only as an opt-in feature with a
  first-use download; the repository has no ML runtime today.

### Safety and observability

- **Sanitiser and query scan (item 1).** Snippets, titles and error text are
  attacker-influenced and only whitespace-collapsed today. The taint model also
  restricts only bash egress, but a search query is an outbound channel: scan it
  with `governance/secrets.rs` before it leaves.
- **Answerers (item 12).** Add `answers: Vec<Answer>` with provenance beside
  `results`. Pure answerers run before the backends and skip them on a full-query
  match, with strict anchored triggers. Time zones, arithmetic, units and semver
  first; currency (ECB rates) and weather (a keyless forecast API) later; skip
  DNS and whois because of SSRF risk.
- **Measurement (items 13, 17).** The benchmark measures similarity to SearxNG,
  not relevance. Record engine responses as fixtures and replay them through the
  ranker offline, grade a small query set by hand, and add an optional LLM judge.
  Tool usage records the tool name only, with no URLs, and `task_outcomes.quality_score`
  is never set, so there is currently no signal for which results an agent opened
  or cited; add a `search_events` log in the governed-tool wrapper, local with an
  opt-out, hash query keys and never store raw queries. Adaptive weights are a
  later step, off until there is data.

## Not recommended now

- wreq/TLS impersonation (see above), Tor, WARP, public SearxNG federation.
- Startpage and Yahoo for datacenter hosts (blocked), and the other engines listed
  under Coverage.
- A cross-encoder as default, and any heavy ML dependency in the default build.
- Page-memory in the vault before the sanitiser lands and before there is a
  freshness policy (never short-circuit live search on recency queries).

## Build order

1. **Safety and hygiene:** sanitiser and query scan (1), cache key and negative
   cache and single-flight (2), Stack Exchange relaxation (10).
2. **Latency and ranking:** early return (3), lexical rerank, domain cap, dedupe,
   MMR (4), with fixture replay so the ranker change is measured (13).
3. **Coverage and egress:** Mojeek and the clean engines (6, 11), peer search (7),
   then `EgressPool` (8).
4. **Durability:** shared health store (9), status surfaces (16), persistent cache (15).
5. **Agent value:** evidence mode (5), answerers (12), feedback log (13).
6. **Later, if data supports it:** intent routing (14), adaptive weights (17),
   embedding rerank (18).

## What is still unverified

- Startpage, Mojeek and Yahoo were each tried once or a few times from a home
  connection and once from the VPS; behaviour under sustained use, and from a
  GitHub runner, is unknown.
- Rust hash rates for the proof-of-work solvers were estimated, not measured.
- Whether `.vault/_data` is synchronised between machines, and where the tool
  constructors should initialise a process-global state store.
- Real Google CSE quota behaviour, and the terms of the keyless endpoints used
  (Google CSE, Wikimedia, Stack Exchange, Mojeek).
- Latency gains from early return and the benefit of evidence mode are estimates.
- The Brave result discrimination (curl and reqwest/native-tls pass, others 429)
  is unexplained; a Linux/OpenSSL reqwest test against Brave would settle it.
