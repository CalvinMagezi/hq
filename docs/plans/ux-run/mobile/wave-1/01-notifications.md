# Wave 1 · 01 · Ask for notifications (finding #1)

Board: **FixNotifications** ("Fix notifications"), finding #1, revised after review, 5 Nov 2026.
Area: first run, upgraders, the No path, and asking again later.
Shared rules: `../../rules-shared.md` sections 1, 4, 5, 7 win. The notification words and the details
switch are owned by `../wave-0/01-lock-screen.md`; where the two boards word a notice differently, see
"Open conflicts" in `../README.md`.

Headline: **Ask for notifications once, in first run or on the first open after an update, and keep a
calm way back in.**

## 1. Finding and the review items it closes

**Problem.** The app never asks for `POST_NOTIFICATIONS`, so on Android 13 and newer a fresh install
posts nothing, not even the running notice, until the owner finds Settings, Notifications by chance.

**Who and how often.** Every new owner on Android 13 or newer, on every fresh install, and every owner
who updates from a build that never asked (they skip first run). Helpers and tasks then wait in silence
for an OK that never reaches the phone. On Android 8 to 12 notifications are on by default; there the
fix only covers an owner who turned them, or the Needs you channel, off. This board follows rules
section 1: five channels, no content text by default, Open only when details are off.

Closes finding **#1** and, in this revision ("What changed in this revision"):

- **M6** upgraders: the step shows once as a sheet on the first open after the update (frame 11), with
  its own check.
- **R1** no command or answer text by default: every notice on this board uses the general words, Open
  only. The "Allow answers" switch is replaced by "Show details in notifications", off (frames 8, 12).
- **M3** phones with no secure lock never show details or Allow (frame 12).
- **M7** Android 8 to 11 show Open only, and the step's example shows Open (frames 1, 14).
- **M8** server questions count as "on your server HQ", with Open (frame 13).
- **M14, M15** five channels by exact name, "Updates" and "App updates" settled, Ready to start drawn as
  rules section 5, "Connected" with a grey dot, red only on "Allow delete", buttons side by side, a 360
  dp frame at font scale 2.0 (frame 15).
- **Claims corrected:** a dismissed dialog is not a refusal; channels are made in `Alerts.java:51` and
  `HubService.java:173`, not `ShellActivity` line 367 (the `sampleAlerts` debug fixture); the running
  notice coming back after a grant is unverified and now a device check.
- **Words:** Decline · Allow once · Open everywhere, safe first (`../../resolutions.md` A4); "Open Android settings"; Settings line "Needs you on ·
  Updates on · App updates on".

## 2. Before (as built, verified at `c04a100`)

Paths under `apps/android/app/src/main/` (`K` = `kotlin/example/hq/spike/`).

- `AndroidManifest.xml:9` declares `POST_NOTIFICATIONS`, but no code calls `requestPermissions` for it
  (no match for `requestPermissions` or `POST_NOTIFICATIONS` in Kotlin).
- `K/ui/firstrun/FirstRun.kt:6` `enum class Step { Welcome, Model, Chat, Start, Restore }`; no
  Notifications step.
- `K/ui/firstrun/FirstRunWords.kt:35-36` "Ready to start" / "HQ will run all the time on this phone. Run
  it on one machine only: a second copy on another machine would answer the same chats twice."
- `java/example/hq/spike/Alerts.java:51` creates Needs you and Updates from `AlertPoster.channels()`
  (`K/AlertPoster.kt:38`); `Alerts.java:52` deletes the old "alerts" id.
- `java/example/hq/spike/HubService.java:173` creates `"hub"` ("HQ is running") at `IMPORTANCE_LOW`;
  `:190` returns `START_STICKY`.
- `K/ShellActivity.kt:357` `sampleAlerts()` (a debug fixture, reached by the `a-alerts` fixture at line
  268) also creates channels at line 367. It is not where production channels are made.
- `K/ui/settings/SettingsScreens.kt:747` `Notifications()`; line 778 "Allow answers from notifications".
- `K/ui/settings/SettingsViewModel.kt:281` the home row: "Off in Android" or "Needs you on · Updates on"
  (no App updates), `warn = !allowed` only.
- `K/ui/settings/PhoneWords.kt:127` `channelLine()`: "High · sound on · heads-up" style words; "Off in
  Android" only when the channel importance is 0, ignoring the app switch.
- `K/AlertPoster.kt:136-141` `VISIBILITY_PRIVATE` with a public version; `:149` buttons need API 31.

### Before frames

- **B1 · Ready to start, the last step.** "Ready to start" / "HQ will run all the time on this phone.
  Run it on one machine only: a second copy on another machine would answer the same chats twice." (pin
  A) / Start HQ / Back.
- **B2 · Settings, Notifications, when off.** "Notifications are off for AgentHQ / Approvals wait in the
  Inbox until they are on again." (pin B) [Open settings] (pin C). "Channels": "Needs you · High · sound
  on · heads-up", "Updates · Default · silent", "HQ is running · Low · required while HQ runs" (pin D).
  "Each opens that channel in Android settings. Turning Needs you off means approvals only wait in the
  Inbox." "Buttons on notifications": switch "Allow answers from notifications / Allow and Not now on the
  notification. Allow asks you to unlock first." (pin E).

What goes wrong:

- **A No step asks.** Start HQ ends the flow; nothing has requested `POST_NOTIFICATIONS`. On Android 13
  and newer the app posts nothing from here on, the running notice included. An owner who updates never
  sees first run at all.
- **B Only Settings says so.** The Inbox, Chat and Helpers never mention that questions are no longer
  reaching the shade.
- **C Open settings always leaves the app,** even while one tap on Android's own prompt would do it.
- **D Channel lines ignore the app switch.** With the app off they still read "High · sound on". A
  blocked Needs you channel changes only this line; Settings home shows no warning for it.
- **E On by default, with the command text in the body.** "Run this command? cargo build --release"
  reaches the lock screen, notification history and a watch on most phones (`AlertPoster.kt:137` hides
  it only when the owner chose to hide sensitive content).

## 3. Decision

**A first-run step that explains, then asks once through Android, and the same screen once as a sheet
for owners who update.** HQ's screen says why and shows what a question looks like with details off;
only its Allow button calls Android's dialog. Saying no is a normal path: the app works, three lines say
what is lost, and the same Turn on button returns later on Ready to start, in the Inbox and in Settings.
Every one of those buttons asks one function what to do, so none of them sends the owner to Android when
Android's own one-tap prompt is still allowed.

**Private by default (R1):** no note, task, chat, answer or command text in any notification unless the
owner turns on "Show details in notifications". With it off, the shade offers Open only, so nothing is
approved unseen.

**Why first run:** the owner has just set up their AI key and chat apps and knows what HQ is; Android
lets an app show its dialog only twice, so it must not be spent before the owner understands the reason.

Rejected:

- **Ask on the first launch, before Welcome.** No context yet, more refusals, and a refusal there spends
  one of the two asks.
- **Ask when the first question arrives.** That moment is a notification that cannot be shown; inside
  the app it is already too late, the helper is waiting.
- **Ask on every launch, or a banner on every screen.** Android stops after two refusals anyway, and a
  banner everywhere teaches the owner to ignore it. The banner lives on Needs you only, where the cost is
  felt.
- **Make Allow the green button on the No path too.** The owner already said no once; Continue is
  primary there, Allow stays one tap away.
- **Rely on Android's "private" visibility.** It hides text only when the owner chose to hide sensitive
  content; most phones show everything.

## 4. Screens and states (After)

First run: Welcome, AI key, Messaging apps (was "Chat apps", `../../resolutions.md` B15), Notifications,
Ready to start. No step counter. Skipped when
notifications are already allowed. Restore: Welcome, Restore, Notifications, Ready to start.

Shared layout of the first-run frames: a full-height column, title 24-ish bold ink centred under a 72 dp
round icon (surface-100 fill, hairline border, green bell glyph), body ink-muted, a spacer, then the
primary button (48 dp tall, full width, pill, green `#22FF37` with black text), outlined secondary (full
width pill, line border, ink text), and text buttons in blue (`#00ADEE`).

1. **The Notifications step (new).** Bell icon, title "Let HQ reach you", body "Helpers and tasks stop
   to ask for your OK. A notification tells you, even with the app closed." (pin 1). Label "Example"
   over an app-drawn notification card (pin 2): header "AgentHQ · now", title "Claude needs your OK",
   body "Open HQ to see what it wants to do.", one action "Open". It is read as one image, "Example
   notification". Primary "Allow notifications" (pin 3). A row with text buttons "Back" (left) and "Not
   now" (right).
2. **Android asks.** Frame 1 dimmed behind Android's own dialog (generic): "Allow **AgentHQ** to send
   you notifications?" [Allow] [Don't allow] (pin 4).
3. **Not now, or Don't allow once.** Title "Notifications are off" (pin 5). Body "HQ still works.
   Without notifications:". A surface-100 list, 12 dp radius, hairline dividers, each line with an
   ink-muted icon: "A helper that stops to ask waits until you open the app.", "Approvals from tasks
   wait in the Inbox.", "App updates show only in Settings." Hint "You can turn them on later in
   Settings, Notifications." Buttons: "Continue" (green primary), "Allow notifications" (outlined, pin 6),
   "Back" (text).
4. **Denied with no reason to ask.** Title "Notifications are off"; body "Android will not ask again.
   Open Android settings and turn on notifications for AgentHQ." (pin 7). Blue caption "On Android 8 to
   12, with them off" and hint "Notifications for AgentHQ are off on this phone. Open Android settings
   and turn them on." Buttons "Continue" (green), "Open Android settings" (outlined), then text buttons
   "Back" (left) and "Ask again" (right, pin 8). Blue caption "Back from Android, with them on" and the
   line "Notifications are on." (pin 9).
5. **Android's page for the app: five channels** (generic). "Notifications", "AgentHQ", "Show
   notifications" switch, "Categories" (pin 10): "Needs you · Questions that wait for your answer",
   "Updates · What HQ finished or noticed", "App updates · A new version of AgentHQ", "Downloads · Big
   downloads and how far they are", "HQ is running · Shows while HQ runs".
6. **Ready to start, still off** (rules section 5). Title "Ready to start". Card 1 (surface-100,
   hairline border, 12 dp radius, pin 11): "Notifications are off" / "Questions will wait in the Inbox."
   / green pill [Turn on]. Card 2 (pin 12): small label "Optional", "Use HQ as your phone's assistant",
   "Hold Home on any screen and ask by voice. Understands English. You can also do this later in
   Settings." / outlined pill [Set up]. Hint: "Run HQ on one machine only. A second copy would answer the
   same chats twice." (the card body and the one-machine line are FixAssistantSetup's,
   `../../resolutions.md` D-6) Spacer, "Start HQ" (green), "Back" (text).

### Later: the Inbox, Settings, and what happens when they come back on

7. **Inbox, Needs you, while off.** Top bar "Inbox" with a grey dot and "Connected", the balance chip
   "Copilot 912 left", Search and Settings icons. Tabs "Needs you 3", "Updates 4", "Done". Banner
   (black fill, amber `#FFB300` border, 16 dp radius, amber icon; pins 13, 14): "Notifications are off" /
   "3 questions waited here without telling you." / green pill [Turn on] and text [Not now]. Cards:
   - "Claude needs your OK · Mac mini · 2 min ago" / "It stopped to ask: Do you want to edit
     Chat.dc.html?" / [Decline] (text) [Allow once] (green).
   - "Delete 3 old backups? · Backup task · 5 min ago" / "The nightly backup wants to remove the 3
     oldest copies, from before Sep 21." / [Decline] [Allow delete] (red `#FF6B6B`, black text; pin 15).

   Bottom bar Notes, Tasks, Chat (raised), Helpers, Inbox with badge 3.
8. **Settings, Notifications: app off.** Card "Notifications are off for AgentHQ / Approvals wait in the
   Inbox until they are on again." [Turn on] (pin 16). "Channels": every line "Off in Android settings" (Needs
   you, Updates, App updates, Downloads, HQ is running). "What notifications show": "Show details in
   notifications / Off: they say who and what kind" (pin 17; FixPlainWords' line, D-1).
9. **Needs you off, or only quieter.** Amber card "Needs you is off / Questions wait in the Inbox
   without telling you." [Turn on] (pin 18). Channels: "Needs you · Off in Android settings" (amber with
   the mark), "Updates · Silent", "App updates · Silent", "Downloads · Silent · shows progress", "HQ is
   running · Silent, folded away · Android needs it while HQ runs". Variant "Or, when Needs you is only quieter": "Needs you · Silent" (pin 19) and the plain
   note "Needs you arrives silently, without a pop-up." Settings home, This phone: "Notifications · Needs
   you off · Updates on · App updates on" (amber with the warning mark).
10. **Turned back on: what was waiting.** Label "Posted once, newest on top, details off" (pin 20):
    "AgentHQ · now / Claude needs your OK / Open HQ to see what it wants to do. / Open", "A task needs
    your OK / Open HQ to see what it wants to do. / Open" (never the task's name, B19), "A helper is stuck / Open HQ to see which
    one. / Open". "The running notice, if Android shows it again" (pin 21): "AgentHQ / HQ is running /
    Answering your chats and running scheduled tasks." Caption: "Check on device: does it come back
    without a restart?"

### Upgraders, the details switch, a server HQ, Android 8 to 11, the narrowest phone

11. **Upgrader: the first open after the update.** Over Chat (top bar "Chat · Connected"), a bottom sheet
    with frame 1's content: "Let HQ reach you", body, "Example" card, [Allow notifications] (green),
    [Not now] (text only, no Back; pin 22).
12. **Show details: on, and a phone with no lock.** "What notifications show": "Show details in
    notifications" on, line "Details show only while the phone is unlocked. Allow asks you to unlock
    first." (pin 23). "The same question, unlocked, Android 12 and newer": "Claude needs your OK / Run
    this command? / `cargo build --release` / Decline · Allow once · Open". "On a phone with no screen
    lock": the switch greyed, "Set a screen lock to use this. Without one, anyone holding this phone can
    answer." (pin 24), [Open Android settings], "Then tap Screen lock."
13. **Owners with a server HQ.** Shade, details off (pin 25): "AgentHQ · Server HQ · now / Claude needs
    your OK on your server HQ / Open it to see what it wants to do. / Open". Inbox banner while off (pin
    26): "Notifications are off / 1 question waited here without telling you. / 2 are waiting on your
    server HQ. [Open]" with [Turn on] [Not now]. Settings, Notifications, Server HQ (as today): "Your
    server HQ · hq.example.ts.net · its questions arrive here".
14. **Android 8 to 11.** "Android 11, details on, unlocked" (pin 27): "Claude needs your OK / Run this
    command? / `cargo build --release` / Open". "Android 8 to 11, details off (the default)": "Claude
    needs your OK / Open HQ to see what it wants to do. / Open". "What else differs": "No permission to
    ask for. The step shows only when notifications or Needs you are off, with Open Android settings."
    "No Allow in the shade: Android 8 to 11 cannot ask for the unlock from a button." "Android 8 to 10
    have no notification history; Android 11 adds it, so the default words matter there too." "Channel
    pages open the same way from Android 8 (API 26, the app's minSdk)."
15. **360 dp wide, font scale 2.0** (pin 28). "Let HQ reach you", body, "Example ↓" (the example below
    the fold), buttons stacked full width: "Allow notifications", "Not now", "Back". The screen scrolls,
    the title wraps, nothing is cut, no sideways scroll.

### Which screen, when (one decision, `NotifAccess.next()`, behind every Allow and Turn on button)

| The phone says | First run, or the first open after an update, shows | Turn on, later |
|----------------|------------------------------------------------------|----------------|
| Android 13+, fresh install, never asked | The Notifications step with Allow notifications (frame 1) | Android's dialog |
| Android 13+, updated from a build that never asked | The same screen once, as a sheet, on the first open (frame 11) | Android's dialog |
| Android 13+, denied, and Android would show a reason | No path; the second button is Allow notifications (frame 3) | Android's dialog, its last time |
| Android 13+, denied, and no reason to show | No path with Open Android settings and Ask again (frame 4) | AgentHQ's page in Android (frame 5) |
| Android 8 to 12, notifications on, Needs you on | The step is skipped; Messaging apps goes straight to Ready to start | Nothing to turn on |
| Android 8 to 12, notifications off | The step with Open Android settings in place of Allow | AgentHQ's page in Android |
| Any version, app on, Needs you blocked | The step, button Turn on Needs you | The Needs you channel's own page |
| Any version, Needs you only quieter | Nothing; it is the owner's choice | A plain note in Settings (frame 9) |
| Granted, app on, Needs you on | The step skipped, or Allow moves on at once | No banner, no card |

Nothing counts refusals. As on the microphone board (`wave-3/03-assistant-mic.md`), the screen comes
from each request's result and `shouldShowRequestPermissionRationale` read just after it. A dialog
closed with Back or a tap outside is not a refusal: Android 11 and newer do not count it, and the
rationale flag does not change. A first dialog dismissed that way reads like "no reason to show", so
frame 4 keeps Ask again; if Android then shows nothing, the screen stays as it is. One stored flag,
"asked", only remembers that HQ has shown Android's dialog, for the upgrader sheet.

### Across the boards

- **Five channels, rules section 1:** Needs you, Updates, App updates, Downloads, HQ is running, by those
  exact names. All are made once, at the hub's start, in wave 0.
- **"Updates" and "App updates":** Updates is the channel for what HQ finished or noticed, and keeps its
  name as the Inbox tab. App updates is the channel for new versions of AgentHQ, and the group in Backup
  and about.
- **Ready to start** is drawn as rules section 5. The assistant card itself belongs to FixAssistantSetup;
  when chosen it shows a check and an ink border (`../../resolutions.md` A3).
- **The lock screen, watches and notification history** for every notification kind are drawn on
  FixLockScreen. This board draws the switch and the default words.
- **Restore** now ends at the Notifications step, not at Ready to start, so a restored phone is asked
  too.

## 5. Exact copy

The Notifications step (frame 1) and the upgrader sheet (frame 11):

| Element | String |
|---------|--------|
| Title | "Let HQ reach you" |
| Body | "Helpers and tasks stop to ask for your OK. A notification tells you, even with the app closed." |
| Over the example | "Example" |
| Example card | "Claude needs your OK" · "Open HQ to see what it wants to do." · "Open" |
| Example, accessibility | "Example notification" |
| Primary | "Allow notifications" |
| Text buttons | "Back" · "Not now" (the sheet has Not now only) |
| Android 8 to 12, off | "Open Android settings" (in place of Allow notifications) |
| Needs you blocked | "Turn on Needs you" |

No path (frames 3, 4):

| Element | String |
|---------|--------|
| Title | "Notifications are off" |
| Body | "HQ still works. Without notifications:" |
| Line 1 | "A helper that stops to ask waits until you open the app." |
| Line 2 | "Approvals from tasks wait in the Inbox." |
| Line 3 | "App updates show only in Settings." |
| Hint | "You can turn them on later in Settings, Notifications." |
| Buttons | "Continue" · "Allow notifications", or "Open Android settings" · "Back" · "Ask again" (frame 4) |
| Android 13+, no reason | "Android will not ask again. Open Android settings and turn on notifications for AgentHQ." |
| Android 8 to 12, off | "Notifications for AgentHQ are off on this phone. Open Android settings and turn them on." |
| Back and on | "Notifications are on." |

Ready to start (frame 6):

| Element | String |
|---------|--------|
| Card | "Notifications are off" · "Questions will wait in the Inbox." · "Turn on" |
| Assistant card | "Optional" · "Use HQ as your phone's assistant" · "Hold Home on any screen and ask by voice. Understands English. You can also do this later in Settings." · "Set up" (FixAssistantSetup's, D-6) |
| One machine | "Run HQ on one machine only. A second copy would answer the same chats twice." (FixAssistantSetup's, D-6) |

Notifications, details off (rules section 1; as worded on this board):

| Kind | String |
|------|--------|
| Helper question | "{Helper} needs your OK" · "Open HQ to see what it wants to do." · "Open" |
| Task approval | "A task needs your OK" · "Open HQ to see what it wants to do." · "Open" (B19) |
| Stuck helper | "A helper is stuck" · "Open HQ to see which one." · "Open" |
| Server HQ | "Your server HQ needs your OK" · "Open HQ to see what it wants to do." · "Open" (B19) |
| Folded | "{N} more waiting for you" |
| Buttons, details on, 12+ | "Decline" · "Allow once" · "Open", in that order (A4; a losing request keeps Open only) |

The task, stuck and server wordings are `../../resolutions.md` B19, the same as `../wave-0/01-lock-screen.md`.

Inbox (frames 7, 13):

| Element | String |
|---------|--------|
| Banner title | "Notifications are off" / "Needs you notifications are off" |
| Body, n waited | "1 question waited here without telling you." / "{N} questions waited here without telling you." (plural resource) |
| Server line | "1 is waiting on your server HQ." / "{N} are waiting on your server HQ." · "Open" |
| Body, none yet | "Questions from helpers and tasks wait here without telling you." |
| Buttons | "Turn on" · "Not now" |
| Card buttons | "Decline" · "Allow once"; a losing request: "Decline" · "Allow delete" (red) |

Settings (frames 8, 9, 12):

| Element | String |
|---------|--------|
| App off card | "Notifications are off for AgentHQ" · "Approvals wait in the Inbox until they are on again." · "Turn on" |
| Channel lines | "Sound and pop-up" · "Sound" · "Silent" · "Silent · shows progress" · "Silent, folded away · Android needs it while HQ runs" · "Off in Android settings" (FixPlainWords, D-1) |
| Needs you off card | "Needs you is off" · "Questions wait in the Inbox without telling you." · "Turn on" |
| Quieter, Default | "Needs you makes a sound but no longer pops up on screen." |
| Quieter, Low or Min | "Needs you arrives silently, without a pop-up." |
| Home row | "Needs you on · Updates on · App updates on" (amber with the warning mark when one is off) |
| Switch | "Show details in notifications" |
| Switch, off | "Off: they say who and what kind" (FixPlainWords, D-1) |
| Switch, on | "Details show only while the phone is unlocked. Allow asks you to unlock first." |
| No screen lock | "Set a screen lock to use this. Without one, anyone holding this phone can answer." · "Open Android settings" · "Then tap Screen lock." |
| New channels (name · description) | "App updates" · "A new version of AgentHQ." "Downloads" · "Big downloads and how far they are." "HQ is running" · "Shows while HQ runs." |
| Existing channels (frame 5) | "Needs you" · "Questions that wait for your answer"; "Updates" · "What HQ finished or noticed" |
| Server HQ row (as today) | "Your server HQ" · "{address} · its questions arrive here" |

## 6. Behaviour rules

1. **The Notifications step** sits between Messaging apps and Ready to start, with no step counter. It
   shows when `NotifAccess` says anything is missing; otherwise Messaging apps' Next goes straight to Ready to
   start. A kill on this step reopens it (the saved step).
2. **The example is drawn by the app,** not posted, so it can show before the permission exists. It
   shows the default words and Open only, on every Android version. It is read as one image, "Example
   notification".
3. **Only Allow notifications calls Android** and sets the "asked" flag. Not now goes to the No path
   without spending an ask.
4. **Android's dialog.** Allow moves to Ready to start at once. Don't allow goes to frame 3 or 4 by the
   rationale flag. Back or a tap outside is not a refusal: the same rule reads it, and Ask again covers
   it.
5. **The No path never blocks.** Continue moves on; the three lines say what is lost in the owner's
   terms.
6. **The second button follows the table.** Allow notifications while Android would show a reason; Open
   Android settings after that.
7. **No reason to show:** the body says Android will not ask, and the button opens AgentHQ's page in
   Android.
8. **Ask again** (Android 13+ only) asks once more. If Android shows nothing, the screen stays, with no
   error.
9. **Coming back** (`onResume`) reads the state again. On: "Notifications are on." for one second, then
   Ready to start. Still off: the screen stays, no error.
10. **Five channels,** by rules section 1 names, made once at the hub's start. The hub's channel moves
    from Low to Min under a new id, since an app cannot change a channel's importance after it is made.
11. **Ready to start** shows the off card only while off, first in the list. Turn on uses the same
    decision; on success the card disappears in place.
12. **The assistant card** comes next, then the one-machine warning, then Start HQ and Back (rules
    section 5).
13. **Inbox banner:** Needs you tab only, while the app or the Needs you channel is off. The count is the
    questions this phone's HQ would have posted while off; it resets when they are on again.
14. **Not now** hides the banner for 7 days; after that it returns if still off. Turn on: Android's
    dialog while allowed, else the app's page, else the channel's page.
15. **A losing request in the app** gets a red "Allow delete" on the right and Decline on the left (rules
    section 4). In the shade it has Open only, even with details on.
16. **Settings, app off:** same card words, button Turn on. Every channel line reads "Off in Android
    settings" while the app switch is off (D-1).
17. **Show details in notifications** is off on every install and upgrade. Off: titles say who and what
    kind, bodies stay general, the shade has Open only. It replaces "Allow answers from notifications".
18. **Needs you blocked** (importance none) with the app on: the amber card, an amber channel line with
    the mark, and an amber Settings home row. Turn on opens that channel's page.
19. **Needs you quieter** (Default, Low or Min): no warning and no banner, one plain note. HQ cannot
    raise it back; only the owner can.
20. **Back on:** the next poll posts what still waits in Needs you once, newest five then "{N} more
    waiting for you", in the default words. Updates are not replayed.
21. **The running notice** after a grant: unverified. If the device check shows it does not return by
    itself, the hub posts it again when it reads the grant.
22. **Upgrader sheet:** on the first open after an update (the stored version code is older), if
    notifications are not allowed and "asked" is false, once. Not now, Back or a swipe down close it and
    are not refusals. It never shows again for that install.
23. **Details on:** HQ posts the general words, swaps in the details when the phone unlocks, and back
    when the screen turns off. Allow asks for the unlock. Watches may still copy the details;
    FixLockScreen draws them.
24. **No secure lock** (`KeyguardManager.isDeviceSecure()` false): the switch is off and greyed, with
    the line and a way to set a lock. No details and no Allow in the shade.
25. **Server questions** post on Needs you with "on your server HQ" in the title. Open goes to that HQ's
    notifications page, as a tap does today (`AlertPoster.kt:104-111`).
26. **The banner counts the two apart:** "here" is this phone's HQ only. The server line appears only
    when a server HQ is signed in and has questions waiting.
27. **Android 8 to 11:** Open only, details on or off, because `setAuthenticationRequired` needs Android
    12.
28. **360 dp at font scale 2.0:** the screen scrolls, the example moves below the fold, and the buttons
    stack full width. No text is cut; the title wraps.

## 7. Android versions

| Version | What this fix does there |
|---------|--------------------------|
| 8 and 9 | No runtime permission and no notification history. The step and the upgrader sheet show only when notifications or Needs you are off, with Open Android settings. Shade buttons: Open only. The hub's channel goes to Min with a new id (frame 5). Undo lasts 10 s. |
| 10 and 11 | As 8 and 9, plus notification history on 11, which keeps what was posted. Shade buttons: Open only, details on or off. The accessibility timeout sets Undo. |
| 12 | No runtime permission. With details on and a secure screen lock: Decline · Allow once · Open, and Allow asks for the unlock (`setAuthenticationRequired`). Details off, or no secure lock: Open only. |
| 13+ | `POST_NOTIFICATIONS`: the Notifications step, the upgrader sheet once, frames 3 and 4. Buttons as on 12. Android 14+ lets the owner swipe the running notice away; it comes back at the next state change. |

Open question N11 (decided "Android 13+ only") settles the upgrader sheet on 8 to 12: not shown there.

Android constraints, as checked on the board:

- `POST_NOTIFICATIONS` is a runtime permission from Android 13 (API 33). The app targets 35
  (`build.gradle.kts:84`), so Android never shows the dialog by itself: only `requestPermissions` does.
  It is already in the manifest (`AndroidManifest.xml:9`).
- Two asks at most. From Android 11, a second Don't allow means `requestPermissions` returns denied at
  once with no dialog. Back or a tap outside returns denied too, but is not counted, and the rationale
  flag stays as it was.
- `shouldShowRequestPermissionRationale` is true only after one refusal; it is false before the first ask
  and false once Android stops asking. Hence Ask again on frame 4.
- Android 8 to 12 has no runtime permission. `areNotificationsEnabled()` is the app-level answer on every
  version, including 13+.
- Channels: `getNotificationChannel(id).importance`; `IMPORTANCE_NONE` means blocked. After creation only
  the owner can change importance, so a new level needs a new id (`Alerts.java:52` already deletes the
  old "alerts" id). Channel and app pages open with `ACTION_CHANNEL_NOTIFICATION_SETTINGS` and
  `ACTION_APP_NOTIFICATION_SETTINGS` (API 26, the minSdk).
- Answer buttons: `setAuthenticationRequired` is API 31, as `AlertPoster.kt:149` already checks. Below
  that a button cannot ask for the unlock, so it is not offered.
- Visibility: `setVisibility(PRIVATE)` with a public version (`AlertPoster.kt:136-141`) hides text only
  when the owner chose to hide sensitive content. So the main notification itself must use the general
  words.
- Notification history (Android 11+) and watches keep the text that was posted. Default words keep them
  clean.
- Changes while away: from API 28 Android sends `ACTION_APP_BLOCK_STATE_CHANGED` and
  `ACTION_NOTIFICATION_CHANNEL_BLOCK_STATE_CHANGED`; on 26 and 27 the state is read on resume and before
  each poll posts.
- Android's dialog cannot be reworded, so every reason is on HQ's own screen before it.

## 8. Code touchpoints

| File | Change | Exists |
|------|--------|--------|
| `K/ui/firstrun/FirstRun.kt` | `Step` gains `Notify`; `next(Chat) = Notify`, `next(Notify) = Start`, `next(Restore) = Notify`; `back(Start) = Notify`, `back(Notify) = Chat`. `FirstRunStore` (line 42) keeps "asked" and the last seen version code. | yes |
| `K/ui/firstrun/FirstRunScreens.kt`, `FirstRunViewModel.kt` | `NotifyScreen`, the No path, Ask again, Ready to start in rules section 5 order. Words move to string resources (FixPlainWords). | yes |
| `K/ui/firstrun/FirstRunWords.kt` | The third step's title (`CHAT_TITLE`, line 29, "Chat apps (optional)") becomes "Messaging apps (optional)" (`../../resolutions.md` B15); the Ready to start card body and one-machine line are FixAssistantSetup's (D-6). | yes |
| `K/NotifAccess.kt` (NEW, `../../resolutions.md` D-12; package not fixed by the board) | Read (sdk, granted, enabled, channel importances, rationale, asked) and the pure `next()` that returns `Done`, `AskAndroid`, `OpenApp` or `OpenChannel`. Unit-tested over the table. | NEW |
| `K/ShellActivity.kt` | The permission launcher, a re-read in `onResume`, and the upgrader sheet. Line 367 is inside `sampleAlerts` (line 357), a debug fixture; it is not where channels are made. | yes |
| `java/example/hq/spike/Alerts.java` | Today line 51 makes Needs you and Updates from `AlertPoster.channels()`. Wave 0 makes all five in one place at the hub's start. Count what would have been posted while off, this HQ and the server apart (`ServerHq.isServer`); after it turns on, post what still waits, once. | yes |
| `java/example/hq/spike/HubService.java` | Line 173 makes "HQ is running" (Low); moves to Min under a new id. | yes |
| `K/AlertPoster.kt` | `build()`: general words unless details are on and the phone is unlocked. `actions()`: buttons only with details on, API 31+ and a secure lock; "Take a look" and "Not now" become Open and Decline. | yes |
| `K/ui/inbox/InboxScreen.kt`, `InboxViewModel.kt` | The banner, its server line and its 7-day snooze; Decline and the red Allow delete. | yes |
| `K/ui/settings/SettingsScreens.kt` | `Notifications()`: five channels, Show details replaces the switch at line 778. | yes |
| `K/ui/settings/SettingsViewModel.kt:281` | The home line gains App updates and warns when Needs you is off. | yes |
| `K/ui/settings/PhoneWords.kt:127` `channelLine` | FixPlainWords' words; "Off in Android settings" while the app switch is off (D-1). | yes |

## 9. Server, release or desktop changes

None for this board.

## 10. Acceptance checks (each fails on today's build)

1. Android 13, fresh install: after Messaging apps, the Notifications step shows. Allow notifications, then
   Allow: Ready to start, no card.
   The third step's title reads "Messaging apps", never "Chat apps" (B15).
2. Upgrader on Android 13: install today's build, never grant, install the new build over it, open: the
   sheet shows once. Not now, reopen: no sheet. Android's dialog was not shown.
3. At Android's dialog, tap outside it: frame 4 with Ask again. Tap Ask again: Android's dialog shows
   again, so the dismissal was not counted.
4. Default settings, a helper asks to run a command: the shade, the lock screen and notification history
   show "Claude needs your OK · Open HQ to see what it wants to do." with Open only, and no command text.
5. Show details on, Android 12+, secure lock: unlocked shows the command with Decline · Allow once ·
   Open, in that order; locked shows the general words. On an Android 11 emulator: Open only.
6. A phone with no screen lock: the switch is greyed with "Set a screen lock to use this." and the shade
   never shows Allow.
7. Fresh install, Android's app page lists exactly five categories: Needs you, Updates, App updates,
   Downloads, HQ is running.
8. Switch off the Needs you category: Settings home reads "Needs you off · Updates on · App updates on"
   in amber with the mark; the Inbox banner shows. Ask a helper a question: "1 question waited here
   without telling you." Switch it back: the question is posted once.
9. Signed in to a server HQ, notifications off, a server question: the banner says "1 is waiting on your
   server HQ." and Open opens that HQ's notifications page. "here" does not count it.
10. In the Inbox, "Delete 3 old backups?" shows Decline on the left and a red "Allow delete" on the
    right.
11. 360 dp emulator at font scale 2.0: the step scrolls, no text is cut, Allow notifications is
    reachable, with no sideways scroll.
12. TalkBack on the step: the example is read once as "Example notification"; focus moves from the body
    to Allow notifications.
13. Welcome, Restore, restore a backup, Next: the Notifications step, not Ready to start. Kill the app on
    the step: it reopens there.

## 11. Check on device (before wave 1)

- **The running notice after a grant:** on Android 13, grant at the step and look at the shade without
  restarting. If "HQ is running" does not come back by itself, the hub posts it again after the grant.
  Unverified today.
- **Turning the permission off** on Android 13: does it end the app's process? The hub restarts by
  itself (`START_STICKY`, `HubService.java:190`), and the next read sees the new state either way.
- **A dismissed dialog on One UI:** Back and a tap outside leave the rationale flag unchanged, and Ask
  again shows the dialog.
- **Swapping details on unlock** (frame 12) on the Galaxy A32, a Tecno, Infinix or itel phone, an Android
  Go phone, and 8.1 and 10 emulators.
- FixPlan wave 1: large text on the A32.

## 12. Open questions and decisions it depends on

Open questions for the owner, with the board's recommendation:

- **How long does Not now hide the Inbox banner?** Recommendation: 7 days. A day is too often for
  someone who chose no; never again loses an owner who said no by mistake. (Decided: D13, "Not now hides
  it 7 days.")
- **New: show the upgrader sheet on Android 8 to 12 when notifications are off?** There HQ never asks, so
  "never asked" is always true, and off was the owner's own choice in Android. Recommendation: Android
  13+ only; on 8 to 12 the Settings card and the Inbox banner are enough. (Decided: N11, "Android 13+
  only.")

Depends on: D13, N5, N11; rules sections 1, 4, 5; `../wave-0/01-lock-screen.md` (the details switch and
the general words); `../wave-0/04-measure.md` (the `notifications.*` counters, which read
`NotifAccess`); `../wave-3/01-assistant-setup.md` (the assistant card on Ready to start).
