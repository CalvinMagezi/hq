# Wave 3 · 02 · The assistant on a locked phone (finding #3, R2, R6)

Board: **FixAssistantLocked** ("Fix: assistant when locked"), finding #3, design only, revised after
review, 5 Nov 2026. Shared rules: `../../rules-shared.md` sections 1, 9, 10 win.

Headline: **Holding Home on a locked phone asks for the unlock, then listens: the press is kept, the sheet
starts empty, and nothing of HQ shows before the owner is in.**

**Precondition (R6).** Nothing here is built until a locked Home hold is logged on the Galaxy A32 and a
Pixel image. The result decides between the main design and the fallback. FixPlan: wave 3 "starts only
after the locked Home hold is logged on the A32 and a Pixel image".

## 1. Finding and the review items it closes

**Problem.** Locked, a Home hold does nothing: no unlock prompt, no message, and the microphone stays off.
The sheet's own unlock-first path is never reached, so the feature looks broken.

**Who it hurts, how often.** Every owner who reaches for the assistant with the phone on a desk or just out
of a pocket, which is the moment an assistant is for. Every locked press today, on every phone with a
screen lock.

What changed in this revision:

- **R6:** a logged locked Home hold on the A32 and a Pixel image is now a precondition. The fallback no
  longer repeats the finding: the press shows Android's own unlock, and Settings says why (After 8 to
  10).
- **R2:** a locked press always starts a fresh, empty sheet. Only the unlock bar is drawn until Android
  reports the unlock. New acceptance check: ask, lock, hold Home (rule 2; Before B shows the risk).
- **#3, pin B reasoning fixed:** `Assistant.kt:47` returns `hide()` while locked, so `VoiceActivity` is
  never started. The review cited line 45; the code has it at 47.
- **Android 8.0:** `setShowWhenLocked` is API 27, so 8.0 uses the `FLAG_SHOW_WHEN_LOCKED` window flag.
  Versions card added.
- **M3:** phones with no secure screen lock keep the lock-screen path closed (After 7).
- **R1:** the lock-screen notice is "HQ answered · Tap to read." with no answer text (After 6).
- **Added:** a 360 dp frame at font scale 2.0 (After 11), letter pins on Before, grey facts instead of
  green, and acceptance checks that fail on today's build.

## 2. Before (as built, verified at `c04a100`)

Paths under `apps/android/app/src/main/`.

- `kotlin/.../voice/Assistant.kt:44-50` `AssistantSession.onShow`: line 46 comment "Locked: nothing opens
  and the microphone stays off. The owner unlocks and asks again."; line 47 `if (… isKeyguardLocked)
  return hide()`.
- `kotlin/.../voice/VoiceActivity.kt:108` `unlockThen` would ask for the unlock but is never reached while
  locked; the activity does not allow showing when locked. `:77` `onNewIntent`; `:102` `micPressed`.
- `AndroidManifest.xml:80` `VoiceActivity` is `launchMode="singleTask"` (one reused screen).
- `res/xml/voice_interaction_service.xml:7` `android:supportsLaunchVoiceAssistFromKeyguard="false"`.
- `kotlin/.../voice/VoiceSession.kt:131` `VoiceState(VoicePhase.Preparing, reply = it.reply)` keeps the
  last reply.

### Before frames

- **Before A · Phone locked: holding Home.** "No sheet opens" / "The microphone stays off and nothing says
  why. Unlock, then hold Home again." (pins A, B).
- **Before B · If only show-when-locked were added** (a risk, not built). Lock screen "9:41 · Thursday 5
  November" with the old sheet drawn over it: "Getting ready" / "What is on my list today" / "You have
  three tasks due today. The dentist call is due at 4 pm, and the backup review and the invoice draft are
  in progress." / [Wait] (pin C).

Pins: **A** nothing at all (`AssistantSession.onShow`, `voice/Assistant.kt:47`, checks `isKeyguardLocked`
and returns `hide()` before it starts anything). **B** the unlock-first code is never reached
(`VoiceActivity.unlockThen`, `VoiceActivity.kt:108`; and `supportsLaunchVoiceAssistFromKeyguard="false"`).
**C** the sheet is one reused screen (R2): `singleTask` (`AndroidManifest.xml:80`), a new press arrives
through `onNewIntent` with the old state, and `Preparing` keeps the last reply (`VoiceSession.kt:131`).
Adding show-when-locked alone would draw the last question and answer over the lock screen.

## 3. Decision

- **Unlock first, then listen, in one press.** A locked Home hold brings up Android's own unlock prompt
  straight away. On success a new sheet opens and listens; the owner does not hold Home a second time.
- **A locked press is always a new, empty sheet (R2).** Over the lock screen the app draws one line,
  "Unlock to ask HQ", and nothing else. No question, no answer, no notes, no microphone.
- **Built only after the device log (R6).** If HQ's window cannot show, the press goes straight to
  Android's unlock. If the press never reaches HQ, Settings says why.
- **No secure lock, no lock-screen path (M3).** A swipe is not an unlock. The bar points to Settings, and
  the press is not kept.
- **Also opened up:** the lock screen's own assist shortcut, where a phone offers one, goes through the
  same unlock-first path.

Rejected: **answering on the lock screen.** Anyone holding the phone could ask about the owner's notes,
tasks and helpers, and start tools that spend money.

### Precondition before wave 3 (R6)

- **The log.** With HQ as the assistant and a PIN set, screen on at the lock screen: hold Home, and on
  gesture phones swipe from a bottom corner. A debug build writes one line under `HqVoice`:
  `assist while locked: onShow=yes|no window=shown|behind prompt=yes|no`. Run on the Galaxy A32 (13), a
  Pixel emulator image (14) and the 8.1 emulator. The results and build go on the board before wave 3
  starts.
- **Pass** (onShow yes, window shown, prompt yes): build After 1 to 7.
- **Fail A** (onShow yes, HQ's window cannot come over the lock screen): HQ draws nothing. The press asks
  Android for the unlock directly (After 8), then the fresh sheet listens. Settings shows card A (After
  9). The owner still gets the unlock and the press still counts, so the finding does not come back.
- **Fail B** (onShow never arrives while locked): no HQ code runs at the press. The lock-screen assistant
  kill switch (rules section 10) stays off for that model. Settings says why (After 10). Where the lock
  screen has its own assistant shortcut, that shortcut still gets the unlock-first path.

## 4. Screens and states (After)

1. **The press, still locked: a fresh sheet.** Lock screen "9:41 · Thursday 5 November", dimmed, with one
   bar: lock icon and "Unlock to ask HQ" (pins 1, 2). Nothing else.
2. **Android's unlock prompt, at once** (generic, pin 3). "Enter PIN", keypad 1 to 0, "Emergency call",
   "Cancel" (pin 5).
3. **Unlocked: empty, and listening** (pins 4, 2). The sheet shows "Listening" and [Done]; no earlier
   question or answer.
4. **Unlock cancelled** (pin 5). Back to the lock screen "9:41 · Thursday 5 November", nothing drawn.
5. **First locked press: then the microphone** (pin 6). After the unlock: "Tap to ask" / [Ask], then
   Android's dialog (generic) "Allow AgentHQ to record audio?" / "While using the app" / "Only this time"
   / "Don't allow".
6. **Locked again while HQ answers** (pin 7). Lock screen "9:43 · Thursday 5 November", notice "AgentHQ ·
   Updates · now / HQ answered / Tap to read." No answer text.
7. **No secure screen lock (M3)** (pin 8). "Swipe lock, no PIN": lock screen with the bar "Set a screen
   lock to ask HQ from here." / "Or unlock, then hold Home." It goes after 4 s or a touch.
8. **Fallback A: Android's unlock, no HQ window** (pin 9). Android's PIN prompt directly over the lock
   screen, nothing of HQ.
9. **Settings after fallback A.** Phone assistant page: "HQ is your phone's assistant / Hold Home on any
   screen to ask. Understands English."; "Right now": "Assistant app · AgentHQ", "Microphone · Allowed",
   "Speech model · On this phone · [U] MB" (grey facts). In the card slot (pin 10): "When the phone is
   locked / On this phone, Android asks for your unlock first. Then HQ listens."
10. **Settings after fallback B.** Same page; card (pin 10): "When the phone is locked / This phone does
    not let other apps answer a locked Home hold. Unlock first, then hold Home."
11. **360 dp at font scale 2.0** (pin 11). The bar "Unlock to ask HQ" wraps to two lines and the lock icon
    stays at the start of the first.

## 5. Exact copy

| Where | Words |
|-------|-------|
| Over the lock screen | "Unlock to ask HQ" |
| TalkBack, same bar | "Unlock to ask HQ" (announced as a status) |
| No secure lock | "Set a screen lock to ask HQ from here." · "Or unlock, then hold Home." |
| Cancelled | (nothing: the lock screen returns) |
| Answer notice (was "HQ answered your question · Unlock to read it.") | "HQ answered" · "Tap to read." |
| Card A title and body | "When the phone is locked" · "On this phone, Android asks for your unlock first. Then HQ listens." |
| Card B title and body | "When the phone is locked" · "This phone does not let other apps answer a locked Home hold. Unlock first, then hold Home." |
| Retired (it repeated the finding) | "On this phone, unlock first, then hold Home. Android does not pass a locked press to HQ here." |

"Enter PIN", Emergency call and Cancel are Android's and vary by phone. Server HQ owners see the same
words; the answer lives on the server's thread.

## 6. Behaviour rules

1. **The only thing drawn over the lock screen** is the dimmed screen and one bar: lock icon and "Unlock
   to ask HQ". TalkBack announces it as a status. Most phones put the prompt over it at once.
2. **A fresh, empty sheet (R2).** A locked press finishes any open sheet and starts a new one with an
   empty state and no thread. The sheet's content is not composed until Android reports the unlock. The
   earlier question, answer and thread are never drawn, before or after it. The hub may warm up, but no
   chat frame is sent and the recogniser is not fed.
3. Android's prompt: PIN, pattern, password, fingerprint or face, whatever the owner set. Target: under
   300 ms from the press.
4. **Unlocked: the press counts.** The new sheet goes to listening. If the microphone or the model is
   missing, those steps come first (rule 6, FixAssistantModel).
5. **Cancel, Back, too many tries, or the screen going off:** the window closes and the lock screen is left
   as it was. Nothing is kept, no notification. The "locked presses unlocked or cancelled" counter (rules
   section 10) counts it.
6. The very first locked press ends in Android's microphone dialog after the unlock, never before it.
7. **Locked again mid-answer:** the sheet closes, HQ keeps answering (FixAssistantHandoff). The notice
   reads "HQ answered · Tap to read." with no answer text, on the lock screen and everywhere else (R1).
8. **No secure lock** (`isDeviceSecure` false, a swipe or none): the bar reads "Set a screen lock to ask HQ
   from here." and "Or unlock, then hold Home." `requestDismissKeyguard` is not called, since it would
   dismiss a swipe lock at once. The press is not kept. The bar goes after 4 s or a touch.
9. **Fallback A:** no HQ window over the lock screen. The press asks Android for the unlock directly, then
   the fresh sheet listens.
10. **The Settings card** sits in FixAssistantSetup's card slot, and only on models listed as failing in
    the signed channel file, beside the lock-screen assistant kill switch. Card A or B, by the logged
    result.
11. **360 dp at font scale 2.0:** the bar wraps to two lines and the icon stays at the start of the first.

- A second Home hold while the prompt is up is ignored.

## 7. Android versions

| Version | What differs for this fix |
|---------|---------------------------|
| 8 and 9 | 8.0 (API 26): `setShowWhenLocked` does not exist, so the window gets `FLAG_SHOW_WHEN_LOCKED`. 8.1 and 9: `setShowWhenLocked(true)`. `requestDismissKeyguard` works from 8.0. The session opens the sheet with `startActivity` and `FLAG_ACTIVITY_NEW_TASK`; background start limits begin at 10. Tested on the 8.1 emulator and an 8.0 image. |
| 10 and 11 | `startAssistantActivity` (API 29) opens the sheet; it is allowed while locked. Otherwise as 8.1. |
| 12 | No change to the locked path. The microphone switch belongs to FixAssistantMic. |
| 13+ | The "HQ answered" notice needs notification permission. Without it, FixAssistantHandoff's next-open line covers the gap. |

Android constraints, as checked against the platform APIs:

- The session can start an activity while locked, but Android keeps it behind the lock screen unless that
  activity allows showing when locked. `Activity.setShowWhenLocked(true)` is API 27 (8.1). On 8.0 the
  window gets `WindowManager.LayoutParams.FLAG_SHOW_WHEN_LOCKED`. Either is set in code for a locked press
  only, never in the manifest, and cleared once unlocked.
- `KeyguardManager.requestDismissKeyguard(activity, callback)` (API 26) needs a showing activity and
  raises Android's own prompt. Three outcomes: succeeded, cancelled, error.
- `isKeyguardLocked` picks the path; `isDeviceSecure` (API 23) tells a real lock from a swipe.
- `startAssistantActivity` (API 29) from the session is allowed while locked and exempt from background
  start limits. On 8 and 9 those limits do not exist.
- `supportsLaunchVoiceAssistFromKeyguard="true"` plus `onLaunchVoiceAssistFromKeyguard()` (API 23) is how
  a lock screen's assist shortcut reaches HQ.
- Fallback A uses the old `FLAG_DISMISS_KEYGUARD` window flag, deprecated since 8.0. Whether One UI still
  honours it: check on device.
- Not known until the log: whether One UI 5 or stock Android calls `onShow` for a locked Home hold.
- The microphone: HQ chooses not to record before the unlock. Android would allow it for a visible
  activity, so this rule lives in our code and has a test.

## 8. Code touchpoints

| File | Change | Exists |
|------|--------|--------|
| `voice/Assistant.kt` | `onShow` drops the early return at line 47. Locked, it starts `VoiceActivity` with `EXTRA_LOCKED` and `NEW_TASK | CLEAR_TASK`. `AssistantService.onLaunchVoiceAssistFromKeyguard()` starts the same intent. | yes |
| `res/xml/voice_interaction_service.xml` | `supportsLaunchVoiceAssistFromKeyguard="true"` (line 7) and its comment. | yes |
| `voice/VoiceActivity.kt` | On a locked intent, a new `VoiceSession` and `unlocked = false` before the first frame, in `onCreate` and `onNewIntent` (line 77). Show-when-locked by version; the `UnlockBar`; the `isDeviceSecure` branch; `micPressed` (line 102) guarded until unlocked. | yes |
| `voice/VoiceSession.kt` | No carried `reply` into `Preparing` for a new sheet (line 131). | yes |
| `res/values/strings.xml` | The bar, the no-lock bar and both cards. | yes |
| The channel file (FixUpdates, FixSafeMode) | The lock-screen assistant switch with its list of failing models. | NEW field (`switches.lock_screen_assistant` exists in FixSafeMode's example as a boolean; the per-model list is not drawn there) |
| `apps/android/README.md` | "It does not open over the lock screen" becomes the unlock-first behaviour. | yes |
| Debug logging | The `HqVoice` line `assist while locked: onShow=… window=… prompt=…` for the precondition. | NEW |

## 9. Server, release or desktop changes

- The signed channel file's `lock_screen_assistant` switch, with a list of failing phone models (rule 10).
  FixSafeMode's example has the switch as a single boolean; the model list is not shaped anywhere yet. See
  "Open conflicts" in `../README.md`.

## 10. Acceptance checks (each fails on today's build)

1. **First, the precondition:** the three log lines from the A32, the Pixel image and the 8.1 emulator are
   on the board.
2. PIN set, at the lock screen: hold Home. Android's prompt shows within half a second. Enter the PIN: the
   sheet is listening with no second press (`adb logcat -s HqVoice` shows `phase=Listening` after the
   unlock).
3. **R2:** ask "what is on my list today" and wait for the answer. Press the power key, wake, hold Home. In
   `adb shell screenrecord`, frame by frame, nothing of that question or answer appears before the unlock.
   After it, the sheet is empty and listening.
4. Cancel at the prompt: back to the lock screen; the status-bar microphone dot never lit.
5. Fingerprint and face unlock: the same as the PIN.
6. **M3:** swipe-only lock, hold Home: the bar reads "Set a screen lock to ask HQ from here." No sheet
   opens after the swipe, and the microphone dot never lit.
7. Power key while HQ answers: on wake the lock screen shows "HQ answered · Tap to read." `adb shell
   dumpsys notification --noredact` holds no answer words.
8. 8.0 image and 8.1 emulator: the PIN check passes; the 8.0 log shows the window flag path.
9. Android 14 emulator at 360 dp and font scale 2.0: the bar's text is not cut off.

## 11. Check on device

- **The R6 log** on the A32 (13), a Pixel emulator image (14) and the 8.1 emulator: Pass, Fail A or Fail
  B. (FixPlan "Still to prove" for R6: "A32 and Pixel log".)
- Whether One UI still honours `FLAG_DISMISS_KEYGUARD` (Fail A path).
- The R2 screen-record check (FixPlan "Still to prove" for R2).

## 12. Open questions and decisions it depends on

- **New: if the A32 log is Fail B.** Ship wave 3 with the unlocked assistant only on that model, or hold
  wave 3 until a way in is found. Recommendation: ship it. Keep the lock-screen kill switch off for the
  failing models and show card B. Holding Home after unlocking works today and is worth having. (Decided:
  N21, "Ship the unlocked assistant; lock-screen switch off on that model.")
- **For later:** should a locked press ever answer questions that need no private data, like the time?
  Recommendation: no. HQ cannot know in advance which tools a question will start. (D9, "Answers while
  locked: No.")

Depends on: D9, N21, rules sections 1, 9, 10; `01-assistant-setup.md` (card slot);
`04-assistant-handoff.md` (answer notice); `../wave-0/04-measure.md` (`locked_press.*`);
`../wave-0/02-safe-mode.md` (the switch).
