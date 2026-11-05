# Wave 2 · 01 · One download engine for every large download (finding #7, the shared rule)

Board: **FixDownloads** ("Large download rule"), finding #7, the shared rule, revised after review,
5 Nov 2026. Area: large downloads; one pattern, used by app updates and the speech model.
Shared rules: `../../rules-shared.md` sections 1 (Downloads channel), 2, 3, 4 (D8) win.

Headline: **Before anything over 10 MB downloads, say how big it is, and ask first on data that costs
money.**

## 1. Finding and the review items it closes

**Problem.** Updates up to 200 MB and the 57 MB speech model start the moment they are tapped, with no
size up front, no Wi-Fi check, and (for updates) no percentage or Cancel.

**Who and how often.** Every owner on mobile data, at every update and once for the speech model. Many
owners are on prepaid metered bundles, where 50 MB is a real share of a week's data, and many use a
pocket router or a phone's hotspot that Android thinks is free.

**The rule, as fixed:**

1. Before any download over 10 MB, show its size.
2. On a paid connection, ask first with two equal buttons: Wait for Wi-Fi, then Download · {size}. A paid
   connection is mobile data, roaming, a Wi-Fi the owner said is paid by the MB, or any network when "Ask
   before big downloads on any network" is on.
3. While downloading, show a percentage, Pause and Cancel. Cancel acts at once and offers Undo.
4. A cut-off download resumes, and the finished file must match the signed size and hash before it is
   used.

What changed in this revision (review items):

- **M14, rules sections 1 and 4:** the notification's Cancel works one way, the app-wide way: it acts at
  once and shows "Download cancelled · Undo" for the accessibility timeout (10 s on Android 8 and 9).
  Progress posts on the Downloads channel, with Pause · Cancel.
- **D8 revised:** two equal outlined buttons, Wait for Wi-Fi first, the size in the label, side by side,
  stacked at font scale 1.5+ (frames 3, 4, 9). Green is gone from every ask.
- **M10:** HQ asks once per Wi-Fi "Is this Wi-Fi paid by the MB?", and a setting "Ask before big
  downloads on any network" (frames 4, 7). Data used per purpose is shown (frame 7).
- **Data Saver:** the old claim was probably wrong for a foreground service. It is now unverified, drawn
  dashed, with a device check.
- **Space:** one formula for every download.
- **Partial downloads** expire after 14 days and show under Storage (frame 7).
- **D14:** "Used about {N} MB of mobile data" on the done state, in exact copy.
- **Resume:** the finished file, resumed or not, is checked against the signed size and SHA-256 before
  use. The old "kept bytes are hashed first" line is removed: a part cannot be checked alone.
- **R5 words:** the download server and full phone lines come from the rules section 2 catalogue. The
  idioms in the old row and notice words are gone.
- **Acceptance checks** rewritten so each fails on today's build.

## 2. Before (as built, verified at `c04a100`)

Paths under `apps/android/app/src/main/`.

- `kotlin/example/hq/spike/update/Updater.kt:168` `download()`; line 169 writes to
  `File(context.cacheDir, DIR)` (Android may clear the cache when space runs low). No size shown before
  the tap, no network check, no percentage, no Cancel; a cut-off update starts again from zero.
- `kotlin/example/hq/spike/voice/VoiceModels.kt:29` the model root is `noBackupFilesDir/voice`; lines
  55 to 65 already resume with a Range request and hash every byte; line 138 the pinned size `57_267_600`
  bytes (the UI says "about 54 MB", 1024-based).
- `java/example/hq/spike/Alerts.java:43` `SERVER_POLL_MS = 60_000` (the server HQ poll).
- `AndroidManifest.xml:3-14` has `INTERNET`, `ACCESS_NETWORK_STATE` (line 4), the foreground service
  permissions (`FOREGROUND_SERVICE_SPECIAL_USE` line 8), no location permission.

### Before frames

- **B1 · The two big downloads today.** Settings, Backup and about, Updates: "Install update · Version
  1.5.0 is ready" (pin A); "Check for updates · Downloading…" (pin B). Voice sheet, speech models:
  "Downloading speech models 42%" / "Your speech is recognised on this phone, so it needs about 54 MB of
  speech models, downloaded once from the sherpa-onnx project on GitHub. The download sends nothing about
  you." / "The download did not finish or did not match. Check the connection and tap Get again." (pin
  C).

Pins: **A** no size before the tap (the row names the version only; the APK can be up to 200 MB). **B**
no network check, no percentage, no Cancel (a spinner runs until it ends; a cut-off update starts again
from zero). **C** the model shows a percentage but has no Cancel and no Wi-Fi check; "About 54 MB" is in
1024-based units; Android's storage screen will show 57 MB.

## 3. Decision

**One download engine and three shared parts** (the download row, the data ask, the Downloads
notification) that every large download uses, so an update and the speech model look, ask and resume the
same way. The engine runs in the hub service, which is already a foreground service, so a download keeps
going with the app closed.

- **Paid, not just "metered":** Android's metered flag misses pocket routers and many hotspots. HQ asks
  once per Wi-Fi, remembers the answer, and offers a switch to ask on every network.
- **Spent data cannot be put back (D8):** the ask gives both choices the same weight, Wait first, with the
  size in the button.

Rejected:

- **Android's DownloadManager.** It can wait for Wi-Fi and resume, but its notification and buttons are
  Android's and differ by phone, the file lands outside private storage, and the updater and the model
  both need the byte stream to enforce the size cap and the pinned hash.
- **A warning toast on mobile data, then go.** Telling is not asking; the data is spent before the owner
  can react.
- **Never download on mobile data.** Many owners have no Wi-Fi at all; they could never update.
- **"Don't ask again" on the dialog.** Downloads this big are rare, and on a prepaid bundle each one is a
  real cost worth a tap.
- **Ask for location to read the Wi-Fi name.** A location permission for a data question would scare
  owners and is not needed.

## 4. Screens and states (After)

Example item: AgentHQ 1.5.0, a 36 MB download. The speech model uses the same parts with its own name and
size. The progress bar is 4 dp, blue while working, amber while waiting or paused (rules section 7).

1. **The download row, every state.**
   - **Ready: size first** (pin 1): "AgentHQ 1.5.0" / "36 MB download" / [Download and install].
   - **Waiting for Wi-Fi** (pin 2): "AgentHQ 1.5.0" / "36 MB" / amber with icon "Waiting for Wi-Fi" /
     [Download · 36 MB] / [Cancel].
   - **Downloading** (pin 3): "AgentHQ 1.5.0" / "Over Wi-Fi" / blue bar / "45% · 16 of 36 MB" / [Pause]
     [Cancel].
   - **Paused: left Wi-Fi** (pin 4): amber "Paused at 16 of 36 MB. It goes on with Wi-Fi." / [Download ·
     20 MB] / [Cancel].
   - **Cut off** (pin 5): "Stopped at 22 of 36 MB. It starts again when you are back online."
   - **Checked, then done** (pin 6): "Checking the download…", then "Used about 12 MB of mobile data".
2. **The row when something is in the way.**
   - **Paused by the owner** (pin 12): "Paused at 16 of 36 MB" / [Resume] [Cancel].
   - **Did not match** (pin 8): "AgentHQ 1.5.0 · 36 MB" / amber "The download did not match. Nothing was
     kept." / [Try again].
   - **Download server keeps failing** (pin 9): "AgentHQ 1.5.0 · 36 MB" / amber "Couldn't reach the
     download server. Check your internet and try again." / "16 of 36 MB is kept." / [Try again] [Cancel].
   - **Under 10 MB: no rule, no size line** (pin 10): "[An item under 10 MB]" / "Downloading…" (spinner).
   - **Data Saver, only if the device check finds it stops the hub** (drawn dashed, pin 7): "Waiting. Data
     Saver is on." / [Open Android settings] / "Then turn on Unrestricted data." Label "Check on device".
   - **Cancelled: Undo for the accessibility timeout** (pin 11): row back to "AgentHQ 1.5.0 · 36 MB
     download · Download and install", snackbar "Download cancelled · Undo".
3. **On mobile data: ask first (D8).** "Update AgentHQ" page behind. Dialog "Download on mobile data?" /
   "AgentHQ 1.5.0 is 36 MB. Your data plan may charge for it." / [Wait for Wi-Fi] [Download · 36 MB],
   both outlined, equal width, side by side (pins 13, 14).
4. **Roaming, paid Wi-Fi, start over.**
   - "Download while roaming?" / "AgentHQ 1.5.0 is 36 MB. Roaming may cost more than at home." / [Wait for
     Wi-Fi] [Download · 36 MB] (pin 15).
   - "Is this Wi-Fi paid by the MB?" / "Pocket routers and phone hotspots often charge for each MB. HQ
     asks once for each Wi-Fi." / [It's paid] [It's free] (pin 16).
   - "Start over?" / "The server could not go on from 16 MB. Starting over uses 36 MB." / [Wait for Wi-Fi]
     [Download · 36 MB] (pin 17).
5. **With the app closed: the Downloads channel** (Android's shade, generic).
   - Running (pin 18): "AgentHQ · Downloads" / "Downloading AgentHQ 1.5.0" / "45% · 16 of 36 MB" / Pause ·
     Cancel.
   - After Cancel, in place (pin 19): "Download cancelled" / "AgentHQ 1.5.0" / Undo.
   - Waiting (pin 20): "Waiting for Wi-Fi" / "AgentHQ 1.5.0 · 36 MB. It starts on Wi-Fi." / Cancel.
   - Paused: "Download paused" / "AgentHQ 1.5.0 stopped at 16 of 36 MB when you left Wi-Fi." / Cancel.
6. **Not enough space, before a byte is used** (pin 21). Dialog "Not enough space" / "The phone is almost
   full. Free up 41 MB and try again." / "This needs 82 MB free: the download, its unpacked copy, and 10
   MB to spare." / [Not now] [Open Android settings] / "Android shows what you can remove."
7. **Settings: mobile data and storage.** Spending limits, group "Mobile data": switch "Ask before big
   downloads on any network / Also on Wi-Fi that seems free. Use this if your Wi-Fi counts data."; row
   "Wi-Fi you told HQ about · 2 free · 1 paid by the MB · Forget" (pin 22); "Data used by HQ this month":
   "App updates 36 MB", "Speech model 0 MB", "Talking to your server HQ [measured] MB". Backup and about ·
   Storage: "Unfinished downloads · 16 MB · removed 14 days after the last byte · Remove" (pin 23).
8. **Android 8 to 11.** "Android 8 and 9: Undo for 10 s" (pin 24): the cancelled notice with Undo.
   "Android 8.0 and 8.1: roaming from the SIM": the roaming dialog. Notes: "Android 8.0 and 8.1 have no
   NOT_ROAMING flag on the network (it is API 28), so roaming comes from
   TelephonyManager.isNetworkRoaming(). Check on device." "Android 10 and 11 use the phone's accessibility
   timeout for Undo, never less than 10 s." "The Downloads notification, its Pause and Cancel, the space
   check and the network wait work the same from Android 8 (API 26, the minSdk)."
9. **360 dp wide, font scale 2.0** (pin 25). "Download on mobile data?" / body / buttons stacked, [Wait
   for Wi-Fi] on top, [Download · 36 MB] below, both full width.

## 5. Exact copy

`{name}`, `{size}`, `{done}`, `{left}`, `{pct}`, `{N}` are filled in.

The row:

| Element | String |
|---------|--------|
| Ready, size | "{size} download" |
| Waiting | "Waiting for Wi-Fi" |
| Downloading | "{pct}% · {done} of {size}" |
| Which network | "Over Wi-Fi" / "Over mobile data" |
| Checking | "Checking the download…" |
| Data used | "Used about {N} MB of mobile data" |
| Paused, left Wi-Fi | "Paused at {done} of {size}. It goes on with Wi-Fi." |
| Paused by owner | "Paused at {done} of {size}" |
| Cut off | "Stopped at {done} of {size}. It starts again when you are back online." |
| No match | "The download did not match. Nothing was kept." |
| Server failing | "Couldn't reach the download server. Check your internet and try again." · "{done} of {size} is kept." |
| Data Saver (if needed) | "Waiting. Data Saver is on." · "Open Android settings" · "Then turn on Unrestricted data." |
| Buttons | "Download and install" · "Download · {size}" · "Download · {left}" · "Pause" · "Resume" · "Cancel" · "Try again" |
| Snackbar | "Download cancelled" · "Undo" |

The ask (D8):

| Case | String |
|------|--------|
| Mobile data | "Download on mobile data?" · "{name} is {size}. Your data plan may charge for it." |
| Roaming | "Download while roaming?" · "{name} is {size}. Roaming may cost more than at home." |
| Paid Wi-Fi, known | "Download on this Wi-Fi?" · "You said this Wi-Fi is paid by the MB. {name} is {size}." |
| Start over | "Start over?" · "The server could not go on from {done}. Starting over uses {size}." |
| Buttons, always | "Wait for Wi-Fi" · "Download · {size}" (equal, outlined, Wait on the left or on top) |
| Wi-Fi question | "Is this Wi-Fi paid by the MB?" · "Pocket routers and phone hotspots often charge for each MB. HQ asks once for each Wi-Fi." · "It's paid" · "It's free" |
| No space | "Not enough space" · "The phone is almost full. Free up {size} and try again." · "This needs {need} free: the download, its unpacked copy, and 10 MB to spare." · "Not now" · "Open Android settings" · "Android shows what you can remove." |

The notification, Downloads channel:

| State | String |
|-------|--------|
| Running | "Downloading {name}" · "{pct}% · {done} of {size}" · "Pause" · "Cancel" |
| Speech model | "Downloading the speech model" · "{pct}% · {done} of {size}" · "Pause" · "Cancel" |
| Cancelled | "Download cancelled" · "{name}" · "Undo" (for the accessibility timeout, then it goes) |
| Waiting | "Waiting for Wi-Fi" · "{name} · {size}. It starts on Wi-Fi." · "Cancel" |
| Paused | "Download paused" · "{name} stopped at {done} of {size} when you left Wi-Fi." · "Cancel" |
| Cut off | "Download stopped" · "{name} stopped at {done} of {size}. It starts again when the phone is back online." |

Settings (frame 7):

| Element | String |
|---------|--------|
| Switch | "Ask before big downloads on any network" · "Also on Wi-Fi that seems free. Use this if your Wi-Fi counts data." |
| Known Wi-Fi | "Wi-Fi you told HQ about" · "{N} free · {N} paid by the MB" · "Forget" |
| Data used | "Data used by HQ this month" · "App updates" · "Speech model" · "Talking to your server HQ" |
| Storage | "Unfinished downloads" · "{size} · removed 14 days after the last byte" · "Remove" |
| After Remove | "Unfinished downloads removed" · "Undo" |

Numbers:

- **Units:** 1 MB = 1,000,000 bytes, as Android's own storage screen counts
  (`Formatter.formatShortFileSize`). The model's 57,267,600 bytes (`VoiceModels.kt:138`) read "57 MB",
  not "about 54 MB".
- **Rounding:** Size: nearest whole MB. Done: whole MB, one decimal below 10 ("3.4 of 36 MB"). Left: size
  minus done, rounded up. Percent: rounded down, never 100 until the check passes.
- **Space needed:** download size + unpacked size + 10 MB. An update unpacks into Android's install
  session at its own size, so twice its size + 10 MB. The model's unpacked size: measured from the release
  (`03-assistant-model.md`).
- **TalkBack:** "45 percent, 16 of 36 megabytes".

## 6. Behaviour rules

1. **Over 10 MB** (10,000,000 bytes) the rule applies. The size is shown before the first tap and comes
   from a signed source: the update's channel file, or the model's pinned byte count.
2. **Wait for Wi-Fi** saves the request (survives a restart), shows the waiting row in amber with its icon
   and one silent notification, and starts by itself within seconds of a free network.
3. **Progress:** one 4 dp bar, blue while working, amber while waiting or paused. The row updates at most
   four times a second, the notification once a second when the percent changes, never with sound.
4. **Left Wi-Fi mid-way** with no consent: pause at once, keep the bytes, no dialog (the owner may not be
   looking). The row offers Download · {left}.
5. **Cut off** (signal lost, timeout, the app stopped): the bytes are kept and the download resumes with a
   Range request when the network is back. Server errors retry after 30 s, 2 min, then 10 min, then show
   Try again.
6. **Checked before use:** the finished file, resumed or not, must match the signed size and SHA-256. Then
   the done state shows "Used about {N} MB of mobile data" if any byte came over a paid connection.
7. **Data Saver:** expected to have no effect, since the hub is a foreground service. Unverified. The row
   is built only if the device check shows the hub's download stops.
8. **A hash or size mismatch** deletes the file, partial bytes included. Try again starts over and asks
   again on a paid connection.
9. **Download server errors** use the rules section 2 line; the kept bytes are said on their own line.
10. **At or under 10 MB** a download just starts with a spinner.
11. **Cancel** acts at once, never asks, and is a plain text button. Undo shows for the accessibility
    timeout (10 s on Android 8 and 9) and goes on from the kept bytes; after it, the partial file is
    deleted.
12. **Pause** keeps the bytes and the consent. Resume goes on with no new ask, unless roaming began.
13. **At the tap,** a free network starts at once with no dialog; a paid one shows the ask. Nothing is
    fetched before the answer. Download · {size} is consent for this download only, until it finishes or
    is cancelled.
14. **Back or a tap outside** takes the safe side: nothing is fetched and nothing is saved.
15. **Roaming** always asks, even with earlier consent; a download running when roaming begins pauses.
16. **Wi-Fi question:** at the first big download on a Wi-Fi HQ has not seen and Android calls free. It's
    paid: the paid Wi-Fi ask follows. It's free: the download starts. Back: treated as paid this once,
    asked again next time.
17. **No resume possible** (the server answers 200, not 206, or the file changed): on a free network,
    start over quietly; on a paid one, ask again with the full size.
18. **Downloads channel** (low, silent, ongoing): Pause and Cancel, one notice per item.
19. **Cancel from the notification** works exactly as from the row: the notice becomes "Download
    cancelled · Undo" in place, for the same timeout, then goes.
20. **Waiting and Paused notices** have Cancel only. Spending data needs the ask, so a tap opens the row,
    where Download · {left} is.
21. **Space** is checked before the ask, with one formula: download size + unpacked size + 10 MB. The line
    comes from the rules section 2 catalogue.
22. **Ask on any network** (off by default) treats every network as paid. Forget clears the Wi-Fi answers
    at once, with Undo.
23. **Unfinished downloads** are kept 14 days after the last byte, then deleted at the hub's next start or
    daily check. Storage lists them; Remove acts at once, with Undo.
24. **Android 8 to 11:** Undo for 10 s on 8 and 9; roaming from `TelephonyManager` on 8.0 and 8.1.
25. **Font scale 1.5 and up,** or a label over 18 characters: the ask's buttons stack, Wait for Wi-Fi on
    top, both full width.

## 7. Android versions

| Version | What this fix does there |
|---------|--------------------------|
| 8 and 9 | Undo lasts 10 s. On 8.0 and 8.1 roaming comes from the SIM, since the network's NOT_ROAMING flag needs API 28. Metered, the network wait, `getAllocatableBytes` and the notification work as on newer phones. |
| 10 and 11 | Undo follows the phone's accessibility timeout (`getRecommendedTimeoutMillis`, API 29), at least 10 s. Android 11 may mark a 5G plan temporarily free; HQ follows `isActiveNetworkMetered()` and still asks about Wi-Fi it has not seen. |
| 12 | As 10 and 11. A foreground service cannot be started from the background, but the hub is already running, so the download thread needs no new start. |
| 13+ | Without notification permission the download still runs; only the row shows it. Android 14+ lets the owner swipe the Downloads notice away while it runs; the next percent change posts it again. |

**Server HQ:** no server variant is needed here. Downloads are only what the phone itself fetches: its own
app update and the speech model. A server HQ updates itself, and FixUpdates draws its version line. The
data a server HQ costs the phone, its 60 s poll (`Alerts.java:43`), is counted as its own line in frame 7.

Android constraints, as checked on the board:

- **Metered:** `ConnectivityManager.isActiveNetworkMetered()` at the tap; `NetworkCapabilities`
  `NET_CAPABILITY_NOT_METERED` and the transport (cellular or Wi-Fi) pick the wording.
  `NET_CAPABILITY_NOT_ROAMING` is API 28; on 8.0 and 8.1 roaming comes from `TelephonyManager`.
- **Wi-Fi name:** from Android 8.1 the SSID needs a location permission (and location on, from Android
  10). The app has none, so HQ does not read it.
- **Waiting:** `registerNetworkCallback` with a request for `NOT_METERED`, `INTERNET` and `VALIDATED`,
  held by the hub service; `ACCESS_NETWORK_STATE` is in the manifest (line 4).
- **Where it runs:** the hub service is a foreground service (specialUse, manifest line 8), so Android
  does not stop its thread in the background or in Doze. JobScheduler jobs stop after 10 minutes before
  Android 14, and user-initiated data transfer jobs need API 34.
- **Notification:** `setProgress(100, pct, false)`, `setOnlyAlertOnce(true)`, `setOngoing(true)` while
  running. Android drops updates sent faster than a few a second.
- **Undo time:** `AccessibilityManager.getRecommendedTimeoutMillis` is API 29; below that, 10 s.
- **Data Saver:** `getRestrictBackgroundStatus()` (API 24). Android is believed to let foreground
  services use the network with Data Saver on. Check on device.
- **Space:** `StorageManager.getAllocatableBytes()` (API 26); `ACTION_MANAGE_STORAGE` opens Android's
  free-up-space screen.
- **Resume:** a Range request must come back 206 with a matching Content-Range; If-Range with the ETag
  makes a changed file come back whole. `voice/VoiceModels.kt` already resumes this way (lines 55 to 65).
  Partial files go in `noBackupFilesDir`, not the cache (`Updater.kt:169` uses `cacheDir` today), which
  Android may clear when space runs low.

## 8. Code touchpoints

| File | Change | Exists |
|------|--------|--------|
| `kotlin/.../download/LargeDownload.kt` | The engine, one StateFlow per item, Range resume, the final size and hash check, the space check, the network wait, Pause, the 14-day expiry, persistence of a waiting or paused request. | NEW (package `download/` is new) |
| `kotlin/.../download/DownloadRules.kt` | Pure, unit-tested: the 10 MB threshold, paid or free, ask or start, the space formula, size and progress words, rounding. Strings in resources, failure lines from the rules section 2 catalogue. | NEW |
| `kotlin/.../download/PaidWifi.kt` | The per-Wi-Fi answers, the fingerprint, Forget. | NEW |
| `kotlin/.../ui/download/DownloadRow.kt`, `AskDataDialog.kt` | Compose, HqTheme tokens. | NEW |
| `kotlin/.../download/DownloadNotices.kt` | Posts on the Downloads channel. | NEW |
| `java/example/hq/spike/HubService.java` | Runs the engine's thread and holds the network callback. The Downloads channel is made with the other four (FixNotifications). | yes |
| `kotlin/.../update/Updater.kt` | `download()` (line 168) moves onto the engine. | yes |
| `kotlin/.../voice/VoiceModels.kt` | Keeps its Range code but takes the row, the ask, Pause and Cancel (FixAssistantModel owns that screen). | yes |
| `kotlin/.../ui/settings` | The Mobile data group in Spending limits, the data-used lines, and Storage in Backup and about. | yes |

## 9. Server, release or desktop changes

- The update's size must come from the signed channel file (`size`, added in `../wave-0/02-safe-mode.md`).
- No HQ server change.

## 10. Acceptance checks (each fails on today's build)

1. Mobile data only, tap a download over 10 MB: the ask shows Wait for Wi-Fi on the left and Download ·
   36 MB on the right, the same style. Android's data usage for AgentHQ does not move until a choice.
2. Wait for Wi-Fi, then turn Wi-Fi on: it starts within 10 s with no tap, and a notice on the Downloads
   channel shows the percent.
3. Start on Wi-Fi, turn Wi-Fi off at about 40%: paused; tap Download · {left}: the percent goes on from
   40%.
4. Cancel from the notification: it reads "Download cancelled" with Undo. Undo within the time: it goes on
   from the same byte. Wait: the partial file is gone from the app's storage.
5. Android 9 emulator: Undo stays 10 s. Android 10+ with a 1 minute accessibility timeout: Undo stays 1
   minute.
6. First big download on a new Wi-Fi Android calls free: "Is this Wi-Fi paid by the MB?" It's paid: the
   paid Wi-Fi ask. Next download there: no question. "Ask before big downloads on any network" on: the ask
   on home Wi-Fi too.
7. Debug build, change one byte of a partial file, then resume: "The download did not match. Nothing was
   kept." and nothing installs.
8. Free space below download + unpacked + 10 MB: the space dialog before any data is used, with "The phone
   is almost full. Free up {size} and try again."
9. Leave a partial download, move the clock 15 days on, restart the hub: gone from Storage and from the
   app's storage in Android settings.
10. Finish a download partly on mobile data: "Used about {N} MB of mobile data", within 1 MB of Android's
    data usage for the app.
11. 360 dp emulator at font scale 2.0: the ask's buttons stack, Wait for Wi-Fi on top, nothing cut, no
    sideways scroll.
12. Block the download host: the row reads "Couldn't reach the download server. Check your internet and
    try again."

## 11. Check on device

- **Data Saver** on, app in the background, a download running in the hub: does it go on? On the Galaxy
  A32, a Tecno, Infinix or itel phone, an Android Go phone, and 8.1 and 10 emulators.
- **The Wi-Fi fingerprint:** home Wi-Fi, a pocket router and a phone hotspot give three different keys,
  and the same Wi-Fi gives the same key after a reboot.
- **Roaming on Android 8.1** with a roaming SIM.
- FixPlan wave 2 also lists: the 200 MB threshold; the unpacked model size (`03-assistant-model.md`).

## 12. Open questions and decisions it depends on

- **New: how does HQ tell one Wi-Fi from another?** Android gives the Wi-Fi's name only with a location
  permission, which the app does not have (`AndroidManifest.xml` lines 3 to 14). Recommendation: a
  fingerprint from what Android gives freely (gateway, DNS servers, search domain). When it cannot tell,
  it asks again. Check on device. (Decided: N9, "A fingerprint; never ask for location.")
- **New: where does "Ask before big downloads on any network" live?** Recommendation: Spending limits, in
  a Mobile data group, since for a prepaid owner data is money. (Decided: N10.)
- **Decided: show the data a download used?** Yes (D14): "Used about {N} MB of mobile data", only when any
  of it went over a paid connection.

Depends on: D8, D14, N9, N10; rules sections 1, 2, 4; `../wave-0/02-safe-mode.md` (`size` in the signed
channel file); `../wave-0/04-measure.md` (`data_mb.*` counters); `02-updates.md` and
`03-assistant-model.md` (the two users).
