# Wave 0 · 01 · Lock screen privacy (R1)

Board: **FixLockScreen** ("Fix R1: lock screen privacy"), wave 0, added after the review, 5 Nov 2026.
Area: notifications (`AlertPoster`, `AnswerReceiver`, `Alerts`, `HubService`, Settings › Notifications).
Shared rules: `../../rules-shared.md` sections 1, 2, 4, 7, 8, 9 win over anything here.

## 1. Finding and the review items it closes

**The problem.** Every HQ notification carries its full text: a helper's command, a task's name, an
answer. The code marks them private and sets a public version, and its comment says the lock screen
shows only that something is there. That is true only for owners who chose "Hide sensitive content".
On default settings the lock screen shows everything, with the Allow button.

**Who it hurts, how often.** Every owner, on every notification, once wave 1 asks everyone for
notification access. Anyone who picks up the phone reads what helpers want to run. The same text lands
in Android's notification history, on a paired watch and in any app that holds notification access. On
a phone with no secure screen lock, a swipe is enough to press Allow.

Closes:

- **R1** (blocker): no content in any notification by default, an opt-in that shows details only while
  unlocked.
- **M3** for notifications: no details and no Allow in the shade on a phone without a secure lock.
- **The false claim** in `AlertPoster.kt:138`, removed with the code it describes.
- **FixRules 1**: words and buttons for all five channels, including "HQ answered" with no answer text
  (the voice side is in `wave-3/04-assistant-handoff.md`).

## 2. Before (as built, verified at `c04a100`)

Paths are under `apps/android/app/src/main/kotlin/example/hq/spike/` unless they start with `java/`.

- `AlertPoster.kt:132-144` `build()`: title is `card.title`, body is `card.body` (cut at 400 chars),
  `BigTextStyle` with the full body, `setVisibility(VISIBILITY_PRIVATE)`, a public version "Something
  needs you · Unlock to see what." (needs) or "1 new update · Unlock to read it." (updates).
- `AlertPoster.kt:138` the comment "The lock screen shows only that something is there, never what."
  It is false on default lock-screen settings.
- `AlertPoster.kt:115-120` `serverSignedOut()`: body at line 118 is "Its questions no longer arrive
  here. To get them again, sign in from Settings, Notifications with a new code from $address." (the
  server's address on the lock screen).
- `AlertPoster.kt:128-130` `simple()`: sets no visibility and no public version.
- `AlertPoster.kt:146-160` `actions()`: `buttons = answersAllowed(c) && SDK_INT >= 31` (line 149);
  Allow once (`setAuthenticationRequired(true)`), Decline or "Not now", then "Open" or "Take a look" for
  a stuck helper.
- `AlertPoster.kt:33` `ALLOW_ANSWERS = "allow_answers"`, default `true` at line 47.
- `AlertPoster.kt:38-41` `channels()`: only "Needs you" (HIGH) and "Updates" (DEFAULT, no sound).
- `java/example/hq/spike/Alerts.java:49-52` `watch()` creates those channels and deletes the old
  "alerts" channel.
- `java/example/hq/spike/HubService.java:33,173` the foreground channel `"hub"` named "HQ is running"
  at `IMPORTANCE_LOW` (rules say Min); text at line 194: "Answering your chats and running scheduled
  tasks." or "Only the app is on. Chats and scheduled tasks are paused."
- `AnswerReceiver.kt:184` `say()` builds the result notice; `ui/helpers/AnswerWords.kt:11` `failed =
  Say("Couldn't send your answer", "HQ didn't reply. It may be restarting.")` (retired wording).
- `ui/settings/SettingsScreens.kt:747` `Notifications()`; line 778 the switch "Allow answers from
  notifications"; wired at `ui/settings/SettingsViewModel.kt:192` and read at line 514.

### Before frames (drawn from `AlertPoster.build()`, `simple()` and `serverSignedOut()`)

- **B1 · Lock screen, default settings.** Clock "14:32", "Thursday 5 November". Three notices:
  - (A) "AgentHQ · now" / "Claude needs your OK" / "Run this command?" / `cargo build --release`,
    buttons Allow once · Decline · Open.
  - (B) "AgentHQ · 2 min" / "Delete 3 old backups?" / "The nightly backup wants to remove the 3 oldest
    copies, from before Sep 21." buttons Allow · Not now · Open.
  - (C) "AgentHQ · 9 min" / "Your server HQ signed this phone out" / "Its questions no longer arrive
    here. To get them again, sign in from Settings, Notifications with a new code from
    https://hq.example.ts.net."
- **B2 · No secure lock: a swipe opens it.** (D) "Claude needs your OK" / "Do you want to delete
  Notebooks/Projects/plan.md?" with Allow once · Decline · Open; then "Allowed. Claude is working
  again." / "Sent at 14:31." Footer "Swipe to open".
- **B3 · Notification history, Android 11+.** "Dismissed in the last 24 hours": "Run this command?
  cargo build --release", "Delete 3 old backups?", "Codex needs your OK · Run this command? git push
  --force origin main", "Board meeting notes · Draft agenda saved to Notebooks/Work." (E)
- **B4 · A paired watch.** "Claude needs your OK / Run this command? cargo build --release / Allow
  once". (F)

What goes wrong, by pin:

- **A** The comment is false. `VISIBILITY_PRIVATE` plus a public version is used by Android only when
  the owner chose "Hide sensitive content". Most phones ship with "Show all notification content".
- **B** The title is content. A task's approval uses the task's own words as the title.
- **C** A server address on the lock screen (`serverSignedOut()`), and `simple()` sets no visibility.
- **D** "Unlock first" means nothing without a lock: `setAuthenticationRequired(true)` is met by a
  swipe.
- **E** History keeps it: with notification history on (Android 11+) Android keeps title and text of
  every notification for 24 hours.
- **F** Watches and listeners see everything: a paired watch mirrors title, text and buttons; any app
  with notification access reads the full extras. No visibility flag reaches them.

## 3. Decision

**Build every notification twice in words, never in flags.** The default words say who and what kind,
never what. Details are an opt-in, "Show details in notifications", off for everyone, upgraders
included. With it on, details are posted only while the phone is unlocked, and the app posts the
general words again the moment it locks. Allow is offered only with details on, on Android 12+, on a
phone with a secure lock, and never for a losing request.

Why words: a lock screen flag cannot reach notification history, a watch or a notification listener.
Only text that was never posted is safe everywhere.

Rejected:

- **Keep `VISIBILITY_PRIVATE` and the public version.** Works only for owners who chose "Hide sensitive
  content". It stays, for them, as a second layer (frame 2).
- **`VISIBILITY_SECRET`.** Hides the notice from the lock screen entirely, so the owner misses that a
  helper waits. History, watches and listeners still get the text.
- **Details on by default, with `setLocalOnly`.** Keeps text off Wear OS bridging only. Lock screen,
  history and listeners still get it.
- **Ask on first run whether to show details.** One more decision in first run, before the owner knows
  what a helper asks. The switch lives in Settings.

## 4. Screens and states (After)

Lock-screen frames show a clock "14:32" (frame 4: "14:33") and "Thursday 5 November". Notices are drawn
as Android's own notification cards with the header "AgentHQ · {age}". Settings frames use the app's
page header (back arrow, title "Notifications"), group labels in small caps, rows in surface-200 groups.

1. **Locked, details off (the default).** Group "Needs you":
   - "AgentHQ · now" / "Claude needs your OK" / "Open HQ to see what it wants to do." / [Open] (pin 1)
   - "AgentHQ · 2 min" / "A task needs your OK" / "Open HQ to see what it wants to do." / [Open] (pin 2)
   - "AgentHQ · 6 min" / "Codex is stuck" / "Open HQ to see where it stopped." / [Open] (pin 3)

   Group "Updates" (no buttons):
   - "AgentHQ · 8 min" / "HQ answered" / "Tap to read."
   - "AgentHQ · 1 h" / "An update from HQ" / "Tap to read it."
2. **Locked, owner hides sensitive content.** The public versions: "Something needs you" / "Unlock to
   see what." (twice, pin 4 on the second), "1 new update" / "Unlock to read it.". Running notice
   unchanged: "AgentHQ" / "HQ is running" / "Answering your chats and running scheduled tasks."
3. **Unlocked, details on, Android 12+** (shade):
   - (5) "Claude needs your OK" / "Run this command?" / `cargo build --release` / [Allow once]
     [Decline] [Open]
   - (6) "A task needs your OK" / "Delete 3 old backups from before Sep 21." / [Open] only (losing
     request)
   - (7) "HQ answered" / "Your flight to the conference leaves at 07:40 from gate B12."
4. **Details on, then the phone locks.** Label "Posted again, silently, at lock" (pin 8): "Claude needs
   your OK / Open HQ to see what it wants to do. / Open", "A task needs your OK / Open HQ to see what
   it wants to do. / Open", "HQ answered / Tap to read.". Caption: "On unlock the details come back,
   again without sound or pop-up."
5. **Android 8 to 11, details on** (shade, unlocked, pin 9): "Claude needs your OK / Run this command?
   / `cargo build --release` / [Open]" and "A task needs your OK / Delete 3 old backups from before Sep
   21. / [Open]". Caption: "Android 8 to 11 cannot ask for the unlock from a notification button, so
   Allow is never offered here."
6. **No secure screen lock.** "Claude needs your OK / Open HQ to see what it wants to do. / [Open]",
   "HQ answered / Tap to read." (pin 10). Caption: "Same words locked and unlocked, whatever the switch
   says. No Allow in the shade." Footer "Swipe to open".
7. **Watch and history, details off.** History "Dismissed in the last 24 hours": "Claude needs your OK
   · Open HQ to see what it wants to do.", "A task needs your OK · Open HQ to see what it wants to
   do.", "An update from HQ · Tap to read it." Watch (pin 11): "Claude needs your OK / Open HQ to see
   what it wants to do. / Open on phone".
8. **Settings › Notifications.**
   - Top info card (info icon in ink-muted, pin 12): bold "Notifications now hide details", line "They
     say who needs you, not what. You can show details below." Shown to upgraders once.
   - Group label "What notifications show"; one row (pin 13): bold "Show details in notifications",
     secondary "Off. Notifications say who needs you, not what.", switch off at the right.
   - Note under the group (13/18): "Details are commands, answers and task names. With this on, they
     show only while the phone is unlocked. They also reach your watch and notification history."
   - Group label "Channels"; five rows, each with an external-link icon that opens Android's channel
     settings: "Needs you · High · sound and pop-up", "Updates · Default · silent", "App updates · Low ·
     silent", "Downloads · Low · silent", "HQ is running · Min · required while HQ runs".
9. **Turning details on.** Same page with a scrim and a dialog (pin 14): title "Show details in
   notifications?", two paragraphs "Details show only while the phone is unlocked. Your watch and
   notification history get them too." and "On Android 12 and newer, Allow appears on questions. It
   asks you to unlock first.", buttons side by side [Keep hidden] (left, outlined) [Show details]
   (right, outlined; not a losing action, so not red).
10. **Settings, no secure screen lock.** The row is dimmed: "Show details in notifications" /
    secondary "Needs a screen lock" / switch drawn off and not available (pin 15). Note: "This phone
    has no screen lock. Anyone holding it could read details and answer. Set a PIN, pattern or password
    to use this." Text button (link style, blue) "Open Android settings", then the line "Then tap
    Screen lock." Channels group follows (Needs you, Updates shown).
11. **Server HQ, locked.** "Claude needs your OK / On your server HQ. Open it to see what it wants. /
    [Open]" (pin 16); "Your server HQ signed this phone out / Its questions stop arriving here. Sign in
    again in Settings." (pin 17, no address); "Backup finished. / On your server HQ.".
12. **360 dp, font scale 2.0.** Settings › Notifications: the switch line wraps and the switch moves to
    the top of the row (pin 18); the note shortens to its first sentence on this frame; Channels show
    "Needs you · High · sound and pop-up". Nothing is cut off.

## 5. Exact copy

Every string is a whole sentence with its own key in the string resources (rules 2 and 7). `{Helper}` is
the helper's kind (Claude, Codex), never a session title or a repository name. The board gives no key
names; suggested keys are in the last column.

### Details off (all versions)

| Case | Title · Body | Buttons | Suggested key |
|------|--------------|---------|---------------|
| Helper asks | "{Helper} needs your OK" · "Open HQ to see what it wants to do." | Open | `notice_helper_ok_title`, `notice_open_to_see_body` |
| Task or action | "A task needs your OK" · "Open HQ to see what it wants to do." | Open | `notice_task_ok_title` |
| Losing request | "A task needs your OK" · "Open HQ to see what it wants to do." | Open only, details on or off | |
| Helper stuck | "{Helper} is stuck" · "Open HQ to see where it stopped." | Open | `notice_helper_stuck_title`, `notice_stuck_body` |
| Voice turn waits | "HQ needs your OK to finish" · "Open HQ to see what it wants to do." | Open | `notice_voice_ok_title` |
| Server question | "{Helper} needs your OK" · "On your server HQ. Open it to see what it wants." | Open | `notice_server_ok_body` |
| More than five | "{N} more waiting for you" · "Open HQ to see them." | | plural resource |
| Server sign-out | "Your server HQ signed this phone out" · "Its questions stop arriving here. Sign in again in Settings." | | |
| Answer ready (Updates) | "HQ answered" · "Tap to read." | none | |
| notify_owner (Updates) | "An update from HQ" · "Tap to read it." | none | |
| Fixed kinds (Updates) | "Backup finished." · "The speech model is ready." (server: body "On your server HQ.") | none | |
| Many (Updates) | "{N} updates from HQ" | none | plural resource |

### After an answer from the shade

| Case | Words | Buttons |
|------|-------|---------|
| Sent | "Allowed. {Helper} is working again." · "Sent at {time}." (kept) | |
| Failed | "Couldn't send your answer." · "HQ didn't answer in time. Try again." | Try again · Open |

### Owners who hide sensitive content (public version, kept)

| Channel | Words |
|---------|-------|
| Needs you | "Something needs you" · "Unlock to see what." |
| Updates | "1 new update" · "Unlock to read it." |

### Settings › Notifications

| Element | Words |
|---------|-------|
| Group label | "What notifications show" |
| Switch | "Show details in notifications" |
| Line, off | "Off. Notifications say who needs you, not what." |
| Line, on | "On while the phone is unlocked." |
| Note | "Details are commands, answers and task names. With this on, they show only while the phone is unlocked. They also reach your watch and notification history." |
| No lock, line | "Needs a screen lock" |
| No lock, note | "This phone has no screen lock. Anyone holding it could read details and answer. Set a PIN, pattern or password to use this." · button "Open Android settings" · "Then tap Screen lock." |
| Dialog | "Show details in notifications?" · "Details show only while the phone is unlocked. Your watch and notification history get them too." · "On Android 12 and newer, Allow appears on questions. It asks you to unlock first." · "Keep hidden" · "Show details" |
| Upgrader card | "Notifications now hide details" · "They say who needs you, not what. You can show details below." |
| Channel rows | "Needs you" · "High · sound and pop-up"; "Updates" · "Default · silent"; "App updates" · "Low · silent"; "Downloads" · "Low · silent"; "HQ is running" · "Min · required while HQ runs" |
| Running notice | "HQ is running" · "Answering your chats and running scheduled tasks." (unchanged) |
| Watch action | "Open on phone" (Android's own label on a watch) |

## 6. Behaviour rules

1. **One builder, two texts.** `NoticeWords.general(card)` gives title and body from the card's kind
   and helper only. `NoticeWords.detail(card)` gives today's body. Only the general text is posted
   unless rule 5 holds.
2. **Titles carry no content.** A task's or a notify_owner item's own title is never a notification
   title. It shows only inside the app.
3. **Open is the only button with details off,** on every version. Tap and Open both go through the
   link screen (rules section 8).
4. **The public version stays** for owners who hide sensitive content, and is added to every notice,
   `simple()` and the answer results included.
5. **Details are posted only when** the switch is on, `isDeviceSecure()` is true and
   `isKeyguardLocked()` is false at the moment of posting.
6. **Allow once and Decline** appear only under rule 5, on Android 12+, with
   `setAuthenticationRequired(true)` on Allow. The words are the shipped ones: Allow once · Decline ·
   Open.
7. **A losing request** (delete, remove, stop, sign out) shows its details under rule 5 but keeps Open
   only (rules section 4).
8. **At lock,** the hub hears `ACTION_SCREEN_OFF` and posts the general text over every detailed
   notice, with `setOnlyAlertOnce`. At `ACTION_USER_PRESENT` it posts the details back, silently. A
   notice posted while locked is general from the start.
9. **Android 8 to 11:** details under rule 5, Open only.
10. **No secure lock:** general text always, Open only, the switch reads "Needs a screen lock". The
    stored choice is kept, so a lock set later brings details back.
11. **History and watches** get exactly what was posted. With details off, that is the general text,
    nothing else.
12. **Upgraders** see the card once on this page and one Inbox note under Updates. Both go after the
    first visit.
13. **The switch** replaces "Allow answers from notifications". Its stored value is not carried over.
14. **Turning it on** asks once with the dialog. Keep hidden and Back leave it off. Turning it off
    needs no dialog and re-posts every shown notice general at once.
15. **Open Android settings** opens `ACTION_SECURITY_SETTINGS`. On return the page reads the lock again.
16. **Server questions** say "On your server HQ" and open the server's web app, as today.
17. **No address** in any notification, the server's included.
18. **At 360 dp and font scale 2.0** the switch line wraps and the switch moves to the top of the row.
    Nothing is cut off.

## 7. Android versions

| Version | Behaviour |
|---------|-----------|
| 8 and 9 | General text by default. Details under rule 5, Open only. Lock read by `KeyguardManager` (API 26 has both `isDeviceSecure` and `isKeyguardLocked`). No notification history. |
| 10 and 11 | As 8 and 9. Android 11 adds notification history, off unless the owner turns it on. It then keeps exactly what was posted. |
| 12 | Allow once · Decline · Open under rule 5; Allow asks for the unlock. Losing requests keep Open only. |
| 13+ | As 12, once the notification permission is granted (`wave-1/01-notifications.md`). Android 15's own redaction covers one-time codes only and changes nothing here. |

Android constraints, as checked on the board:

- Lock screen content is the owner's setting: show all, hide sensitive, or none. The public version is
  used only for "hide sensitive" and in a work profile.
- A channel's lock screen visibility can only be changed by the system after the channel exists. Not
  relied on.
- `SCREEN_OFF` and `USER_PRESENT` reach only receivers registered at run time, so `HubService`
  registers them while it runs. When it is not running nothing new is posted, and shown notices stay as
  posted. (Device check: a detailed notice when the hub is killed while unlocked.)
- Notification listeners and Wear OS receive the full extras, `BigTextStyle` included. Only what is
  posted reaches them.
- `setAuthenticationRequired` is API 31. Without a secure lock it is met by a swipe, hence rule 10.

## 8. Code touchpoints

| File | Change | Exists |
|------|--------|--------|
| `kotlin/.../AlertPoster.kt` | `build()` (lines 132 to 144) takes text from `NoticeWords`; delete the comment at line 138; `simple()` (128) gets visibility and a public version; `serverSignedOut()` (115) loses the address (line 118); `actions()` (146, the `buttons` check is line 149) checks rules 5, 6 and 7; `ALLOW_ANSWERS` (line 33, default `true` at 47) becomes `show_details`, default `false`; new `redactShown()`, `detailShown()`. | yes |
| `kotlin/.../AnswerReceiver.kt` | `say()` (184): visibility and public version; failure words from the rules section 2 catalogue. Line 38 checks `answersAllowed`; switch it to the new rule. | yes |
| `kotlin/.../ui/helpers/AnswerWords.kt` | Line 11 "HQ didn't reply. It may be restarting." is retired. `answersOff` ("Answers from notifications are off") goes with the old switch. | yes |
| `java/example/hq/spike/Alerts.java` | Keep the shown ids and their items so rule 8 can re-post them. | yes |
| `java/example/hq/spike/HubService.java` | Register the `SCREEN_OFF` and `USER_PRESENT` receiver in `onStartCommand` (171), unregister in `onDestroy` (354). | yes |
| `kotlin/.../ui/settings/SettingsScreens.kt` | `Notifications()` (747): the new group, note, dialog, no-lock state, upgrader card, channel rows. | yes |
| `kotlin/.../ui/settings/SettingsViewModel.kt` | `onAnswers` (192) and the state read (514) become the details switch and a lock read. | yes |
| `kotlin/.../ui/settings/PhoneWords.kt` | Words move to resources. | yes |
| `kotlin/.../ui/settings/PhoneIntents.kt` | Add `ACTION_SECURITY_SETTINGS`. | yes |
| `kotlin/.../NoticeWords.kt` | `general(card)`, `detail(card)`, backed by string resources. | NEW |
| `res/values/strings.xml` | All strings above. | yes (file exists) |
| Voice handoff | "HQ answered" through `NoticeWords`, never the reply text (`wave-3/04-assistant-handoff.md`). | |

## 9. Server, release or desktop changes

None for this board. The server sign-out notice stops naming the address on the phone side only.

## 10. Acceptance checks (each fails on today's build)

1. Galaxy A32 on its default lock screen setting. Lock it, have a helper ask to run `cargo build
   --release`. The lock screen reads "Claude needs your OK · Open HQ to see what it wants to do." with
   Open only. Today it shows the command and Allow once.
2. Instrumentation test with a test `NotificationListenerService`: for every inbox fixture kind, with
   details off, `EXTRA_TITLE`, `EXTRA_TEXT` and `EXTRA_BIG_TEXT` contain no substring of the item's
   title, description or excerpt longer than 3 words.
3. Android 11+ with notification history on: after a question, a task approval and a notify_owner
   update, history holds only the general words.
4. Details on, unlocked, Android 12+: the command and Allow once show. Press power: within 2 s the lock
   screen shows the general words and no Allow. Unlock: details back with no sound.
5. Phone with swipe or no lock, details stored as on: no details, no Allow, the switch reads "Needs a
   screen lock". Set a PIN: details come back.
6. Android 8.1 and 10 emulators, details on and unlocked: details show with Open only.
7. "Delete 3 old backups?" never shows Allow in the shade, details on or off, on every version.
8. Server sign-out: no notification text contains the server's address (test with
   `hq.example.ts.net`).
9. Upgrade from today's build with answers allowed: details are off, the card shows once on Settings ›
   Notifications and once in Inbox › Updates.
10. A failed answer from the shade reads "Couldn't send your answer." and "HQ didn't answer in time. Try
    again." Never "HQ didn't reply".

## 11. Check on device

- Details swap on unlock (FixPlan, wave 0 device check; "Still to prove" for R1).
- On the A32: a detailed notice when the hub is killed while unlocked (what stays shown, since the
  runtime receiver is gone).
- Run checks 1, 3, 4 and 5 on the A32, a Tecno, Infinix or itel phone, an Android Go phone, and the 8.1
  and 10 emulators (rules section 9).

## 12. Open questions and decisions it depends on

Open questions for the owner, with the board's recommendation:

- **Upgraders who used Allow from the shade.** After the update their details are off and Allow is gone.
  Recommendation: off, with the one-time card (frame 8, pin 12) and an Inbox note. Privacy first, one
  switch to get it back. (Matches decision N5: "Details off, a one-time note.")
- **The helper's name in the title** ("Claude needs your OK"). Recommendation: keep it. It is a kind,
  not content, and owners who hide sensitive content get "Something needs you".
- **`setLocalOnly` on detailed notices,** so details never reach a watch. Recommendation: no. The switch
  says the watch gets them. Revisit if an owner asks.

Depends on: N5; rules sections 1, 4 (losing requests Open only), 8 (links); FixNotifications (the five
channels and the permission, `wave-1/01-notifications.md`); FixAssistantHandoff ("HQ answered" and "HQ
needs your OK to finish").
