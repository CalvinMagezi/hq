# Web wave 2 · 02 · Embedded mode `?embed=1` beside `?host=android`, and the Lite edition

Boards: **AlignedFlow** ("Embedded mode, ?embed=1", "Editions and the phone's pages", "Lite and the
gear", "A link to a place Lite lacks"), **AlignedShell2** section 5, **AlignedNotes2** section 4,
**AlignedNotesNote2** section 4, **AlignedSearch2** section 4, **AlignedInbox2** section 5,
**AlignedTasks2** section 5, **AlignedTasksViews** section 5, **AlignedHelpers2** section 6,
**AlignedSettings2** section 4, **AlignedSpending2** section 2, **AlignedChat2** section 1,
**AlignedFirstRun** note 6, AlignedReview ("Not drawn at all": Lite and the phone's web pages).
Follows `../../rules-web.md` section 7 and `../../decisions.md` WR1 to WR4. Where this spec and the
rules differ, the rules win; where boards disagree, see `../README.md` Open conflicts 1 and 2.

## 1. Finding and the review items it closes

- **Review "Not drawn at all": the Lite edition.** It refuses chat, helpers and settings, so W1 (open on
  Chat) breaks there.
- **Review "Not drawn at all": the phone's web pages,** which would show a second bottom bar inside the
  phone (needs an embedded mode).
- **Decision N, W1 revised, WR1 to WR4.**

Who it hurts: a Windows owner on Lite who lands on an error page; a phone owner who sees two bottom bars.

## 2. Before (as built, verified at `e5f9d5f`)

- `apps/hq-web/src/lib/embedHost.ts:4-36`: `?host=android` sets `sessionStorage['hq-embed-host'] =
  'android'` for the tab's life; `EMBED_HEAD_SCRIPT` (line 35) sets `data-host="android"` before first
  paint. `routes/__root.tsx:66-86` `useEmbedded()` hides the rail, header, bottom bar and overlays.
- No `?embed=1`.
- `crates/hq-web/src/profile_gate.rs:18-44` `LITE_API`: tasks, spaces, folders, initiatives,
  work-sessions, task-time-report, note, search, tree, vault, vault-status, vault-signals, vault-asset,
  pinned, pin, notifications, ws-ticket, auth device-code, exchange, devices, rotate. `:73`
  verification and `:81` activity are not served; `:88` `is_export` (`/api/note/pdf`, `/api/note/export`)
  not served. Everything else under `/api` answers the same JSON 404 as a missing path
  (`docs/HQ_LITE.md`).
- `crates/hq-core/src/config/profile.rs:130` `lite.allow_egress`: nothing leaves Lite unless listed.
- The web app has no idea it runs on Lite: no route or flag tells it (`grep -i lite apps/hq-web/src`
  finds nothing relevant). `/` goes to `/vault`; Chat, the Workbench, Usage and Settings render and fail
  with errors.
- `crates/hq-web/src/vault_api.rs` hides folders starting with `_` or `.` under Lite (`lite` flags at
  `api.rs:82, 262, 275, 280`).

## 3. Decision

**Accept both `?embed=1` and `?host=android` (WR1). HQ tells the web its edition (new, in `/api/hello`),
and the shell shows only the places that edition serves.**

| Edition | Places | Opens on | Status line while connected | Chip |
|---------|--------|----------|-----------------------------|------|
| Full · desktop | Notes, Tasks, Chat, Helpers, Inbox | Chat | "Connected" (· the place's fact) | while Connected, with a balance or limit |
| Lite · desktop | Notes, Tasks, Inbox | Notes | "Connected · HQ Lite" (WR3) | none: Lite runs no AI service |
| Embedded · `?embed=1` | the one page asked for | that page | none: the phone draws its own bars | none |

Rejected:

- **Show Lite's missing places, disabled.** Five places of which two never work teach nothing.
- **A page "Chat needs the full HQ" for `/chat` on Lite** (AlignedChat2). WR4 chose a redirect to Notes
  with a toast, so a stored link lands somewhere useful.
- **Drop `?host=android`.** Phones already shipped use it.

## 4. Screens and states

### Embedded

- **E1 · The phone's Usage page** (`/settings/spending?embed=1`, from `hq://web/usage`): no rail, no top
  bar, no bottom bar. The page's own content only: limits, forecast, what used the most, by AI service.
  The phone draws "Usage" / "Web page · spend and forecast" around it.
- **E2 · The phone's Board and Timeline** (`/tasks?view=board&embed=1`, `view=timeline`): the board's
  columns, one at a time with jump chips at the phone's width; Timeline with the names column. Links
  inside open the phone's own screens, not a second web app (`hq://tasks/{id}` through the phone's link
  screen).
- **E3 · Signed out inside embedded mode.** The phone's sign-in is required; the page shows one line
  "Sign in to HQ on your phone first." with no shell (Recommended; not drawn on the boards).

### Lite

- **L1 · Notes home** (Shell 2 frame 5, Notes 2 frame 4). Rail: Notes, Tasks, Inbox, then Settings. No
  raised Chat button. Top bar "Notes" / "Connected · HQ Lite" / Search field "Search notes and tasks" /
  no chip. Notes home: "Your notes", New note, Ctrl Alt N, the search pill "Search your notes", folder
  chips (Projects, Daily, Logs), Pinned, Recent. No suggested pins, no "by HQ" trails.
- **L2 · No notes yet.** "Your notes will show up here" / "Write your first one." / [New note]. No "Ask HQ
  to write one".
- **L3 · A note.** Edit, Make a task, Pin; no Ask HQ, no Export (Lite runs no chats and does not serve
  the export routes, `profile_gate.rs` `is_export`). Who wrote it: only "You" or "Not known". Copy note
  text stays: "It is the way to take a note out of Lite. "hq lite export" copies the whole folder."
  (`crates/hq-cli/src/commands/lite.rs:260`).
- **L4 · Search.** Scopes All, Notes and Tasks; the field "Search notes and tasks"; "Lite finds the words
  you type. It does not search by meaning."; no Ask HQ; folders starting with `_` or `.` never searched.
  Failure: "Couldn't search your notes. Your HQ can't be reached. It will try again by itself."
- **L5 · Tasks.** "HQ Lite · 9 active". The same lists, without "Working on it now" and without Check it
  from HQ (Lite runs housekeeping only, not the checking of finished work, `docs/HQ_LITE.md`). A task
  moved to Ready to check by hand still shows there. Board and Timeline work, without "Codex is
  working".
- **L6 · Inbox.** Needs you holds tasks to check and new sign-ins only. Left out: helper questions, chat
  approvals and the stuck-helper card. Updates are HQ's own housekeeping. No notifications card unless
  `lite.allow_egress` lists the push service ("A web push goes through the browser maker's push service,
  and Lite sends nothing to another service unless it is listed (HQ_LITE.md). Listing it in
  lite.allow_egress would turn the card on.").
- **L7 · Settings (the gear).** This browser and Other devices only (WR2): This browser without
  Notifications (unless allowed), Time zone, Install and version, Data used, Sign out this browser; Other
  devices with the code card. One line: "HQ Lite has no other settings here. Keys, spending, messaging
  apps and helpers are not part of HQ Lite." No Spending: no chip, no Spending row, and
  `/settings/spending` opens `/settings/browser` (OD-W2).
- **L8 · A link to a place Lite lacks.** `/chat`, `/chat/:id`, `/helpers*`: Notes opens with the toast
  "This HQ is HQ Lite. It has no Chat." (or "… It has no Helpers."). The address is not an error page.
- **L9 · First run.** Lite serves no `/setup` (`docs/HQ_LITE.md`), so that page never shows there.
- **L10 · Other states.** Every other status line state reads as on the full edition (Starting HQ · 12 s,
  Your HQ can't be reached, You're offline, Signed out of this HQ). "The AI service isn't answering"
  never shows.
- **Widths.** 390 (Lite on a phone browser): the bottom bar with Notes, Tasks, Inbox, no raised button;
  the gear in the top bar. 768 to 2200: the rail with three places. 320 and 200%: as everywhere.
- **HQ Hub and Lite.** HQ Hub is not part of Lite (Hub 3: "Lite (Windows without WSL2) serves no
  helpers, no host and no pairing, so it has nothing for HQ Hub to run"). Lite opens in the browser and
  starts on Notes.

## 5. Exact copy

| Key | String |
|-----|--------|
| `state_lite` | "Connected · HQ Lite" |
| `lite_no_chat` | "This HQ is HQ Lite. It has no Chat." |
| `lite_no_helpers` | "This HQ is HQ Lite. It has no Helpers." |
| `lite_settings_line` | "HQ Lite has no other settings here. Keys, spending, messaging apps and helpers are not part of HQ Lite." |
| `lite_search_field` | "Search notes and tasks" |
| `lite_search_note` | "Lite finds the words you type. It does not search by meaning." |
| `lite_notes_empty` | "Your notes will show up here" · "Write your first one." · "New note" |
| `lite_copy_text_note` | "It is the way to take a note out of Lite. "hq lite export" copies the whole folder." |
| `fail_search_lite_unreachable` | "Couldn't search your notes. Your HQ can't be reached. It will try again by itself." |
| `embed_signed_out` | Recommended: "Sign in to HQ on your phone first." |

## 6. Behaviour rules

1. **Embedded is per tab.** `?embed=1` or `?host=android` sets `sessionStorage['hq-embed'] = '1'` and
   `data-embed="1"` before first paint (extend `EMBED_HEAD_SCRIPT`); in-app navigation keeps it.
2. **Embedded hides all chrome:** rail, top bar, bottom bar, Search overlay, New note sheet, the chat
   overlay and toasts that point outside the page. The status line's state still decides what the page
   shows (saved strip, actions off), drawn inside the page.
3. **Links in embedded mode** that lead to another place navigate the top window to `hq://{place}/{id}`
   when the page runs inside the phone app (the phone intercepts `hq://`), and to the web route
   otherwise.
4. **The edition comes from `/api/hello`** (`edition: "full" | "lite" | "mobile"`). Until it answers, the
   shell draws the last known edition from the saved copy; on a first visit it draws Notes, Tasks and
   Inbox only (safe on both).
5. **Lite's places.** The rail and bar show Notes, Tasks, Inbox; `/` opens Notes; Back ends at Notes.
6. **Lite's redirects.** `/chat*` and `/helpers*` (and the old `/sessions*`) go to `/notes` with the
   toast; `/settings/*` other than `browser`, `time`, `version`, `data` and `devices` go to
   `/settings/browser`.
7. **Lite hides what it cannot serve,** never shows it disabled: Ask HQ, Export, suggested pins, "by HQ",
   Working on it now, Check it from HQ, Spending, helper questions, the chip.
8. **Lite never asks for notifications** unless HQ reports that `lite.allow_egress` lists the push
   service (`/api/hello` `push_allowed`).
9. **Mobile edition** (`profile: mobile`, Android): the phone serves the web pages it embeds; a browser
   that opens it gets the full places the phone's HQ serves (the phone as the hub, rules-web 5).

## 7. Browser and platform notes

- Lite runs on Windows without WSL; the browser is usually Edge or Chrome on Windows. Forced colours
  (Windows contrast themes) are more common there: check L1 to L7 with one on.
- The phone's WebView (Android System WebView) runs embedded pages; `sessionStorage` lasts for the
  WebView's life, which the phone keeps per screen.

## 8. Code touchpoints

| File | Change | Exists |
|------|--------|--------|
| `apps/hq-web/src/lib/embedHost.ts` | Accept `embed=1` and `host=android`; one flag. | yes |
| `apps/hq-web/src/routes/__root.tsx` | `useEmbedded()` (66) reads the one flag; edition from `useHello()`. | yes |
| `apps/hq-web/src/lib/edition.ts` | `useEdition()`, the places per edition, Lite redirects. | NEW |
| `apps/hq-web/src/components/AppRail.tsx`, `BottomNav.tsx` | Places per edition. | yes |
| `apps/hq-web/src/lib/routes.ts` | Lite rows in the table. | NEW (wave 2 01) |
| `crates/hq-web/src/api.rs` | `/api/hello` `edition`, `push_allowed`; allow `/api/hello` in `LITE_API`. | NEW route |
| `crates/hq-web/src/profile_gate.rs` | Add `/api/hello` to `LITE_API` (18-44). | yes |

## 9. Server, desktop or release changes

Server: the edition in `/api/hello` (`../../changes.md` "Which edition is running"). Phone: the phone's
web page links use `?embed=1` from its next release (`../../mobile/wave-4/03-web-pages.md`).

## 10. Acceptance checks (each fails on today's build)

1. `?embed=1` hides the rail, top bar and bottom bar before first paint (the head script sets the
   attribute). Today only `?host=android` does.
2. With `/api/hello` answering `edition: "lite"`, the rail has exactly Notes, Tasks, Inbox and `/` lands
   on `/notes`. Today Lite is not known.
3. On Lite, `/chat` lands on `/notes` with the toast "This HQ is HQ Lite. It has no Chat." Today it shows
   the chat and fails.
4. On Lite the status line in Connected reads "Connected · HQ Lite" and there is no chip.
5. On Lite a note has no Ask HQ and no Export. Today both show and fail.

## 11. Check on a laptop or phone

- Windows without WSL: install Lite (`docs/WINDOWS.md`), open the web app in Edge: three places, Notes
  first, "Connected · HQ Lite".
- The phone app's Usage, Board and Timeline pages: one set of bars, the phone's own.

## 12. Open decisions it depends on

- **WR1, WR2, WR3, WR4** (Recommended). **OD-W2** (`../README.md`): Lite's `/settings/spending`.
- Depends on: `../wave-0/02-shared-parts.md` (`/api/hello`), `01-route-table.md`, the place specs in
  wave 4 (each lists its Lite frame).
