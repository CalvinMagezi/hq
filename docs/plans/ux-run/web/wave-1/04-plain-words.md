# Web wave 1 · 04 · Plain words and names, "This browser" (M4, W35)

Boards: **AlignedFlow** ("Words", the was and now table), **AlignedWebRules** section 5 (Names),
**AlignedReview** M4, W09, W35, M12, M13, and Drift 08, 09, 11, 14. Follows `../../rules-web.md`
section 5 and `../../rules-shared.md` section 7. The phone's twin is `../../mobile/wave-1/04-plain-words.md`.
Where this spec and the rules differ, the rules win.

## 1. Finding and the review items it closes

- **M4 Names.** This device, this browser, this computer, This PC, Mac only; "helper" meaning a running
  job, an installed program and an app.
- **W09 (Drift 09), partly closed.** Internal names leak: "claude-code", "native", "blocked" in the
  Watching panel; Session folders list the host "native"; "Return to to_do", "waiting for your agree or
  not"; "claude-code on claude-code" in Working now. And "This computer" is wrong for a server HQ.
- **W35.** "This device" becomes "This browser" for the web group; machines by their name.
- **Drift 08.** Inbox copy is out of date: "Inbox & Approvals", "relay notifications", "Search
  proposals, skills...", "View Details & Diff", raw "action needed" and "value item" badges.
- **Drift 11.** Approval dates read as times: "approved 05:27 AM for Claude" for a grant from twelve days
  ago.
- **Drift 01.** "14m ago ago" in the Workbench details panel.
- **Drift 14** (words half). "Overlay" on the header chat button; five page title styles.
- **M15** (words half): plurals and dates through the browser's formatters.

Who it hurts: owners who are not developers, every day.

## 2. Before (as built, verified at `e5f9d5f`)

| Today | Where | After |
|-------|-------|-------|
| "Inbox & Approvals" | `apps/hq-web/src/routes/notifications.tsx:137` | "Inbox" |
| "Review proposals, value insights, and relay notifications in real time" | `notifications.tsx:139` | removed (relays are chat only) |
| "Search proposals, skills..." | `notifications.tsx:225` | "Search the Inbox" |
| "Pending", "All Activity", "Resolved History" | `notifications.tsx:202, 214` | "Needs you", "Updates", "Done" |
| "Mark all read" | `notifications.tsx:164` | "Clear all updates" (`03-undo.md`) |
| "View Details & Diff" | `components/notifications/NotificationCard.tsx:123` | the card's title is the link; "Details" in the pane |
| "action needed", "value item" badges | `NotificationCard.tsx` kind badge | "Needs your OK", "Task ready to check", "A helper is stuck", "Update" |
| "Push on" / "Turn on push" | `components/notifications/PushToggle.tsx:63, 75` | a "Notifications" switch in Settings (`06-settings-groups.md`) |
| "blocked", "done", "idle", "working" | `components/chat/SessionsPanel.tsx:144` | "Needs your OK", "Finished", "Ready", "Working on it" |
| "Drive" and "HQ answers the agent and approves its prompts toward the task's goal" | `SessionsPanel.tsx:16, 31` | "Let HQ say yes to its requests" (wave 4 Chat) |
| "claude-code", "native" | `SessionsPanel.tsx`, `routes/settings.tsx:340` Session folders | "Claude Code", the computer's name ("Mac mini") |
| "Return to to_do" | `components/tasks/verification/TaskVerification.tsx:166` | "Send back" |
| "… waiting for your agree or not." | `components/tasks/verification/BriefingCard.tsx:160` | "{N} finished tasks wait for your look." (see copy) |
| "{x} on {y}" ("claude-code on claude-code") | `components/tasks/WorkingNowPanel.tsx` | "Codex · on your Mac mini" |
| "approved 05:27 AM for Claude" | `lib/sessionFoldersApi.ts:26` (`clockTime`, `lib/connectorApi.ts:99`) | "approved 24 Oct for Claude", "approved yesterday for Codex" |
| "Started 14m ago ago", "Last seen 1m ago ago" | `components/sessions/SessionInspector.tsx:37-38` | "Started · 40 min ago", "Last seen · 1 min ago" |
| "This device ({city})" | `routes/settings.tsx:385` | "Same as this browser ({city})" |
| "This device was signed out, or the server no longer accepts its sign-in." | `components/SignInPrompt.tsx:138` | "This browser was signed out, or HQ no longer accepts its sign-in." |
| "This device is not subscribed." | `lib/webPush.ts:123` | "This browser does not have notifications on." |
| "This device is offline" | `components/ReachabilityBanner.tsx:45` | "You're offline" (status line) |
| "New agent" | `routes/sessions.tsx:104, 248`, `components/sessions/NewAgentDialog.tsx:100` | "Start a helper" |
| "Stop all" | `components/sessions/EmergencyStop.tsx:32` | "Stop every helper" |
| "Workbench" | `components/BottomNav.tsx:81`, `AppRail.tsx:41` | "Helpers" (the place renames in wave 2) |
| "Connect a model" (mono, upper case) | `routes/setup.tsx:42` | "Set up HQ" title, "Your AI model" heading (wave 4) |
| "Chat apps" | `components/settings/ChatAppsSection.tsx:367`, `lib/getStarted.ts:68`, `routes/settings.tsx:440` | "Messaging apps" |
| "Session folders" | `routes/settings.tsx:340` | "Folders helpers may use" |
| "Overlay" | `routes/__root.tsx:331` | gone (Chat is a place, wave 2) |
| "Agent HQ Vault" (document title) | `routes/__root.tsx:46` | "HQ" (the installed app's name is AgentHQ, wave 2) |
| "Deny", "Approve" | `routes/connect.tsx:166` | "Decline", "Allow" (wave 4) |
| "Approve", "Reject", "Approve & Promote" | `NotificationCard.tsx:157, 177`, `NotificationDetailDrawer.tsx:206` | "Decline", "Allow once" (`02-dialogs-approvals.md`) |
| hand-built "{n}m ago", "{n}h ago", "{n}d ago" | `components/tasks/activity.ts:27-30`, `lib/time.ts` | `Intl.RelativeTimeFormat` short form: "3 min ago", "2 h ago", "4 days ago"; rows use the compact "3m", "1h", "2w" |

## 3. Decision

**The phone's words, everywhere; one naming table; no config words, ids, paths or the words agent,
session, harness, sandbox, provider or scope in what the owner reads.** Every string moves to the
catalogue (`../wave-0/02-shared-parts.md` rules 19 to 22) as it changes.

Rejected:

- **"This computer" for the web's group.** Wrong for a server HQ, and wrong in HQ Hub's HQ window.
- **"This device".** A phone, a laptop and a browser profile are all devices; the group is about this
  browser only.

## 4. Screens and states

No new screens. Every place keeps its layout in wave 1 and changes only its words, through the
catalogue. Notes on placement:

- **Titles.** One title style, 18/24 Bold, in the top bar on every place (rules-web 11). In wave 1 the
  five styles (Workbench 20 px Nunito, Tasks 24 px with an icon tile, Settings 30 px, Inbox 20 px Inter
  with an icon tile, Usage 18 px Inter; Setup and Connect mono upper case) become `.a-title` in place.
- **Dates and spans.** A span is written once ("Started" then "40 min ago"). A date older than today is
  a date ("24 Oct"), "yesterday" for yesterday, never a bare clock time.
- **Names of computers.** Each computer by its own name ("Mac mini", "Studio PC", "Pixel 8"). "On your
  server HQ · {Office}" for a server HQ. "HQ on your phone" when the phone is the hub. "This browser"
  for the web's own group; "This window" inside HQ Hub's HQ window (`../../resolutions.md` B11,
  confirmed). "This PC" is HQ Hub's tab name only; anywhere else the word is the machine's own name or
  "this computer" (B12).
- **320 px and 200%.** Longer words wrap; no label is cut with an ellipsis where a word carries meaning
  (row titles may clamp to two lines, then "…").

## 5. Exact copy: the naming table (rules-web 5 plus AlignedFlow "Words")

| Was | Now |
|-----|-----|
| Workbench | Helpers |
| agent, coding agent, session | helper |
| Home, Vault | Notes |
| Inbox & Approvals | Inbox |
| Pending, All Activity, Resolved History | Needs you, Updates, Done |
| blocked, waiting for you | Needs your OK |
| native, local, this device | the machine's own name ("Mac mini"), and "This browser" for the web's group |
| New agent | Start a helper |
| Stop all | Stop every helper |
| Usage, OpenRouter spend | Spending |
| Provider keys, Model, Backend chain | AI service and keys |
| Devices | Other devices |
| Live | Connected |
| Can't reach HQ, HQ is not answering | Your HQ can't be reached |
| Allow, Not now (approvals) | Decline, Allow once |
| HQ edge worker | HQ's own work: "HQ is working on it" while working, "Done by HQ" when finished (`../../resolutions.md` B4) |
| Claude Desktop under General helpers | Apps that use HQ |
| HQ Control Center | AgentHQ |
| Later | Not now |
| Chat apps | Messaging apps |
| Session folders | Folders helpers may use |
| Claude connector, claude.ai connector | "Claude on the web"; "connector" only in Technical details (B14) |
| This PC (anywhere but HQ Hub's tab) | the machine's own name, or "this computer" (B12) |
| Take a look | Open it to see. (B18) |
| Cancel (on the pairing code) | Stop pairing (B18) |
| 9 of 14 checked | 9 of 14 done (B18) |
| Search notes, Search the vault (placeholders) | Search notes, tasks, chats and helpers (B18) |
| Device removed, Removed. | {device} removed. (B18) |
| Daily limit 85%, Over the limit 104% | Today 85%, Today 104% (B5) |
| See all | Show all {N} (B6) |
| to_do, ready_for_review, blocked | To do, Ready to check, Stuck |
| 2/3, 1 of 3 steps | 1 of 3 sub-tasks done |
| Push on (a state on a button) | Notifications, a switch |
| Connect a model | Connect an AI service |
| approved 05:27 AM | Allowed 12 days ago (in a row: "approved 24 Oct") |
| Verification | Ready to check |
| Verify now | Check them now |
| Second opinion | Ask for a second look |
| Veto | Keep it open |
| Complete anyway | Mark done |
| Return to to_do | Send back |
| Tier 0, 1, 2 | Notes and plans, Code changes, Risky |
| Space, Initiative | area, list (WR14) |
| Canary | planted mistake |
| sweep, tier, shadow, enforce, veto | only under Technical details |
| Copy diagnostics | Copy details |

Other fixed lines:

| Key | String |
|-----|--------|
| `briefing_audits_waiting` | "{N} finished tasks wait for your look." (plural; "1 finished task waits for your look.") Recommended wording, the board names only the retired one |
| `helper_started` | "Started" · "{40 min ago}" |
| `helper_last_seen` | "Last seen" · "{1 min ago}" |
| `folder_approved` | "{Mac mini} · {~/code/shop-site} · approved {24 Oct} for {Claude}" |
| `zone_same` | "Same as this browser ({Nairobi})" |
| `push_not_subscribed` | "This browser does not have notifications on." |

## 6. Behaviour rules

1. **Display names only.** A helper's kind (`claude-code`) maps to its display name (Claude Code) from
   one table shared with HQ Hub (`crates/hq-tools/src/harness_session/spec.rs` names). A host id
   (`native`) maps to the computer's name from `/api/workbench/hosts`.
2. **No raw enum on screen.** Every status, kind and state is rendered through the catalogue; an unknown
   value renders "Not known", never the raw string.
3. **Relative time once.** `relTime()` returns the whole phrase; callers never add "ago".
4. **Dates by age.** Under 1 minute "now"; under 1 hour "{n} min ago"; today "{14:32}"; yesterday
   "yesterday"; this year "24 Oct"; older "24 Oct 2025". In compact rows: "now", "3m", "3h", "1d",
   "2w".
5. **At most 15 words a sentence, no semicolons, no idioms** ("carries on" becomes "keeps going", "needs
   a look" becomes "Check this").
6. **HQ, never "hq"** in the UI; "AgentHQ" only where the system shows the app's name.
7. **The words agent, session, harness, sandbox, provider and scope** do not appear in owner-facing
   text outside Technical details.
8. **Word checks run on the web too (added by resolutions.md C-13).** As on the phone
   (`../../mobile/wave-1/04-plain-words.md`), three checks fail the build: (a) a rendered-text check
   that mounts every place's fixtures and reads every text node and accessible name, not only the
   catalogue file; (b) a lint against literals in JSX text and string props (`title`, `aria-label`,
   `placeholder`), which is `../wave-0/02-shared-parts.md` rule 22; (c) at most 15 words a sentence,
   no semicolons and none of the retired words of section 5. Repository names like `{owner}/{repo}`,
   code samples and Technical details are allowed.

## 7. Browser and platform notes

- `Intl.RelativeTimeFormat` and `Intl.DateTimeFormat` follow `navigator.language`; times follow the
  browser's 12 or 24-hour preference (`hourCycle` from `resolvedOptions()`), in the display zone
  (`../wave-2/04-time-zones.md`).
- Screen readers read "3m" as "3 m"; compact rows carry the full phrase in `aria-label` ("3 minutes
  ago").

## 8. Code touchpoints

Every file in the Before table, plus:

| File | Change | Exists |
|------|--------|--------|
| `apps/hq-web/src/lib/time.ts` | `relTime()`, `shortAge()`, `dateByAge()` through `Intl`. | yes |
| `apps/hq-web/src/lib/names.ts` | Display names for helper kinds and hosts. | NEW |
| `apps/hq-web/src/strings/en.ts` | The keys above. | NEW (wave 0) |
| `apps/hq-web/src/lib/sessionFoldersApi.ts` | `formatApprovedText()` by date (line 25-26). | yes |
| `apps/hq-web/src/components/sessions/SessionInspector.tsx` | Drop the extra "ago" (37-38). | yes |
| `apps/hq-web/src/strings/words.test.ts` | Rule 8 (a) and (c): render every fixture, read the text, count words a sentence, refuse retired words (added by resolutions.md C-13). | NEW |
| `apps/hq-web/src/strings/catalogue.test.ts` | Rule 8 (b), shared with wave 0 rule 22. | NEW (wave 0) |

## 9. Server, desktop or release changes

None. The desktop's own words change in wave 3.

## 10. Acceptance checks (each fails on today's build)

1. The catalogue check finds none of the "Was" strings above in `apps/hq-web/src` outside tests. Today
   each is present at the cited line.
2. `bun test lib/time.test.ts`: `relTime(now - 14 min)` is "14 min ago" and `SessionInspector` renders
   "14 min ago" once. Today it renders "14m ago ago".
3. `formatApprovedText(twelve days ago)` gives "approved 24 Oct for Claude" (with the fixture date).
   Today it gives a clock time.
4. The Watching panel renders "Needs your OK" and "Claude Code" for `{agent_status: 'blocked', harness:
   'claude-code'}`. Today it renders "blocked" and "claude-code".
5. No owner-facing string contains "agent", "session", "harness" or "native" (allow-list for Technical
   details). Today many do.
6. `bun test strings/words.test.ts` (added by resolutions.md C-13): rendering every place's fixtures
   finds no sentence over 15 words, no semicolon and no retired word in any text node or accessible
   name; a fixture with the inline text "This PC is offline" outside HQ Hub's tab fails it. Today
   there is no such check.

## 11. Check on a laptop or phone

- Read every place once at 1440 and at 390 with a fresh vault: no internal word, no doubled "ago".
- Set the browser to a 12-hour locale: times read "2:32 PM".

## 12. Open decisions it depends on

- **W35** (Revised), **WR5** (its saved line replaced by B11), **WR14**, **N19** (the catalogue),
  **W44** ("This PC" on Macs, owner to confirm), and `../../resolutions.md` B4, B5, B6, B11, B12, B14,
  B18, C-13.
- Depends on: `../wave-0/02-shared-parts.md` (catalogue), `01-shell-status.md` (state pills),
  `02-dialogs-approvals.md` (approval words), `../wave-2/01-route-table.md` (place names in the rail).
