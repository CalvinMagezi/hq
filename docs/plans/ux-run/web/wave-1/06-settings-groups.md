# Web wave 1 · 06 · Settings in the phone's four groups, the card slot, and This browser (M5, W37)

Boards: **AlignedSettings** sections 1 and 4 (the home, "What moved where"), **AlignedSettingsMore**
sections 1 and 2 (This browser: Notifications, Time zone, Install and version, Data used),
**AlignedInbox** section 5 (the notifications card), AlignedReview M5, W05, W37, W40, and Drift 05, 07.
Follows `../../rules-web.md` sections 1, 5 and 9 and `../../rules-shared.md` section 5. Where this spec
and the rules differ, the rules win.

This wave regroups today's Settings and builds the This browser group. The other sub-pages (AI service
and keys, Messaging apps, Other devices' full page, Claude on the web, Folders, Safety and limits,
Backup and about) keep their content and move into their groups now; their redesign and every
connection state and width are `../wave-4/08-settings.md`. Spending limits is `../wave-4/09-spending.md`.

## 1. Finding and the review items it closes

- **M5 Settings groups.** Spending under Your data; no card slot.
- **W05 (Drift 05), partly closed.** No index on a phone; the nav misses Session folders, OpenRouter
  spend and Copilot credits; push lives on the Inbox although Get started says "Notifications".
- **W37 revised.** `/settings` always opens the same section; anything needing a look takes the one
  card slot (the old board always landed on Backup, whose warn was a constant).
- **W40, W16.** Notifications per browser, Show details off, reply alerts one Updates switch.
- **Drift 07.** The push button names its state ("Push on") and tapping it turns push off.

## 2. Before (as built, verified at `e5f9d5f`)

- `apps/hq-web/src/routes/settings.tsx:437-449` `NAV`: Clock, Devices, Chat apps, Model, Backend chain,
  Provider keys, Limits, Safety, Coding agents, Claude connector, Integrations; shown from 1024 px only
  (`:509`), as `#hash` links (`:453-460` scroll to `#section`). No entry for Session folders (`:340`),
  OpenRouter spend or Copilot credits.
- `settings.tsx:385` Clock: "This device ({city})" and a zone list, stored in localStorage by
  `lib/clock.ts:59-70`; `components/HeaderClock.tsx` shows it in the header.
- `apps/hq-web/src/components/notifications/PushToggle.tsx:12-75` on the Inbox: "Push on" when on (a
  tap turns it off), "Turn on push" when off; `lib/webPush.ts:120-125` sends a test.
- `apps/hq-web/src/components/InstallPrompt.tsx` a banner; the new-version prompt in `__root.tsx:277`.
- `apps/hq-web/src/routes/usage.tsx` Usage as its own place in the rail.
- Reply alerts: a bell in the chat header (`components/chat/ChatHeader.tsx:29`).
- No Data used, no Sign out this browser, no backup status.

## 3. Decision

**Settings sits behind the gear and uses the phone's groups, names and order. The groups are the section
index at every width. One card slot on top. Each section has its own address; `/settings` always opens
the same one.**

Groups and rows (AlignedSettings 1; rules-shared 5 with the web's own group first):

- **This browser:** Notifications · Time zone · Install and version · Data used · Sign out this browser
- **Your accounts:** AI service and keys · Messaging apps · Other devices · Claude on the web
- **How HQ behaves:** Spending limits · Safety and limits · Folders helpers may use
- **Your data:** Backup and about

Rejected:

- **Open on the first row that needs a look (W37 as first drawn).** Its warn was a constant, so it always
  landed on Backup. The card slot carries what needs a look instead.
- **A separate Usage place.** Spending limits is a Settings row, reached from the balance chip.

## 4. Screens and states

### Settings home

- **H1 · 1440.** The gear opens Settings in the shell, like a place (title "Settings", the status line).
  The four groups (`.a-label` heading, `.a-set` of `.a-li` rows) are the list pane, 360 wide; the open
  section is the detail pane. One card slot on top of the list. Rows and their trailing words:
  - This browser: Notifications "Needs you on · Updates on"; Time zone "Nairobi, same as HQ"; Install and
    version "Not installed · HQ 0.9.2"; Data used "6 MB today in this browser"; Sign out this browser
    "Wipes what this browser saved".
  - Your accounts: AI service and keys "GitHub Copilot · 3 services"; Messaging apps "Telegram on ·
    Discord off"; Other devices "4 signed in"; Claude on the web "Off".
  - How HQ behaves: Spending limits "$4.25 of $5.00 today"; Safety and limits "Read only"; Folders
    helpers may use "3 approved".
  - Your data: Backup and about "Last backup 9 days ago" (amber only when HQ reports a problem; today
    "Backups are set up where HQ runs." in grey, since HQ has no backup route yet).
- **H2 · The card slot**, one card, in this order: "A newer HQ is running" (Reload), else "Notifications
  are off in this browser" ("Questions from helpers wait in the Inbox until you look." [Turn on]), else
  "Check your backups" ("The last backup is 9 days old. It should run every day." [Open Backup and
  about]). With none, no card.
- **H3 · 390.** The four groups are the page; each row opens its section as a pushed page with a back
  arrow. The status line and the balance chip stay in the top bar.
- **H4 · 768 to 1023.** The rail and one pane: the home list fills it; a row opens its section in the
  same pane with a back arrow. With the card "Notifications are off in this browser", the Notifications
  row is amber too ("Off in this browser").
- **H5 · 320.** Title and state in the top bar; Search, the chip and the gear in More. A second line
  that does not fit ends in an ellipsis; the section shows it in full. Switch rows keep a 48 px target.
- **H6 · 2200.** The list keeps 360, the section its reading width (760), centred.
- **H7 · 200%.** The phone layout with the bottom bar.
- **H8 · Lite.** The gear opens This browser and Other devices only (WR2), with "HQ Lite has no other
  settings here. Keys, spending, messaging apps and helpers are not part of HQ Lite." Notifications is
  hidden unless `lite.allow_egress` allows the push service.
- **H9 · HQ window (phone hub).** The group is "This window": "Shown by HQ Hub on this Mac"; Data used
  "Counted on your phone · 4 MB today"; "Sign out this window · Wipes what this window saved";
  Notifications "Needs you on this Mac · HQ Hub shows a notice on this Mac. Its only button is Open." and
  "Updates · This window has no web push. Your phone shows its own notifications." No Install and
  version (HQ Hub updates itself). One line on top: "These are the settings of HQ on your phone." and
  "The phone's own settings, like Background work and Keep running, are on the phone."

### This browser › Notifications

- **N1 · On.** "In this browser": switch **Needs you** ("Questions and approvals from helpers and
  tasks."), switch **Updates** ("What HQ finished, like "HQ answered" after you leave a chat."), switch
  **Show details in notifications**, off ("Off: they say only who and what kind. On: the item's first
  line, never a command."). [Send a test], after sending: "Sent at 14:31: "Notifications from HQ are on
  in this browser."" with [Send again]. A specimen: "Example · AgentHQ · now / Claude needs your OK /
  Open HQ to see what it wants to do. / Open" and "Open is the only button. A browser cannot check your
  screen lock. So HQ never puts Allow in a web notification." "Good to know": "Each browser has its own ·
  Every browser and phone turns these on for itself. The phone app has its own Notifications page." and
  "While they are off · Questions wait in the Inbox. The Inbox shows a card once to ask for
  notifications."
- **N2 · Blocked for the site.** Card "Notifications are blocked for this site" / "Allow them in this
  browser's site settings for hq.example.ts.net. Then come back here." Switches off with "Turn them on in
  the browser first."
- **N3 · iPhone or iPad, not from the Home Screen.** "Add HQ to your Home Screen first" / "On iPhone and
  iPad, notifications work only from the Home Screen. Tap Share, then Add to Home Screen. Open HQ from
  there, then turn these on." [How to install].
- **N4 · A shared computer.** "Not on a shared computer" / "You chose "This is a shared computer" at
  sign-in. HQ sends nothing to this browser."
- **N5 · An HQ that sends none** (push not configured on HQ): the switches off with a line naming why.
- **The Inbox card** (until answered): "Let HQ reach you" / "Asked once in this browser" / "Get a notice
  when something needs you, even with HQ closed. It says who is asking, never what. Open it to answer
  here." [Turn on notifications] [Not now]. On yes: the card leaves and the test notice says
  "Notifications from HQ are on in this browser." On no or blocked: "Notifications are blocked for this
  site. Allow them in the browser settings, then try again." Not now hides it for 7 days, then a quieter
  line only if a question waited unseen: "1 question waited here without telling you." with [Turn on].

### This browser › Time zone, Install and version, Data used

- **Z1 · Time zone.** "Show times in" with "Europe/London", "Same as this browser (Nairobi)",
  "Africa/Kampala", "America/New York". "Kept in this browser only. A phone and a laptop can each have
  their own." "HQ's time zone" / "HQ sends exact times. This setting only changes how they show here." /
  "Nairobi". "Now · 11:32 London · 14:32 Nairobi". "The zones differ, so HQ names them" / "Times show in
  London time. Where HQ's own zone matters, it is named: "Daily summary at 07:00 Nairobi time". Spending
  limits name theirs: "resets at 00:00 UTC"." (Behaviour in `../wave-2/04-time-zones.md`.)
- **Z2 · Install and version.** "Install HQ on this computer" / "It opens in its own window as AgentHQ,
  with shortcuts to Chat, Notes and Inbox." / "On iPhone and iPad: tap Share, then Add to Home Screen."
  Install shows only when the browser offers it. "Version" / "HQ in this browser · 0.9.1" / "HQ · Updated
  by itself from a signed release at 14:20. · 0.9.2". When they differ: "A newer HQ is running" / "Reload
  to keep going. Drafts are kept." [Reload] (`../wave-2/03-version-and-data.md`).
- **Z3 · Data used.** "Today · 6 MB", "This week · 38 MB", "An hour with HQ open and idle · HQ aims for
  under 1 MB an hour. · 0.4 MB", "Opening Chat · 240 KB". "HQ gets changes over one link and pauses while
  this tab is hidden. It follows your browser's data saver. Big readers, like PDF and spreadsheets, load
  only when opened." Technical details: What HQ noticed (`../wave-0/02-shared-parts.md`).
- **Z4 · Sign out this browser.** The row opens the stacked red dialog of `05-sign-in-safety.md` D7.

These pages work with HQ out of reach, since nothing on them comes from HQ, except HQ's own zone and
version, which show as saved.

### What moved where (AlignedSettings 4)

| Was (web, as built) | Now | Group |
|---------------------|-----|-------|
| Clock (header clock zone) | Time zone: this browser's display zone, with HQ's own zone named when they differ | This browser |
| Push toggle on the Inbox | Notifications: Needs you, Updates and Show details, per browser, plus a one-time card in the Inbox | This browser |
| Install prompt and new-version banner | Install and version, plus "A newer HQ is running. Reload to keep going." | This browser |
| (none) | Data used, and What HQ noticed under its Technical details | This browser |
| (none) | Sign out this browser, which wipes what the browser saved | This browser |
| Model, Backend chain, Provider keys | AI service and keys, with effort and address kept | Your accounts |
| Chat apps | Messaging apps | Your accounts |
| Devices (code and QR) | Other devices | Your accounts |
| Claude connector | Claude on the web (W34), off by default | Your accounts |
| Usage page, Copilot credits, OpenRouter spend | Spending limits, with By AI service holding credits, balances and rate limits | How HQ behaves |
| Limits, Safety, Coding agents, Integrations | Safety and limits, tabs Safety, Time limits, Helpers, Tools. The helpers' safe box is kept in plain words | How HQ behaves |
| Session folders | Folders helpers may use | How HQ behaves |
| (none) | Backup and about: backup state when HQ reports it, HQ version, notes count, Copy details | Your data |

## 5. Exact copy

All strings in section 4 are exact. Keys follow `settings_{group}_{row}` and `notif_*`. In addition:

| Key | String |
|-----|--------|
| `settings_title` | "Settings" |
| `group_browser` | "This browser" |
| `group_accounts` | "Your accounts" |
| `group_behaves` | "How HQ behaves" |
| `group_data` | "Your data" |
| `card_newer` | "A newer HQ is running" · "Reload to keep going. Drafts are kept." · "Reload" |
| `card_notif_off` | "Notifications are off in this browser" · "Questions from helpers wait in the Inbox until you look." · "Turn on" |
| `card_backups` | "Check your backups" · "The last backup is {9} days old. It should run every day." · "Open Backup and about" |
| `backup_not_reported` | "Backups are set up where HQ runs." |

Retired: "Push on", "Turn on push", "Clock", "Devices" (as the section name), "Chat apps", "Provider
keys", "Backend chain", "Coding agents", "Integrations", "Session folders", "Claude connector" (as the
row name), "This device".

## 6. Behaviour rules

1. **Addresses.** Each section has its own path: `/settings/browser` (Notifications on top),
   `/settings/time`, `/settings/version`, `/settings/data`, `/settings/keys`, `/settings/messaging`,
   `/settings/devices`, `/settings/claude`, `/settings/spending`, `/settings/safety`,
   `/settings/folders`, `/settings/backup`. Old `#hash` links redirect (`../wave-2/01-route-table.md`).
2. **`/settings` opens the same section every time:** This browser › Notifications on a wide screen, the
   home list on a phone.
3. **The card slot** shows the first true card of H2 and nothing else. A card is never the only way to
   reach its page.
4. **Rows from HQ** keep their saved values under one strip when HQ is out of reach; no row says
   "Checking…" (states in `../wave-4/08-settings.md`).
5. **Notifications are per browser.** The switches write the subscription's `kinds` and `details`
   (`../wave-0/01-live-bugs.md` rule 4). Turning both kind switches off unsubscribes. The switch rows
   are real switches (`role="switch"`), named by what they turn on, never by their state.
6. **Send a test** only while on; the outcome stays on the row.
7. **Reply alerts** are the Updates switch; the chat header's bell goes.
8. **The Inbox card** shows on Needs you once per browser until Turn on or Not now; Not now hides it 7
   days (W31, D13).
9. **Lite and the HQ window** follow H8 and H9.

## 7. Browser and platform notes

- `Notification.permission` "denied" cannot be asked again from the page; N2's card is the only path.
- iOS and iPadOS (16.4+) allow push only for a Home Screen web app (`navigator.standalone`); N3.
- `beforeinstallprompt` exists in Chromium only; Firefox and Safari show the platform's own install
  steps (Safari: File, Add to Dock on macOS 14+; Share, Add to Home Screen on iOS).
- The HQ window in HQ Hub has no push, no install.

## 8. Code touchpoints

| File | Change | Exists |
|------|--------|--------|
| `apps/hq-web/src/routes/settings.tsx` | The four groups as the index at every width (replace `NAV` at 437 and the 1024-only nav at 509); one section per path. | yes |
| `apps/hq-web/src/routes/settings/*.tsx` | One file per section path (rule 1). | NEW |
| `apps/hq-web/src/components/settings/SettingsHome.tsx`, `CardSlot.tsx` | The list and the slot. | NEW |
| `apps/hq-web/src/components/settings/NotificationsPage.tsx` | N1 to N5; replaces `PushToggle`. | NEW |
| `apps/hq-web/src/components/notifications/PushToggle.tsx` | Removed from the Inbox; logic moves to the page. | yes |
| `apps/hq-web/src/lib/webPush.ts` | Subscribe with `kinds` and `details`; test words. | yes |
| `apps/hq-web/src/components/HeaderClock.tsx`, `lib/clock.ts` | The header clock goes; the zone moves to Time zone. | yes |
| `apps/hq-web/src/components/InstallPrompt.tsx` | Moves to Install and version. | yes |
| `apps/hq-web/src/components/chat/ChatHeader.tsx` | The bell goes (line 29). | yes |
| `apps/hq-web/src/components/AppRail.tsx`, `BottomNav.tsx` | Usage leaves the rail; the gear opens Settings. | yes |

## 9. Server, desktop or release changes

- Server: none new beyond wave 0's `kinds` and `details`. A backup status route is needed before Backup
  and about can turn amber (`../../changes.md` "Backup status, update status and diagnostics routes").
- Desktop: the HQ window's "This window" group needs to know it is inside HQ Hub: the proxy adds a
  response header `x-hq-window: hub` (NEW in `crates/hq-desktop/src/proxy.rs`) that the page reads once.

## 10. Acceptance checks (each fails on today's build)

1. At 390, `/settings` shows the four group headings in order; at 1440 it shows This browser ›
   Notifications in the detail pane. Today the nav is hidden under 1024 and there are eleven sections.
2. `/settings#keys` redirects to `/settings/keys`. Today there is no such path.
3. The Notifications page has three switches; "Show details in notifications" is off on a new
   subscription. Today there is one button on the Inbox labelled "Push on".
4. The card slot shows "Notifications are off in this browser" when permission is default and nothing
   newer is running; "A newer HQ is running" takes it first.
5. The header has no clock and the rail has no Usage. Today both are there.

## 11. Check on a laptop or phone

- Chrome, Firefox and Safari: turn notifications on from the Inbox card, then from Settings; send a test.
- iPhone Safari without the Home Screen: N3 shows; after Add to Home Screen, the switches work.

## 12. Open decisions it depends on

- **W37, W40, W16, W31, W36, WR2, WR5** (Recommended or Revised). **W34** (owner to confirm) for the
  Claude row.
- Depends on: `../wave-0/01-live-bugs.md` (push kinds and details), `05-sign-in-safety.md` (Sign out
  this browser, Other devices), `../wave-2/03-version-and-data.md`, `../wave-2/04-time-zones.md`,
  `../wave-4/08-settings.md`, `../wave-4/09-spending.md`.
