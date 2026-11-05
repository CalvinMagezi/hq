# Wave 1 · 03 · Destructive actions, the shared dialog and Undo (finding #10)

Board: **FixDestructive** ("Fix 10: destructive actions"), revised after review, 5 Nov 2026.
Area: every destructive action: `ui/theme/HqTheme.kt`, a shared dialog, eight screens.
Follows rules section 4 (D2 and D8 revised, approvals, Undo), section 2 (failure lines) and section 3
(colour) in `../../rules-shared.md`. Where this board differs, the rules win, and `../../resolutions.md`
wins over the rules (here A2, A6, B1, B2, B3, B8, B18 and C-1, C-4, C-5, C-7).

## 1. Finding and the review items it closes

**#10 Stop is green.** In the Helpers Stop dialog the destructive "Yes, stop it" is the green primary
button, and across the app the same kind of action is drawn five different ways, or not asked at all.

**Who it hurts, how often.** Any owner in a hurry, each time they stop a helper, sign a device out or
remove a key. Rare taps, so nobody learns the screen. A wrong one stops paid work, cuts a laptop off, or
leaves HQ with no AI to answer.

What changed in this revision:

- **D2 revised on every dialog:** side by side, the safe choice on the left, the red action on the right.
  Stacked only for a label over 18 characters or font scale 1.5 and up, safe on top.
- **Approvals of losing requests:** "Delete 3 old backups?" in the Inbox gets a red "Allow delete" on the
  right. In the shade it shows Open only, even with details on. (Review: "Approvals of losing requests
  still get a green Allow first".)
- **Labels match the title's verb:** A7 "Unlink account", A8 "Keep connected · Disconnect server", A1
  "Stop Codex". A14 now reads "Task discarded".
- **Failure lines from rules section 2,** amber with an icon, never red. "HQ didn't reply" and "HQ is not
  answering" are gone (R5).
- **Undo:** the phone's accessibility timeout, 10 s on Android 8 and 9. A soft delete on HQ where HQ
  holds the item, and HQ accepts the restore for 10 minutes (`../../resolutions.md` A6, C-1). If HQ stops
  inside the window, the change waits in the outbox under "Waiting to send".
- **Red only on the button that acts.** Triggers are ink text. No green status dots, no green chosen card.
- **Added:** server HQ approvals and notices, Android 8 and 9 Undo, a 360 dp frame at font scale 2.0, the
  Android versions card, Delete speech model in the audit (review: "Delete speech model uses the old
  pattern"), and acceptance checks that fail on today's build.

## 2. Before (as built, verified at `c04a100`)

Paths under `apps/android/app/src/main/kotlin/example/hq/spike/`.

- `ui/helpers/HelperPages.kt:294` `StopDialog`; line 301 `PrimaryButton("Yes, stop it", …)` green, on
  top, with outlined "Keep going".
- `ui/settings/SettingsScreens.kt:669` the shared `ConfirmDialog` and `:685` `DialogButton`: Cancel and
  the action as the same white text, side by side, for Disconnect, Link a different account, Sign out
  everywhere, Pause background work and Restart HQ.
- `ui/settings/SettingsWords.kt:415` `RESTART_WORDS` (action "Restart"); `:425` `modeDialog`; `:462`
  `disconnectWords` ("Disconnect {app}?" … "Disconnect"); `:463` `REPLACE_WORDS` ("Link a different
  account?", body ends "Continue?", action "Continue"); `:488` `SIGN_OUT_ALL_WORDS` (action "Sign out").
- `ui/pairing/PairingScreen.kt:254` `RemoveDialog`: a Material `AlertDialog` on Surface 100 with 18 sp
  titles, red text "Remove" beside "Cancel".
- `ui/firstrun/StoredKeysPage.kt:43` `StoredKeysPage`: the row turns into white "Back · Remove" inline;
  nothing checks for the only AI key.
- `ui/settings/SettingsScreens.kt:795` `ServerHqSection`: a white "Disconnect" that acts at once. Device
  Sign out is a blue link that acts at once.
- `ui/notes/NotesPages.kt:511` `LeaveDialog`: green Save, outline Keep editing, red text "Discard
  changes" last.
- `ui/tasks/TasksScreen.kt:83` the `bulkAsk` sheet: a "Mark 3 tasks done" row first, Cancel last.
- `ui/chat/ChatViewModel.kt:468` and `ui/inbox/InboxViewModel.kt:183` `UNDO_MS = 5_000L` (fixed 5 s).
- `ui/settings/SettingsViewModel.kt:191` `onDiscard = { id -> Outbox.of(ctx()).remove(id); … }`:
  deleted at once, no undo, no question.
- `ui/theme/HqTheme.kt:28` `DangerText = Color(0xFFFF6B6B)`; line 70 sets `error = Mts.DangerText` only.
- HQ: `crates/hq-web/src/lib.rs:436` has `/api/tasks/{id}/restore`; `crates/hq-web/src/notifications_api.rs`
  keeps cleared items as `state = 'dismissed'` (lines 110 and 395) with no route to set one back.

### Before frames

- **B1 · Helpers, Stop.** "Build the Android app · Working on it · 12 min". Dialog "Stop this helper?" /
  "Codex stops what it is doing and its terminal on your Mac mini closes. Nothing it already saved is
  undone." / green [Yes, stop it] on top (pin A) / outlined [Keep going].
- **B2 · Messaging apps, Disconnect.** "Disconnect Telegram?" / "HQ stops answering on Telegram and
  forgets the bot token. HQ restarts for a moment." / white [Cancel] [Disconnect] (pin B).
- **B3 · Pair a device, Remove.** "Remove “Work laptop”?" / "It loses access to this phone at once and
  its open connection is closed. You can pair it again later." / [Cancel] red text [Remove] (pin C).
- **B4 · Keys on this phone, Remove.** "These are the names of the keys sealed on this phone. Their
  values are never shown." Row `OPENROUTER_API_KEY` with inline white [Back] [Remove] (pin D). "The only
  AI key on this phone. One tap on Remove and HQ restarts with no AI to answer."
- **B5 · Other devices, Sign out.** "Signed in: MacBook · Signed in Nov 3 · used 3 h ago · [Sign out]"
  (blue, pin E); "Sign out everywhere". "Your HQ on a server · Signed in · https://hq.example.com ·
  [Disconnect]" (pin F). "Drawn together here; in the app the server row is on Notifications."

What is wrong across these: **A** green on a loss, on top, "Yes" says nothing; **B** no difference at
all between Cancel and the action; **C** the one red dialog is a different component; **D** removing a
key is inline and never checks for the only AI key; **E** signing one device out does not ask, and the
link is blue; **F** disconnecting from a server HQ does not ask. Undo is uneven (a fixed 5 s; a waiting
capture is deleted at once). Approvals that lose things get the same green Allow once as a harmless one,
in the app and in the shade. Trigger colours today: Stop in the helper menu red, Disconnect red, Sign
out everywhere red, Remove a device red, Remove a key white, Sign out blue, Link a different account
blue, server Disconnect white, Discard (waiting to send) white.

## 3. Decision (D2 revised)

**Sort by what can be put back. Ask only when it cannot. Never green on a loss. Safe on the left.**

- **What:** one shared `ConfirmDialog` with two kinds, Ask (green action) and Red (Danger action).
  Buttons side by side, safe left, action right. Undo snackbars for everything HQ or the phone can
  restore.
- **Why side by side now:** it matches Android, the web app and the desktop app. One order on every
  screen matters more than the old "way out first" stack. The hurried-thumb risk is met by the label and
  the colour.
- **Why stacked for long labels:** two 18-character labels fill a 360 dp dialog. Past that, or at font
  scale 1.5, side by side would cut a label.
- **Why filled red:** a 14 sp red word beside a white one is easy to miss in sunlight. A filled pill with
  a named verb is clear and still 48 dp.
- **Approvals:** approving a losing request is itself a losing action. It gets the same red, and it is
  never offered where the owner cannot read what they approve (the shade).

Rejected:

- **Type the word to confirm, or hold to confirm.** Slow and hard on a phone keyboard for owners who type
  little.
- **Undo for everything.** HQ cannot undo stopping a helper, signing a device out or forgetting a bot
  token. A fake Undo is worse than a question.

### The rule: three kinds of action, one way each

**Undo · HQ can put it back: act, then offer Undo.** Archive, clear, mark done, move, discard a waiting
capture. No question. The change shows at once and a snackbar says what happened, with Undo (example
snackbar: "Task archived" · "Undo").

- Colour: none. The trigger is ink or blue like any action.
- Time: the phone's accessibility timeout, at least 5 s. Android 8 and 9 have no such setting, so 10 s
  there.
- Where HQ holds the item (tasks, chats, updates), it is a soft delete on HQ, shared with the web. Undo
  restores it there.
- Where the phone holds it (a capture not sent yet), it is hidden at once and deleted when the snackbar
  closes.
- HQ stops inside the window: the change waits in the outbox under "Waiting to send" (the outbox's own
  words, `../../resolutions.md` A2). An Undo that waits in the outbox and reaches HQ after the 10 minutes
  fails with "Couldn't undo. It changed on another device, or too much time passed." (A6).
- Copy: "{Thing} {done}" plus Undo. Past tense, no "successfully".

**Ask · nothing is lost, but something stops.** Pause or start background work, restart HQ, mark
several tasks done. The dialog says what stops and that it comes back. Example: "Restart HQ?" / "HQ
stops for a moment and starts again. Anything it is answering right now is cut off." / [Not now]
[Restart HQ]. Colour: the normal green primary, on the right. Green means "go", and this loses nothing.

**Red · it loses something or cuts someone off.** Stop a helper, remove a device or a key, sign out,
disconnect, unlink, delete, discard unsaved changes, and approve a request that does one of these.
Example: "Stop Codex?" / "It stops now. Its work so far is kept." / [Keep it running] [Stop Codex].
Colour: filled Danger, black label, a verb and its object. Never green, never on the left.

**Shared by Ask and Red (D2 revised):**

- Order: side by side, equal width, 48 dp tall, 8 dp apart. The safe choice on the left, the action on
  the right.
- Stacked only when a label is longer than 18 characters or the font scale is 1.5 or more. Then the safe
  choice is on top and the action at the bottom, full width.
- A third choice that loses nothing (Save, Replace it instead) sits above the pair, full width.
- Back, the back gesture and a tap outside always choose the safe side. They never act.
- Title: a question with the verb and the thing: "Stop Codex?", "Remove “Work laptop”?".
- Body: at most three short sentences, 15 words each at most: what stops or is lost, what stays, how to
  get it back.
- Action label: the title's verb and object: "Stop Codex", "Sign out device". Never "Yes", "OK",
  "Continue" or "Confirm".
- Safe label: "Keep …" naming what stays ("Keep it running", "Keep the key"); "Not now" when nothing is
  kept.
- Busy: the action shows a spinner and "Stopping…"; both buttons are off so a double tap sends one
  request.
- Failed: the dialog stays, an amber line with an icon gives the rules section 2 sentence, and the action
  keeps its label.
- Done: the dialog closes and a snackbar says what happened, with no Undo.
- The trigger (menu row, text button) is ink text with the same verb. Red is only on the button that
  acts.

**Colour tokens (`ui/theme/HqTheme.kt`):**

- `Mts.Danger #FF6B6B` (today's `DangerText`, line 28, now for fills). Black on it 7.6:1.
- `Mts.OnDanger #000000`, the label on a Danger fill.
- Green `#22FF37` has a luminance of 0.72 against Danger's 0.33, so in greyscale green reads near white
  and Danger mid grey. Hue is never the only cue: the label names the loss, and it is always on the right
  or at the bottom.
- Amber `#FFB300` for failure lines and status that needs you, always with an icon. Never on a button.

### Every destructive or interrupting action in the app, today and after

| Action | Where | Today | Kind | After: the safe choice · the action · what it says when done |
|--------|-------|-------|------|--------------------------------------------------------------|
| Stop a helper | `helpers/HelperPages.kt` StopDialog, MenuSheet | Green "Yes, stop it" on top, outline "Keep going"; menu row red with a pause icon | Red | Keep it running · Stop Codex · "Codex stopped." Menu row: ink "Stop". |
| Remove a paired device | `pairing/PairingScreen.kt` RemoveDialog | Material AlertDialog, red text "Remove" beside "Cancel" | Red | Keep it paired · Remove device · "Work laptop removed." (`../../resolutions.md` B18) |
| Sign out one device | `settings/SettingsScreens.kt` Devices, onSignOut | Blue text link, signs out at once, no question | Red | Keep signed in · Sign out device · "Safari on macOS is signed out." (B3) |
| Sign out everywhere | `SettingsWords.SIGN_OUT_ALL_WORDS` | Red link; dialog with white "Cancel" and "Sign out" side by side | Red | Stacked (labels over 18): Keep everyone signed in · Sign out everywhere · "Signed out everywhere. HQ is restarting." (B2) |
| Disconnect Telegram or Discord | `SettingsWords.disconnectWords` | Red link; white "Disconnect" beside "Cancel" | Red | Stacked: Keep Telegram · Disconnect Telegram · "Telegram is disconnected. HQ is restarting." |
| Link a different account | `SettingsWords.REPLACE_WORDS` | Blue link; white "Continue"; body ends "Continue?" | Red | Title "Unlink your Telegram account?" · Keep my account · Unlink account · goes on to the pairing code |
| Disconnect from a server HQ | `SettingsScreens.ServerHqSection` | White "Disconnect" in the row, acts at once | Red | Keep connected · Disconnect server · "Disconnected from your server HQ." |
| Remove a key on this phone | `firstrun/StoredKeysPage.kt` | Row turns into white "Back · Remove", inline | Red | Keep the key · Remove key · "Key removed. HQ is restarting." |
| Remove the only AI key | same, plus `KeyVault`, `SettingsWords.configuredKeys`, `UsageWords.chain` | Same as any key: nothing says HQ will have no AI | Red | Replace it instead above · Keep the key · Remove key · "Key removed. HQ has no AI key now." with Add a key |
| Delete the speech model | FixAssistantModel, settings/speech | Its own pattern | Red | Stacked: Keep the speech model · Delete speech model · "Speech model deleted." |
| Discard unsaved note changes | `notes/NotesPages.kt` LeaveDialog | Green Save, outline Keep editing, red text "Discard changes" last | Red | Save changes above · Keep editing · Discard changes · no snackbar |
| Approve a request that loses something | `inbox/InboxScreen.kt` cards, `AlertPoster`, `AnswerReceiver` | Green "Allow once" like any request; Allow in the shade on 12+ | Red | In the app: the card quotes the tool and its arguments verbatim, then "In HQ's words" (B1); Decline · Allow delete (the request's verb). In the shade: Open only, also with details on. |
| Discard a capture waiting to send | `SettingsScreens` Capture, `SettingsViewModel.kt:191` | White "Discard", deleted at once, no undo | Undo | "Task discarded" or "Note discarded" · Undo, for the accessibility timeout; the file is deleted when the snackbar closes |
| Archive a task | `tasks/TaskSheets.kt`, `TasksViewModel` | "Task archived" · Undo | Undo | Soft delete on HQ (`/api/tasks/{id}/restore` exists). Timeout as above. |
| Archive a chat (swipe, or /clear) | `chat/ChatsSheet.kt`, `ChatViewModel` | Swipe: "Chat archived" · Undo. /clear: no undo line | Undo | /clear shows the same "Chat archived" · Undo |
| Clear one update, or all | `inbox/InboxScreen.kt`, `InboxViewModel.clear` | "Update cleared" or "Updates cleared" · Undo, held on the phone for 5 s | Undo | "3 updates cleared" · Undo. Sent at once as a soft delete on HQ; Undo restores it, on the web too. |
| Mark done, move to tomorrow | `tasks/TasksViewModel` | "Done: …" or "Moved to tomorrow" · Undo | Undo | Timeout as above |
| Mark several tasks done | `tasks/TasksScreen.kt` bulkAsk sheet | A sheet: "Mark 3 tasks done" row first, Cancel last | Ask | Shared dialog: Keep them as they are · Mark 3 tasks done, stacked · "3 tasks done · Undo" (`../../resolutions.md` B8) |
| Mark one task done that others wait on, or with a check pending | `tasks/TasksViewModel` | Acts at once | Ask (B8) | Not now · Mark done (or Wait for the check · Mark done) · "Done: {title} · Undo" |
| Pause or start background work | `SettingsWords.modeDialog` | White "Cancel" and "Pause" side by side | Ask | Stacked: Keep it on · Pause background work; Not now · Start background work |
| Restart HQ | `SettingsWords.RESTART_WORDS` | White "Cancel" and "Restart" side by side | Ask | Not now · Restart HQ |

**Left as they are:** Stop a reply in chat (what was written stays). Replace a key (checked before it is
saved). Restore a backup at first run (the phone has no notes yet, and the old folder is kept aside as
`.vault-before-restore`). Remove a chip or a file from the composer. The Paired devices switch (switches
act at once and say what they do). A future delete is Red unless it goes to an archive HQ can restore.

## 4. Screens and states (After: every one, drawn with the shared dialog)

Dialog geometry is the rules section 4 card (surface-100, hairline border, 28 dp radius, 20 dp padding).
Buttons 48 dp tall pills, equal width, 8 dp apart. Safe button outlined (line border, ink label). Red
action filled `#FF6B6B` with a black label. Ask action filled green with a black label.

- **A1 · Stop a helper.** Behind: helper page "Build the Android app · Working on it · 12 min", "Right now
  · Updating the build files", "Codex · on your Mac mini". Dialog "Stop Codex?" / "It stops now. Its work
  so far is kept." / [Keep it running] [Stop Codex] side by side (pin 1).
- **A2 · Stop: busy, then it failed.** Busy: same dialog, right button shows a spinner and "Stopping…",
  both buttons off. Failed: dialog stays, amber line with icon "Couldn't stop Codex. HQ didn't answer in
  time. Try again.", buttons back to [Keep it running] [Stop Codex] (pin 2).
- **A3 · Remove a paired device.** Behind: "Pair a device", "Paired devices": "Work laptop · a81c03fd ·
  seen 3 min ago · Remove", "Unnamed device · 7d20e9b4 · not seen yet · Remove" (ink triggers). Dialog
  "Remove “Work laptop”?" / "It loses access to this phone at once. Its open connection is closed. You can
  pair it again later." / [Keep it paired] [Remove device] (pin 3).
- **A4 · Sign out one device (new question).** Behind: "Other devices · Signed in · Safari on macOS ·
  Signed in Nov 3 · used 3 h ago · Sign out" (devices are named by browser and system,
  `../../resolutions.md` B3). Dialog 'Sign out "Safari on macOS"?' / "It can no longer open HQ until it
  signs in again." / [Keep signed in] [Sign out device] (pin 4). Done: "Safari on macOS is signed out."
- **A5 · Sign out everywhere (stacked: long labels).** Dialog "Sign out everywhere?" / "Every other device
  is signed out. This one stays signed in." / stacked: [Keep everyone signed in] on top, [Sign out
  everywhere] (red) below (pin 5; `../../resolutions.md` B2).
- **A6 · Disconnect Telegram (stacked).** Behind: "Messaging apps · Telegram · Connected · Unlink account
  · Disconnect". Dialog "Disconnect Telegram?" / "HQ stops answering on Telegram and forgets the bot token.
  To use Telegram again, you set it up from the start. HQ restarts for a moment." / stacked [Keep
  Telegram] · [Disconnect Telegram] (pin 6).
- **A7 · Unlink a Telegram account.** Dialog "Unlink your Telegram account?" / "Your Telegram account
  stops reaching HQ now. Then you pair the new account with a code." / [Keep my account] [Unlink account]
  (pin 7).
- **A8 · Disconnect your server HQ (new question).** Behind: "Server HQ · Your server HQ · Connected ·
  https://hq.example.com · Disconnect". Dialog "Disconnect your server HQ?" / "Questions from your server
  HQ at hq.example.com stop coming to this phone. To connect again, you need a new one-time code." / [Keep
  connected] [Disconnect server] (pin 8).
- **A9 · Remove a key.** Behind: "Keys on this phone": "OpenRouter · Replace · Remove", "Anthropic ·
  Replace · Remove". Dialog "Remove the OpenRouter key?" / "HQ forgets it on this phone and restarts for a
  moment. HQ keeps using your Anthropic key." / [Keep the key] [Remove key] (pin 9).
- **A10 · Remove the only AI key.** Behind: one row "OpenRouter · Replace · Remove" and "HQ has no other
  AI key". Dialog "Remove your only AI key?" / "Without it HQ cannot think. Chats, tasks and helpers stop
  answering until you add a key. HQ restarts for a moment." / full-width [Replace it instead] above, then
  [Keep the key] [Remove key] (pin 10).
- **A11 · Discard unsaved note changes.** Behind: note "Shopping list" / "Rice 5 kg, cooking oil, sugar,
  soap for the shop, airtime". Dialog "Save your changes?" / "You edited Shopping list and did not save.
  Your text stays on this phone unless you discard it." / full-width [Save changes] above (a third choice
  that loses nothing), then [Keep editing] [Discard changes] (pin 11).
- **A12 · Ask: pause background work (stacked).** Behind: "Background work": "Working in the background ·
  HQ answers your chats and runs scheduled tasks, even when the app is closed.", "Only when I open the app
  · Your notes and tasks work. Chats and scheduled tasks pause." Dialog "Pause background work?" / "HQ
  stops answering your chats and running scheduled tasks. It restarts for a moment." / stacked [Keep it
  on] · [Pause background work] (green, Ask kind; pin 12).
- **A13 · Undo: clear updates (soft delete on HQ).** Inbox, top bar "Connected · Copilot 912 left", tabs
  "Needs you 3 · Updates · Done", "No updates.", snackbar "3 updates cleared · Undo" (pin 13). Bottom bar
  with Inbox badge 3.
- **A14 · Undo: a waiting capture (held on the phone).** "Capture and widgets · Waiting to send · Nothing
  is waiting." Note: "The task "Call the supplier about the fridge" is hidden at once. Its file is deleted
  when the snackbar closes." Snackbar "Task discarded · Undo" (pin 14).
- **A15 · Inbox, approving a request that loses something.** Tabs "Needs you 2 · Updates · Done". Card
  "Claude needs your OK · Mac mini · 2 min ago" / "What it will run" / `bash {"command": "rm
  ~/backups/2026-09-14.tar ~/backups/2026-09-17.tar ~/backups/2026-09-20.tar"}` (the tool and its
  arguments verbatim from HQ's runtime, monospace, `../../resolutions.md` B1) / "In HQ's words: Delete 3
  old backups, from before Sep 21. They cannot be brought back." / [Decline] (left, text) [Allow delete]
  (right, red; pin 15). Card "Claude needs your OK · On your server HQ · hq.example.com · 5 min ago" /
  "What it will run" / `bash {"command": "cargo build --release"}` / "In HQ's words: Build the release
  version." / [Decline] [Allow once] (green, harmless).
- **A16 · Notification shade, a losing request shows Open only** (pin 16). "Details off (the default)":
  "AgentHQ · now / Claude needs your OK / Open HQ to see what it wants to do. / Open". "Details on,
  unlocked, Android 12+": "Claude needs your OK / Delete 3 old backups? / Open". Caption: "A losing
  request never gets Allow in the shade. A harmless one may, as rules section 1 says." "From your server
  HQ": "AgentHQ · 3 min / Your server HQ needs your OK / Open HQ to see what it wants to do. / Open"
  (`../../resolutions.md` B19).
- **A17 · Undo on Android 8 and 9 (10 s).** Tasks "8 active · Connected", tabs Today, Upcoming, All, "2
  tasks today.", "Pay the electricity bill · due today", "Order more rice · due today", snackbar "Task
  archived · Undo" (pin 17). Caption: "The same snackbar on every version. Android 8 and 9: 10 s. Android
  10+: `getRecommendedTimeoutMillis(5000, …)`, the owner's own setting."
- **A18 · HQ stops inside the Undo window.** Inbox top bar "HQ has stopped" (amber dot with stop icon).
  Tabs "Needs you 3 · Updates · Done". An amber-marked row "Clear 3 updates · Waiting to send" / "It will
  try again when HQ is back." (pin 18; A2), then "No updates." Caption: "The cleared items stay hidden. The
  change also lists in Capture and widgets, Waiting to send, where Undo still works."
- **A19 · 360 dp, font scale 2.0, stacked** (pin 19). "Sign out everywhere?" / "Every other device is
  signed out. This one stays signed in." / stacked [Keep everyone signed in] · [Sign out everywhere]. Caption: "At font scale 1.5 and up, every dialog stacks,
  safe on top. The body scrolls inside the dialog if it must. The buttons never leave the screen."

## 5. Exact copy

Each dialog's title, body and both labels are drawn verbatim in A1 to A19 (above). The audit table lists
the safe choice, the action and the done line for every row.

| Element | String |
|---------|--------|
| Stop a helper | "Stop {helper}?" / "It stops now. Its work so far is kept." / "Keep it running" · "Stop {helper}" (the rules section 4 example, word for word) |
| Approval card (every request) | title "{Helper} needs your OK", "What it will run", the tool and its arguments verbatim from HQ's runtime, then "In HQ's words" and HQ's description (B1); "Decline" · "Allow once", or for a losing request "Decline" · "Allow {verb}" ("Allow delete", "Allow remove", "Allow stop", "Allow sign out") |
| After declining | "Declined. Nothing was done." (N23) |
| A request left 24 hours | "This request ran out. Ask again from the helper." (C-7) |
| Same, in the shade | "{Helper} needs your OK" · "Open HQ to see what it wants to do." · "Open" |
| Only AI key when HQ can't be reached | the body adds "The app cannot ask HQ about keys set elsewhere right now." |
| Busy labels | "Stopping…", "Removing…", "Signing out…", "Disconnecting…", "Unlinking…", "Deleting…" |
| Undo | "Task archived", "Chat archived", "Update cleared", "{N} updates cleared" (plural resource), "Note discarded", "Task discarded", "Done: {title}", "Moved to tomorrow" · "Undo" |
| Held for HQ | "{Change} · Waiting to send" / "It will try again when HQ is back." (A2) |
| Late Undo | "Couldn't undo. It changed on another device, or too much time passed." (A6) |
| Sign out one device | 'Sign out "{device}"?' · "It can no longer open HQ until it signs in again." · "Keep signed in" · "Sign out device" (B3) |
| Sign out everywhere | "Sign out everywhere?" · "Every other device is signed out. This one stays signed in." · "Keep everyone signed in" · "Sign out everywhere" (B2) |
| Several tasks done | "Mark {N} tasks done?" · "Keep them as they are" · "Mark {N} tasks done" · then "{N} tasks done" · "Undo" (B8) |
| Done lines | "Codex stopped." · "{Work laptop} removed." · "{Safari on macOS} is signed out." · "Signed out everywhere. HQ is restarting." · "Telegram is disconnected. HQ is restarting." · "Disconnected from your server HQ." · "Key removed. HQ is restarting." · "Key removed. HQ has no AI key now." (with "Add a key") · "Speech model deleted." |
| Remove-only-key context line | "HQ has no other AI key" |

Failure lines (rules section 2), keys in resources:

| Key | String |
|-----|--------|
| `fail_stop_helper_no_reply` | "Couldn't stop Codex. HQ didn't answer in time. Try again." |
| `fail_sign_out_stopped` | "Couldn't sign out Safari on macOS. HQ has stopped. Start HQ to try again." |
| `fail_remove_device_pairing_off` | "Couldn't remove “Work laptop”. Pairing is off. Switch it on and try again." |
| `fail_disconnect_server_unreachable` | "Couldn't disconnect. Your server HQ can't be reached. Try again when it's back." |
| `fail_undo_too_late` | "Couldn't undo. It changed on another device, or too much time passed." |

Every line is "Couldn't {action}." + the reason + the outcome of `../../resolutions.md` A2 (the
pairing reason is A2's "Pairing is off" row).

## 6. Behaviour rules (numbers match the After pins)

1. **Red kind:** Danger fill, black label, verb and object, on the right. Nothing in the dialog is focused
   first; TalkBack starts on the title.
2. **Busy and failed:** both buttons off while the request runs. On failure the dialog stays with the
   amber line and the same action label.
5. **Stacking:** `maxOf(safe.length, act.length) > 18 || fontScale >= 1.5f` stacks the pair. Measured on
   the label as shown, in the phone's language.
8. **New questions:** signing one device out and disconnecting a server HQ now ask first.
10. **The only AI key:** after removal no AI service in HQ's chain would answer: no other AI key on this
    phone, none set outside the app, no GitHub Copilot plan, no AI on the phone. Tool keys (Brave Search)
    never count.
11. **Third choice:** one that loses nothing (Save changes, Replace it instead) goes above the pair, full
    width.
12. **Ask kind:** green action on the right, or at the bottom when stacked. The safe label is "Keep …" or
    "Not now".
13. **Undo where HQ holds it:** sent at once as a soft delete. Undo sends the restore, which HQ accepts
    for 10 minutes and only while the item has not changed since (added by resolutions.md C-1, the same
    restore routes as the web, `../../web/wave-1/03-undo.md`). Tasks have `/api/tasks/{id}/restore`.
    Cleared updates are kept as `state = 'dismissed'` (`notifications_api.rs:110` and `:397`), but no
    route sets one back yet. A refused restore shows "Couldn't undo. It changed on another device, or
    too much time passed."
14. **Undo where the phone holds it:** hidden at once, deleted when the snackbar closes or the app goes to
    the background. The word follows the item's kind.
15. **Losing request:** HQ marks the approval as losing from the tool's category (delete, remove, stop,
    sign out), set by the runtime, never by the model. Unmarked requests keep the green Allow once.
16. **Shade:** a losing request has exactly one action, Open, on every version and with details on or
    off.
17. **Undo time:** API 29+ the recommended timeout for 5 s of content and controls; API 26 to 28, 10 s.
18. **HQ stops inside the window:** the change goes to the outbox under "Waiting to send", and is sent
    when HQ is back. Undo there removes it from the outbox. A restore that waited in the outbox and
    reaches HQ after the 10 minutes fails with the A6 line.
19. **Large text:** stacked at 1.5 and up; the body scrolls inside the dialog; buttons stay on screen.
20. **Approval cards quote what runs** (`../../resolutions.md` B1): the tool and its arguments verbatim
    from HQ's runtime (`owner_approval.rs` without HQ's own `_hq_` keys, up to 3000 characters,
    monospace), then HQ's description under "In HQ's words". Never the description alone. Notifications
    stay general (rules section 1).
21. **Marking done** (B8): one task acts at once with "Done: {title} · Undo", unless other tasks wait on
    it or a check is pending (then it asks); several ask first ("Keep them as they are" · "Mark {N}
    tasks done"), then "{N} tasks done · Undo".
22. **A new sign-in on the phone hub (added by resolutions.md C-4).** When the phone is the hub, Other
    devices' "Sign in another device" code screen says "Type this only into HQ on your own device. HQ
    never asks for it in a chat or a call." above the code at all times. A new sign-in is a Needs you
    item, "A new sign-in to your HQ" / "{Chrome on Windows} signed in with a one-time code at {14:32
    today}." with [It was me] (left) and [Sign it out] (right, ink: only the dialog's action is red). Sign
    it out opens A4's dialog. Sign-ins expire after 90 days without use (HQ's rule, shared with the web,
    `../../web/wave-1/05-sign-in-safety.md`).
23. **The lockdown banner on Helpers (added by resolutions.md C-5).** While Stop every helper holds a
    computer: "All helpers are stopped. Allow them again in HQ Hub on that computer." No button: the lock
    lifts only in HQ Hub on that computer.
24. **An approval lapses after 24 hours (added by resolutions.md C-7).** Its card loses its buttons and
    reads "This request ran out. Ask again from the helper."; its notification is removed.

- **Dismiss:** Back, the back gesture and a tap on the scrim choose the safe side. They never act.
- **Triggers:** ink text or an ink menu row with the same verb. Red appears only on the button that acts.

(The board numbers its rules by pin, so 3, 4, 6, 7 and 9 have no rule of their own.)

## 7. Android versions

| Version | What differs for this fix |
|---------|---------------------------|
| 8 and 9 (API 26 to 28) | No `getRecommendedTimeoutMillis` (API 29), so Undo lasts 10 s. The shade shows Open only on 8 to 11 for every request. |
| 10 and 11 | Undo uses `AccessibilityManager.getRecommendedTimeoutMillis(5000, FLAG_CONTENT_TEXT or FLAG_CONTENT_CONTROLS)`. The shade still shows Open only. |
| 12 | Allow once and Decline may appear in the shade with details on, for harmless requests only. A losing request stays Open only. Open goes straight to the activity, since Android 12 blocks notification trampolines. |
| 13+ | Without notification permission nothing posts. The request waits in the Inbox with the same red Allow. From Android 14, the predictive back preview shows the safe side and never acts. |
| Font scale | 2.0 exists in Android 14 settings. One UI offers large sizes earlier. |

## 8. Code touchpoints

| File | Change | Exists |
|------|--------|--------|
| `ui/theme/HqTheme.kt` | Add `Mts.Danger = Color(0xFFFF6B6B)` (alias today's `DangerText`, line 28) and `Mts.OnDanger = Color(0xFF000000)`; set `onError` in the colour scheme (line 70 sets `error` only). | yes |
| `ui/shell/Confirm.kt` | `enum ConfirmKind { Ask, Red }`, `data class ConfirmWords(title, body, keep, act, busy, kind, extra = null)` with string resource ids, `@Composable ConfirmDialog(words, busy, error, onKeep, onAct, onExtra)` that lays out side by side or stacked by the rule, and `DangerButton`. | NEW |
| `ui/shell/Undo.kt` | One snackbar host with the API split, a soft-delete path for HQ items, a hold-then-delete path for phone items, and the hand-off to the outbox. Replace `UNDO_MS` in `ChatViewModel.kt:468` and `InboxViewModel.kt:183`. Maps a 409 from a restore route to the A6 line (C-1). | NEW |
| Replace | `HelperPages.StopDialog` (line 294; the green button at 301) and the menu icon; `SettingsScreens.ConfirmDialog` (line 669) and `DialogButton` (line 685); `PairingScreen.RemoveDialog` (line 254); the inline confirm in `StoredKeysPage`; `NotesPages.LeaveDialog` (line 511); the bulk sheet in `TasksScreen` (line 83). | yes |
| Add questions | `SettingsScreens` device Sign out and `ServerHqSection` (line 795) Disconnect. `onDiscard` (`SettingsViewModel.kt:191`) becomes held with Undo. | yes |
| Inbox and shade | The approval card reads a `loses` field, the tool and its arguments (B1) and the lapse time (C-7); `AlertPoster` posts Open only when it is set. | yes |
| `ui/settings/SettingsScreens.kt` Devices | The code screen line and the new sign-in card's two buttons (rule 22, C-4); device rows by browser and system (B3). | yes |
| `ui/helpers/` (Helpers screen) | The lockdown banner (rule 23, C-5). | yes |
| HQ (Rust) | A `loses` flag on approval items set from the tool category, and a restore action for a dismissed inbox item. Neither exists today. | NEW |

## 9. Server, release or desktop changes

- **Approvals carry a `loses` flag** set by the runtime from the tool's category, never the model (FixPlan;
  N17).
- **A restore route for a cleared update,** so Undo is shared with the web (FixPlan; N18). Today
  `notifications_api.rs` can set `dismissed` (line 254 maps "dismiss" and "reject") but nothing sets it
  back. Every restore route accepts a restore for 10 minutes only (WR7, C-1; built once for both, see
  `../../changes.md` "Restore routes for every Undo").
- **Approval items carry the tool and its arguments** (B1) and a 24-hour lapse (C-7); a device sign-in
  writes a Needs you item and sign-ins expire after 90 days without use (C-4). These are the web's
  server changes (`../../web/wave-1/02-dialogs-approvals.md`, `../../web/wave-1/05-sign-in-safety.md`),
  reused by the phone's own HQ.

## 10. Acceptance checks (each fails on today's build)

1. Compose test over every `ConfirmWords` of kind Red: the action is right of the safe button at font
   scale 1.0 with labels of 18 characters or fewer, and below it at 1.5. Today the Stop dialog stacks with
   the green action on top.
2. No Red dialog draws `PrimaryButton` or `Mts.BrandGreen`. Today `HelperPages.kt:301` does.
3. A unit test fails when a Red action label does not contain the title's verb, or is Yes, OK, Continue or
   Confirm. Today "Yes, stop it" and "Continue" fail it.
4. Signing one device out and Disconnect on a server HQ open a dialog before any request is sent. Today
   both act at once.
5. An approval fixture with `loses = true`: the Inbox card shows red "Allow delete" on the right, and the
   posted notification has exactly one action, Open, also with details on. Today it is green Allow once.
6. On an API 28 emulator the Undo snackbar stays 10 000 ms. On API 29+ it equals
   `getRecommendedTimeoutMillis(5000, …)` with a 30 s accessibility timeout set. Today it is 5 000 ms on
   both.
7. Discarding a waiting task shows "Task discarded" · Undo, and Undo brings it back. Today it is deleted
   at once.
8. Kill HQ during a "3 updates cleared" snackbar: the change shows under "Waiting to send" and reaches HQ after
   it starts. Check on device.
9. A grep for "HQ didn't reply", "HQ is not answering" and "Could not sign out" in `helpers/`,
   `settings/`, `pairing/` and `inbox/` finds nothing. Today it finds them in HelperWords,
   HelpersViewModel, SettingsWords and others.
10. Back, the back gesture and a scrim tap call `onKeep` and never `onAct`, in every dialog (a Compose
    test per dialog).
11. An approval card for `vault_delete {"path": "Projects/Old plan.md"}` shows the tool name and
    arguments above "In HQ's words" (B1). Today only the request's prose shows.
12. Sign out one device reads 'Sign out "Safari on macOS"?' / "It can no longer open HQ until it signs
    in again."; Sign out everywhere's body is "Every other device is signed out. This one stays signed
    in."; removing a paired device ends with "Work laptop removed." (B2, B3, B18).
13. A restore sent 601 s after the change shows "Couldn't undo. It changed on another device, or too
    much time passed." (added by resolutions.md C-1, A6). Today there is no window.
14. Phone as the hub: a code sign-in from a browser posts the Needs you card with It was me and an ink
    Sign it out; the code screen shows "Type this only into HQ on your own device." (added by
    resolutions.md C-4).
15. With a lockdown on the Mac mini, Helpers shows "All helpers are stopped. Allow them again in HQ Hub
    on that computer." and no Allow button (added by resolutions.md C-5).
16. An approval fixture 24 h and 1 min old shows "This request ran out. Ask again from the helper." and
    no buttons (added by resolutions.md C-7).
17. Marking one task with no dependents done shows "Done: {title}" with Undo and no dialog; marking
    three asks first and ends with "3 tasks done" (B8).

## 11. Check on device

- Undo across an HQ stop (FixPlan "Still to prove"; acceptance check 8).
- Android 8 and 9 Undo: the 8.1 emulator and an Android Go phone.
- On the Galaxy A32, every dialog stacks at font scale 1.5.

## 12. Open questions and decisions it depends on

Open questions for the owner, with the board's recommendation:

1. **Who marks a request as losing?** Recommendation: HQ's runtime, from the tool's category, never the
   model and never the phone guessing from the text. Until HQ sends the flag, every request keeps today's
   words, and the shade keeps Open only on 8 to 11. (Decided: N17.)
2. **Should "Sign out everywhere" ask twice?** Recommendation: no. One Red dialog that says every device
   needs a new code is enough. A second step mostly trains people to tap through. (Decided: D3, "One red
   dialog.")
3. **Add a restore action for a cleared update on HQ?** Recommendation: yes, a small route that sets a
   dismissed item back. It makes Undo work across the phone and the web, as rules section 4 asks.
   (Decided: N18.)

Depends on: D2, D3, N17, N18, N23, `../../resolutions.md` A2, A6, B1, B2, B3, B8, B18, B19, C-1, C-4,
C-5, C-7; rules sections 2, 3, 4; `../wave-0/01-lock-screen.md` (Open only in the
shade); `../wave-0/04-measure.md` (`undo.{kind}` counters); `../wave-2/03-assistant-model.md` (Delete
speech model).
