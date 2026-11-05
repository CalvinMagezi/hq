# Shared rules for the UX fixes

**Phone and web. Web additions live in `rules-web.md`.**

Source: the "Shared rules" board (FixRules) of the HQ Mobile Design canvas, page 2, as revised on
5 Nov 2026. Every fix spec under `mobile/` follows this file. Where a fix spec and this file
disagree, this file wins. It applies to the phone first; the web and the desktop follow.

The board was written after the adversarial review. It settles the conflicts between boards (review
item M14), the blockers that are rules rather than screens (R1, R4, R5), and the seven revised
decisions (D1, D2, D7, D8, D11, D16, D17). Decided with the owner on 5 Nov 2026.

Colours below are named by the design token and its Kotlin name in
`apps/android/app/src/main/kotlin/example/hq/spike/ui/theme/HqTheme.kt` (object `Mts`):

| Token | Hex | Kotlin |
|-------|-----|--------|
| surface-100 | `#231F20` | `Mts.Surface100` |
| surface-200 | `#322E2F` | `Mts.Surface200` |
| ink | `#FFFFFF` | `Mts.Ink` |
| ink-muted | `#D0D2D3` | `Mts.InkMuted` |
| line | `#8A8687` | `Mts.Line` |
| hairline | `#4A4647` | `Mts.Hairline` |
| brand-blue | `#00ADEE` | `Mts.BrandBlue` |
| brand-green | `#22FF37` | `Mts.BrandGreen` |
| amber | `#FFB300` | `Mts.Amber` |
| danger-text (the red) | `#FF6B6B` | `Mts.DangerText` |

---

## 1. Notification channels and what may show on them

**Privacy rule (R1): no note, task, chat, answer or command text in any notification by default**, on
any screen, because the same text reaches notification history, watches and any app with notification
access. Titles say who and what kind; bodies stay general.

Settings › Notifications gets "Show details in notifications" (off). With it off, a request has no
Allow button in the shade, only Open: **never approve what you cannot see**. With it on, the details
show only while the phone is unlocked and Allow asks for the unlock. Phones with no secure screen lock
never show details and never offer Allow in the shade (M3).

| Channel | Importance | What posts on it | Default words (details off) | Grouping and replay |
|---------|-----------|------------------|-----------------------------|---------------------|
| Needs you | High, sound and pop-up | Helper and task questions and approvals; "HQ needs your OK to finish" from a voice turn; a stuck helper | "Claude needs your OK" · "Open HQ to see what it wants to do." Buttons: Open (details off); Allow once · Decline · Open on Android 12+ with details on; Open only on 8 to 11 | Newest five, then "{N} more waiting for you". When notifications come back on, waiting items post once. |
| Updates | Default, silent | What HQ finished or noticed: an answer ready after the sheet closed, backup finished, speech model ready | "HQ answered · Tap to read." · "Backup finished." · "The speech model is ready." | More than three in an hour fold into "{N} updates from HQ". |
| App updates | Low, silent | A new version (once per version, one reminder at 7 days, daily only when the channel file marks it urgent); "Downloaded · Install"; "Updated to {version}" | "A new version of AgentHQ is ready." · "AgentHQ {version} is downloaded." | One per version, replaced in place. |
| Downloads | Low, silent, ongoing | Progress of any download over 10 MB: app updates and the speech model | "Downloading the speech model · 45% · 26 of 57 MB" · Pause · Cancel | Cancel acts at once and shows "Download cancelled · Undo" in the notification, for the accessibility timeout. |
| HQ is running | Min, silent, required | The foreground notice. Its text follows the Connection state (section 6). Tap opens the app. | "HQ is running" · "HQ is starting" · "HQ is restarting" · "HQ has stopped · Tap to see why" | Never more than one. |

All five channels are created once, at the hub's start, in wave 0. The Settings line reads
"Needs you on · Updates on · App updates on". The Inbox tab keeps the name "Updates"; the app-version
group in Backup and about is "App updates".

## 2. Failure lines: one catalogue, whole sentences (R5, M12)

Every failure line is a whole sentence with its own key in the string resources, written once per
action and reason, so translators never assemble fragments. Keys look like
`fail_send_message_starting`. The first sentence says what could not happen ("Couldn't send your
message."). The reasons and their second sentence:

| Reason | Second sentence |
|--------|-----------------|
| HQ starting | HQ is still starting. It will try again by itself. |
| HQ restarting | HQ is restarting. Try again in a moment. |
| HQ stopped | HQ has stopped. Start HQ to try again. |
| No reply in time | HQ didn't answer in time. Try again. |
| Server unreachable | Your server HQ can't be reached. Check your internet. |
| Signed out of server | This phone is signed out of your server HQ. Sign in again. |
| No internet | You're offline. It will send when you're back online. |
| AI service down | The AI service isn't answering. HQ is fine. Try again soon. |
| Download did not match | The download did not match. Nothing was kept. Try again. |
| Download server | Couldn't reach the download server. Check your internet and try again. |
| Phone almost full | The phone is almost full. Free up {size} and try again. |

Retired everywhere, including `voice/`, `data/` and `capture/`: "HQ is not answering", "HQ didn't
reply", "HQ is not running", "HQ did not answer", "Not answering", any HTTP code or exit code outside
Technical details.

Plurals use plural resources; times and dates use the phone's own format and its 12 or 24-hour
setting.

## 3. Colour means one thing each

- **Grey:** fine, nothing to do. A healthy row is never green.
- **Amber `#FFB300` with an icon:** needs you, waiting, stopped, a failure line. "HQ has stopped" is
  an amber dot with a stop icon.
- **Blue:** working, links, progress while working.
- **Green:** the main action when it loses nothing, and the brand. Never on a status, never on a
  destructive button, never on spending money or data.
- **Red `#FF6B6B`, black text:** only on a button that destroys or cuts off. Never a status, never
  text.

Colour never carries meaning alone: every amber or red item also has an icon or a word. Pins on
boards use `#FFB300`, not `#FFB547`.

## 4. Dialogs and Undo (D2 and D8 revised)

The board draws two reference dialogs. Both are a surface-100 card with a hairline border, 28 dp
corner radius, 20 dp padding, 330 dp wide; title 17/22 bold ink, body 14/20 ink-muted; buttons 40 dp
tall pills, right-aligned in one row with an 8 dp gap.

**Losing action:**

> **Stop Codex?**
> It stops now. Its work so far is kept.
>
> [Keep it running] (outlined, line border, ink text) [Stop Codex] (filled `#FF6B6B`, black text)

**Spending data:**

> **Download on mobile data?**
> The speech model is 57 MB. Your data plan may charge for it.
>
> [Wait for Wi-Fi] (outlined) [Download · 57 MB] (outlined, same weight)

Rules:

- **Side by side:** the safe choice on the left, the action on the right, as Android, the web and the
  desktop do. Stack only when a label is longer than 18 characters or the font scale is 1.5 or more;
  then the safe choice is on top and the action at the bottom.
- **Losing actions** (stop, remove, sign out, disconnect, delete, discard changes): the action is red
  with a verb and object. Back and a tap outside always choose the safe side.
- **Approvals of losing requests** ("Delete 3 old backups?"): in the app, Allow is red "Allow delete"
  on the right; in the shade, Open only.
- **Spending data or money** (D8): two equal outlined buttons, Wait first, the size in the label.
- **Reversible actions** act at once with "{Thing} {done} · Undo" for the phone's accessibility
  timeout (10 s on Android 8 and 9). Where HQ holds the item, Undo is a soft delete on HQ, shared with
  the web. If HQ stops inside the window, the action waits in the outbox as "Waiting for HQ".

## 5. First run, Ready to start and Settings home

- **First run:** Welcome, AI key, Chat apps, Notifications, Ready to start. Restore: Welcome, Restore,
  Notifications, Ready to start. Notifications is skipped when they are already allowed. No step
  counter.
- **Upgraders** (M6): on the first open after the update, if notifications are not allowed and HQ
  never asked, the "Let HQ reach you" screen shows once as a sheet.
- **Ready to start**, top to bottom: the notifications-off card (if off), "Use HQ as your phone's
  assistant" (optional; when chosen it shows a check and a grey border, not green), the one-machine
  warning (if any), then Start HQ (green) and Back.
- **Settings home**, one card slot on top (Update ready, else Notifications off, else Check your
  backups), then:
  - **This phone:** Background work · Keep running · Notifications · Phone assistant (never amber) ·
    Speech model (only once downloaded) · Capture and widgets · Keys on this phone
  - **Your accounts:** AI service and keys · Messaging apps · Other devices ("1 signed in") · Server HQ
    (when used)
  - **How HQ behaves:** Spending limits · Safety and limits
  - **Your data:** Backup and about (with App updates inside)
- When HQ can't be reached, rows that come from HQ show their saved values under one strip, "Saved on
  this phone at 14:32". No row says "Checking…".

## 6. Connection (R4, M8, M9, D16, D17)

- **States and their status-line words:** Connected · Starting HQ · 12 s · Restarting HQ · HQ has
  stopped · Reconnecting · Your server HQ can't be reached · Signed out of your server · You're
  offline · The AI service isn't answering. The state comes first in the status line and wraps; it is
  never cut off.
- **Stopped** only when the process is dead, or its start step has not moved for the longer of 60 s
  and twice the usual start time. While the process is alive, the sheet says "HQ is still opening your
  notes. Restarting starts over." with Restart as the second button.
- **The full-screen start cover shows only on the very first launch.** After that the Connection sheet
  holds the four start steps, "usually about {N} s", "Stopped at: {step}" and "The phone is almost
  full" when free space is low. The sheet opens by itself at most once a day.
- **Saved copy:** "Saved on this phone at 14:32", never "last copy". Kept until replaced, its age
  always shown. What came from a server HQ is cleared on server sign-out, Disconnect or a refused
  sign-in. Excluded from phone-to-phone transfer by data-extraction rules (wave 0).
- **Server HQ:** every board that shows HQ's state, notifications, backups, updates or the briefing
  also draws the server version ("Your server HQ" with its name).
- **AI service status** comes from a signed-in route (for example `/api/status/ai`), never `/health`.

## 7. Words (M12, M13)

- **HQ or AgentHQ:** "AgentHQ" only where Android shows the app's name (launcher, Android settings,
  permission dialogs, notification header) and in steps that point there ("Find AgentHQ in the
  list"). "HQ" everywhere else. Never lower-case "hq" in the app. A coding agent is a "helper"; the
  edge worker is "HQ".
- **Buttons that leave the app:** "Open Android settings", with one line naming the switch to tap.
- **Putting off:** "Not now" for anything that comes back; "Cancel" only in a sheet that edits
  something; "Later" is retired.
- **Plain for English as a second language:** at most 15 words a sentence, no semicolons, no idioms
  ("carries on", "picks up", "needs a look" become "keeps going", "starts again", "Check this").
  "Connected", not "Live".
- **Counts:** "{N} of {M} done" for items ("1 of 3 sub-tasks done"); "45% · 26 of 57 MB" for
  downloads; "Step 3 of 4" for starting. One progress bar: 4 dp, rounded, blue while working, amber
  while waiting.
- **Strings live in resources,** not Kotlin objects. The word check renders every fixture and reads
  every Text node, not just Words files; repository names like `{owner}/{repo}` are allowed.
- Key screens are also drawn at 360 dp wide and at font scale 2.0.

## 8. Links (M2)

**A link only opens a place.** It never starts a download, an install, the microphone, or answers
anything. Those need the app's own immutable intent, and the link screen drops any extras.

| Link | Opens |
|------|-------|
| `hq://chat`, `chat/{id}` | Chat, or that chat |
| `hq://chat?compose=1` | Chat with the composer focused (the keyboard, never the microphone) |
| `hq://inbox`, `inbox/{id}` | Inbox, or that item |
| `hq://tasks`, `tasks/{id}`, `tasks/away` | Tasks, a task, While you were away (new) |
| `hq://helpers`, `helpers/{id}` | Helpers, or that helper |
| `hq://notes/{path}` | A note |
| `hq://search`, `search/notes` | The one Search, on All or on Notes (changed) |
| `hq://settings/{screen}` | A Settings page; new: `assistant`, `update`, `speech` |
| `hq://web/{page}` | Usage, board, timeline; `home` and `vault` now open Notes |

Notification taps use these links: "HQ answered" opens `chat/{id}`, "The speech model is ready" opens
`settings/speech`, "Downloaded" opens `settings/update`.

## 9. Android versions and devices (M7, M11, R6)

Every fix spec states its Android 8 and 9, 10 and 11, 12, and 13+ behaviour.

Known splits:

- assistant check by `VoiceInteractionService.isActiveService` on all versions (not `RoleManager`);
- show over the lock screen by `setShowWhenLocked` on 8.1+, the window flag on 8.0;
- Allow on notifications only on 12+;
- the notification permission only on 13+;
- the accessibility timeout on 10+, 10 s before that;
- silence detection on 10+.

Device checks before each wave: the Galaxy A32, one Tecno, Infinix or itel phone, one Android Go
phone, and 8.1 and 10 emulators. The locked Home hold is logged on the A32 and a Pixel image before
wave 3 starts.

## 10. Safety switches and proof (R3, D11, M16)

- **Kill switches in the signed channel file:** install without confirm, the lock-screen assistant,
  the task outbox, the start cover removal, a withdrawn version, a staged rollout share.
- **Install without confirm (D11)** stays off until safe mode and the idle gate exist; never on test
  versions.
- **On-phone counters, "What HQ noticed":** no content, never sent, included in Copy details.
  Permission granted at step 4 or later; assistant chosen; locked presses unlocked or cancelled; answer
  notices tapped; data used per purpose; Undo taps per kind; times Start HQ was tapped while HQ was
  alive; saved-copy age when shown.
- **Roll back a fix** when crash-on-start reports follow a release, Start HQ is often tapped while HQ
  is alive, or Undo is used on more than about 20% of a kind's actions.
- **Spoken threads (D7):** HQ records a source field set by the runtime from the connection, never by
  the model, and never used for permissions.
