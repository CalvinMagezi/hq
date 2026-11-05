# Web wave 0 · 01 · The four live bugs (WB1, WB2, WB3, guarded routes)

Boards: **AlignedReview** ("4 live bugs to fix now", blockers WB1 to WB3) and **AlignedWebRules**
section 10 ("Live bugs to fix before the redesign, ship on their own"), with the screens on
AlignedSettingsMore (Notifications), AlignedFirstRun2 (signed out), AlignedHub2 and AlignedHubDevices
(a port another program holds). Follows `../../rules-web.md` sections 1, 3, 6 and 10 and
`../../rules-shared.md` section 1. Where this spec and the rules differ, the rules win.

These four ship on their own, before any redesign, each in its own small PR with its test. They are
risks today. Nothing in this spec waits on wave 1 or later, and nothing here changes the look of the
app beyond the words of a notification and two status lines.

## 1. Findings and the review items they close

| Bug | Review id | Who it hurts |
|-----|-----------|--------------|
| A. Web push shows content and working Allow buttons | WB2 (blocker), W16, W40 | Every owner with push on. A helper's question, its command and two answer buttons reach the lock screen, the notification history and any synced watch. A 15-minute token in the push lets anyone holding the device answer it. |
| B. HQ Hub can send the phone hub's sign-in to another program | WB3 (blocker) | An owner whose phone is the hub, on a computer where another program holds the forward's port. The HQ window proxies there anyway and passes `Authorization` through, so that program gets the token. |
| C. A signed-out browser keeps showing private data | WB1 (blocker) | Anyone who signs a laptop out from Settings, or rotates the token. The 7-day query cache, the worker's API cache, drafts and the push subscription stay, so cached notes and chats remain readable. |
| D. `drive`, `goal` and `answer` routes take no HQ client header | Review "Recommended next steps" 1, WB6, W15 | Any owner whose browser visits a hostile page while signed in. These routes take over or answer a session, yet a cross-site request needs no preflight to reach them. |

## 2. Before (as built, verified at `e5f9d5f`)

### A. Web push

- `crates/hq-web/src/web_push/mod.rs:223` `payload()`: `title` is `clip(&item.title, 120)` (line 226),
  `body` is `clip(&item.body, 300)` (line 227). For a blocked session the body becomes the question
  and the command joined by a newline (line 237). Actions: the yes-once option by its own label (line
  243) and "Decline" (line 250), with a signed token each (lines 247 and 253) and the item id (line
  255).
- `web_push/mod.rs:205` `answer_buttons()` builds the tokens from the approval key, valid for
  `TOKEN_TTL_SECS` (`crates/hq-web/src/dialog_answer.rs:33`, `15 * 60`).
- `web_push/mod.rs:87` `wants()`: pushes `ActionNeeded`, `Proposal` and the always-pushed sources
  (`notify_owner`'s `NOTIFY_SOURCE` and a device sign-in, line 42), or everything with `push_all`.
  No per-subscription kind filter: the table `push_subscriptions`
  (`crates/hq-db/sql/088_web_push.sql`, `device_id` from `090_web_device_tokens.sql`) has no kinds.
- `crates/hq-tools/src/notify.rs:56-90`: `notify_owner` writes a title (required) and a body; the
  dispatcher pushes both.
- `crates/hq-web/src/lib.rs:374` registers `dialog_answer::PUSH_ANSWER_PATH`
  (`/api/notifications/push-answer`, `dialog_answer.rs:30`), the token-only answer route.
- `apps/hq-web/src/sw.ts:159-178`: the `push` handler shows `data.title`, `data.body`, keeps the
  actions that have a token, sets `requireInteraction` when there are actions. `sw.ts:181-209`
  `answerFromPush()` posts the token and shows "Allowed once", "Declined", "Already answered", "This
  button expired" or "Not sent". `sw.ts:213-237` `notificationclick` answers on `approve` or `decline`.
- `apps/hq-web/src/lib/replyAlerts.ts:43-60` `alertReplyDone()`: the body is the reply's first 140
  characters (line 45), the title is the chat's title, the tag `hq-chat-{id}`, the link
  `/chat?thread={id}`. The switch is one value per browser in localStorage `hq.chat.replyAlerts`, set
  from a bell in the chat header (`components/chat/ChatHeader.tsx:29`), so it reads as a per-chat
  switch.
- `crates/hq-web/src/web_push/api.rs:119-120`: the test push says "HQ push works" / "Notifications from
  HQ and its agents will show up here."
- Tests that assert today's behaviour: `web_push/mod.rs:448`
  `payload_clips_text_and_keeps_links_on_the_app` (title and body at full clip length) and `:486`
  `a_blocked_sessions_push_carries_two_buttons_and_a_token_for_each` (actions "Yes" and "Decline", the
  body "Do you want to allow this?\n/tmp -> /private/tmp", two valid tokens).

### B. The hub proxy

- `crates/hq-desktop/src/desktop.rs:115` `upstream_addr()`: when the config has a hub web forward
  (`config.bridge.hub_web_listen()`, `crates/hq-core/src/config/bridge.rs:107`) it returns that address,
  whether or not this app bound it. `Desktop::open` (line 161) stores it once.
- `crates/hq-bridge/src/from_config.rs:66` `start_forwards()`: a port that is taken is reported as
  `running_elsewhere: true` (line 85) and nothing is started for it. `crates/hq-desktop/src/pairing.rs:285`
  calls it and copies the flag (line 501).
- `crates/hq-desktop/src/proxy.rs:283-300` `forward()`: copies every request header except the hop by
  hop list (lines 26-35), so `Authorization` goes upstream to whatever answers on that port.
- `crates/hq-desktop/src/desktop.rs:296` `proxy_origin()` starts the proxy toward `self.upstream`
  without asking whether the forward is this app's own.
- The control window says "Running elsewhere" for the forward (`apps/desktop/ui/src/main.ts:160`) but
  Open HQ still opens the HQ window.

### C. Wiping on a refused sign-in

- `apps/hq-web/src/lib/hqAuth.ts:127-134` `noteAuth()`: a 401 only sets `signInNeeded` (line 129).
  Nothing is cleared. The token stays in localStorage (`hq-web-token`, line 7).
- `apps/hq-web/src/lib/offlineCache.ts:10` `CACHE_MAX_AGE_MS = 7 days`; `:44` the React Query
  persister in IndexedDB under `hq-query-cache`; `:13` `hq-last-sync` in localStorage.
- `apps/hq-web/src/routes/__root.tsx:407-413` `PersistQueryClientProvider` restores that cache on load.
- `apps/hq-web/src/store/hqStore.ts:107` (`hq-store`) and `src/store/threadStore.ts:322`
  (`hq-thread-store`): zustand stores persisted to IndexedDB.
- `apps/hq-web/src/components/chat/draftStore.ts:18-29`: chat drafts in localStorage.
- `apps/hq-web/src/sw.ts:9` `API_CACHE = 'hq-api'`, `:31-38` the cached reads (notifications, vault
  assets, search, folders, task comments, thread messages), served "of any age" (line 258) on failure.
- `apps/hq-web/src/components/SignInPrompt.tsx:116`: the banner shown on 401, over the cached screens.
- `crates/hq-db/src/web_auth.rs:90-93` `mint()`: "a device's sign-in never expires on its own".
- Server side already drops a signed-out device's push subscription
  (`crates/hq-web/src/device_auth/devices.rs:96` `delete_for_device`) and every subscription on Sign out
  everywhere (line 124). The browser keeps its local push registration.

### D. Unguarded routes

- `crates/hq-web/src/origin.rs:117-118`: `HEADER_GUARDED_ACTIONS = ["send", "adopt", "stop", "resume",
  "rename", "archive"]`, with the comment "`drive`, `goal` and `unwatch` predate the header and must
  keep working for a cached PWA that does not send it yet."
- `origin.rs:126` `missing_client_header()` guards `/api/harness-sessions/{id}/{action}` only for those
  six, plus spawn, workbench, setup, oauth, auth and the provider key.
- Not guarded: `/api/harness-sessions/{id}/drive`, `/goal`, `/unwatch`, `/answer`
  (`lib.rs:599`), and `/api/notifications/{id}/answer` (`lib.rs:370`).
- `origin.rs:311-314` asserts `drive`, `goal` and `unwatch` "stay open to cached clients".
- The web client already sends `X-HQ-Client: web` on every non-GET request (`hqAuth.ts:163`), so a
  current PWA is unaffected by guarding them.

### Before frames (AlignedReview, Drift and the as-built boards)

- **B1 · A push on a lock screen today.** "Claude needs your OK"-style title from the item, body "Do
  you want to allow this? /tmp -> /private/tmp", buttons "Yes" and "Decline".
- **B2 · A reply alert.** Title "Release checklist", body "Three things are left before Friday: sign
  the build, write the release notes, and…" (140 characters).
- **B3 · Settings › Devices after signing this laptop out.** The Sign in banner over Home, which still
  shows Pinned and Recent from the 7-day cache.
- **B4 · HQ Hub, Devices, a forward held elsewhere.** "In use elsewhere" on the web forward row; Open
  HQ enabled; the HQ window loads.

## 3. Decision

**Ship the four fixes now, each small and tested, without waiting for the redesign.**

- **A:** a web push never carries content by default and never carries an answer. The payload is the
  rules' words, built on the server; the service worker shows exactly what it is given and offers only
  Open. Tokens in pushes and the push-answer route are retired.
- **B:** the proxy goes only to a forward HQ Hub bound itself. A forward another program holds is a
  refusal with the rules' line, never a fallback.
- **C:** a 401 from HQ (not from the worker's cache) wipes every browser store at once and shows only
  Sign in. Device sign-ins gain the 90-day expiry later (wave 1); the wipe does not depend on it.
- **D:** `drive`, `goal`, `unwatch` and both `answer` routes need the HQ client header, like every
  changing route.

Rejected:

- **Keep Allow in the push behind a "details" switch.** A browser cannot check a screen lock, so the
  phone's "Allow asks for the unlock" has no equivalent (rules-web 1).
- **Strip content in the service worker only.** An older worker, or any other client reading the push,
  would still get it. The server builds the words.
- **Proxy to the forward but drop `Authorization`.** The web app then cannot sign in to the phone's HQ,
  and the other program still gets every page's content. Refuse instead.
- **Keep `drive` and `goal` open for cached PWAs.** A cached PWA older than the header is a smaller risk
  than any page on the internet driving a session. The worker updates the shell on the next visit.

## 4. Screens and states

### A. Notifications

Built with the system's notification; the words come from the server.

- **A1 · A helper or chat needs the owner (details off, the default).** App name as the browser shows it
  (AgentHQ once installed), title "Claude needs your OK" (or "{Helper} needs your OK", "HQ needs your
  OK"), body "Open HQ to see what it wants to do.", one button "Open". A tap anywhere or Open opens the
  item's page (the Inbox item, the chat, the helper or the task).
- **A2 · An update (details off).** "HQ answered" / "Tap to read." for a reply; for `notify_owner`, the
  item's title only, body empty; for a new sign-in, "A new sign-in to your HQ" / "Open HQ to see it."
- **A3 · Details on (wave 1 adds the switch; the payload field is added now).** The body is the item's
  own first line, never a command, never an argument. A helper's question with a command shows "Claude
  needs your OK" and its first line ("Claude wants to run a command"), not the command.
- **A4 · Test push.** Title "HQ", body "Notifications from HQ are on in this browser."
- **A5 · Older service worker still installed.** It receives the new payload (no `actions`, no
  `tokens`), so it shows title and body only, with no buttons. A tap opens `url`.
- At every width, in Lite (if push is allowed), and with the phone as the hub: the same words. The HQ
  window in HQ Hub has no push (wave 3).

### B. The hub proxy

Drawn on AlignedHub2 and AlignedHubDevices. Today's control window keeps its look in wave 0; only its
words and the button state change.

- **B5 · This PC (Host tab today), the forward held by another program.** The status reads "Your HQ
  can't be reached". The card: title "Couldn't open HQ on your phone", tag "Port in use" (amber), line
  "Another program holds this port. HQ Hub won't send your sign-in there.", then "Often it is another HQ
  pairing started in a terminal. Close it, then look again." Open HQ is off with the why line "Open HQ
  waits until HQ Hub holds the port itself." Under Technical details: "web forward to Pixel 8:
  127.0.0.1:47480, held by another program" and "gate on this PC: 127.0.0.1:7747, opened by HQ Hub".
- **B6 · Devices, the same.** The web forward row reads the same line, amber with an icon, tag "Port in
  use".
- **B7 · The forward is free again.** "Look again" (or the next poll) restarts the forward; Open HQ
  turns on.

### C. Signed out

- **C1 · HQ refuses this browser's token.** Every store is wiped before the next paint. Today's shell
  shows the Sign in prompt and no cached content: each place shows an empty view with "Sign in to see
  your notes" (or "your chats", "what needs you", "your settings") / "HQ no longer accepts this
  browser's sign-in, so nothing from it is kept here." / Sign in. The full redesign of this state is in
  `../wave-4/02-first-run-sign-in.md`.
- **C2 · A request answered from the worker's cache.** A 401 inside a cached copy (header
  `x-hq-cache`) wipes nothing, as `noteAuth` already rules.
- **C3 · Another tab of the same HQ.** Gets the wipe through a `BroadcastChannel('hq-auth')` message and
  shows the same empty view.

### D. Guarded routes

No screen. A request without the header gets today's refusal from the origin guard (403 with the
guard's reason in the log).

## 5. Exact copy

| Where | String |
|-------|--------|
| Push, Needs you, details off | title "{Who} needs your OK" ("Claude needs your OK", "HQ needs your OK"), body "Open HQ to see what it wants to do." |
| Push, reply finished (Updates) | "HQ answered" · "Tap to read." |
| Push, `notify_owner` | its title only; no body |
| Push, new sign-in | "A new sign-in to your HQ" · "Open HQ to see it." |
| Push, details on | title as above; body the item's own first line (never a command) |
| Push button | "Open" (the only one) |
| Test push | "HQ" · "Notifications from HQ are on in this browser." |
| Hub, held port | "Another program holds this port. HQ Hub won't send your sign-in there." |
| Hub, card title | "Couldn't open HQ on your phone" |
| Hub, tag | "Port in use" |
| Hub, hint | "Often it is another HQ pairing started in a terminal. Close it, then look again." |
| Hub, Open HQ off | "Open HQ waits until HQ Hub holds the port itself." |
| Hub, technical | "web forward to {device}: {addr}, held by another program" · "gate on this PC: {addr}, opened by HQ Hub" |
| Signed out, empty view | "Sign in to see your {notes, chats, tasks, settings}" or "Sign in to see what needs you" · "HQ no longer accepts this browser's sign-in, so nothing from it is kept here." · "Sign in" |

Retired: "HQ push works", "Notifications from HQ and its agents will show up here.", "Allowed once",
"Declined", "Already answered", "This button expired", "Not sent", "The agent has your answer.", "The
agent did not take it yet. Open HQ to check.", "HQ could not be reached. Open HQ to answer.", a reply's
own text in an alert, "Running elsewhere" for the hub's own forward.

## 6. Behaviour rules

1. **The server owns the words.** `payload()` builds title and body from the item's kind: Needs you
   (`ActionNeeded`, `Proposal`, a blocked session's alert) gives "{Who} needs your OK" and "Open HQ to
   see what it wants to do."; a `notify_owner` item gives its title with an empty body; a device sign-in
   gives "A new sign-in to your HQ". `{Who}` is the helper's display name when the item names one, else
   "HQ".
2. **No answer in a push.** The payload never has `actions`, `tokens` or `item`. `answer_buttons()` and
   its token minting go. `PUSH_ANSWER_PATH` is removed from `lib.rs`; a request to it gets 404 (or 410
   with "This button is retired. Open HQ to answer." for one release, then 404).
3. **The worker shows what it is given.** `sw.ts` `push`: title, body, icon, badge, tag, `data.url`;
   no actions, no `requireInteraction`. `notificationclick` only opens; `answerFromPush` is deleted.
4. **Details are opt-in per subscription.** `push_subscriptions` gains `details INTEGER NOT NULL DEFAULT
   0` and `kinds TEXT NOT NULL DEFAULT 'needs,updates'` (new migration). Wave 0 adds the columns and the
   filter; the switches arrive in wave 1. With `details = 1` the body is the item's first line, cut at
   120 characters, never a command or JSON argument (the dialog alert's `command` is never used).
5. **Kinds before sending.** The dispatcher sends a Needs you item only to subscriptions whose `kinds`
   has `needs`, and an Updates item (a finished reply, a `notify_owner`) only to `updates`.
6. **Reply alerts are Updates.** `alertReplyDone()` shows "HQ answered" / "Tap to read." with tag
   `hq-chat-{id}` and `data.url` `/chat/{id}` (wave 2 routes; `/chat?thread={id}` until then). It ignores
   the reply text entirely.
7. **The proxy's upstream is checked every time it starts.** `Desktop::proxy_origin()` asks the pairing
   controller whether this app holds the hub web forward (`ForwardStatus.running_elsewhere == false`
   and the bridge running). If not, it returns an error `ForwardNotOurs` and starts nothing.
8. **No fallback.** When the config names a hub web forward, the proxy never falls back to this
   machine's own web server either. The window says the held-port line.
9. **A 401 from HQ wipes everything.** `noteAuth()` on a non-cached 401: clear the React Query cache
   (`queryClient.clear()`), remove the persisted key `hq-query-cache`, remove `hq-store` and
   `hq-thread-store`, the chat drafts, `hq-last-sync`, delete the `hq-api` cache, unsubscribe the push
   registration (`registration.pushManager.getSubscription()?.unsubscribe()`), drop the stored token,
   and post `{type: 'signed-out'}` on `BroadcastChannel('hq-auth')`. Then set `signInNeeded`.
10. **The worker refuses to answer from its cache when signed out.** The page posts a message to the
    worker on wipe; the worker deletes `hq-api` and keeps a flag in memory so it answers cached reads
    with 401 until the next successful sign-in.
11. **Keep what is this browser's own.** Not wiped: the install prompt flag, the clock zone, sidebar
    layout and other per-browser conveniences that hold no HQ content.
12. **The routes need the header.** `HEADER_GUARDED_ACTIONS` gains `drive`, `goal`, `unwatch` and
    `answer`; `missing_client_header()` also guards `/api/notifications/{id}/answer` and
    `/api/notifications/{id}/action`.

## 7. Browser and platform notes

- **Chrome on Android** ignores `actions` beyond two and shows the app name; with no actions it shows
  only the tap target. Desktop Chrome and Edge show the site or the installed app's name.
- **Safari (macOS and iOS 16.4+ from the Home Screen)** does not support notification actions at all, so
  today's buttons never showed there; the new payload changes only the words.
- **Firefox** shows actions on Windows and Linux; with none it shows the body only.
- An installed PWA with an old worker keeps showing whatever the server sends (rule 3 of the old
  worker only adds buttons when `tokens` exist), so the server change alone closes the leak.
- The `BroadcastChannel` wipe reaches other tabs of the same origin. A tab of the same origin signed in
  to a different HQ id is wiped too: one origin has one token today (wave 0 shared parts keys stores by
  HQ id, rule there).
- macOS: the HQ window in HQ Hub is a WKWebView on loopback; it never had push.

## 8. Code touchpoints

| File | Change | Exists |
|------|--------|--------|
| `crates/hq-web/src/web_push/mod.rs` | `payload(item, details)` builds the words per kind (rule 1), drops actions, tokens and the item id; delete `AnswerButtons` and `answer_buttons()`; the dispatcher filters by `kinds` (rule 5) and passes `details` per subscription (so one push per subscription, not one `broadcast`). Update the two tests at lines 448 and 486. | yes |
| `crates/hq-web/src/web_push/api.rs` | Test push words (line 119-120); subscribe takes optional `kinds` and `details`. | yes |
| `crates/hq-web/src/dialog_answer.rs` | Remove `PUSH_ANSWER_PATH` and `push_answer_handler`; keep `TOKEN_TTL_SECS` only if another caller uses it (none outside push today). | yes |
| `crates/hq-web/src/lib.rs` | Drop the route at line 374. | yes |
| `crates/hq-db/src/push_subscriptions.rs`, `crates/hq-db/sql/1xx_push_kinds.sql` | `kinds` and `details` columns. | file yes, migration NEW |
| `apps/hq-web/src/sw.ts` | `push` without actions (lines 159-178); delete `answerFromPush` (181-209) and the action branch (215-218); a `message` handler for the wipe (rule 10). | yes |
| `apps/hq-web/src/lib/replyAlerts.ts` | Fixed words (rule 6); no reply text. | yes |
| `apps/hq-web/src/lib/hqAuth.ts` | `noteAuth()` calls `wipeBrowser()` on a non-cached 401 (line 129). | yes |
| `apps/hq-web/src/lib/wipe.ts` | `wipeBrowser()`: every store in rule 9, plus the channel. | NEW |
| `apps/hq-web/src/lib/offlineCache.ts` | Export `clearSavedCopy()` used by `wipe.ts`. | yes |
| `apps/hq-web/src/components/SignInPrompt.tsx` | Show the empty view, never over cached content. | yes |
| `crates/hq-desktop/src/desktop.rs` | `proxy_origin()` refuses when the hub forward is not ours (rule 7); a typed error the window shows. | yes |
| `crates/hq-desktop/src/pairing.rs` | `hub_web_forward_ours() -> Option<bool>` from the latest `Forwards.status` (line 285, 501). | yes |
| `crates/hq-desktop/src/proxy.rs` | No change to forwarding; the refusal happens before it starts. | yes |
| `apps/desktop/ui/src/main.ts`, `ui/index.html` | The held-port card and the why line; Open HQ off. | yes |
| `crates/hq-web/src/origin.rs` | `HEADER_GUARDED_ACTIONS` plus `drive`, `goal`, `unwatch`, `answer`; the notifications answer and action routes; flip the test at lines 311-314. | yes |

## 9. Server, desktop or release changes

- Server: the payload, the kinds filter, the migration, retiring the push-answer route (all here).
- Desktop: the proxy refusal and its words. No new command; `host_status` and `pairing_snapshot` already
  carry what the window needs (`running_elsewhere` per forward).
- Release: the next web build ships the new worker. `docs/WEB_PUSH.md` says pushes carry no content and
  no buttons by default (listed in `../../changes.md`).

## 10. Acceptance checks (each fails on today's build)

1. `cargo test -p hq-web web_push`: a new test makes a blocked-session item with a question and the
   command `rm -rf /tmp/x` and asserts the payload has no `actions`, no `tokens`, no `item`, title "HQ
   needs your OK" (or the helper's name), body "Open HQ to see what it wants to do.", and that
   `rm -rf` appears nowhere in the bytes. Today the body is the question and command and there are two
   actions.
2. A `notify_owner` item with body "Your card ending 4242 was charged" pushes title only: the payload's
   `body` is "". Today it is the body.
3. A subscription with `details = 1` gets the item's first line and never the command; with
   `kinds = 'updates'` it gets no Needs you item. Today neither column exists.
4. `POST /api/notifications/push-answer` answers 404 (or 410). Today it answers the dialog.
5. `bun test`: `replyAlerts` shows "HQ answered" / "Tap to read." for a reply "secret plan". Today the
   body is "secret plan".
6. `cargo test -p hq-desktop`: with a config whose hub web forward is held by a listener the test opened
   first, `proxy_origin()` returns `ForwardNotOurs` and no request reaches that listener. Today the
   proxy starts and forwards `Authorization: Bearer t`.
7. `bun test`: after `noteAuth(new Response(null, {status: 401}))`, `idb-keyval` has no
   `hq-query-cache`, `hq-store` or `hq-thread-store`, localStorage has no `hq-web-token` or drafts, and
   `caches.has('hq-api')` is false. Today all remain.
8. The same with the header `x-hq-cache: 1` wipes nothing (guards rule C2). Passes today and must keep
   passing.
9. `cargo test -p hq-web origin`: `missing_client_header(POST, "/api/harness-sessions/hs-1/drive",
   false)` is true, and the same for `goal`, `unwatch`, `answer` and `/api/notifications/v1/answer`.
   Today the test at line 311 asserts the opposite.

## 11. Check on a laptop or phone

- Chrome on Android, installed: have a helper ask; the lock screen shows "Claude needs your OK" / "Open
  HQ to see what it wants to do." and Open only. Pull down the history: no command anywhere.
- Desktop Chrome and Firefox: the same, and a reply alert reads "HQ answered".
- Sign a laptop out from another device's Settings › Devices, then reload it offline (DevTools,
  Offline): nothing from HQ shows, only Sign in. Application › Storage shows no `hq-*` stores.
- macOS with HQ Hub: start `hq bridge serve` in a terminal so it holds the web forward, then open HQ Hub:
  Open HQ is off with the held-port line. Stop the terminal one, Look again, Open HQ works.

## 12. Open decisions it depends on

- **Who is "{Who}"?** Recommendation: the helper's display name when the item comes from a helper,
  "HQ" otherwise. No decision id; follows rules-shared 1.
- Depends on: rules-web 1, 3, 6, 10; `../../changes.md` (server and desktop rows marked "Web wave 0");
  `02-shared-parts.md` (stores keyed by HQ id refine rule 9); `../wave-1/05-sign-in-safety.md`
  (expiry, the switches in This browser).
