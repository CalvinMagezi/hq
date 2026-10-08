# Web search and page reading

`web_search` and `web_fetch` work on a fresh install with no key, no Docker and no
other service. Everything below runs inside the `hq` process.

## What `web_search` does

One query goes to several free engines at once. Results are merged the way SearxNG
merges them (agreement between engines and high positions count most), then scaled by
how well the title, snippet and URL match the query, with an exact title match ranked
first. Results are cached for ten minutes, identical concurrent searches share one run,
and once a main engine has answered the others get 1.5 seconds before the pool returns.

| Category (`category` argument) | Engines |
|---|---|
| general (default) | Google (through its Programmable Search endpoint), DuckDuckGo, Brave Search, Mojeek, Wikipedia |
| news | Bing News, Hacker News |
| science | arXiv, OpenAlex, Crossref, Europe PMC |
| images | Google Images, Wikimedia Commons, Openverse |
| code | GitHub, Stack Overflow, Ask Ubuntu, Super User, MDN, crates.io, npm |

Filters (`freshness`, `language`, `country`, `include_domains`, `exclude_domains`,
`page`) are applied where an engine supports them, and the result lists the filters it
could not apply.

## What to expect from a server

Search engines judge a request by the address it comes from. From a home connection all
the engines above answer. From a datacenter address Google, DuckDuckGo and Brave often
refuse, and **Mojeek, an independent index, carries the search**. HQ solves the small
proof-of-work puzzle Mojeek serves to every visitor (about a second, once; the
verification cookie is kept for weeks). `hq doctor` shows which engines answered and
which are cooling down. A blocked engine is suspended for a few minutes and the
suspension survives a restart.

Ways to make a server stronger, all optional:

- `web_search_peer: <name>`: name a `remote_mcp` entry (another HQ, for example one on a
  home connection). When the local engines come back empty or thin, the query is sent to
  that HQ's `web_search`. Only name an HQ you control, because it receives your queries.
  A forwarded call carries `peer_hop: true` so two HQs naming each other do not loop.
- `brave_api_key`: the Brave Search API, tried last (paid, but reliable from anywhere).
- `searxng_url`: a SearxNG instance, tried first. Not needed; `scripts/setup-searxng.sh`
  starts one in Docker if you want it.
- `web_search_native: false`: turn the built-in engines off and use only the above.

## Safety

- A query that looks like a credential (API key, token, private key) is refused before it
  leaves the process, because a query is an outbound channel.
- Every result passes a sanitiser: invisible and direction-control characters are
  stripped, fields are cut to a fixed length, and text that reads like instructions to a
  model sets `flagged` so the agent treats it as data. A search marks the session as
  having read untrusted content.
- Engines are reached through a client that refuses private and loopback addresses on
  redirects. Details: `docs/security/WEB_FETCH.md`.

## What HQ remembers

`~/.hq/search_state.db` (mode 0600) holds the Mojeek cookie and which engines are
suspended, so a restart does not repeat the puzzle or walk back into a block. It never
holds queries or results. `HQ_SEARCH_STATE=off` keeps everything in memory.

## `web_fetch`

Reads a page in this order: main-content extraction, recovery from the page's own
embedded data (JSON-LD, `__NEXT_DATA__`, OpenGraph), and, only if both give nothing, the
Jina Reader (`r.jina.ai`), which receives the URL. `HQ_WEB_FETCH_JINA=0` never sends a
URL to it. PDFs use their text layer, and scanned PDFs use `pdftoppm` and `tesseract`
when installed. Measured on 64 real pages, the native path returned a useful page for
78%; most of the rest were sites that refuse non-browser clients, not pages that need
JavaScript (`docs/plans/agent-browser-stage0.md`).

## Checking it

- `hq doctor` runs one test query per backend.
- `cargo test -p hq-tools live_native -- --ignored --nocapture` queries every engine for
  every category. The weekly `search-canary` workflow does the same from CI.
- Plans and measurements: `docs/plans/native-search-roadmap.md`,
  `docs/plans/agent-browser.md`.
