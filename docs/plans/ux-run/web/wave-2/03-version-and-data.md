# Web wave 2 · 03 · Version skew and the data budget (M14, M10)

Boards: **AlignedWebRules** sections 7 ("A newer HQ…") and 9 (Data budget), **AlignedSettingsMore**
section 2 (Install and version, Data used), **AlignedSettings** (the card slot), **AlignedShell2** frame
6 and **AlignedSearch2** section 5 (the phone pays the data), AlignedReview M10 and M14. Follows
`../../rules-web.md` sections 7 and 9. Where this spec and the rules differ, the rules win.

## 1. Finding and the review items it closes

- **M14 (versions half).** An old bundle talks to a newer HQ with no word to the owner; "Reload to keep
  going" for an old bundle.
- **M10 Data and first load.** About 25 pollers; the phone pays both ways when it is the hub; heavy
  readers in the first load. A data budget per hour, events over the one socket, pause when hidden, lazy
  readers, data used under This browser.

Who it hurts: an owner on a metered link or a phone hub (every poll is paid twice), and anyone who keeps
a tab open across an HQ update.

## 2. Before (as built, verified at `e5f9d5f`)

- `apps/hq-web/src/routes/__root.tsx:165` `swUpdateAvailable`, `:277-292` a green bar "A new vault app
  version is available" with "Update now" (`handleSwUpdate`). It is about the service worker's shell, not
  about HQ's version, and it says nothing about drafts.
- `crates/hq-web/src/api.rs:27-34` `/health` returns `version`, `git_sha`, `build_time`; no minimum
  client; the page never compares.
- Polling: 40 call sites of `refetchInterval`, `setInterval` or `usePolled(` in `apps/hq-web/src` (by
  grep), for example `routes/tasks.tsx:59` and `:178` (the archived list), `components/sessions/usePolled.ts`,
  `lib/useRefreshOn.ts` (socket-driven refresh exists for some, `routes/tasks.tsx:176` `task:sync`).
  18 sites look at `visibilityState`; the rest keep polling in a hidden tab.
- Heavy readers: the note page already loads PDF, code, Word and spreadsheet viewers lazily
  (`routes/vault/$.tsx:27-31`), `shiki` is a dynamic import (`components/MarkdownViewer.tsx:15`,
  `CodeViewer.tsx:8`), `mammoth` and `xlsx` load on demand (`lib/vaultApi.ts:196, 206`). Charts
  (`recharts`) and `framer-motion` are in the main bundle (check with `bun run build` output).
- No data counter, no Data used page, no data saver check (`navigator.connection.saveData`).

## 3. Decision

**Tell the owner when the page is older than HQ, and keep drafts across the reload. Keep to a measured
data budget: one socket for changes, nothing while hidden, heavy code on demand, and show the numbers in
This browser › Data used.**

Targets to measure: **under 1 MB an hour idle, a Chat first load under 300 KB.** When the phone is the
hub, the phone's data is counted.

Rejected:

- **Reload by itself.** It would drop an unsent draft or a half-typed answer.
- **Keep polling but slower.** Every poll is a request the phone pays for twice as the hub; changes
  already flow over the socket for tasks.

## 4. Screens and states

### A newer HQ

- **V1 · Toast,** from 768 bottom right, below 768 above the bottom bar: "A newer HQ is running. Reload
  to keep going." [Reload] and a close button. It stays until used or closed. It never covers the
  focused field.
- **V2 · The card slot** in Settings: "A newer HQ is running" / "Reload to keep going. Drafts are kept."
  [Reload]. It takes the slot first (`../wave-1/06-settings-groups.md`).
- **V3 · Install and version:** "HQ in this browser · 0.9.1" and "HQ · Updated by itself from a signed
  release at 14:20. · 0.9.2", then the card "A newer HQ is running · Reload to keep going. Drafts are
  kept." [Reload].
- **V4 · A call HQ refuses for an old page** (HQ answers 426 with `{"min_client": "0.9.2"}`): the
  action's failure line is "Couldn't {action}. A newer HQ is running. Reload to keep going." and the
  toast shows.
- Widths and editions: the same at every width, in Lite and in embedded mode (embedded: the phone shows
  its own update prompt; the page shows only V4's line). HQ window: HQ Hub reloads the window itself when
  the hub updates; V1 still applies when HQ (not the hub) updated.

### Data used (This browser)

- **D1.** "Today · 6 MB", "This week · 38 MB", "An hour with HQ open and idle · HQ aims for under 1 MB an
  hour. · 0.4 MB", "Opening Chat · 240 KB". "HQ gets changes over one link and pauses while this tab is
  hidden. It follows your browser's data saver. Big readers, like PDF and spreadsheets, load only when
  opened." Technical details: What HQ noticed.
- **D2 · The HQ window with the phone as the hub:** "Counted on your phone · 4 MB today".
- **D3 · Over the target:** the idle row "1.6 MB" with an amber warn icon and "More than HQ aims for."
  (Recommended; the board draws only the in-target state.)

## 5. Exact copy

| Key | String |
|-----|--------|
| `newer_hq` | "A newer HQ is running. Reload to keep going." |
| `newer_hq_card` | "A newer HQ is running" · "Reload to keep going. Drafts are kept." · "Reload" |
| `version_browser` | "HQ in this browser" · "{0.9.1}" |
| `version_hq` | "HQ" · "Updated by itself from a signed release at {14:20}." · "{0.9.2}" |
| `fail_{action}_newer` | "Couldn't {action}. A newer HQ is running. Reload to keep going." |
| `data_today`, `data_week` | "Today" · "This week" |
| `data_idle` | "An hour with HQ open and idle" · "HQ aims for under 1 MB an hour." |
| `data_chat_first` | "Opening Chat" |
| `data_note` | "HQ gets changes over one link and pauses while this tab is hidden. It follows your browser's data saver. Big readers, like PDF and spreadsheets, load only when opened." |
| `data_phone` | "Counted on your phone · {4 MB} today" |

Retired: "A new vault app version is available", "Update now".

## 6. Behaviour rules

### Version

1. **Two numbers.** The page knows its own build version (injected at build, `__APP_VERSION__`) and
   reads HQ's `version` and `min_client` from `/api/hello`. When `min_client` is newer than the page, or
   HQ's version is newer and the shell in the worker's cache is older, V1 shows.
2. **Drafts are kept.** Before Reload, the page writes the composer, the note editor's unsaved text and
   any open answer field to the HQ-keyed drafts store; after reload they come back. The outbox survives
   (IndexedDB).
3. **HQ may refuse an old client.** For calls whose shape changed, HQ answers 426 with `min_client`; the
   page shows V4 and does not retry.
4. **The worker updates the shell** on the next navigation (today's behaviour); Reload calls
   `registration.waiting?.postMessage('skip-waiting')` then `location.reload()`.

### Data

5. **One socket carries changes.** Every list that polls today subscribes to a socket event instead
   (`task:sync` exists; add `inbox:change`, `helpers:change`, `notes:change`, `budget:change`,
   `settings:change`). Remaining timers run only while a screen that needs them is visible (a helper's
   screen while Technical details is open).
6. **Pause when hidden.** On `visibilitychange` to hidden: close the screen stream, stop timers, keep the
   socket (it is idle). On visible: one refresh of the visible place.
7. **Heavy readers on demand.** PDF, spreadsheets, Word, code colouring, charts and the QR code library
   load only when a screen needs them. The Chat start screen loads no reader and no chart code.
8. **Data saver.** With `navigator.connection.saveData` true (or the phone hub), pictures in replies do
   not preload, the screen stream is off until asked, and Search waits 300 ms after the last key (WR12).
9. **Measure.** Count bytes from `PerformanceResourceTiming.transferSize` (and the socket's frames) per
   hour, per day and per week, in this browser only; first load is the Chat route's transfer size and
   `loadEventEnd`. Kept with "What HQ noticed".
10. **The phone pays.** In the HQ window with the phone as the hub, the counter is labelled "Counted on
    your phone"; the same budget applies.

## 7. Browser and platform notes

- `transferSize` is 0 for cached and cross-origin responses without `Timing-Allow-Origin`; HQ is same
  origin, so it is reported. Safari reports it from 16.4.
- `navigator.connection` exists in Chromium only; Firefox and Safari never report data saver, so rule 8
  applies there only for the phone hub.
- Socket frames have no transfer size API; count the message lengths in `WebSocketContext`.

## 8. Code touchpoints

| File | Change | Exists |
|------|--------|--------|
| `apps/hq-web/src/routes/__root.tsx` | V1 toast from `/api/hello`; remove the green bar (277-292). | yes |
| `apps/hq-web/vite.config.ts` | Inject `__APP_VERSION__`. | yes |
| `apps/hq-web/src/lib/version.ts` | Compare versions, keep drafts, reload. | NEW |
| `apps/hq-web/src/lib/dataUsage.ts` | Rule 9. | NEW |
| `apps/hq-web/src/context/WebSocketContext.tsx` | Change events (rule 5), frame byte count. | yes |
| `apps/hq-web/src/components/sessions/usePolled.ts`, `lib/useRefreshOn.ts` and the 40 polling sites | Socket events; pause when hidden. | yes |
| `apps/hq-web/src/sw.ts` | `skip-waiting` message. | yes |
| `crates/hq-web/src/api.rs` | `min_client` in `/api/hello`; 426 for refused shapes. | NEW |
| `crates/hq-web/src/ws/mod.rs` | Change events per kind. | yes |

## 9. Server, desktop or release changes

Server: `min_client` and change events over the one socket (`../../changes.md` "A minimum-client
header" and "Change events over the one socket instead of polling loops"). Release: the web build carries
its version.

## 10. Acceptance checks (each fails on today's build)

1. With `/api/hello` answering `min_client` newer than the page, the toast reads "A newer HQ is running.
   Reload to keep going." Today nothing compares.
2. Type in the composer, press Reload: the text is back after the reload. Today it is lost unless the
   draft store saved it.
3. A hidden tab for an hour makes no HTTP request (Playwright with `page.evaluate` setting visibility,
   or a 60-minute run counting requests). Today timers keep running.
4. The Chat first load transfers under 300 KB (Lighthouse or `transferSize` sum). Measure today's as the
   baseline; record it in the PR.
5. Data used shows the four rows. Today there is no page.

## 11. Check on a laptop or phone

- Keep a tab open, update HQ (`./scripts/install-hq.sh`): the toast shows; Reload keeps a draft.
- Leave Chat idle for an hour with DevTools' network panel open: under 1 MB.
- With the phone as the hub, watch the phone's data counter for an hour of the HQ window idle.

## 12. Open decisions it depends on

- **Decision O**, **WR12**. Depends on `../wave-0/02-shared-parts.md` (`/api/hello`, counters),
  `../wave-1/06-settings-groups.md` (the pages).
