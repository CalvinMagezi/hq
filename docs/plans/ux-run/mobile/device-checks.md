# Device checks for the mobile fixes

Every item here is still unverified on a real phone. The boards marked them "Check on device" or "Still to
prove". Run the items for a wave before that wave starts (FixPlan: "Device checks before each wave"), write
the result (pass, fail, the value found) next to the item in the PR that builds the wave, and paste Copy
details from "What HQ noticed" (`wave-0/04-measure.md`) for each phone you used.

## The device matrix (rules section 9, review M7 and M11)

| Device | Why | Notes |
|--------|-----|-------|
| Samsung Galaxy A32 (Android 13, One UI 5) | The owner's reference phone | Holds Home for the assistant (its side key opens only the power menu). |
| One Tecno, Infinix or itel phone (Transsion) | The common phone among target owners | Vendor clone app and battery rules differ from Samsung. |
| One Android Go phone | Low memory, Files by Google, may lack an assistant page | |
| Android 8.1 emulator (API 27) | Oldest supported behaviour, slow-start test bed | minSdk is 26. Some checks also need an 8.0 (API 26) image. |
| Android 10 emulator (API 29) | First version with accessibility timeouts, silence detection, `startAssistantActivity` | |
| A Pixel emulator image (Android 14) | Stock Android; font scale 2.0; predictive Back | Required with the A32 for the wave 3 locked-hold log. |
| A 360 dp wide emulator at font scale 2.0 | Narrow screen and largest text | Run every board's 360 dp check here. |

## How to set a phone up for these checks

- **Install the build under test.** Debug build beside the real app:
  `./gradlew :app:assembleDebug -Phq.appId=online.agenthq.<name>` plus the four `-P` paths for a real hq
  (see `README.md`, "Build, install and test"). Install with `adb install -r -g <apk>`. For checks that
  need the real app id (updates, phone-to-phone copy), build with the default id and the same
  `-Phq.keystore`.
- **Logs:** `adb logcat -s HQHUB` (hub), `adb logcat -s HqVoice` (assistant phases, never words).
- **Notifications as posted:** `adb shell dumpsys notification --noredact`.
- **Screen record frame by frame:** `adb shell screenrecord /sdcard/check.mp4`, then pull and step
  through it.
- **App files:** `adb shell run-as online.agenthq.app ls -la files files/hqhome no_backup` (debug builds
  only).
- **Font scale:** `adb shell settings put system font_scale 2.0` (1.5 for the stacking checks); reset with
  `1.0`.
- **Width:** `adb shell wm size 1080x2400` with `adb shell wm density` chosen to give 360 dp
  (width px / density × 160 = 360); reset with `wm size reset`, `wm density reset`.
- **Accessibility timeout (10+):** Settings, Accessibility, Time to take action; or `adb shell settings
  put secure accessibility_interactive_ui_timeout_ms 30000`.
- **Assistant role (10+):** `adb shell cmd role add-role-holder android.app.role.ASSISTANT
  online.agenthq.app 0`, `remove-role-holder` to undo. Note `settings get secure assistant`,
  `voice_interaction_service` and `voice_recognition_service` first and restore them afterwards.
- **Hold Home without a hand:** `adb shell input keyevent 219` (KEYCODE_ASSIST). A real hold is still
  needed for the locked-hold log.

## Wave 0 · safety and shared parts

| Check | Board | How to perform it |
|-------|-------|-------------------|
| Details swap on unlock | FixLockScreen (R1) | Details on, Android 12+, phone unlocked, have a helper ask. Press power: within 2 s the lock screen shows the general words and no Allow. Unlock: details return with no sound. Repeat on the A32, the Transsion phone, the Go phone, 8.1 and 10 emulators (8 to 11 show Open only). |
| A detailed notice when the hub is killed while unlocked | FixLockScreen | Details on, unlocked, a detailed notice shown. `adb shell am kill online.agenthq.app` (or force-stop the hub). Lock the phone. Record what the lock screen shows: the runtime `SCREEN_OFF` receiver is gone, so note whether the detailed text stays. |
| Smart Switch and Google copy with a sideloaded app | FixDataTransfer (M1) | Two phones, the old on Android 12+. Install AgentHQ by APK on the old one, make notes. Reset the new phone and use Google's "Copy apps and data" (cable, then Wi-Fi). Then repeat with Samsung Smart Switch from the A32. On the new phone `run-as … ls -R files` must list only `files/hqhome/.vault`. Write down if the app is not moved at all: it changes the decision text. |
| Restore runs when the app is installed later | FixDataTransfer | After the copy above, if AgentHQ was not reinstalled during setup, install it afterwards and see whether Android restores the vault. |
| A 4 GB zip on 8.1 | FixSafeMode (R3) | 8.1 emulator with a 5 GB fixture vault. Run "Save a copy of my notes" to a file; open the .zip on a computer (Zip64). |
| A 340 MB copy straight to Drive with 200 MB free | FixSafeMode | On the A32 fill storage to 200 MB free, choose Google Drive in the picker, save a 340 MB vault. Record whether the provider stages the whole file first and fails, and what the screen shows. |
| Safe mode opened on every device-check phone | FixSafeMode (D11 precondition) | Open the "Save notes" launcher shortcut on each matrix phone; complete one copy. |
| `hasFragileUserData` keep-data dialog | FixSafeMode (if adopted) | Android 10+: uninstall and check that the dialog offers to keep the app's data. |
| The locked Home hold | FixAssistantLocked (R6; FixPlan lists it under wave 0, it gates wave 3) | See wave 3 below. Do it in wave 0 so wave 3 can start. |

## Wave 1 · reach every owner, cheaply

| Check | Board | How to perform it |
|-------|-------|-------------------|
| The running notice after a grant | FixNotifications | Android 13 fresh install. At the Notifications step tap Allow notifications, then Allow. Open the shade without restarting: does "HQ is running" appear by itself? If not, the hub must post it again when it reads the grant. |
| Turning the permission off ends the process? | FixNotifications | Android 13: App info, Notifications off. Watch `adb logcat -s HQHUB` for the process dying and the `START_STICKY` restart. |
| A dismissed dialog on One UI | FixNotifications, FixAssistantMic | On the A32 tap outside Android's notification dialog (and the microphone dialog). Check `shouldShowRequestPermissionRationale` is unchanged (debug log) and that Ask again shows the dialog. For the microphone, also time the result: does the 300 ms line tell a dismissal from an instant denial? |
| Large text on the A32 | FixPlan wave 1; FixPlainWords | One UI's largest font and display zoom: every dialog stacks at 1.5, status lines wrap, nothing ends in "…". |
| Every dialog stacks at font scale 1.5 | FixDestructive | A32 and an Android 14 emulator at 1.5: Stop Codex, Remove device, Sign out device, Disconnect server, Remove key. Safe choice on top. |
| Undo on Android 8 and 9 | FixDestructive | 8.1 emulator and the Go phone: archive a task; the snackbar stays 10 s. |
| Undo across an HQ stop | FixDestructive | Clear 3 updates; within the Undo window kill hq. The change shows "Waiting for HQ" and reaches HQ after Start HQ. |
| Where a copy was saved (provider names) | FixBackup | Save a copy to: One UI My Files, Files by Google, an SD card, a cloud drive. Log each document's provider authority. Only off-phone places may clear amber; Downloads, internal storage, media and (until checked) a memory card count as "on this phone too". |
| The save screen on 8 and 9 | FixBackup | The Go phone with Files by Google and the 8.1 emulator: Save a copy now works and cloud places show only when their app offers one. |
| Line length on old phones | FixPlainWords | 8.1 emulator and the Go phone at their default font: no owner line is cut. |
| Five-owner reading test | FixPlainWords | Five owners who read English as a second language read frames A1 to A8 and say back what each line means. A line two of them get wrong is rewritten. |
| Count never cut at 360 dp, font scale 2.0 | FixProgress | A32 at font scale 2.0 and 360 dp: "1 of 3 sub-tasks done" never shows "…" inside the count. |

## Wave 2 · stay current without burning data

| Check | Board | How to perform it |
|-------|-------|-------------------|
| Data Saver and the hub | FixDownloads, FixAssistantModel | Data Saver on, app in the background, a large download running in the hub (an update or the speech model). Does it continue? Run on the A32, the Transsion phone, the Go phone, 8.1 and 10 emulators. Build the "Data Saver is on" row only if it stops. |
| The Wi-Fi fingerprint | FixDownloads | Join home Wi-Fi, a pocket router and a phone hotspot: three different keys. Reboot: the same Wi-Fi gives the same key. |
| Roaming on 8.0 and 8.1 | FixDownloads | A roaming SIM on Android 8.1 (no `NOT_ROAMING` before API 28): the roaming ask shows from `TelephonyManager.isNetworkRoaming()`. |
| The 200 MB threshold | FixPlan wave 2; FixStartup (N15) | Fill storage to just under and just over 200 MB free for the app's files. The almost-full line shows under 200 MB, and HQ can still open its database at that level. |
| The unpacked model size | FixAssistantModel | Unpack `sherpa-onnx-streaming-zipformer-en-kroko-2025-08-06.tar.bz2`, sum the four kept files (encoder, decoder, joiner, tokens), pin the byte count beside the archive's in `VoiceModels.SOURCES`. Replace every [U] on the screens. |
| Samsung Auto Blocker | FixUpdates | A Galaxy on One UI 6+ with Auto Blocker on: start an update install, record the installer status and message so the "Samsung Auto Blocker stopped the update" line can be picked. |
| Developer verification | FixUpdates | Where Android's developer verification is enforced (Android 13+ certified devices, country by country), install an unregistered build and record what Android shows and the installer status. |
| App restart when the install switch changes | FixUpdates | On the A32 turn on "Install unknown apps" for AgentHQ from the first-time sheet. Does the app restart? The pending request on disk must resume the download either way. |
| `setRequireUserAction` conditions | FixUpdates (before D11 is reconsidered) | Android 12+: with `UPDATE_PACKAGES_WITHOUT_USER_ACTION`, check whether an app updating itself skips the confirm, and what target SDK conditions apply. |
| Min channel on 8.0 and 8.1 | FixStartup, FixOffline | 8.0 and 8.1 images: is the foreground "HQ is running" notice shown at Min, or raised to Low? Does a status bar icon still show? |
| Usual start time with a 2,860-note vault | FixStartup | A32, Transsion phone, Go phone: time five starts to a good health answer with a 2,860-note fixture. Use the result for the slow-start fixture (acceptance check 2). |

## Wave 3 · an assistant people find and trust (starts only after the locked hold log)

| Check | Board | How to perform it |
|-------|-------|-------------------|
| **The locked Home hold (R6, precondition)** | FixAssistantLocked | HQ as the assistant, a PIN set, screen on at the lock screen. Hold Home (and on gesture phones swipe from a bottom corner). A debug build writes `assist while locked: onShow=yes\|no window=shown\|behind prompt=yes\|no` under `HqVoice`. Run on the A32 (13), a Pixel emulator image (14) and the 8.1 emulator. Result Pass, Fail A or Fail B; put the three lines and the build on the PR. |
| `FLAG_DISMISS_KEYGUARD` on One UI | FixAssistantLocked (Fail A path) | On the A32 open a test activity with the old window flag over the lock screen: does One UI still raise the unlock? |
| Screen-record check (R2) | FixAssistantLocked | Ask "what is on my list today", wait for the answer, power off, wake, hold Home. In `screenrecord`, frame by frame, nothing of that question or answer appears before the unlock. |
| Assist page labels on 8 and 9 | FixAssistantSetup | 8.1 emulator (and an 8.0 image if available): Open Android settings lands on "Assist & voice input", then "Assist app". Note the exact labels. |
| An assistant page on an Android Go phone | FixAssistantSetup | Does `ACTION_VOICE_INPUT_SETTINGS` resolve? If not, the row reads "Not available on this phone" and no card shows. |
| Screen lock chooser on One UI | FixAssistantSetup | `ACTION_SET_NEW_PASSWORD` opens the chooser on the A32. |
| Two apps recording on 8 and 9 | FixAssistantMic | 8.1 emulator and the Transsion phone: start another recorder, then hold Home. Does HQ's recorder fail to start, or record silence? |
| Child process after a kill | FixAssistantHandoff | During a lookup, close the sheet and `adb shell am kill` the app process (debug build). Does hq (the hub's child) keep running? After the restart exactly one notice posts. |
| A tool silent for 200 s | FixAssistantHandoff | Find a way to stage a turn whose tool is silent for 200 s; check nothing posts at 180 s and one notice posts when it ends. |

## Wave 4 · one app, one way

| Check | Board | How to perform it |
|-------|-------|-------------------|
| The saved-copy size on a 2,000-note vault | FixOffline (N2) | Fill every place on a 2,000-note vault, read `files/savedcopy/` sizes. Confirm 256 KB a place and 2 MB in all are enough, or adjust before building. |
| Vendor copy tools and the saved copy | FixOffline | Repeat the phone-to-phone copy with Samsung Smart Switch and a Tecno or Infinix clone app: `savedcopy/`, the outbox and spoken ids must be absent on the new phone. |
| Almost-full size formula | FixOffline | On the A32 measure what HQ needs to start (database size plus 100 MB, per the board) and check it against FixStartup's 200 MB rule. |
| Predictive Back from a search result | FixSearch | Pixel image, Android 14: from a result, the back preview shows search with its list. |
| Board and Timeline on the 8.1 WebView | FixWebPages | 8.1 emulator with its shipped Android System WebView: Board and Timeline load, or the page shows "The page stopped working." with Try again. |
| Predictive Back on a web page | FixWebPages | Pixel image: Back goes back inside the page first, then to Tasks. |

## Shared for every wave

- Every board's "360 dp and font scale 2.0" frame on the 360 dp emulator.
- TalkBack on each new screen: headings, the example notification read once, steps read once, status
  changes announced politely.
- Copy details from What HQ noticed on each matrix phone after each wave, for the roll-back rules in
  `wave-0/04-measure.md`.
