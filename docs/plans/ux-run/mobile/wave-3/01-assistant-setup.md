# Wave 3 · 01 · Assistant setup: say HQ can be the assistant, and walk there (finding #2)

Board: **FixAssistantSetup** ("Fix: assistant setup"), finding #2, design only, revised after review,
5 Nov 2026. Rules: `../../rules-shared.md` wins over this board wherever they differ (sections 3, 5, 7, 9
especially).

Headline: **The assistant cannot be found: say that HQ can be the phone's assistant, and walk the owner
to the one Android page that sets it.**

Wave 3 gate (FixPlan): wave 3 starts only after the locked Home hold is logged on the A32 and a Pixel
image (R6, see `02-assistant-locked.md`).

## 1. Finding and the review items it closes

**Problem.** Nothing in the app says HQ can answer when Home is held, and nothing links to Android's
Digital assistant app setting. The only instructions are in the README.

**Who it hurts, how often.** Every owner who has not read the README, which is nearly all of them: they
never meet Hey HQ at all. Owners who upgrade skip first run, so for them the Settings row is the only way
in.

What changed in this revision:

- **#2, Android 8 and 9:** HQ being the assistant is read with `VoiceInteractionService.isActiveService`
  on every version. RoleManager is API 29 and minSdk is 26, so it is never the main check. New After 11
  and 12 draw Android 8 and 9. (Review #2: "RoleManager needs Android 10 but the app runs on 8".)
- **Colour:** the chosen card has a check and a grey border, not green (After 4, 8). No green status
  anywhere. The Phone assistant row is never amber; only the page's microphone line can be (After 14).
- **Speech model:** the card says "Understands English". The model never downloads by itself: a tap,
  with the size shown first.
- **M3:** phones with no secure screen lock get "Set a screen lock to ask HQ from the lock screen" and
  "Anyone holding this phone can answer" (After 9).
- **Room for FixAssistantLocked:** one card slot under the facts holds its "When the phone is locked" card
  (After 8).
- **Shared screens:** Ready to start and Settings home redrawn as rules section 5 (After 1, 6, 15). Server
  HQ owners drawn (After 15).
- **Words:** Not now in place of Cancel, Open Android settings with the switch named, sentences of 15
  words or fewer, strings in resources.
- **Added:** the Android versions card, a 360 dp frame at font scale 2.0 (After 13), letter pins on
  Before, and acceptance checks that fail on today's build.

## 2. Before (as built, verified at `c04a100`)

Paths under `apps/android/app/src/main/kotlin/example/hq/spike/`.

- `ui/settings/SettingsViewModel.kt:271` `phoneRows()` lists Background work, Keep running,
  Notifications, Capture and widgets, Keys on this phone (lines 279-283). No assistant row.
- `ui/settings/PhoneWords.kt:18` `enum class PhoneFix { Battery, AutoRevoke, Notifications,
  SleepingApps }`; `ui/settings/PhoneIntents.kt` opens only those pages.
- `ui/firstrun/FirstRunScreens.kt:150` `StartStep`: one sentence and Start HQ.
- `voice/Assistant.kt:28` `AssistantService`; `:29-33` `onReady()` calls `setDisabledShowContext(SHOW_WITH_ASSIST
  or SHOW_WITH_SCREENSHOT)`; `:49` opens the sheet with `startAssistantActivity` on Q+, else
  `startActivity(… FLAG_ACTIVITY_NEW_TASK)`.
- No call to `RoleManager` or `isActiveService` anywhere in the app.
- `apps/android/README.md` "Hey HQ" section: the manual path (Settings, Apps, Choose default apps,
  Digital assistant app, AgentHQ).

### Before frames

- **Before A · Settings home.** This phone: "Background work · On · answers chats, runs tasks", "Keep
  running · Good · 1 thing to check", "Notifications · Needs you on · Updates on", "Capture and widgets ·
  Sharing opens a new note", "Keys on this phone · Add, replace or remove" (pin A). Your accounts: "AI
  service and keys · GitHub Copilot · 3 services", "Messaging apps · Telegram on · Discord on", "Other
  devices · 1 signed in". How HQ behaves: "Safety and limits · Read only".
- **Before B · First run: Ready to start.** "Ready to start" / "HQ will run all the time on this phone.
  Run it on one machine only: a second copy on another machine would answer the same chats twice." /
  [Start HQ] (pin B) / [Back].

Pins: **A** no assistant row (`PhoneIntents` opens only battery, unused-app, notification and
sleeping-app pages). **B** first run ends without a word about it; the README path is the only guide, and
One UI words it differently.

## 3. Decision

**Two doors, one destination.** An optional card on Ready to start, and a "Phone assistant" row and page in
Settings. Both show the same short "what to tap" sheet, then open Android's own page. Back in HQ, the app
asks Android whether it worked.

- **Why the sheet first:** Android's page cannot be changed or pointed at, Samsung hides the choice one
  level deeper, and Android's warning about reading the screen scares people off. Saying both before they
  leave costs one tap and saves the setup.
- **One check on every version.** `VoiceInteractionService.isActiveService` (API 21) answers "is HQ the
  assistant" from Android 8 up. RoleManager only adds an availability read on 10+.
- **Setting up spends nothing.** No microphone request and no download. The speech model is a separate
  tap, with its 57 MB shown first (FixAssistantModel).
- **No secure lock, no lock-screen path (M3).** The page says so plainly and links to Android's screen
  lock chooser.
- **Try it before choosing:** "Try it without setting up" opens the voice sheet straight from the app,
  without changing a phone setting.

Rejected:

- **Asking Android for the role.** `RoleManager.createRequestRoleIntent` does not exist on 8 and 9, and on
  10+ the assistant role is not one an app may request.
- **A first-run step of its own.** Most owners will not use voice on day one; a full step adds a screen
  for everyone. A card on the last screen is skipped by doing nothing.

## 4. Screens and states (After)

Cards on Ready to start are the shared option card (surface-100, hairline border, 12 dp radius, 16 dp
padding, a small "Optional" label, bold title, ink-muted body, an outlined pill button). The chosen state
adds a check icon and keeps a grey (line) border, never green (rules section 5).

1. **Ready to start, as rules section 5** (pin 1). "Ready to start". Card: "Optional" / "Use HQ as your
   phone's assistant" / "Hold Home on any screen and ask by voice. Understands English. You can also do
   this later in Settings." / [Set up]. Hint: "Run HQ on one machine only. A second copy would answer the
   same chats twice." [Start HQ] (green, pin 2), [Back].
2. **Set up: what to tap (Android 10+).** Sheet over Ready to start: title "Choose HQ in Android's
   settings"; numbered steps "1 Tap Digital assistant app. On Samsung, then tap Device assistance app.",
   "2 Choose AgentHQ.", "3 Tap OK."; note (pin 4) "Android will say the assistant can read what is on your
   screen. HQ turns that off and never reads your screen."; [Open Android settings] (pin 3) and [Not now].
3. **Android's page opens (generic).** "Digital assistant app" / "Device assistance app · Google" (pin 5)
   / "Assist settings": "Use text from screen · Allow the assist app to access the screen contents as
   text", "Use screenshot · Allow the assist app to access an image of the screen".
4. **Back in HQ: chosen** (pin 6). Card with check and grey border: "Optional" / "HQ is your phone's
   assistant" / "Hold Home on any screen to ask. Understands English. Before your first question, you tap
   to download the speech model, 57 MB." Then the hint, Start HQ, Back.
5. **Back in HQ: not chosen** (pin 7). Card "Optional" / "Use HQ as your phone's assistant" / "Not set
   yet. You can do it later in Settings, Phone assistant." / [Try again]. No warning colour.
6. **Settings home, as rules section 5.** This phone: Background work, Keep running, "Notifications ·
   Needs you on · Updates on · App updates on", "Phone assistant · Not set up" (grey, pin 8), Capture and
   widgets, Keys on this phone. Your accounts: AI service and keys, Messaging apps, Other devices. How HQ
   behaves: "Spending limits · $2.10 of $5.00 today", "Safety and limits · Read only". Your data: "Backup
   and about · HQ version 0.9.1".
7. **Phone assistant, not set up.** Page intro "When HQ is your phone's assistant, holding Home opens a
   small sheet over any app. Speak, and HQ answers in text. Understands English." Group "Right now":
   "Assistant app · Another app", "Microphone · Not asked yet", "Speech model · Not downloaded · 57 MB"
   (pins 9, 11). [Make HQ the assistant] (green, pin 3). [Try it without setting up] (pin 10). Note "On
   Samsung phones the side key opens Bixby or the power menu. Hold Home instead, or swipe up from a bottom
   corner if you use gestures." Privacy note "Your screen is never read. Your voice stays on this phone.
   Only the words of your question go to HQ."
8. **Phone assistant, set up.** Card (check, grey border) "HQ is your phone's assistant / Hold Home on any
   screen to ask. Understands English." "Right now": "Assistant app · AgentHQ", "Microphone · Allowed",
   "Speech model · On this phone · [U] MB". **Card slot** (pin 13): "Empty here. Holds FixAssistantLocked's
   "When the phone is locked" card on a phone model whose device check failed, or the screen-lock card
   (After 9)." [Try it now]. "To stop using HQ, choose another app in the same Android setting." [Open
   Android settings].
9. **No secure screen lock (M3).** As frame 8, and in the card slot (pin 14): "Set a screen lock to ask HQ
   from the lock screen" / "This phone has no screen lock. Anyone holding this phone can answer. HQ does
   not open over the lock screen until you set one." / [Open Android settings] / "Tap Screen lock, then
   choose a PIN, pattern or password."
10. **A phone that keeps the slot** (pin 12). "Not available on this phone" / "This phone keeps the
    assistant for its maker's apps. You can still ask HQ in the chat." / [Open the chat].
11. **Android 8 and 9: what to tap** (pin 15). Sheet "Choose HQ in Android's settings": "1 Tap Assist app.
    On Samsung, then tap Device assistance app.", "2 Choose AgentHQ.", "3 Tap OK.", the same note, [Open
    Android settings] [Not now].
12. **Android 8 and 9: Android's page (generic)** (pin 15). "Assist & voice input" / "Assist app · Google"
    / "Use text from screen …", "Use screenshot …" / "Voice input · Google". Label "Android 8 and 9, drawn
    generic · check on device".
13. **360 dp at font scale 2.0** (pin 16). Ready to start with the card's icon above its text; Start HQ and
    Back in a bar at the bottom while the page scrolls.
14. **Set up, microphone off in Android** (pin 9). "Right now": "Assistant app · AgentHQ", "Microphone ·
    Off in Android" (amber with icon) with [Open Android settings] and "Tap Permissions, then Microphone,
    then Allow only while using the app.", "Speech model · On this phone · [U] MB". "On Settings home this
    reads: Phone assistant · On · microphone off" (grey).
15. **Server HQ owner, Settings home** (pin 17). This phone: …, "Phone assistant · On · hold Home to ask",
    "Speech model · On this phone · [U] MB · English", …. Your accounts: "AI service and keys · On your
    server HQ", "Messaging apps · Telegram on · Discord on", "Other devices · 1 signed in", "Server HQ ·
    Your server HQ, Office · Connected".

## 5. Exact copy

| Where | Words |
|-------|-------|
| Card label | "Optional" |
| Card title | "Use HQ as your phone's assistant" |
| Card body | "Hold Home on any screen and ask by voice. Understands English. You can also do this later in Settings." |
| Card button | "Set up" |
| Card, chosen | "HQ is your phone's assistant" · "Hold Home on any screen to ask. Understands English. Before your first question, you tap to download the speech model, 57 MB." |
| Card, not chosen | "Not set yet. You can do it later in Settings, Phone assistant." · Button: "Try again" |
| One machine (if any) | "Run HQ on one machine only. A second copy would answer the same chats twice." |
| Sheet title | "Choose HQ in Android's settings" |
| Sheet steps, 10+ | "1 Tap Digital assistant app. On Samsung, then tap Device assistance app." · "2 Choose AgentHQ." · "3 Tap OK." |
| Sheet steps, 8 and 9 | "1 Tap Assist app. On Samsung, then tap Device assistance app." · "2 Choose AgentHQ." · "3 Tap OK." |
| Sheet note | "Android will say the assistant can read what is on your screen. HQ turns that off and never reads your screen." |
| Sheet buttons (was "Open Android settings · Cancel") | "Open Android settings" · "Not now" |
| No page opened | "This phone did not open its assistant setting. Look for it in Settings, Apps, Default apps." |
| Settings row title | "Phone assistant" |
| Row status | "Not set up" · "On · hold Home to ask" · "On · microphone off" · "On · speech model not downloaded" · "Not available on this phone" |
| Page intro | "When HQ is your phone's assistant, holding Home opens a small sheet over any app. Speak, and HQ answers in text. Understands English." |
| Group label | "Right now" |
| Assistant app | "AgentHQ" · "Another app" |
| Microphone | "Allowed" · "Not asked yet" · "Off in Android" (amber, with "Open Android settings" and: "Tap Permissions, then Microphone, then Allow only while using the app.") |
| Speech model | "On this phone · [U] MB" · "Not downloaded · 57 MB" · "Downloading · 45%" |
| Buttons | "Make HQ the assistant" · "Try it without setting up" · "Try it now" · "Open Android settings" · "Open the chat" |
| Set up card | "HQ is your phone's assistant" · "Hold Home on any screen to ask. Understands English." |
| Stop note | "To stop using HQ, choose another app in the same Android setting." |
| Samsung note | "On Samsung phones the side key opens Bixby or the power menu. Hold Home instead, or swipe up from a bottom corner if you use gestures." |
| Privacy note | "Your screen is never read. Your voice stays on this phone. Only the words of your question go to HQ." |
| Privacy note, server HQ | "… Only the words of your question go to your server HQ, {name}." |
| No secure lock | "Set a screen lock to ask HQ from the lock screen" · "This phone has no screen lock. Anyone holding this phone can answer. HQ does not open over the lock screen until you set one." · "Open Android settings" · "Tap Screen lock, then choose a PIN, pattern or password." |
| Not available | "Not available on this phone" · "This phone keeps the assistant for its maker's apps. You can still ask HQ in the chat." |
| Speech model row (server frame) | "On this phone · [U] MB · English" |
| AI service row (server frame) | "On your server HQ" |
| Server HQ row | "Your server HQ, {name} · Connected" |

"AgentHQ" stays where Android shows that name, so the steps match the screen. Everywhere else it is HQ.
Every string is a whole sentence with its own key in `res/values/strings.xml` (keys like
`assist_card_body`).

## 6. Behaviour rules

1. **The card shows only when it can work:** Android's assistant page resolves on this phone, and
   `isActiveService` is false. HQ already chosen (a restore, a reinstall) shows the chosen state. No page
   hides the card.
2. **Start HQ never waits on it.** No microphone request and no download in first run. Doing nothing with
   the card is skipping it.
3. **Set up and Make HQ the assistant open the same sheet.** Open Android settings tries the pages in
   order (constraints). If none opens, the sheet keeps its steps and adds the "No page opened" line. Not
   now closes it.
4. The note answers Android's warning before it appears. It is true because `AssistantService.onReady`
   turns off assist data and the screenshot.
5. Android's page, drawn generic. HQ cannot draw on it or come back by itself; the owner presses Back.
6. **On return the app asks Android, never a saved flag:** `isActiveService` at every resume. Chosen shows
   a check and a grey border within one frame. Nothing is stored.
7. Back without choosing: the card says so and offers Try again. No warning colour; it is optional.
8. **The row sits after Notifications** and opens `hq://settings/assistant`. Its status is the first that
   applies: Not available on this phone, Not set up, On · microphone off, On · speech model not
   downloaded, On · hold Home to ask. **Always grey.** Not using voice is fine.
9. Three facts, read live: the assistant, the microphone permission, the model files. "Another app"
   because Android does not say which app holds it. **Only the microphone line can turn amber,** with its
   icon and Open Android settings, when it is off in Android (After 14).
10. **Try it without setting up** opens the voice sheet. The microphone and model steps (FixAssistantMic,
    FixAssistantModel) run there as on a Home press.
11. **Speech model line:** not downloaded, it opens the download with its size first and "Understands
    English" (FixAssistantModel). Nothing starts until Download is tapped. On the phone, it opens
    `hq://settings/speech`.
12. No assistant page on the phone: the page says so once and points to the chat. The row reads "Not
    available on this phone"; the first-run card never shows.
13. **One card slot under the facts.** It holds the screen-lock card when the phone has no secure lock.
    Otherwise it holds FixAssistantLocked's "When the phone is locked" card, only on a model where that
    board's device check failed. Empty otherwise.
14. **No secure lock** (`KeyguardManager.isDeviceSecure` false): the card shows, re-read at every resume.
    Open Android settings opens Android's screen lock chooser. The lock-screen path stays closed
    (FixAssistantLocked).
15. **Android 8 and 9:** the first step names Assist app. Everything else is the same.
16. **Font scale 1.5 and up:** the card's icon sits above its text, and Start HQ with Back stay in a bar at
    the bottom while the page scrolls.
17. **Server HQ owners:** the same rows, plus Server HQ under Your accounts. The privacy line names the
    server.

## 7. Android versions

| Version | What differs for this fix |
|---------|---------------------------|
| 8 and 9 | No RoleManager on these versions. "HQ is the assistant" is `isActiveService` (API 21); "available" is Android's assistant page resolving. The page is "Assist & voice input", then "Assist app", so the sheet's first step names Assist app (After 11, 12; check on the 8.1 emulator). The session opens the sheet with `startActivity` and `FLAG_ACTIVITY_NEW_TASK` (as built), since `startAssistantActivity` is API 29. The microphone dialog has no "Only this time". |
| 10 and 11 | The page is "Digital assistant app"; One UI adds "Device assistance app". `RoleManager.isRoleAvailable(ROLE_ASSISTANT)` is read as an extra availability check, behind a version check. Held is still `isActiveService`. |
| 12 | As 10 and 11. The microphone line on the page can read "Off in Android" (FixAssistantMic); the row stays grey. |
| 13+ | As 12. Setting up never asks for notifications; that stays in FixNotifications' step and upgrader sheet. Font scale 2.0 exists from Android 14 (After 13). |

The build's NewApi lint (`build.gradle.kts`, minSdk 26) fails any RoleManager call that is not behind a
version check.

**Server HQ owners:** the voice sheet and this page work the same. The privacy line names where the words
go: "Only the words of your question go to your server HQ, Office." Settings home shows the Server HQ row
under Your accounts (After 15). The Speech model row shows only once the model is on this phone.

Android constraints, as checked against the platform APIs:

- **An app cannot make itself the assistant.** RoleManager does not exist on 8 and 9, and on 10+ the
  assistant role is not requestable. Only the owner can choose it in Settings (or adb's `cmd role
  add-role-holder` on 10+, which the README uses for tests).
- **Is HQ the assistant:** `VoiceInteractionService.isActiveService(context, ComponentName(context,
  AssistantService::class.java))`, API 21, on every version. No public callback reports a change, so the
  app re-reads in `onResume`.
- **Is there an assistant page:** `PackageManager.resolveActivity` on `Settings.ACTION_VOICE_INPUT_SETTINGS`;
  on 10+ also `RoleManager.isRoleAvailable`.
- **Opening the page, in order:** `ACTION_VOICE_INPUT_SETTINGS` (API 21), then
  `ACTION_MANAGE_DEFAULT_APPS_SETTINGS` (API 24), then `ACTION_SETTINGS`, each wrapped like
  `PhoneIntents.startFirst`.
- **Screen lock:** `KeyguardManager.isDeviceSecure()` (API 23) is true only for a PIN, pattern or
  password. `DevicePolicyManager.ACTION_SET_NEW_PASSWORD` opens Android's chooser.
- **Android's warning dialog** belongs to Android. HQ cannot reword or skip it, only explain it first. The
  other holder's name cannot be read, so copy never says "Google".
- Android binds `AssistantService` as soon as it is chosen; no restart of HQ is needed.

## 8. Code touchpoints

| File | Change | Exists |
|------|--------|--------|
| `voice/AssistantRole.kt` | `held()` over `isActiveService`, `available()`, `deviceSecure()`, `settingsTries()`. RoleManager only inside an `SDK_INT >= Q` branch. | NEW |
| `ui/firstrun/FirstRunScreens.kt` | `StartStep` (line 150): the `AssistantCard` in rules section 5 order. | yes |
| `ui/firstrun/FirstRunViewModel.kt` | `onAssistantSetUp`, held read on resume. | yes |
| `ui/settings/SettingsViewModel.kt` | `phoneRows()` (line 271): the new HomeRow. | yes |
| `ui/settings/SettingsScreens.kt` | `Assistant()` page, the card slot and the shared sheet. | yes |
| `ui/settings/AssistantWords.kt` | Pure status function, with `AssistantWordsTest`. | NEW |
| `ui/settings/PhoneIntents.kt` | `PhoneFix.Assistant` (three tries) and `PhoneFix.ScreenLock`. (The `PhoneFix` enum itself is in `ui/settings/PhoneWords.kt:18`.) | yes |
| `res/values/strings.xml` | Every string above, whole sentences, keys like `assist_card_body`. | yes |
| `ui/shell/Icons.kt` | `HqIcons.Mic`. | yes (icon NEW) |
| `ui/shell/Links.kt` | `"assistant"` and `"speech"` in `SETTINGS_SCREENS` (line 28); `LinksTest`. | yes |
| `apps/android/README.md` | The in-app path replaces the manual one. | yes |

## 9. Server, release or desktop changes

None. Release notes carry one line for upgraders (open question, D6).

## 10. Acceptance checks (each fails on today's build)

1. **Android 8.1 emulator**, fresh install, new HQ: Ready to start shows the card, no crash. Set up, Open
   Android settings lands on Assist & voice input. Choose AgentHQ, Back: the card shows the check and a
   grey border.
2. **Galaxy A32** (13, One UI 5) and one Tecno, Infinix or itel phone: the same, landing on Digital
   assistant app.
3. Back without choosing: "Not set yet". Start HQ works. The status-bar microphone dot never lights, and
   Android's data usage for AgentHQ does not grow by 57 MB.
4. `adb shell cmd role remove-role-holder android.app.role.ASSISTANT online.agenthq.app`, reopen
   Settings: the row reads "Not set up". Add it back: "On · hold Home to ask".
5. Remove the phone's screen lock: the page shows "Set a screen lock to ask HQ from the lock screen". Open
   Android settings lands on the screen lock chooser.
6. Revoke the microphone in App info: the page's microphone line is amber with its icon. The Settings home
   row reads "On · microphone off" in grey.
7. An Android Go phone or an image without an assistant page: no card; row "Not available on this phone".
8. Android 14 emulator at 360 dp and font scale 2.0: no text cut off, and Start HQ is on screen without
   scrolling.
9. `hq://settings/assistant` from adb opens the page. TalkBack reads "Set up, button" after the card text.
10. Unit: `AssistantWordsTest` covers the five statuses in order; `LinksTest` the two new screen names.

## 11. Check on device

- Android 8 and 9 page labels ("Assist & voice input", "Assist app") on the 8.1 emulator (FixPlan "Still
  to prove": "8 and 9 page labels").
- Whether an Android Go phone resolves an assistant page.
- `ACTION_SET_NEW_PASSWORD` on One UI.

## 12. Open questions and decisions it depends on

- **Tell owners who upgrade?** They skip first run, so they only meet the Settings row. Options: nothing
  more; a one-time line on Settings home; an Inbox item. Recommendation: nothing more in the app, plus one
  line in the release notes. An Inbox item for a feature is noise in a place kept for things that need the
  owner. (Decided: D6, "Settings row and release notes.")

Depends on: D6, rules sections 3, 5, 7, 9; `02-assistant-locked.md` (the card slot);
`03-assistant-mic.md` (microphone line); `../wave-2/03-assistant-model.md` (speech model line and page).
