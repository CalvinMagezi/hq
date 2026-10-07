# Agent browser, Stage 0: measured result

Date: 2026-10-07. Method: `bench_web_fetch_vs_jina` in `crates/hq-tools/src/web/fetch_tests.rs`
(ignored; run with `cargo test -p hq-tools bench_web_fetch_vs_jina -- --ignored --nocapture`)
over `src/web/fixtures/browser-corpus.txt`, 64 URLs from a home connection. For each
page it runs HQ's native `web_fetch` path (Jina off) and the Jina Reader, and calls a
page useful at 500 characters or more.

## Result

| | Pages | Share |
|---|---|---|
| Native path gave a useful page | 50 | 78% |
| Native path failed, Jina succeeded | 9 | 14% |
| Both failed | 5 | 8% |

The gate in `agent-browser.md` was at least 15% of pages failing without JavaScript,
with a rendering reader succeeding. The raw figure is 14%, and the cause matters more
than the figure. Of the nine:

- **Four need JavaScript to show any content**: `hn.algolia.com` (search app),
  `app.netlify.com`, `excalidraw.com`, `play.rust-lang.org`. All are applications,
  not documents. A reader is rarely sent there to extract text.
- **One is a thin client-rendered page**: the GitLab project page (210 characters).
- **Four are access refusals, not rendering**: `stackoverflow.com` (403 to every
  non-browser client tried), `linkedin.com` (429), `figma.com/community` (403 to
  this agent's User-Agent, 200 to a browser one), `crates.io` (an error for this
  agent where a plain request succeeds). A JavaScript engine would not change
  these; Jina succeeds because of how it identifies itself and where it fetches from.

Both failed (5): `platform.openai.com/docs`, `npmjs.com/package/react` (238 characters),
`reddit.com`, `nytimes.com`, `reuters.com`. Mostly access walls.

Pages that count as native successes but are thin or gated: `x.com` (login wall, 960
characters), a gated Hugging Face model page, `substack.com/discover`. A character
count cannot see these; they would also defeat a browser.

## Verdict

The gate is not met. Fewer than one page in ten (4 to 5 of 64) fails for want of
JavaScript, and those are interactive apps. Do not start Stage 1 (the no-JS snapshot
layer) or Stage 2 (QuickJS) on this evidence. Jina stays an opt-in last resort.

Cheaper work that the data points to:

- Fetch headers: a short list of sites refuse HQ's agent User-Agent but serve a
  browser one (Figma). A per-host header policy, or API shortcuts for registries
  (crates.io and npm already have JSON APIs the search engines use), is a day of work.
- Stack Overflow and LinkedIn refuse generic clients; the search engines already
  return Stack Exchange content through the API.

## Limits of this measurement

- 64 hand-picked pages, weighted toward documentation sites. What agents read after a
  search skews to docs, blogs, forums and news, so it is representative but not random.
- One network (a home connection). A datacenter address is refused more often, which
  inflates "access refusal" and does not change the JavaScript share.
- The 500-character threshold misses junk that is long and misses short pages that are
  fine. Not hand-judged page by page.
- Jina was queried once per page; a transient Jina failure counts as "both failed".
