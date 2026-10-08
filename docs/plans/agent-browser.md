# An HQ-owned agent browser: research and plan

Status: research, 2026-10-07. Nothing here is built. It follows
`native-search-roadmap.md` and answers one question: should HQ use, vendor or
build a Rust browser, given that search is central and HQ must not depend on a
pile of external tools or assume Tailscale.

## Short answer

- **Do not depend on or vendor Obscura.** Use it as a reference. Its code is
  Apache-2.0 and HQ is MIT, so the licences are compatible, but the engine is
  welded to V8, it is large, and it ships stealth by default.
- **Build an HQ-owned core in stages, starting with a layer that needs no
  JavaScript.** The first stages are small and valuable on their own; the
  JavaScript stage is a long tail that should be gated on measured need.
- **A browser does not make search more reliable.** Search blocks are IP
  reputation and request rate. The browser improves the *read* half of search:
  opening the page you found, even when it is rendered by JavaScript.
- **No Tailscale is assumed anywhere in this plan.** The browser runs in the HQ
  process tree on whatever machine HQ runs on. Where a peer is useful it is any
  MCP-reachable HQ.

## What Obscura is, measured

Apache-2.0, created April 2026, 28k stars, about 170k lines of Rust: an arena
DOM on html5ever (5.8k), JS bindings on V8 through `deno_core` (36k, of which a
17.9k-line `bootstrap.js` shims the Web APIs), a CDP server (23.6k), an MCP
server with 36 `browser_*` tools (3.6k), network with an optional BoringSSL
"stealth" transport, and a 77k-line layout and paint layer.

A sandboxed run of the macOS no-render release (reads of the home directory and
writes outside scratch denied, empty environment, public pages only):

| Page | Result |
|---|---|
| example.com | correct, 0.5 s, 33 MB |
| a client-rendered search page (hn.algolia.com) | 9.7 KB of real results with `networkidle0`; plain HTTP sees a 2.4 KB shell. 5.9 s, 101 MB |
| TodoMVC React and Vue | tiny output (17 bytes), 2.1 s |
| DuckDuckGo html | real results (the same URL answered 202 to curl a moment later, so confounded) |
| Brave search | "Verifying you're not a bot" page, while curl got 261 KB of results |
| Bing | results, with a script error logged, 7.8 s |

So it genuinely renders client-side pages, and it does not unblock search
engines: its stealth transport got a challenge from Brave where plain curl
passed, which matches HQ's own finding that fingerprinting is not the lever.

Why not vendor it:

- Weight: V8 roughly doubles the 58 MB HQ binary (the release tarballs are 51 MB
  plus a 49 MB worker), `rusty_v8` downloads a prebuilt static library at build
  time (a supply-chain trust point), and cross builds need a snapshot step the
  release pipeline does not have.
- Coupling: `obscura-js` and the Web API shims are welded to `deno_core`. The
  engine-agnostic parts are small.
- Project risk: one contributor wrote about 85% of the commits, 196 issues are
  open (worker leaks, slow DOM mutation, placeholder geometry), and there is no
  checksum, signature or attestation for the release binaries.
- Positioning: stealth and `navigator.webdriver` spoofing are on by default, the
  README is a funnel for a hosted product with residential proxies, and the
  sponsors are proxy sellers. Shipping that in HQ conflicts with the policy
  below.

What to take as ideas (not code): element references that are invalidated on
navigation, the `networkidle0/2` wait modes, the page lifecycle states, an
"assets" dump of a page's sub-resources, and the IPv6 embedding checks in its
SSRF classifier (a diff against `web/ssrf.rs` is worth doing). Portable source,
if ever needed: `obscura-ssrf` (139 lines, no dependencies) and the cookie jar.

## What an HQ-owned core would be

Measured engine choices (release hello-worlds, aarch64 macOS):

| JS engine | Binary delta | Cold build | fib(30) | Notes |
|---|---|---|---|---|
| rquickjs (QuickJS, bundled C) | +525 KB | 26 s | 226 ms | no `Intl`; modern ES passes |
| boa, no Intl | +2.2 MB | 44 s | 3.5 s | pure Rust, about 15x slower here |
| V8 via `deno_core` | about +20 to 30 MB (published) | prebuilt `libv8.a` | JIT | not measured |

QuickJS is the only realistic default-on engine. V8 stays an optional feature.
JavaScriptCore is macOS only; SpiderMonkey and Servo are too heavy for anything
but an experiment.

Agents do not need layout or paint. They need what Playwright's accessibility
snapshot needs: a DOM walk with computed visibility (`display`, `visibility`,
`hidden`, `aria-hidden`), roles and names, and stable element references. That is
a small cascade (inline styles, `<style>`, UA defaults) over an arena DOM built
on html5ever's `TreeSink` with `selectors` for querying, not a renderer. Do not
build CSS layout, media or WebGL. A real browser is required for those, for
bot-protected sites and for login flows.

### What an agent should get

- A text snapshot with role, name, state, link target and form values, stable
  `ref` ids, a hard token budget, and a diff against the previous snapshot. This
  is the best token-to-reliability ratio among the public agent browsers
  (Playwright MCP, Browser Use, Stagehand, Claude in Chrome); screenshots cost
  the most.
- Actions by ref: click, type, select, press, scroll, wait for text or network
  idle. Per-session cookies in memory. Structured extraction (the readable
  article we already have, tables, JSON-LD, OpenGraph), a request log and
  console errors.

## Integration in HQ

- **Process model.** A `hq-browser` crate behind a cargo feature, run in a worker
  that is a re-exec of the same `hq` binary speaking JSON lines over stdio. Page
  JavaScript never runs in the agent process, there is still one release
  artifact, and the update system's `--version` check is unaffected.
- **Network through the parent.** The worker has no sockets. Every request,
  redirect and subresource goes through the parent and the existing guard
  (`validate_url`, `GuardedResolver`), so the SSRF protection is shared and the
  worker's sandbox can deny the network outright. No proxies for browser traffic
  (a proxy resolves DNS itself and skips the pinned resolver).
- **Tools.** `browser_open`, `browser_snapshot`, `browser_click`, `browser_type`,
  `browser_extract`, `browser_wait`, `browser_close`, registered next to
  `web_fetch` in `hq-mcp`'s registry and reachable through the `hq_discover` /
  `hq_call` gateway. Every one goes on `UNTRUSTED_SOURCE_TOOLS` in
  `governance/taint.rs`; mutating actions are not read-only, so the permission
  presets and a confirmation gate apply to clicks that submit and to typing.
- **Sessions.** In memory, owned by the worker, keyed by chat session, three tabs
  and an idle timeout. Persist only an audit row (session, URL, action); do not
  persist or replay live DOM state.
- **Limits.** Navigation 15 s, script budget 5 s, 2 MB per response, 256 MB,
  three tabs, one worker per session; the parent kills the worker on any
  overrun. Run JavaScript only when a sandbox backend (the bash sandbox's
  bubblewrap or `sandbox-exec`) is available.
- **`web_fetch`.** The order becomes: readable extract, embedded-data recovery,
  local render, and the third-party Jina Reader only if the operator opts in.
  That closes a privacy gap (Jina receives every URL) and works on a host with no
  outbound access to it.

### Policy (public repository)

- Honest `User-Agent`, robots.txt for the render path, per-host rate limits.
- No fingerprint spoofing, no `webdriver` hiding, no canvas or WebGL noise, no
  captcha-solving services, no residential proxy rotation.
- The line: solving what a server hands to any client (the Mojeek and Startpage
  proof-of-work challenges) is protocol-compliant client work; impersonating a
  human or another browser is not. Blocked pages are reported as blocked.
- No autofill and no access to the secret store; credential flows are out of
  scope; outbound `type` text goes through the secrets governance.
- Snapshots contain visible text only; hidden text, comments and scripts are
  stripped, then the roadmap's sanitiser applies.

## Where a browser helps search, and where it does not

From the SearxNG source: DuckDuckGo's challenge is solved by a regex and
arithmetic, Brave's results are an embedded object literal that is parsed, Bing
web is plain HTTP, and only Google's normal web page needs JavaScript (which is
why HQ uses the Programmable Search endpoint, and it is IP-blocked anyway).
Startpage and Yahoo are blocked from datacenter IPs regardless of the client. A
browser fixes none of these.

It adds real value for reading JavaScript-rendered pages and docs that embedded
data cannot recover, consent walls that need a click, multi-step flows
(pagination, "load more", tabs), and evidence extraction after render. These
numbers are estimates until measured.

## Build order, with gates

| Stage | Deliverable | Effort | Gate to continue |
|---|---|---|---|
| 0 | Measure: a fixture and recorded corpus of about 50 real pages and a benchmark of current `web_fetch` against Jina on it | 1 wk | at least 15% of pages fail without JavaScript, or Jina succeeds where HQ cannot. Otherwise stop and keep Jina opt-in |
| 1 | `browser` feature (default off at first): worker, arena DOM, visibility cascade, snapshot with refs, link follow and form submit over HTTP, cookies, extraction, taint, limits. No JavaScript | 4 to 6 wks | beats embedded-data recovery on the corpus, clean sandbox and SSRF review |
| 2 | `hq-js` on rquickjs: scripts, timers with virtual time, fetch and XHR through the parent, `location`, `history`, storage, minimal DOM binding | 8 to 12 wks | render success at least equal to Jina on the SPA subset (target 80%), p95 under 10 s, adversarial review of the sandbox. If the DOM cost looks like a six-month project, stop at stage 1 |
| 3 | ES modules, MutationObserver fidelity, custom elements, shadow DOM, iframes | 8 to 12 wks | only if stage 2 shows these are what fails |
| 4 | Optional heavy backend or handoff: V8 behind the same trait, or a generic CDP client for any installed Chromium-family browser | 2 to 4 wks | explicit user demand; never in the default release |

Success criteria: at least 80% render success on the SPA subset, zero SSRF
escapes, p95 under 10 s, and every limit enforced by a test. Fixtures: a local
server with representative SPAs, the existing wiremock patterns, and hostile
pages (infinite loop, memory bomb, hidden-text injection, redirect to loopback).

A generic CDP client (stage 4) is the honest answer to "use someone else's
engine": it would work with Chromium, Obscura, Lightpanda or any compatible
browser the operator already has, without HQ depending on or recommending any
one of them.

## Relationship to what already exists

Claude in Chrome is a real, logged-in, visible browser on the user's machine and
stays the right tool for authenticated and human-in-the-loop work; it is not
available on a VPS or to unattended sessions. The host drives terminals, not pages.
An HQ browser adds headless operation on any host, deterministic token-budgeted
output, governance and taint, and an audit trail.

## Unverified

- V8, Servo and Lightpanda sizes, and Servo's headless story; the LOC and
  person-week figures are estimates.
- Whether QuickJS's `WeakRef` failure in the smoke test is an engine bug or a
  test flaw.
- Obscura's own test claims, the `rusty_v8` build script, and a security review of
  its CDP server; the sandboxed run covered seven page loads.
- The share of real pages that need JavaScript for the content HQ agents read,
  which is exactly what stage 0 measures.
