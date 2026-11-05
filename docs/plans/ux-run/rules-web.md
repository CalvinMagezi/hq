# Web rules for the UX fixes

**Web app and HQ Hub. Extends `rules-shared.md`, which applies in full.**

Source: the "Web rules" board (AlignedWebRules) of the HQ Web & Desktop canvas, page 2 ("Aligned with
mobile"), written after the web and desktop adversarial review (AlignedReview) and decided with the
owner on 5 Nov 2026. Every spec under `web/` follows this file and `rules-shared.md`. The order of
authority is: `decisions.md`, then this file, then `rules-shared.md`, then the boards (a later board
over an earlier one). Where a web spec and this file disagree, this file wins.

This file settles the review's blockers that are rules (WB1, WB2, WB5, WB6, WB7) and its missing shared
decisions (A to Q). Its title on the board: "One product on three screens: what the web and the hub do
the same, and what only they need."

Sections 1 to 10 are the board, in its order and words. Section 11 is the colour and component rules
as `aligned-ui.css` (the canvas's stylesheet, summarised in `web/README.md`) applies them.

---

## 1. Notifications on the web (WB2, decision A)

**Nothing private by default.**

- Web push uses the phone's words: **"Claude needs your OK"** · **"Open HQ to see what it wants to do."**
- Reply alerts: **"HQ answered · Tap to read."**
- `notify_owner` pushes its **title only**; the body stays in the Inbox.

**Open is the only button, always.** A browser cannot check a screen lock, so there is no Allow or
Decline in a web notification, with or without details. The 15-minute answer tokens in pushes are
retired.

**Per browser,** in Settings › This browser › Notifications: two switches, **"Needs you"** and
**"Updates"** (HQ filters by kind before sending), and **"Show details in notifications"**, off. With
details on, the body shows the item's own first line, never a command.

**Test push:** "Notifications from HQ are on in this browser."

**The HQ window in HQ Hub** has no web push. HQ Hub posts a system notice for Needs you items on that
computer, with the same words, Open only.

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
- **HQ Hub, which runs HQ,** also uses the phone's **"HQ has stopped"** (amber, stop icon) and a grey
  **"You stopped HQ"** (decision P).
- **Stopped is judged by progress, never by a 60 s timer.** (The phone's rule, `rules-shared.md`
  section 6: stopped only when the process is dead, or its start step has not moved for the longer of
  60 s and twice the usual start time.)

## 3. What a browser keeps (WB1, decision D)

- **Saved copy:** lists and notes already opened, **at most 5 MB in all**, kept until replaced,
  labelled **"Saved in this browser at 14:32"**. Keyed by the HQ's id, so two HQs on one address never
  mix (the HQ window serves both this computer's HQ and a phone's).
- **Wiped at once** (the query cache, the worker's API cache, drafts, the outbox, the push
  subscription) when HQ refuses the token, on "Sign out this browser" and on Sign out everywhere.
  Signed out then shows only Sign in.
- **"This is a shared computer"** on the Sign in sheet: the sign-in lasts for this tab only, nothing
  is saved, no outbox, no push, signed out after **30 minutes idle**, and a warning before any export
  ("The file stays in this computer's Downloads").
- **Device sign-ins** name the browser, the system and when. New sign-ins expire after **90 days
  without use**; the owner can keep one longer.
- **A new sign-in** is a Needs you item with Sign it out. The one-time code screen says **"Type this
  only into HQ on your own device. HQ never asks for it in a chat or a call."**

## 4. Outbox, Undo and edits (WB4, decisions E, F)

- **Waiting to send** (the phone's word): chat messages held **up to 24 h**, in order, each removable;
  new notes and tasks too. Every one carries a **client id** and HQ drops a repeat, so a retry never
  runs or costs twice. **Off on a shared computer.**
- **Undo restores on HQ.** Every Undo calls a restore route inside a **10-minute window** on HQ: chat
  archive, Inbox clear, a spending limit, task archive, removing a helper's connection (in HQ Hub,
  locally). A web toast may stay until closed; after the window, or if the item changed on another
  device, it says **"Couldn't undo. It changed on another device."**
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
| HQ Hub's first tab | This PC (the owner's choice; reviewers suggested "This computer") |
| A coding or general agent HQ runs | A helper, everywhere |
| HQ Hub's list of installed ones | Helpers, with a group "Apps that use HQ" for Claude Desktop |
| The edge worker's runs | HQ's own work ("Done by HQ"), never a helper |
| Where the OS shows the app's name | AgentHQ (Android, the installed web app, the desktop bundle); windows inside it: "HQ Hub" and "HQ" |
| The phone, the hub | "your phone"; "HQ on your phone" when it is the hub |

In HQ Hub's HQ window the web's group is "This window" and its saved line "Saved on this PC at 14:32"
(`decisions.md` WR5).

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
   otherwise it reports **"Needs you on this PC"**. It uses one fixed folder, turns off known
   self-updaters, and says what data it may use.
6. **Helper checks** read versions from package files first; otherwise they run sandboxed, with no
   network and no input, two at a time, cached by path, size and date. A program's full path shows the
   first time it is seen. On Windows, `wsl.exe` by its full path and no `/mnt` paths.
7. **The hub's HQ window** proxies only to a forward the hub opened itself; if another program holds
   the port: **"Another program holds this port. HQ Hub won't send your sign-in there."**

(The boards cite these as "AlignedWebRules 6.5" for Try it and "6.6" for helper checks.)

## 7. One route table and editions (M14, decision N)

- **One table** lists every web route and its phone link (`hq://`), with the old route each replaces.
  Old routes redirect forever, including links already stored in Inbox items, pushes and QR codes:
  `/vault#code=` keeps signing in. The table is `web/wave-2/01-route-table.md`.
- **Embedded mode `?embed=1`:** no rail, no top bar, no bottom bar, for the phone's web pages (Usage,
  Board, Timeline). Today's `?host=android` is accepted too (`decisions.md` WR1).
- **Lite** (Windows without WSL): only the places Lite serves (Notes, Tasks, Inbox); it starts on Notes
  and the status line says **"HQ Lite"** ("Connected · HQ Lite", WR3). HQ tells the web which edition
  it is.
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
- **Charts are data, not status:** blue, sky, white, grey, indigo, in that order, labelled directly.
  **Never amber, red or green in a chart.**

## 9. Data, time, words and proof (M10, M11, M15, decision O)

- **Data budget:** one socket carries changes (no polling loops); everything pauses when the tab is
  hidden; heavy readers (PDF, spreadsheets, Word, code colouring) load on demand; the browser's data
  saver is honoured. **Targets to measure: under 1 MB an hour idle, a Chat first load under 300 KB.**
  "Data used" shows under This browser. When the phone is the hub, the phone's data is counted.
- **Time:** HQ sends instants; HQ has one time zone and each browser a display zone; when they differ
  the zone is named. Spending says when its day resets.
- **Words:** one catalogue of whole sentences builds the phone's resources and the web's strings (N19).
  Plurals and dates through the browser's own formatters (`Intl.PluralRules`, `Intl.DateTimeFormat`).
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
| Inbox badge `.a-badge` | amber, black text: it counts what needs you |

Kept green: the selected tab's 3 px underline (`.a-tab.is-on::after`), the selected place's icon in the
rail and bar (`.a-rail-item.is-on .a-pill .a-ic`), the Chat mark's ring when Chat is open
(`.a-hqb` border), primary buttons, the send button and the floating button.

### Status line and top bar

- `.a-status` wraps (`white-space: normal`), and its span never cuts off with an ellipsis. The top bar
  grows to fit (`.a-top { height: auto; min-height: 64px }`).
- Below 360 px of app width the top bar hides `.a-find`, `.a-bal` and `.a-top-gear` and shows
  `.a-top-more`, one More menu named "Search, Spending and Settings" (`@container app (max-width:
  359px)`).
- The balance chip `.a-bal` shows only while Connected; past a warning point `.a-bal.warn`, over a limit
  `.a-bal.over` (both amber); `.a-bal.wait` (dashed) is not used for "Checking", which is retired.

### Dialogs and toasts on the web

- `.a-dlg`: `--surface-200`, 1 px `--line` border, 28 px radius, 24 px padding, `min(420px, 100% -
  48px)` wide, title 22/28 Bold ink, body 14/20 `--ink-muted`. The phone's card is surface-100,
  hairline, 20 dp padding, 330 dp (rules-shared section 4); on the web the stylesheet's values apply.
- `.a-dlg-actions`: right-aligned, 8 px gap. Safe choice left (`.a-btn.secondary`), the action right
  (`.a-btn.danger` for a losing action, `.a-btn` for one that loses nothing). Stacked, safe on top, when
  a label passes 18 characters or text is at 200% (`rules-shared.md` section 4; at 200% browser zoom
  the layout is the phone layout, so the stacking rule fires there). Esc, a click outside and Back
  choose the safe side.
- Spending money or data: two equal `.a-btn.secondary` buttons, the wait first, the size or cost in the
  label (D8).
- `.a-snack`: ink ground, `--surface-100` text, Undo in `--snack-action` #00668F (6.4:1), 6 px radius,
  48 px tall. Full width below 768; bottom right, 24 px in, `min(480px, 100% - 48px)` from 768. Copy is
  "{Thing} {done}" plus Undo, past tense, no "successfully". A toast that carries an action may stay
  until closed (W23), but Undo restores on HQ only inside the 10-minute window. A toast never covers the
  focused field: it moves above it, or waits until the field loses focus.

### Failure lines and the saved copy

- Every failure line is two whole sentences from the catalogue (`rules-shared.md` section 2, plus the
  web reasons in `web/wave-0/02-shared-parts.md`): "Couldn't {action}." then the reason. Amber with an
  icon. Codes only in Copy details.
- One saved-copy strip per place, `.a-stale`, under the status line, in states 2 to 4 of section 2:
  "Saved in this browser at 14:32" (a copy from before today names the day: "Saved in this browser at
  Tue 3 Nov"). No row says "Checking…". Signed out shows nothing saved.
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
