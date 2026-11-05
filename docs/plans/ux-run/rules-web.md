# Web rules for the UX fixes

resolutions.md settles any conflict with this file.

**Web app and HQ Hub. Extends `rules-shared.md`, which applies in full. The two files are one set of
rules (`resolutions.md` A1).**

Source: the "Web rules" board (AlignedWebRules) of the HQ Web & Desktop canvas, page 2 ("Aligned with
mobile"), written after the web and desktop adversarial review (AlignedReview) and decided with the
owner on 5 Nov 2026, then brought in line with `resolutions.md` section A. Every spec under `web/`
follows this file and `rules-shared.md`. The order of authority is: `resolutions.md`, then
`decisions.md`, then `rules-shared.md` and this file as one set (where they differ and
`resolutions.md` is silent, `rules-shared.md` wins), then the specs and boards (a later board over an
earlier one). Where a web spec and this file disagree, this file wins.

This file settles the review's blockers that are rules (WB1, WB2, WB5, WB6, WB7) and its missing shared
decisions (A to Q). Its title on the board: "One product on three screens: what the web and the hub do
the same, and what only they need."

Sections 1 to 10 are the board, in its order and words. Section 11 is the colour and component rules
as `aligned-ui.css` (the canvas's stylesheet, summarised in `web/README.md`) applies them.

---

## 1. Notifications on the web (WB2, decision A)

**Nothing private by default.**

- Web push uses the phone's words: **"Claude needs your OK"** · **"Open HQ to see what it wants to do."**
- Task approvals, stuck helpers and server questions use `resolutions.md` B19: **"A task needs your
  OK"** (never the task's name), **"A helper is stuck · Open HQ to see which one."**, **"Your server HQ
  needs your OK"**.
- Reply alerts: **"HQ answered · Tap to read."**
- `notify_owner` pushes its **title only**; the body stays in the Inbox.
- **Folding and replay** (`resolutions.md` C-11, as on the phone): past the newest five Needs you
  items one notice reads **"{N} more waiting for you"**; more than three Updates in an hour fold into
  **"{N} updates from HQ"**; when notifications come back on in this browser, waiting items post once.

**Open is the only button, always** (`resolutions.md` A4: buttons are safe first on every surface, and
the web and HQ Hub's system notices have "Open" alone). A browser cannot check a screen lock, so there
is no Allow or Decline in a web notification, with or without details. The 15-minute answer tokens in
pushes are retired.

**Per browser,** in Settings › This browser › Notifications: two switches, **"Needs you"** and
**"Updates"** (HQ filters by kind before sending), and **"Show details in notifications"**, off. With
details on, the body shows the item's own first line, never a command. This differs from the phone on
purpose (`resolutions.md` A5): the phone may show the request's text, the command included, only while
unlocked; a browser cannot check a screen lock.

**Test push:** "Notifications from HQ are on in this browser."

**The HQ window in HQ Hub** has no web push (`resolutions.md` B11, confirmed by the owner on 5 Nov
2026). Needs you items arrive as HQ Hub's system notice on that computer, with the same words, Open
only.

## 2. Connection states a browser can show (decision C)

| Status line | Colour | How the browser knows |
|-------------|--------|-----------------------|
| Signed out of this HQ | Amber | HQ refused the token. Everything saved is wiped (section 3). |
| You're offline | Amber | The browser says it has no network. |
| Your HQ can't be reached | Amber | No answer, or the hub's proxy says the upstream is gone. A browser cannot tell starting from stopped, so it never says "stopped". |
| Starting HQ · 12 s | Blue | A starting flag HQ returns while it opens (new), or HQ Hub, which owns the process. |
| Reconnecting | Blue | The chat socket is retrying. |
| The AI service isn't answering | Amber | The signed-in AI status route (D16). Send stays on. |
| Connected | Grey | Everything above is fine. The balance chip shows only here. |

- **The first true row wins.** The state comes first and wraps.
- **HQ Hub, which runs HQ,** also uses the phone's **"HQ has stopped"** (amber, stop icon),
  **"Restarting HQ"** (blue, `resolutions.md` C-12) and a grey **"You stopped HQ"** (decision P).
- **When HQ counts as stopped** (`resolutions.md` A7): the process is dead, or its start step has not
  moved for the longer of 60 s and twice its usual start time. No fixed timer. Same on phone and hub.
- **HQ Hub's Connection sheet** opens by itself at most once a day, and says **"This computer is
  almost full"** where the phone says "The phone is almost full" (C-12).

## 3. What a browser keeps (WB1, decision D)

- **Saved copy:** lists and notes already opened, kept until replaced, labelled **"Saved in this
  browser at 14:32"** (a copy from before today: "Saved in this browser on Tue 3 Nov"). One cap on phone
  and web (`resolutions.md` B16): **256 KB a place, 2 MB in all, per HQ.** Keyed by the HQ's id, so two
  HQs on one address never mix (the HQ window serves both this computer's HQ and a phone's).
- **The HQ window in HQ Hub** (B11, confirmed by the owner, 5 Nov 2026): its web group is **"This
  window"**. It keeps a saved copy and an outbox, keyed by the HQ's id, labelled **"Saved in HQ Hub at
  14:32"**.
- **Wiped at once** (the query cache, the worker's API cache, drafts, the outbox, the push
  subscription) when HQ refuses the token, on "Sign out this browser" and on Sign out everywhere.
  Signed out then shows only Sign in.
- **"This is a shared computer"** on the Sign in sheet: the sign-in lasts for this tab only, nothing
  is saved, no outbox, no push, signed out after **30 minutes idle**, and a warning before any export
  ("The file stays in this computer's Downloads").
- **Device sign-ins** name the browser, the system and when ("Chrome on Windows"). New sign-ins
  expire after **90 days without use**; the owner can keep one longer.
- **A new sign-in** is a Needs you item with **"It was me"** and **"Sign it out"** (WR10). The card's
  "Sign it out" is an ink trigger; only the dialog's action is red. The one-time code screen says
  **"Type this only into HQ on your own device. HQ never asks for it in a chat or a call."**
- **Signing out one device** (`resolutions.md` B3): title **'Sign out "{device}"?'** · body **"It can
  no longer open HQ until it signs in again."** · **"Keep signed in"** · red **"Sign out device"** ·
  done line **"{device} is signed out."**
- **Sign out everywhere** (B2): **"Sign out everywhere?"** · **"Every other device is signed out. This
  one stays signed in."** · **"Keep everyone signed in"** · red **"Sign out everywhere"**.

## 4. Outbox, Undo and edits (WB4, decisions E, F)

- **Waiting to send** (the phone's word): chat messages held **up to 24 h**, in order, each removable;
  new notes and tasks too. Every one carries a **client id** and HQ drops a repeat, so a retry never
  runs or costs twice. **Off on a shared computer.** It holds **at most 100 items**; the 101st shows
  **"Couldn't add this. 100 things are already waiting to send."** (WR30).
- **Undo restores on HQ.** Every Undo calls a restore route inside a **10-minute window** on HQ: chat
  archive, Inbox clear, a spending limit, task archive, removing a helper's connection (in HQ Hub,
  locally). A web toast may stay until closed (`resolutions.md` A6); after the window, or if the item
  changed on another device, or when an Undo that waited in the outbox reaches HQ late, it says
  **"Couldn't undo. It changed on another device, or too much time passed."**
- **Note edits never overwrite silently.** A save carries the version it started from; HQ answers
  "changed" when it moved on. The note shows **"This note changed somewhere else."** with **See their
  version** and **Keep both**.
- **A dropped link mid-reply** resumes by message id; Stop is also sent over HTTP so a lost socket
  never leaves a turn spending.

## 5. Names (M4)

| Thing | Its word |
|-------|----------|
| The web's own Settings group | This browser |
| A computer or phone | Its own name ("Mac mini", "Pixel 8"); the generic word is "this computer" |
| HQ Hub's first tab | This PC (the owner's choice, W44, owner to confirm). The tab name only (`resolutions.md` B12): everywhere else the machine's own name or "this computer" |
| A coding or general agent HQ runs | A helper, everywhere |
| HQ Hub's list of installed ones | Helpers, with a group "Apps that use HQ" for Claude Desktop |
| The edge worker's runs | HQ's own work, never a helper: "HQ is working on it" while working, "Done by HQ" when finished, in the Helpers group "HQ's own work" (`resolutions.md` B4) |
| The claude.ai connector | "Claude on the web"; "connector" only in Technical details (B14) |
| Telegram, Discord and the like | "Messaging apps" (B15) |
| Where the OS shows the app's name | AgentHQ (Android, the installed web app, the desktop bundle); windows inside it: "HQ Hub" and "HQ" |
| The phone, the hub | "your phone"; "HQ on your phone" when it is the hub |

In HQ Hub's HQ window the web's group is "This window" and its saved line "Saved in HQ Hub at 14:32"
(`resolutions.md` B11, which replaces the saved line in `decisions.md` WR5).

## 6. Approvals and helpers (WB5, WB6, WB7)

1. **An approval card quotes what is approved,** the tool and its arguments verbatim from HQ's runtime,
   with HQ's own description below, labelled **"In HQ's words"**.
2. **Words and order are the phone's:** Decline on the left, **"Allow once"** on the right. A request
   HQ's runtime marks as losing gets a red **"Allow {verb}"** ("Allow delete").
3. **"Let HQ say yes to its requests"** (was "Let HQ answer it"): off by default, a first-time
   explainer, every answer HQ gives is listed in Watching, never a losing request, and its routes need
   the HQ client header like every other changing route.
4. **Lifting "Stop every helper"** happens only in HQ Hub's This PC, on that computer. The web shows
   the lock and says where to lift it.
5. **Try it** never answers a trust prompt unless HQ already does for that helper (`spec.rs`);
   otherwise it reports **"Needs you on this computer"** (`resolutions.md` B12). It uses one fixed
   folder (`~/.hq/desktop/try-it`, WR23), turns off known self-updaters, and says what data it may use.
   It waits up to HQ's own launch limit, `agent_host.launch_bound_secs` (25 s by default,
   `crates/hq-tools/src/agent_host/mod.rs`, WR31).
6. **Helper checks** read versions from package files first; otherwise they run sandboxed, with no
   network and no input, two at a time, cached by path, size and date. A program's full path shows the
   first time it is seen. On Windows, `wsl.exe` by its full path and no `/mnt` paths.
7. **The hub's HQ window** proxies only to a forward the hub opened itself; if another program holds
   the port: **"Another program holds this port. HQ Hub won't send your sign-in there."**

8. **Stop HQ** in HQ Hub (`resolutions.md` B9) is red only when helpers are working: **"Stop HQ?"** ·
   the body names the helpers · **"Keep it running"** · red **"Stop HQ"**. With none working it is the
   Ask kind: **"Stop HQ?"** · **"Chats and scheduled tasks pause until you start it again."** · **"Keep
   it running"** · green **"Stop HQ"**.
9. **Stopping a helper** (B18) is word for word **"Stop Codex?"** · **"It stops now. Its work so far is
   kept."** · **"Keep it running"** · **"Stop Codex"**.
10. **The lockdown banner** on Helpers: **"All helpers are stopped. Allow them again in HQ Hub on that
    computer."** (same on the phone, C-5).

(The boards cite these as "AlignedWebRules 6.5" for Try it and "6.6" for helper checks.)

## 7. One route table and editions (M14, decision N)

- **One table** lists every web route and its phone link (`hq://`), with the old route each replaces.
  Old routes redirect forever, including links already stored in Inbox items, pushes and QR codes:
  `/vault#code=` keeps signing in. The table is `web/wave-2/01-route-table.md`.
- **Embedded mode `?embed=1`:** no rail, no top bar, no bottom bar, for the phone's web pages (Usage,
  Board, Timeline). Today's `?host=android` is accepted too (`decisions.md` WR1).
- **Lite** (Windows without WSL): only the places Lite serves (Notes, Tasks, Inbox); it starts on Notes
  and the status line says **"Connected · HQ Lite"** (WR3). HQ tells the web which edition it is.
  `resolutions.md` B13: the gear shows This browser and Other devices. A link to a place Lite lacks
  opens Notes with **"This HQ is HQ Lite. It has no {place}."** `/settings/spending` opens This browser
  with **"This HQ is HQ Lite. It has no Spending limits."** (WR28). Lite's Needs you never holds HQ's
  checks (Lite refuses the verification routes).
- **The route table marks phone-only Settings screens** (assistant, update, speech) (C-14).
- **The installed web app** is named **AgentHQ**, with shortcuts **Chat, Notes and Inbox**.
- **A newer HQ than the open page:** **"A newer HQ is running. Reload to keep going."** Drafts are kept.

## 8. Keyboard, access, print and charts (M13, decision Q)

- **Search** Ctrl K (Cmd K); **new note** Ctrl Alt N (Cmd Option N). **Browser zoom is never taken
  over;** text size lives in the note menu.
- **Enter sends** unless an input method is composing or the screen is touch-first; a setting
  **"Enter sends"**.
- **Esc closes sheets.** In a helper's terminal Esc is the helper's, only while **"Typing to the
  helper"** is on; **Ctrl ]** always leaves the terminal.
- **Every screen drawn at 320 px and at 200% zoom;** at 320 the top bar holds the title and the state,
  the rest move to a menu. **Toasts never cover the focused field.**
- **Forced colours keep borders;** notes and exports print in black on white.
- **Charts are data, not status:** blue, sky, white, grey, indigo, in that order, the second series
  dashed, labelled directly. **Never amber, red or green in a chart.** This is the one exception to
  "colour means one thing" (`resolutions.md` A3).

## 9. Data, time, words and proof (M10, M11, M15, decision O)

- **Data budget:** one socket carries changes (no polling loops); everything pauses when the tab is
  hidden; heavy readers (PDF, spreadsheets, Word, code colouring) load on demand; the browser's data
  saver is honoured. **Targets to measure: under 1 MB an hour idle, a Chat first load under 300 KB.**
  Data used is monthly on every surface (`resolutions.md` B17): This browser › Data used ("this
  month"); in the HQ window with the phone as hub, "Counted on your phone this month".
- **Time:** HQ sends instants; HQ has one time zone and each browser a display zone; when they differ
  the zone is named. Spending says when its day resets.
- **Words:** one catalogue of whole sentences builds the phone's resources and the web's strings (N19).
  Plurals and dates through the browser's own formatters (`Intl.PluralRules`, `Intl.DateTimeFormat`).
  **Word checks run on the web too** (`resolutions.md` C-13): over rendered text, a lint against
  literals, and at most 15 words a sentence.
- **Proof:** "What HQ noticed" under This browser › Technical details, counts only, never sent: Undo per
  kind, a message edited within 10 s of Enter, outbox repeats dropped, approvals per surface, data per
  hour, first-load time, Try it pass per helper. Same roll-back rules as the phone (`rules-shared.md`
  section 10: roll back when Undo is used on more than about 20% of a kind's actions).

## 10. Live bugs to fix before the redesign (ship on their own)

| Bug | Rules section | Where today |
|-----|---------------|-------------|
| Web push content and buttons | 1 | `crates/hq-web/src/web_push/mod.rs` payload, `apps/hq-web/src/sw.ts`, `apps/hq-web/src/lib/replyAlerts.ts`, `crates/hq-web/src/dialog_answer.rs` push tokens |
| The hub proxy and a taken port | 6 | `crates/hq-desktop/src/desktop.rs` `upstream_addr`, hq-bridge `from_config.rs` `start_forwards` |
| Wipe the browser on a refused sign-in | 3 | `apps/hq-web/src/lib/offlineCache.ts`, `sw.ts` `hq-api` cache, the 401 handler (`lib/hqAuth.ts`) |
| Guard the drive, goal and answer routes with the HQ client header | 6 | `crates/hq-web/src/origin.rs` |

Each with a test that fails on today's build. The spec is `web/wave-0/01-live-bugs.md`.

---

## 11. Colour and components as `aligned-ui.css` applies them

`aligned-ui.css` ends with a block headed "Shared rules alignment, 5 Nov 2026 (see AlignedWebRules)".
That block overrides the earlier component rules. Build from the overridden values; they are the ones
below. Token values are in `web/README.md` ("Design system reference").

### Colour means one thing (`rules-shared.md` section 3, on the web)

- **Grey (`--ink-muted` #D0D2D3):** fine, nothing to do. The default status dot (`.a-dot`) is grey. A
  healthy row, a paired device, a met check, "Done", "Finished", "Stopped" are ink or grey, never green.
- **Amber (`--warn` #FFB300) with an icon or a word:** needs you, waiting, stopped, late or overdue, a
  failure line, a field error, near or over a limit. `.a-dot.warn` and `.a-dot.danger` are both amber;
  `.a-danger-t` text is amber; `.a-trow-meta b.danger` (late) is amber; `.a-field.is-error` border and
  message are amber; `.a-att-item.is-error` is amber; `.a-kind.danger` and `.a-card.danger` are amber;
  `.a-bal.over` (over a limit) is amber; `.a-prog.over` is amber.
- **Blue (`--brand-blue` #00ADEE):** working, links, quiet buttons, the unread dot, progress while
  working, the 3 px focus ring. `.a-dot.work`, `.a-prog > u` (every progress bar) and `.a-work` are blue;
  `.a-banner.work` has a blue icon.
- **Green (`--brand-green` #22FF37):** only the one primary action that loses nothing (`.a-btn`,
  `.a-send`, `.a-fab`), the selected place icon in the rail and bar, the selected tab's underline, and
  the brand. Never a status, never a destructive button, never spending money or data.
- **Red (`--danger` #FF6B6B), black text:** only the fill of `.a-btn.danger`, the button that destroys
  or cuts off. Never a status, never text. `.a-btn.quiet.danger` and `.a-menu-item.danger` are **ink**:
  a trigger that leads to a red dialog is ink.

### Chosen and on states are ink, not green

The alignment block moves every "chosen" state off green:

| Component | Chosen or on state |
|-----------|--------------------|
| Filter chip `.a-chip.is-on` / `[aria-pressed="true"]` | ink fill, ink border, `--surface-000` text |
| Switch `.a-sw[aria-checked="true"]` | ink track and border, black knob |
| Check box `.a-chk[aria-checked="true"]` | ink box, black tick |
| Choice card `.a-rcard[aria-checked="true"]` | ink border, ink radio dot |
| Tab count on the selected tab `.a-tab.is-on em` | ink pill, `--surface-000` text |
| A tab count that needs you `.a-tab em.need`, `em.hub-need` | amber pill, black text |
| Inbox badge `.a-badge` | amber, black text: it counts what needs you (every needs-you count is amber, `resolutions.md` A3) |

Kept green: the selected tab's 3 px underline (`.a-tab.is-on::after`), the selected place's icon in the
rail and bar (`.a-rail-item.is-on .a-pill .a-ic`), the Chat mark's ring when Chat is open
(`.a-hqb` border), primary buttons, the send button and the floating button.

### Status line and top bar

- `.a-status` wraps (`white-space: normal`), and its span never cuts off with an ellipsis. The top bar
  grows to fit (`.a-top { height: auto; min-height: 64px }`).
- Below 360 px of app width the top bar hides `.a-find`, `.a-bal` and `.a-top-gear` and shows
  `.a-top-more`, one More menu named "Search, Spending and Settings" (`@container app (max-width:
  359px)`).
- The balance chip `.a-bal` shows only while Connected and reads "Today 85%" (`resolutions.md` B5);
  past a warning point `.a-bal.warn`, over a limit `.a-bal.over` ("Today 104%"), both amber;
  `.a-bal.wait` (dashed) is not used for a "checking" value, which is retired.

### Dialogs and toasts on the web

- `.a-dlg`: `--surface-200`, 1 px `--line` border, 28 px radius, 24 px padding, `min(420px, 100% -
  48px)` wide, title 22/28 Bold ink, body 14/20 `--ink-muted`. The phone's card is surface-100,
  hairline, 20 dp padding, 330 dp (rules-shared section 4); on the web the stylesheet's values apply.
- `.a-dlg-actions`: right-aligned, 8 px gap. Safe choice left (`.a-btn.secondary`), the action right
  (`.a-btn.danger` for a losing action, `.a-btn` for one that loses nothing). The two buttons stack,
  safe on top, when they do not fit side by side on one line: layout decides, not a zoom reading
  (`decisions.md` WR27; a flex row that wraps, never a zoom check). Esc, a click outside and Back
  choose the safe side.
- Spending money or data: two equal `.a-btn.secondary` buttons, the wait first, the size or cost in the
  label (D8).
- `.a-snack`: ink ground, `--surface-100` text, Undo in `--snack-action` #00668F (6.4:1), 6 px radius,
  48 px tall. Full width below 768; bottom right, 24 px in, `min(480px, 100% - 48px)` from 768. Copy is
  "{Thing} {done}" plus Undo, past tense, no "successfully". A toast that carries an action may stay
  until closed (`resolutions.md` A6), but Undo restores on HQ only inside the 10-minute window; after
  it, "Couldn't undo. It changed on another device, or too much time passed." A toast never covers the
  focused field: it moves above it, or waits until the field loses focus.

### Failure lines and the saved copy

- Every failure line is whole sentences from the one catalogue (`rules-shared.md` section 2, which is
  `resolutions.md` A2): "Couldn't {action}." + the reason sentence + the outcome sentence, the outcome
  chosen by whether the action waits and retries by itself. Amber with an icon. Codes only in Copy
  details.
- One saved-copy strip per place, `.a-stale`, under the status line, in states 2 to 4 of section 2:
  "Saved in this browser at 14:32" (a copy from before today names the day: "Saved in this browser on
  Tue 3 Nov"; in the HQ window, "Saved in HQ Hub at 14:32"). No row shows a "checking" value. Signed
  out shows nothing saved.
- **Backup on the web** (`resolutions.md` B10): the row reads "Last backup 3 h ago"; the card "Last
  backup 3 d ago. A new one should be made every 24 hours." It is amber only when HQ's backup status
  route reports failed or overdue and the page can start a backup; otherwise grey "Backups are set up
  where HQ runs."
- **Search results** bold the matched words, never a colour wash (A3).
- `.a-banner` is one line under the top bar, used only with the status line, never instead of it.

### Type, space, focus

- Clear Sans Regular and Bold, with italics for quotes. One title style: the top bar title is 18/24
  Bold (`.a-title`) on every place at every width, which retires the web's five page title styles.
  Upper case only for group labels (`.a-label`, `.a-grp`).
- Touch targets 48 px (`--target`); nothing interactive under 44 px.
- Every control shows a solid 3 px `--focus-ring` with a 3 px offset on keyboard focus. Fields turn
  their own border into a 2 px blue edge while typing.
- No shadows, no blur, no glass. Raised means one surface lighter; a sheet or dialog sits on a 62%
  black scrim (`--scrim`).
- Reduced motion stops the spinner and the working bar's sweep (`@media (prefers-reduced-motion:
  reduce)`); the working bar then shows a still half-opacity fill.

### Charts

Series take blue (#00ADEE), sky (#00C6FA), white (#FFFFFF), grey (#D0D2D3), indigo (#2E3191), in that
order. Indigo gets a 1 px white outline (2.0:1 on black). A line chart's second series is also dashed
and the legend names the line style. Every series is labelled where it ends. A value on hover or touch;
"The numbers" table under an expanded chart.
