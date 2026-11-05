# Web wave 2 · 01 · One route table, redirects for stored links and QR codes, the manifest (M14)

Boards: **AlignedFlow** ("One route table, shared with the phone", "Links that sign in, and how the
app is installed", "Surfaces that are not addresses", "Retired", "Back and switching places", the flow
map), **AlignedWebRules** section 7, AlignedReview M14 and decision N. Follows `../../rules-web.md`
section 7 and `../../rules-shared.md` section 8 (a link only opens a place). Where this spec and the
rules differ, the rules win.

## 1. Finding and the review items it closes

- **M14 Old addresses and versions.** Device QR codes, push and inbox links stored in the database,
  manifest shortcuts and name ("HQ Control Center"), the phone's web pages.
- **Decision N.** One route table lists every web route and its phone link, with the old route each
  replaces; old routes redirect forever.
- **W1 revised.** The web opens on Chat (Lite on Notes); the start screen stays light to load.
- **Drift 02** (part): Usage cannot be reached from a phone; it becomes Spending limits, reached from the
  balance chip and a Settings row.
- **Drift 12.** Home matches `/vault` exactly, so on a note neither the rail nor the bar shows where you
  are.

Who it hurts: anyone tapping a link HQ stored weeks ago (an Inbox item, a push, a printed QR code), and
anyone who installed the old app.

## 2. Before (as built, verified at `e5f9d5f`)

- `apps/hq-web/src/routes/index.tsx:5` `/` redirects to `/vault`.
- Routes: `/vault`, `/vault/$`, `/chat` (`?thread=`, `components/chat/ChatView.tsx:25`), `/tasks`
  (`?view=`, `?task=`, `routes/tasks.tsx:70`), `/sessions` (`?id=`, `?task=`, `routes/sessions.tsx:28`),
  `/usage`, `/notifications`, `/settings` (`#section`), `/setup`, `/connect`.
- Links HQ stores or sends: `crates/hq-cli/src/commands/start/daemon/tasks_periodic/session_supervisor/alerts.rs:359`
  `/sessions?id={id}` (helper alerts in the Inbox); `crates/hq-tools/src/harness_session/handoff.rs:101-102`
  `/chat?thread={t}` and `/tasks?task={id}`; `crates/hq-tools/src/ask.rs:357` `/chat?thread=`;
  `crates/hq-web/src/web_push/mod.rs:38` `DEFAULT_LINK = "/notifications"`; `web_push/api.rs:121` the
  test push's `/notifications`; reply alerts `/chat?thread=` (`apps/hq-web/src/lib/replyAlerts.ts:50`).
- Sign-in links: `apps/hq-web/src/lib/deviceSignIn.ts:33` builds `{origin}/vault#code={code}` for the QR
  code; `crates/hq-cli/src/commands/web.rs:305` `{url}/#token={t}` and `:823`
  `http://{host}:{port}/vault#token={token}`; `lib/hqAuth.ts:11-24` reads `#token=` (and an old
  `?token=`) and drops it.
- `apps/hq-web/public/manifest.json`: name "HQ Control Center", short name "HQ", description "Agent HQ
  monitoring dashboard", theme and background `#0a0a0f`, shortcuts Vault (`/vault`), Chat (`/chat`),
  Tasks (`/tasks`), Inbox (`/notifications`).
- `apps/hq-web/src/routes/__root.tsx:46` document title "Agent HQ Vault".
- `apps/hq-web/src/components/AppRail.tsx:32` Home matches `/vault` exactly (`exact`).
- The phone opens web pages with `?host=android` (`hq://web/{page}`: usage, board, timeline; home and
  vault open Notes, rules-shared 8).

## 3. Decision

**One route table, shared with the phone. Every old address redirects forever and keeps the item and
any `#fragment`. Each redirect has a test.** Search is an overlay with `?q=`; New note is a sheet; the
connection is the status line, never an address.

Rejected:

- **Drop old routes after a release or two.** Links to them are already stored in Inbox items, pushes,
  QR codes and login links, and the phone shipped with them.
- **Server-side redirects only.** The SPA's own router must redirect too, since the worker serves the
  shell offline and a saved bookmark opens without a server round trip.

## 4. Screens and states

No new screens: the table decides which place opens.

### The route table

| Web address | Phone link | Replaces (kept as a redirect) | Opens |
|-------------|------------|-------------------------------|-------|
| `/` | `hq://chat` | `/`, which went to `/vault` | Chat, the start screen. Lite opens Notes. |
| `/chat`, `/chat/:id` | `hq://chat`, `chat/{id}` | `/chat?thread={id}` (reply alerts, helper reports) | Chat, or that chat. Watching in the details panel. |
| `/chat?compose=1` | `hq://chat?compose=1` | new | Chat with the composer focused, never the microphone. |
| `/notes`, `/notes/$path` | `hq://notes/{path}`, and `hq://web/home` and `web/vault` open Notes too | `/vault`, `/vault/$path` | Notes home (Pinned, Recent), or that note. |
| `/notes/n/:id` | `hq://notes/n/{id}` (`../../resolutions.md` C-10) | new | A note by its id, the "Copy link" form. Opens only on your signed-in devices. |
| `/tasks`, `/tasks/:id` | `hq://tasks`, `tasks/{id}` | `/tasks?task={id}` (task reports, verification) | Tasks on Today, Upcoming or All, or that task in the detail pane. |
| `/tasks/away` | `hq://tasks/away` | the briefing on `/vault` and on `/tasks` | While you were away. |
| `/tasks?view=board`, `?view=timeline` | `hq://web/board`, `web/timeline`, opened with `?embed=1` | same | Board or Timeline, the view switch from 768. |
| `/helpers`, `/helpers/:id` | `hq://helpers`, `helpers/{id}` | `/sessions`, `/sessions?id={id}` (helper alerts in the Inbox) | Helpers, or that helper. |
| `/helpers?task=T-13` | `hq://helpers` | `/sessions?task=` | Helpers filtered to that task. |
| `/inbox`, `/inbox/:id`, `?tab=updates` | `hq://inbox`, `inbox/{id}` | `/notifications` (the default push link and a manifest shortcut today) | Inbox on Needs you, Updates or Done, or that item. |
| `?q=` on any place | `hq://search`, `search/notes` (never with a query) | the Search overlay | The one Search over notes, tasks, chats and helpers. The web address keeps `?q=` for back and forward only; no `hq://` link carries a query (`../../resolutions.md` B6). |
| `/settings/:section` | `hq://settings/{screen}` | `/settings#{section}` | Always the same section for the same address. The card slot sits on top of the Settings home. |
| `/settings/browser` | none: the web only | the install prompt, the header clock | This browser: Notifications, Sign out this browser, Time zone, Install and version, Data used. |
| `/settings/spending` | `hq://web/usage`, opened with `?embed=1` | `/usage` | Spending limits, under How HQ behaves. The balance chip opens it. |
| `/settings/devices` | `hq://settings/devices` | the device sign-in dialog | Other devices, and Sign in another device. |
| (phone only) | `hq://settings/assistant`, `settings/update`, `settings/speech` | none | Phone-only Settings screens (added by resolutions.md C-14). The web has no address for them; `/settings/assistant`, `/settings/update` and `/settings/speech` open the Settings home. |
| `/setup`, `/connect` | none | same | Full-screen covers with no shell. |
| `/signin` | none | the Sign in banner | Sign in, under "Signed out of this HQ". Nothing saved shows. |
| `/vault#code=…` | the phone scans the same QR | kept as is | Signs this browser in with the one-time code, then opens Chat. `apps/hq-web/src/lib/deviceSignIn.ts` builds it today. |
| `/#token=…`, `/vault#token=…` | none | kept as is | The login link from `hq web --json` (`crates/hq-cli/src/commands/web.rs`). Signs in, drops the token from the address bar, opens Chat. |
| Manifest | none | "HQ Control Center", with shortcuts Vault, Chat, Tasks, Inbox (`/notifications`) | Named AgentHQ, with three shortcuts: Chat (`/chat`), Notes (`/notes`), Inbox (`/inbox`). |
| A newer HQ than the open page | none | the new version prompt | A toast: "A newer HQ is running. Reload to keep going." Drafts are kept. |

Old `#hash` sections map: `#clock` › `/settings/time`, `#devices` › `/settings/devices`, `#chat-apps` ›
`/settings/messaging`, `#model`, `#backends`, `#keys` › `/settings/keys`, `#limits`, `#safety`,
`#agents`, `#integrations` › `/settings/safety`, `#connector` › `/settings/claude`,
`#session-folders` › `/settings/folders`. An unknown hash opens `/settings` (section paths beyond the
board's `/settings/browser`, `/settings/spending`, `/settings/devices` and `/settings/keys` are
Recommended in `../wave-1/06-settings-groups.md`).

### Surfaces that are not addresses

Search is an overlay with `?q=`. New note is a sheet, Ctrl Alt N (Cmd Option N), the old Quick note.
The note drawer over Chat is the note sheet. The reachability banner becomes the status line, one
saved-copy strip and the connection sheet. The six-second amber notice becomes a toast.

### Retired

The header clock (now This browser, Time zone), the header Overlay chat button (Chat is a place), the
Usage and Home rail items and the Workbench rail badge. The rail keeps the five places and one badge, on
Inbox.

### The places and the rail

Rail and bar order: **Notes, Tasks, Chat, Helpers, Inbox**, then Settings (the gear). Chat is the HQ
mark, raised in the bar's middle. Each place keeps where you were; choosing the place you are in returns
to its list and scrolls to the top. Browser Back goes up one level, then to the start place. A place
stays lit on every address under it (`/notes/...` lights Notes, Drift 12).

### How you arrive (the flow map)

| Arrival | Lands |
|---------|-------|
| Open HQ, any address with no place, a fresh tab, HQ Hub's Open HQ | Chat (Lite: Notes) |
| A notification | Web push has one button, Open. It opens the Inbox item, the chat, the helper or the task. |
| A link | `/notes/...`, `/tasks/T-12`, `/helpers/...`, an old address: redirected to its new home. |
| Ctrl K or the magnifier | The one Search, over notes, tasks, chats and helpers. |
| Signed out, first run, Claude asking | A full screen with no shell: Sign in, Set up HQ, Connect Claude. |

## 5. Exact copy

| Key | String |
|-----|--------|
| `manifest_name` | "AgentHQ" |
| `manifest_short_name` | "AgentHQ" (Recommended: the short name matches; the board names only `name`) |
| `manifest_shortcuts` | "Chat", "Notes", "Inbox" |
| `document_title` | "{Place} · HQ" ("Chat · HQ", "Notes · HQ"); Recommended, the board says only that the title changes |
| `newer_hq` | "A newer HQ is running. Reload to keep going." |
| `lite_no_place` | "This HQ is HQ Lite. It has no {place}." (`../../resolutions.md` B13): "It has no Chat.", "It has no Helpers.", and on `/settings/spending` "It has no Spending limits." (WR28) |

## 6. Behaviour rules

1. **A redirect keeps the item and the fragment.** `/chat?thread=abc#x` › `/chat/abc#x`;
   `/sessions?id=h-31` › `/helpers/h-31`; `/tasks?task=T-12` › `/tasks/T-12`; `/vault/Projects/HQ/Plan.md`
   › `/notes/Projects/HQ/Plan.md`; `/notifications` › `/inbox`; `/usage` › `/settings/spending`.
2. **Sign-in fragments are read before any redirect** (`hqAuth.ts` `adoptLinkToken()` already runs
   first, line 72): `/vault#code=` and `/vault#token=` and `/#token=` sign in, drop the fragment, then
   open Chat. These three addresses are never redirected away before the fragment is read.
3. **Both routers.** The SPA router redirects (TanStack Router `beforeLoad`), and the Rust server
   answers the same old paths with the SPA shell (it already serves the shell for any client route,
   `crates/hq-web/src/lib.rs:966` test). The service worker caches only the shell, so offline redirects
   work.
4. **A link only opens a place.** No address starts a download, an install or an answer (rules-shared
   8). `?compose=1` focuses the box; it never sends.
5. **HQ stops writing old links.** Server code writes the new forms (`/helpers/{id}`, `/chat/{id}`,
   `/tasks/{id}`, `/inbox/{id}`); `web_push` `DEFAULT_LINK` becomes `/inbox`. The redirects stay forever
   for what is already stored.
6. **`is_app_path`** (`web_push/mod.rs:263`) still refuses another origin; redirects never leave the app.
7. **The manifest** is AgentHQ with three shortcuts; `start_url` `/` (Chat), `scope` `/`. Theme and
   background `#000000` (surface-000).
8. **Lite** redirects `/chat*` and `/helpers*` to `/notes` with the toast, and `/settings/spending` to
   `/settings/browser` with "This HQ is HQ Lite. It has no Spending limits." (`02-embed-and-lite.md`,
   B13, WR28).
9. **Every redirect has a test** (section 10).
10. **Phone-only screens are marked (added by resolutions.md C-14).** `lib/routes.ts` carries a
    `phoneOnly: true` flag on `settings/assistant`, `settings/update` and `settings/speech`, so the
    phone's link table and the web's stay one table. The web never draws a row for them.
11. **No query in a phone link (B6).** `routes.ts` maps `?q=` to `hq://search` without the query; a
    test fails if any `hq://` value in the table has a `?q=`.

## 7. Browser and platform notes

- An installed PWA reads the manifest again on its own schedule: Chrome updates the name and shortcuts
  within a day or two of a new manifest; until then the old "HQ Control Center" may show. Shortcuts are
  shown by Chrome and Edge (right-click on the dock or taskbar icon, long press on Android); Safari and
  Firefox ignore them.
- Changing `start_url` or `scope` would make Chrome treat it as a new app; keep both `/`.
- QR codes already printed carry `/vault#code=`; a code lasts ten minutes, so the risk is only codes in
  use at the release, but the form stays forever as the phone builds it too.

## 8. Code touchpoints

| File | Change | Exists |
|------|--------|--------|
| `apps/hq-web/src/routes/index.tsx` | `/` opens Chat (Lite: Notes), not `/vault` (line 5). | yes |
| `apps/hq-web/src/routes/notes.tsx`, `notes/$.tsx`, `notes/n.$id.tsx` | Notes routes (move from `vault*`). | NEW |
| `apps/hq-web/src/routes/vault.tsx`, `vault/index.tsx`, `vault/$.tsx` | Become redirects (after reading sign-in fragments). | yes |
| `apps/hq-web/src/routes/chat.tsx`, `chat/$id.tsx` | `/chat/:id`; `?thread=` redirects. | yes, `$id` NEW |
| `apps/hq-web/src/routes/helpers.tsx`, `helpers/$id.tsx` | Helpers routes. | NEW |
| `apps/hq-web/src/routes/sessions.tsx` | Redirect to `/helpers` keeping `id` and `task`. | yes |
| `apps/hq-web/src/routes/inbox.tsx`, `inbox/$id.tsx` | Inbox routes. | NEW |
| `apps/hq-web/src/routes/notifications.tsx` | Redirect to `/inbox`. | yes |
| `apps/hq-web/src/routes/tasks.tsx`, `tasks/$id.tsx`, `tasks/away.tsx` | `/tasks/:id`, `/tasks/away`; `?task=` redirects. | yes, others NEW |
| `apps/hq-web/src/routes/usage.tsx` | Redirect to `/settings/spending`. | yes |
| `apps/hq-web/src/routes/settings.tsx` and `settings/$section.tsx` | Section paths; `#hash` redirects. | yes, `$section` NEW |
| `apps/hq-web/src/routes/signin.tsx` | The full sign-in page. | NEW |
| `apps/hq-web/src/lib/routes.ts` | The table as data, used by the redirects and their tests; phone links beside each. | NEW |
| `apps/hq-web/src/lib/routes.test.ts` | One test per row. | NEW |
| `apps/hq-web/public/manifest.json` | Name, shortcuts, colours. | yes |
| `apps/hq-web/src/routes/__root.tsx` | Document title (line 46); rail lit on every address under a place. | yes |
| `apps/hq-web/src/components/AppRail.tsx`, `BottomNav.tsx` | Five places in order, no `exact` on Notes (`AppRail.tsx:32`). | yes |
| `crates/hq-web/src/web_push/mod.rs` | `DEFAULT_LINK` `/inbox` (line 38). | yes |
| `crates/hq-cli/src/commands/start/daemon/tasks_periodic/session_supervisor/alerts.rs`, `crates/hq-tools/src/harness_session/handoff.rs`, `crates/hq-tools/src/ask.rs` | Write new link forms (lines 359, 101-102, 357). | yes |
| `apps/android/app/src/main/kotlin/example/hq/spike/` (link screen) | The same table on the phone (rules-shared 8). | phone spec |

## 9. Server, desktop or release changes

- Server: the new link forms (rule 5). No server redirect is needed (the shell is served for every path).
- Desktop: `open_main_window(path)` in wave 3 opens these addresses (`/helpers/{id}`, `/inbox/{id}`,
  `/settings/devices`).
- Release: the manifest name and shortcuts, the document title, redirects for links already stored, the
  install lines checked each release (`../../changes.md`, "Web app release").

## 10. Acceptance checks (each fails on today's build)

1. `bun test lib/routes.test.ts`: every "Replaces" address in the table lands on its new address with
   the same item id and fragment. Today `/sessions?id=h-31` stays on `/sessions`.
2. Opening `/vault#code=ABCDE-12345` exchanges the code and lands on `/chat`. Today it lands on `/vault`.
3. Opening `/` lands on `/chat` (on a full HQ). Today it lands on `/vault`.
4. `manifest.json` has `name: "AgentHQ"` and exactly three shortcuts with URLs `/chat`, `/notes`,
   `/inbox`. Today it is "HQ Control Center" with four.
5. On `/notes/Projects/HQ/Plan.md` the rail's Notes item has `aria-current="page"`. Today no item is lit
   on a note.
6. `cargo test`: the session alert link is `/helpers/{id}`; the push default link is `/inbox`. Today
   `/sessions?id=` and `/notifications`.
7. `bun test lib/routes.test.ts`: `settings/assistant`, `settings/update` and `settings/speech` are
   `phoneOnly`, and `/settings/assistant` lands on the Settings home (added by resolutions.md C-14);
   `/notes/n/:id` pairs with `hq://notes/n/{id}` (C-10); no phone link carries `?q=` (B6). Today the
   table does not exist.

## 11. Check on a laptop or phone

- Tap an old Inbox item (made before the release) that links to `/sessions?id=…`: it opens that helper.
- Scan a fresh device QR code with a phone camera: the phone browser signs in and opens Chat.
- Installed Chrome PWA: after a day the app's name is AgentHQ and the shortcuts are Chat, Notes, Inbox.

## 12. Open decisions it depends on

- **Decision N**, **W1**, **W2**, **WR1** to **WR4**, **WR28** (Recommended or Revised), and
  `../../resolutions.md` B6, B13, C-10, C-14.
- Depends on: rules-shared 8 (the phone's link table, kept in step), `02-embed-and-lite.md`,
  `../wave-1/06-settings-groups.md` (section paths), `../wave-4/*` (the places these routes open).
