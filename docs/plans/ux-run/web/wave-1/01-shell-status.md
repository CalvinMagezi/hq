# Web wave 1 · 01 · The status line and colours across today's shell (M1, M2)

Boards: **AlignedFoundations** (Colour, Connection states, Type), **AlignedFoundations2** (StatePill,
badge, the phone-to-web class table), **AlignedShell** section 4 ("The top bar's states") and
**AlignedShell2** section 3 (320 px), with Drift 03, 13, 14 and 15. Follows `../../rules-web.md`
sections 2 and 11 and `../../rules-shared.md` section 3. Where this spec and the rules differ, the
rules win.

Wave 1 applies the rules across the app as it is today, before the place redesigns of wave 4. This spec
changes the shell's header, banners, badges and every status colour. It does not move places or routes
(wave 2) and does not redraw any place (wave 4).

## 1. Finding and the review items it closes

- **M1 Status words and order.** "Live", "Saved · 3 min", "Can't reach HQ"; the place's fact before the
  state; the state cut off by an ellipsis.
- **M2 Colour.** Red for late, errors, over budget and quiet triggers; green for Live, done, paired,
  within the limit, task bars and chosen cards; amber used as a chart series.
- **W03 (Drift 03), five colours for waiting.** A blocked agent is rose text in the list
  (`AGENT_STATUS_CLASS`), an amber callout, a blue-bordered card in the details panel, a blue rail badge
  and a green phone-bar badge. The Watching panel says "blocked" in rose.
- **Drift 13.** Signed out or offline, sections show raw errors ("Could not load pinned:
  unauthorized" with a Retry that cannot work); the unreachable banner assumes Tailscale.
- **Drift 15.** `--text-dim #6b6b8a` on black is about 4.1:1, neutral-500 about 4.4:1; Home cards use 8
  to 9 px mono text, some at 55% opacity.
- **Drift 14** (part): the header chat button is labelled "Overlay".

Who it hurts: the owner scanning for what needs them, on every screen, every visit. The one cue that
matters most is the least consistent.

## 2. Before (as built, verified at `e5f9d5f`)

- `apps/hq-web/src/routes/__root.tsx:295-378` the header: `HeaderClock` (310), search (318, "Search
  vault (Cmd+/)"), the chat button "Overlay" (328-331), `CopilotChip` (339), the gear (344), the dot
  with "live", "saved" or "offline" and an age (353-369, `SavedDataLabel` at 389).
- `__root.tsx:249-292` banners above the header: `InstallPrompt`, `SignInPrompt`, `ReachabilityBanner`,
  a system notice (six seconds, amber), the new-version prompt.
- `__root.tsx:113` `CopilotChip`: shows a budget, Copilot or OpenRouter label whenever one exists
  (line 30-32), whatever the connection.
- `apps/hq-web/src/components/chat/SessionsPanel.tsx:9-14` `AGENT_STATUS_CLASS`: idle neutral-400,
  working blue-400, blocked rose-400, done emerald-400; line 144 prints the raw `agent_status`
  ("blocked", "done").
- `apps/hq-web/src/components/sessions/SessionRow.tsx:7` uses the same map in the Workbench list.
- `apps/hq-web/src/components/AppRail.tsx:43,50` `hq-rail-badge` on Workbench and Inbox
  (`apps/hq-web/app.css:1093`).
- `apps/hq-web/src/components/BottomNav.tsx:74-75` the needs-you badge on `var(--accent-green)` with
  black text, 9 px.
- `apps/hq-web/src/components/SignalCard.tsx:34,38,54,61` 8 and 9 px mono text in `--text-dim`, one at
  55% opacity.
- `apps/hq-web/app.css:29` `--text-dim: #6b6b8a`; `:31` `--font-body: 'Inter', 'Clear Sans'`.
- Rose and emerald elsewhere: `components/tasks/TaskDetailDrawer.tsx:181` (delete hover rose), `:197-206`
  (a rose confirm strip), `:261` (emerald chosen chip), `components/tasks/verification/
  TaskVerification.tsx:20` (`pass` emerald), `AuditSheet.tsx:14-15` (met emerald, not met rose),
  `verification/Sheet.tsx:58` (`SHEET_PRIMARY` emerald fill, fine: it is a primary button).

### Before frames

- **B1 · Header, 1440.** "14:32" clock, search, "Overlay", "Copilot 912 left", gear, green dot "live".
- **B2 · Header, saved.** Amber dot "saved · 3 min".
- **B3 · Rail.** Workbench badge blue, Inbox badge blue; phone bar badge green.
- **B4 · Unreachable.** Banner "Can't reach your HQ" with Tailscale steps over Home, which shows "Could
  not load pinned: unauthorized" with Retry.

## 3. Decision

**The status line is the only place HQ's connection is reported, in the rules' words and order, and
colour means one thing each, on every screen of today's app.**

- Replace the header's dot and word with the status line (`../wave-0/02-shared-parts.md`'s Connection module),
  under the place's title, state first, wrapping.
- Replace `ReachabilityBanner` and the per-section raw errors with the saved strip and the connection
  sheet.
- Map every status colour to the rules: grey fine, amber needs you or failed, blue working, green only
  on the primary action, red only on a destroying button's fill.
- Lift the dim greys: `--text-dim` to `--ink-muted` (#D0D2D3), and no text under 12 px.

Rejected:

- **Keep the dot as the summary.** Colour never carries meaning alone; the words must say it.
- **A green "healthy" dot.** Rules-shared 3: a healthy row is never green.

## 4. Screens and states

All in today's layout; the full shell of wave 4 uses the same parts.

- **S1 · Header at 1440, Connected.** Left: the place's title (18/24 Bold) with the status line under it
  (`.a-status`, 12/16, grey dot): "Connected", or "Connected · 9 active" on Tasks and "Connected · 2
  working" on the Workbench. Right: Search, the balance chip "Copilot 912 left" (line border, ink, only
  while Connected), the gear. The clock is gone from the header (it moves to Settings, Time zone, in
  `06-settings-groups.md`). The "Overlay" button is renamed "Chat" with the chat icon until wave 2
  removes it (Chat becomes a place).
- **S2 · Each state.** The seven states of rules-web 2 in the status line, with the dot colour and the
  saved strip exactly as the table in `../wave-0/02-shared-parts.md` section 4. Starting and
  Reconnecting are blue; Signed out, Offline, Can't be reached and AI down amber; Connected grey.
- **S3 · Can't be reached, 1440.** No banner. The status line "Your HQ can't be reached · 9 active"
  wraps if it must; under the header one strip "Saved in this browser at 14:02"; each section of Home
  shows its saved rows with no error of its own. Choosing the status line opens the connection sheet
  (bottom sheet below 768, side sheet from 768) with "This browser can't reach your HQ at
  hq.example.ts.net. It may be starting, restarting or updating. A browser can't tell which." and Try
  again now.
- **S4 · Signed out.** No section loads; each shows the signed-out empty view (`../wave-0/01-live-bugs.md`
  C1). No "unauthorized", no Retry.
- **S5 · Badges.** One badge in the rail and one in the bar, on Inbox, amber (#FFB300) with black text,
  counting Needs you (99+ past 99). The Workbench badge goes (W6); a helper that needs you is counted in
  the Inbox's Needs you. Every other needs-you count (a tab's count, a group's count) is amber too
  (`../../resolutions.md` A3).
- **S6 · Helper states.** In the Workbench list, the Watching panel and the details panel, one pill per
  state (`.a-state`): "Needs your OK" amber with a hand icon, "Working on it" blue with the spinner,
  "Ready" ink, "Computer offline" and "Lost contact" amber with the wifi-off icon, "Finished" and
  "Stopped" grey. The blocked callout's border and the details panel's card edge are amber (`.a-card.needs`).
- **S7 · Task states.** Late "2 days late" and "Stuck" amber with the warn icon; "Done" grey; sub-task
  bars blue; verification "Met" ink with a check, "Not met" amber with a warn icon.
- **S8 · 390.** The status line under the title, the gear in the top bar, the badge on Inbox in the
  bottom bar. 320 px: title and state only, the state wrapping; Search, the chip and the gear in one
  More menu named "Search, Spending and Settings".
- **S9 · 768 to 1023 and 2200.** The same status line; at 2200 nothing new appears.
- **S10 · 200% zoom.** The phone layout; the status line wraps to two lines before anything is cut.
- **S11 · Lite.** "Connected · HQ Lite", no chip. **Phone as hub (HQ window).** "Connected · HQ on your
  phone".

## 5. Exact copy

The state words, saved strip and connection sheet lines are in `../wave-0/02-shared-parts.md` section 5.
In addition:

| Element | String |
|---------|--------|
| Header chat button (until wave 2) | "Chat" |
| Helper state pills | "Needs your OK", "Working on it", "Ready", "Computer offline", "Lost contact", "Finished", "Stopped" |
| Task late | "{N} days late", "1 day late", "due today" |
| Verification marks | "Met." · "Not met." · "Not decided." |
| More menu name at 320 | "Search, Spending and Settings" |
| Spending row in the More menu, Connected | "Spending · Copilot 912 left" |
| Balance chip near and over a limit | "Today 85%" · "Today 104%" (B5) |

Retired: "live", "saved · {age}", "offline" as the dot's word, "Can't reach your HQ", "This device is
offline", every Tailscale line, "Could not load pinned: unauthorized", "blocked", "done", "idle" as
shown words, "Overlay", "Over the limit 104%".

## 6. Behaviour rules

1. **One source.** Every place reads `useConnection()`; no component computes its own reachability.
2. **State first.** The status line is `{state}` or `{state} · {place fact}`. A place may add one fact.
3. **Wrap, never cut.** `.a-status` has `white-space: normal`; the top bar's height follows.
4. **The chip only in Connected,** and only with a balance or a limit; past a warning point it is amber
   with the limit's words ("Today 85%"), over a limit amber "Today 104%" (never red,
   `../../resolutions.md` B5).
5. **No section error while not Connected.** A section shows its own failure line only while Connected
   and only for its own failed call (`fail_load_{section}_no_reply`). In every other state the status
   line and the strip say it.
6. **Colour tokens only.** No Tailwind palette colour (`rose-*`, `emerald-*`, `green-*`, `red-*`) for a
   status. Status classes come from the token map: `--ink-muted` fine, `--warn` needs you or failed,
   `--brand-blue` working.
7. **Never colour alone.** Every amber or blue status has an icon or a word.
8. **Contrast.** `--text-dim` becomes `#D0D2D3`; text smaller than 12 px is raised to 12 px; no text at
   reduced opacity.
9. **Focus.** The status line is a button with the accessible name "Connection: {state}"; it opens the
   connection sheet; Esc closes it and returns focus to the status line.

## 7. Browser and platform notes

- Forced colours (Windows High Contrast): the dot disappears into `CanvasText`, which is why the words
  carry the state. Borders of chips and cards stay (`forced-color-adjust: auto` on everything).
- Safari on iOS: `navigator.onLine` reports the radio; a captive portal is "Your HQ can't be reached".
- Print: the header and status line do not print (`@media print { .a-top, .a-rail, .a-nav { display:
  none } }`).

## 8. Code touchpoints

| File | Change | Exists |
|------|--------|--------|
| `apps/hq-web/src/routes/__root.tsx` | Header: title plus `StatusLine`; remove the dot (353-369) and `SavedDataLabel` (389); chip gated on Connected; "Overlay" becomes "Chat"; remove `ReachabilityBanner` (255) and the six-second notice's amber block (259) in favour of a toast. | yes |
| `apps/hq-web/src/components/StatusLine.tsx` | The status line button. | NEW |
| `apps/hq-web/src/components/ConnectionSheet.tsx` | The sheet. | NEW |
| `apps/hq-web/src/components/SavedStrip.tsx` | The strip. | NEW |
| `apps/hq-web/src/components/ReachabilityBanner.tsx` | Removed. | yes |
| `apps/hq-web/src/components/chat/SessionsPanel.tsx` | `AGENT_STATUS_CLASS` (9-14) becomes the state pill; words from the catalogue (line 144). | yes |
| `apps/hq-web/src/components/sessions/SessionRow.tsx`, `BlockedCallout.tsx`, `SessionInspector.tsx` | The pill and amber edge. | yes |
| `apps/hq-web/src/components/AppRail.tsx`, `BottomNav.tsx` | One amber badge on Inbox; Workbench badge removed (`AppRail.tsx:43`, `BottomNav.tsx:70-79`). | yes |
| `apps/hq-web/app.css` | `--text-dim` to #D0D2D3 (line 29); `.hq-rail-badge` amber, black text (1093); token variables from `aligned-ui.css` added. | yes |
| `apps/hq-web/src/components/SignalCard.tsx` | 12 px minimum, no opacity (34, 38, 54, 61). | yes |
| `apps/hq-web/src/components/vault/HomeSection.tsx` | No error while not Connected (rule 5). | yes |
| `apps/hq-web/src/components/tasks/TaskDetailDrawer.tsx`, `verification/TaskVerification.tsx`, `AuditSheet.tsx`, `TaskCard.tsx` | Late and not met amber with icon; met ink; chosen chip ink. | yes |

## 9. Server, desktop or release changes

None beyond `../wave-0/02-shared-parts.md` (`/api/hello`, `/api/status/ai`).

## 10. Acceptance checks (each fails on today's build)

1. A component test renders the header in each of the seven states and finds the exact words; the
   "Connected" render has the chip and the others do not. Today the header shows "live" or "saved".
2. No element in `apps/hq-web/src` uses `rose-`, `emerald-`, `green-` or `red-` Tailwind colours for
   text or borders (a `bun test` grep with an allow-list for the primary button). Today
   `SessionsPanel.tsx:12-13` and others do.
3. The Inbox badge's computed background is `rgb(255, 179, 0)` in the rail and the bar; the Workbench
   has no badge. Today the bar's badge is green and the rail's blue.
4. With the socket closed for 9 s, no element has the text "Tailscale" and `HomeSection` renders no
   error. Today the banner and "Could not load pinned" show.
5. `getComputedStyle` of `--text-dim` is `#D0D2D3`; no text node renders under 12 px on Home. Today
   8 and 9 px exist.
6. At a 320 px viewport the status line's text is not clipped (`scrollWidth <= clientWidth`) and Search
   is inside the More menu. Today the dot's word truncates.

## 11. Check on a laptop or phone

- Chrome at 1440, 390 and 320 (DevTools device mode), and at 200% zoom: the status line wraps, never
  ellipsed.
- Windows forced colours (Settings, Accessibility, Contrast themes): the state reads in words, chips
  keep borders.
- Stop and start HQ while watching the line: Can't be reached, Starting HQ · {n} s, Connected.

## 12. Open decisions it depends on

- **W6** (Recommended): the Inbox badge counts Needs you only, in amber.
- Depends on: `../wave-0/02-shared-parts.md` (the Connection module and catalogue), rules-shared 3,
  rules-web 2 and 11, `04-plain-words.md` (state pill words), `../wave-2/01-route-table.md` (the Chat
  button goes when Chat is a place).
