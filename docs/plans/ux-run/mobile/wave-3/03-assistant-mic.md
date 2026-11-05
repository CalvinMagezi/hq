# Wave 3 · 03 · The assistant microphone (finding #12)

Board: **FixAssistantMic** ("Fix: assistant microphone"), finding #12, design only, revised after review,
5 Nov 2026. Shared rules: `../../rules-shared.md` sections 3, 7, 8, 9 win.

Headline: **Microphone refused is no longer a dead end: each refusal says what happens next, a dismissed
dialog is not a refusal, and once Android stops asking, the sheet opens the page that turns it back on.**

## 1. Finding and the review items it closes

**Problem.** After a second refusal Android stops showing its dialog. The sheet repeats one red line with
no way to settings, so the owner has to find App info, Permissions, Microphone alone, or decide voice is
broken.

**Who it hurts, how often.** Anyone who taps Don't allow by reflex the first time, then again to make the
dialog go away. Rare per owner, but permanent until fixed: every press after that fails the same way.

What changed in this revision:

- **#12, Android 8 and 9:** telling a silenced microphone apart is API 29. Below it, HQ checks the
  recording state after it starts recording, and silence counts as "did not catch that" (rule 13,
  versions card). (Review #12: "Silence detection is missing below Android 10; not said.")
- **Type instead** also shows after two "did not catch that" in a row (After 8, 9). The microphone switch
  hint shows only on 12+, where the switch exists.
- **A dismissed dialog is not a refusal.** Only Don't allow counts, the same rule as FixNotifications
  (rule 4).
- **Words and colour per the rules:** "Open Android settings" with the taps named, problem lines amber with
  an icon, grey facts, Chat opens with the keyboard (`hq://chat?compose=1`). No notification is posted on
  any channel from this fix.
- **Added:** Android 8 to 10's dialog with "Don't ask again" (After 13), the versions card, a 360 dp frame
  at font scale 2.0 (After 12), letter pins, and acceptance checks that fail on today's build.

## 2. Before (as built, verified at `c04a100`)

Paths under `apps/android/app/src/main/kotlin/example/hq/spike/voice/`.

- `VoiceActivity.kt:58-59` `askMic = registerForActivityResult(RequestPermission()) { granted -> if
  (granted) session.listen() else session.refuse(MIC_REFUSED) }`: one line for every refusal.
- `VoiceActivity.kt:104` `micPressed()` launches `askMic` again; after the second refusal Android returns
  denied with no UI.
- `VoiceActivity.kt:54` `dismissing` flag; `:90-94` `onStop` finishes the sheet unless dismissing.
- `VoiceSession.kt:89` `refuse(problem)`; `:135` `ears.hear(...)`.
- `Ears.kt:37` `class Busy`; `:73` `mic.startRecording()`.
- `ui/shell/Links.kt` has no `compose` parameter for `hq://chat`.

### Before frame

- **Before · Microphone refused.** "Tap to ask" / red line "AgentHQ needs the microphone to hear you.
  Nothing you say leaves this phone as sound." (pin A) / round button [Ask] (pin B).

Pins: **A** one red line for every refusal (`VoiceActivity.MIC_REFUSED`), after the first refusal and the
tenth, worded as a fact, with no next step. A dismissed dialog lands on the same line. **B** Ask asks
Android again; after the second refusal Android answers "denied" at once without showing anything, so the
tap appears to do nothing.

## 3. Decision

- **Three steps, each with the next action.** First Android's own dialog, as now. After one refusal, the
  sheet says why it needs the microphone and the round button becomes Allow. Once Android has stopped
  asking, the sheet says so and Open Android settings goes to AgentHQ's page, then back to the sheet.
- **A dismissed dialog is not a refusal.** Closing Android's dialog is not saying no, so nothing changes.
- **Coming back with it on listens straight away:** the owner came back to ask something.
- **Type instead** on every refusal state, and after two silent tries, so the owner can still ask HQ.
- **The near-misses get their own words:** the phone's microphone switch, a phone call, another app
  holding the microphone. Below Android 10 HQ can tell the last one only when recording fails to start.

Rejected:

- **HQ's own explainer before Android's first dialog.** It adds a tap for everyone. The explainer appears
  only after a refusal, which is when Android's guidance says to show one.
- **Asking in first run.** Out of context, more people refuse, and many owners never use voice.

### Counting refusals the same way as FixNotifications

- **Only Don't allow counts.** HQ reads `shouldShowRequestPermissionRationale` before and after each
  request, and how long the request took.
- **False before, true after:** one refusal, "Microphone needed".
- **True before, false after:** Android stopped asking, "Microphone off".
- **False before, false after, and a dialog was on screen** (the result took longer than about 300 ms):
  the owner dismissed it. Nothing is counted; the sheet stays at "Tap to ask".
- **Fast with no dialog:** Android answered by itself, "Microphone off".
- Nothing about refusals is stored. The 300 ms line is a guess: check on device.

## 4. Screens and states (After)

The voice sheet as built: a label at the top, body text, the round button at the bottom with a text
button beside or under it. Problem lines are amber with an icon (rules section 3), never red.

1. **First press: Android asks (11+).** Sheet "Tap to ask" / [Ask]; Android's dialog (generic) "Allow
   AgentHQ to record audio?" / "While using the app" / "Only this time" / "Don't allow" (pins 1, 4).
2. **Refused once: say why, ask again** (pin 2). Label "Microphone needed" / "HQ needs the microphone to
   hear your question. Your voice is turned into words on this phone. The sound is never kept." / text
   button [Type instead] (pin 10) / round button [Allow] (TalkBack "Allow microphone").
3. **Android will not ask again** (pin 3). Label "Microphone off" (amber with icon) / "Android will not
   ask again. Turn the microphone on in Android's settings." / "Tap Permissions, then Microphone, then
   Allow only while using the app." / [Open Android settings] (pin 5) / [Ask again] (pin 6) / [Type
   instead].
4. **Android's page it opens (generic)** (pin 7). "Microphone permission · AgentHQ": "Allow only while
   using the app", "Ask every time", "Don't allow".
5. **Back in the sheet, allowed: it listens** (pin 8). "Listening" / "What is on my list today" / [Done].
6. **Back, still off** (pin 9). "Microphone off" with "The microphone is still off." above (amber with
   icon), then the frame 3 body, taps and buttons.
7. **The phone's microphone switch is off (12+)** (pin 11). Sheet "Listening · Done" behind Android's
   dialog (generic) "Unblock device microphone? / This unblocks access for all apps and services that are
   allowed to use your microphone. / Cancel · Unblock". HQ draws nothing for it.
8. **Silence twice in a row (12+)** (pin 12). "Tap to ask" / "I did not catch that. If the phone's
   microphone switch is off, turn it on in Quick settings." / [Type instead] / [Ask].
9. **Silence twice in a row (8 to 11)** (pin 12). "Tap to ask" / "I did not catch that. Speak close to
   the phone, or type instead." / [Type instead] / [Ask].
10. **On a call, or another app has it** (pin 13). "Tap to ask" / "You are on a call. Ask HQ when it
    ends." / [Type instead] / [Ask].
11. **Settings, Phone assistant** (pin 14). Intro "When HQ is your phone's assistant, holding Home opens a
    small sheet over any app. Speak, and HQ answers in text. Understands English." "Right now": "Assistant
    app · AgentHQ", "Microphone · Off in Android" (amber with icon) with [Open Android settings], "Speech
    model · On this phone · [U] MB". Line "Tap Permissions, then Microphone, then Allow only while using
    the app." "On Settings home this reads: Phone assistant · On · microphone off" (grey).
12. **360 dp at font scale 2.0** (pin 16). "Microphone off" / body / [Open Android settings], then [Ask
    again] and [Type instead] stacked under it, full width.
13. **Android 8 to 10: the second ask** (pin 15). Sheet "Microphone needed · Allow" behind Android's dialog
    (generic) "Allow AgentHQ to record audio?" with a "Don't ask again" checkbox, [Deny] [Allow].

## 5. Exact copy

| Where | Words |
|-------|-------|
| Retired | "AgentHQ needs the microphone to hear you. Nothing you say leaves this phone as sound." (replaced by the two states below) |
| Refused once, label | "Microphone needed" |
| Refused once, body | "HQ needs the microphone to hear your question. Your voice is turned into words on this phone. The sound is never kept." |
| Refused once, button | "Allow" |
| Blocked, title | "Microphone off" |
| Blocked, body | "Android will not ask again. Turn the microphone on in Android's settings." |
| Blocked, the taps | "Tap Permissions, then Microphone, then Allow only while using the app." |
| Buttons (was "Open settings · Ask again · Type instead") | "Open Android settings" · "Ask again" · "Type instead" |
| Still off | "The microphone is still off." |
| Page did not open | "Open Android settings, Apps, AgentHQ, Permissions." |
| Silence, first | "I did not catch that. Tap and try again." (kept) |
| Silence, second in a row, 12+ | "I did not catch that. If the phone's microphone switch is off, turn it on in Quick settings." · "Type instead" |
| Silence, second in a row, 8 to 11 | "I did not catch that. Speak close to the phone, or type instead." · "Type instead" |
| On a call | "You are on a call. Ask HQ when it ends." |
| Another app recording | "Another app is using the microphone. Try again when it is done." |
| Settings fact | "Microphone" · "Off in Android" · "Open Android settings" · "Tap Permissions, then Microphone, then Allow only while using the app." |
| Settings home row | "Phone assistant" · "On · microphone off" |
| TalkBack, round button | "Allow microphone" |

"AgentHQ" stays where Android shows that name. "Don't allow", "Deny", "Don't ask again", "Unblock device
microphone?" and the permission page are Android's own words and vary by phone. Every HQ line is a whole
sentence with its own key in `res/values/strings.xml`.

## 6. Behaviour rules

1. **First press with no permission:** Android's dialog straight away, as built. Allow (either kind)
   listens at once.
2. **One refusal** (rationale false before, true after): "Microphone needed". Allow asks Android again.
3. **Android will not ask again** (rationale true before and false after, or an instant denial with no
   dialog): "Microphone off".
4. **Dismissed, not refused:** rationale false before and after, with a dialog on screen. Nothing changes:
   the sheet stays at "Tap to ask", and the next press shows Android's dialog again.
5. **Open Android settings** opens AgentHQ's App info page in the sheet's own task. The sheet does not
   close on this `onStop`, so Back returns to it.
6. **Ask again** asks Android once more. If Android shows nothing, the sheet stays as it is.
7. The owner taps Permissions, Microphone, then a choice. The sheet's line names the same three taps.
8. **On return, allowed:** listening starts within a second, only because the owner went to settings from
   the sheet. A permission that turns on any other way never opens the microphone by itself.
9. **On return, still off:** the same state with "The microphone is still off." above it, amber with an
   icon.
10. **Type instead** closes the sheet and opens `hq://chat?compose=1`: Chat with the composer focused and
    the keyboard up, never the microphone.
11. **Android's microphone switch** (12+): Android shows its own unblock dialog when recording starts. HQ
    draws nothing for it.
12. **Silence.** One silent try keeps the short line. Two in a row: the hint and Type instead. On 12+ the
    hint names the microphone switch; on 8 to 11 it says to speak closer.
13. **Before the microphone opens:** on a call (`getMode` is in-call or in-communication), the sheet does
    not try. **Another app recording:** on 10+ `isClientSilenced`; on 8 and 9 the recording state after
    `startRecording()`.
14. **Settings** shows the microphone line amber with its icon and Open Android settings. The Settings
    home row reads "On · microphone off" and stays grey.
15. **Android 8 to 10:** the second dialog has "Don't ask again". Ticking it is what blocks; the rules
    above read the same.
16. **Font scale 1.5 and up:** Ask again and Type instead stack under Open Android settings, full width.

"Only this time" (11+) ends when the sheet closes; the next press shows Android's dialog again. That is
Android's design and needs no words from HQ.

## 7. Android versions

| Version | What differs for this fix |
|---------|---------------------------|
| 8 and 9 | The dialog is Deny or Allow; the second time it adds "Don't ask again", and only ticking it blocks (After 13). No "Only this time". **No silence detection:** `isClientSilenced` is API 29. HQ checks `AudioRecord.getRecordingState()` right after `startRecording()`; not recording means another app has the microphone. Any other silence is "did not catch that", and two in a row offer Type instead. No microphone switch, so the hint never names one (After 9). |
| 10 and 11 | 10: as 9 for the dialog, plus `getActiveRecordingConfigurations()` with `isClientSilenced()` to name another app. 11: "Only this time" arrives, and a second Don't allow blocks without a checkbox. A dismissal does not count toward the two. |
| 12 | The microphone privacy switch: Android shows its own unblock dialog when recording starts (After 7). The silence hint names the switch (After 8). |
| 13+ | As 12. The notification permission plays no part here; this fix posts no notification. |

The rationale logic in rule 4 is the same on every version.

Android constraints, as checked against the platform APIs:

- **When Android stops asking.** From Android 11, a second Don't allow means no more dialogs. On 8 to 10
  it stops only when "Don't ask again" is ticked. Then `requestPermissions` returns denied with no UI.
- `shouldShowRequestPermissionRationale(RECORD_AUDIO)` is false before the first request, true after one
  refusal, and false again once Android has stopped asking.
- **A dismissed dialog** (a tap outside, Back) returns denied without counting as a refusal on 11+; the
  rationale does not change.
- **No intent opens the microphone permission itself.** `Settings.ACTION_APPLICATION_DETAILS_SETTINGS`
  with `package:` opens App info, two taps away. Started without `FLAG_ACTIVITY_NEW_TASK` so it joins the
  sheet's task.
- **The privacy switch** (Android 12, API 31): Android shows its own unblock dialog to a foreground app
  that starts recording, and otherwise delivers silence. Apps cannot read its state, so HQ can only hint
  after silence.
- **Telling silence apart:** `AudioRecordingConfiguration.isClientSilenced()` is API 29. On 8 and 9, a
  second recorder either fails to start or records silence, by phone.
- `AudioManager.getMode()` returns in-call or in-communication during a call on every version, with no
  permission.
- "Only this time" (11+) is a one-time grant Android revokes soon after the app leaves the screen.

## 8. Code touchpoints

| File | Change | Exists |
|------|--------|--------|
| `voice/VoiceActivity.kt` | `askMic` (line 58) reads the rationale before and after and the request's duration, then picks the state; `openAppSettings()` with an `awayToSettings` flag that `onStop` (line 90) respects like `dismissing` (line 54); `onResume` (line 83) re-checks and listens if the flag was set. | yes |
| `voice/VoiceSession.kt` | States `MicNeeded` and `MicBlocked` in place of `refuse(MIC_REFUSED)` (line 89); a silent-try counter; the call and busy checks before `ears.hear` (line 135). | yes |
| `voice/Ears.kt` | Report a silenced capture (10+) or a recorder that did not start (8 and 9) as `Busy` (line 37). | yes |
| `ui/settings/PhoneIntents.kt` | `PhoneFix.Microphone` (App info). (`PhoneFix` is declared in `PhoneWords.kt:18`.) | yes |
| `ui/settings/AssistantWords.kt` | The fact and its action. | NEW (from `01-assistant-setup.md`) |
| `ui/shell/Links.kt` | `hq://chat?compose=1`; `LinksTest`. | yes (parameter NEW) |
| `res/values/strings.xml` | The strings. | yes |
| `ui/shell/Icons.kt` | `HqIcons.MicOff`. | yes (icon NEW) |

## 9. Server, release or desktop changes

None.

## 10. Acceptance checks (each fails on today's build)

1. **Galaxy A32 (13), fresh install.** Press, Don't allow: "Microphone needed". Allow, Don't allow:
   "Microphone off". Press again: "Microphone off" at once, no dialog.
2. **Dismissed:** tap outside Android's first dialog. The sheet stays at "Tap to ask"; the next press
   shows the dialog again.
3. Open Android settings, Permissions, Microphone, Allow only while using the app, Back: the sheet is
   listening. Back without changing it: "The microphone is still off."
4. **Android 8.1 emulator:** Deny, then Deny with "Don't ask again" ticked: "Microphone off", and Open
   Android settings opens App info.
5. A32, Quick settings, Microphone access off: Android's unblock dialog appears. Cancel, stay silent
   twice: the switch hint and Type instead show on the second.
6. 8.1 emulator with its virtual microphone muted, silent twice: the line without the switch hint, and
   Type instead.
7. During a phone call: "You are on a call." The status-bar microphone dot does not light for HQ.
8. Another app recording, on the A32 and on the 8.1 emulator: "Another app is using the microphone."
9. Type instead opens Chat with the keyboard up, and the microphone dot never lights.
10. Android 14 emulator at 360 dp and font scale 2.0, "Microphone off": no text cut off, all three buttons
    reachable.

## 11. Check on device

- How a Samsung on One UI reports a dismissed dialog (rationale unchanged), and whether the 300 ms line
  tells a dismissal from an instant denial.
- On 8 and 9, a second recorder: fails to start or records silence, by phone (the 8.1 emulator and a
  Transsion phone). FixPlan "Still to prove": "Two apps recording on 8 and 9".

## 12. Open questions and decisions it depends on

None on the board: every choice follows from Android's rules and the shared rules.

Depends on: rules sections 3, 7, 8 (`hq://chat?compose=1`), 9 (silence detection on 10+);
`01-assistant-setup.md` (`AssistantWords`, the microphone fact); `../wave-1/01-notifications.md` (the
same refusal rule).
