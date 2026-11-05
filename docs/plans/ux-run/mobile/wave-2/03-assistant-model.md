# Wave 2 · 03 · The speech model download (finding #7, the assistant part)

Board: **FixAssistantModel** ("Fix: speech model download"), finding #7, the assistant part, design only,
revised after review, 5 Nov 2026.
Shared rule: the engine, the Wi-Fi question, the data ask, the row, the Downloads notice and the Storage
line are FixDownloads' parts, with its exact words (`01-downloads.md`). This board draws how they sit in
the voice sheet and in Settings. `../../rules-shared.md` sections 1, 4, 5, 8 win.

Headline: **The speech model follows the large download rule: its size and language before the tap, an
equal question on paid data, a percentage, Pause and Cancel with Undo, and a resume when the signal comes
back.**

## 1. Finding and the review items it closes

**Problem.** Get starts the 57 MB download at once, on mobile data too. The size sits in the middle of a
paragraph (as "about 54 MB", in 1024-based units), the button reads "Wait" and does nothing, and there is
no Cancel.

**Who it hurts, how often.** Once per phone, for every owner who tries voice, and it is the first thing
they see. On a prepaid bundle in Uganda, 57 MB is a real share of a week's data, spent on one tap with no
warning.

What changed in this revision:

- **Delete speech model** is a losing action (rules section 4): red "Delete speech model", with "Keep the
  speech model" as the safe choice. Both labels are over 18 characters, so they stack, safe on top (After
  14). (Review #10: "Delete speech model uses the old pattern".)
- **D8:** the paid-data ask has two equal outlined buttons, Wait for Wi-Fi first, the size in the label
  (After 3, 15). The old green "Download now" and its open question are gone.
- **Notification Cancel = Undo:** it acts at once and shows "Download cancelled · Undo" for the
  accessibility timeout, like the sheet (After 8, 12). Progress posts on the Downloads channel with Pause ·
  Cancel. (Review #7: "Notification Cancel works two ways".)
- **"The speech model is ready."** opens `hq://settings/speech`. It says "Hold Home to ask HQ." only when
  HQ is the assistant.
- **Sizes:** "Understands English" and the size before any tap. Space needed is FixDownloads' formula, 57
  MB + unpacked size + 10 MB. The unpacked size [U] is marked to measure from the release (review #7: "the
  unpacked size is a placeholder"). "On this phone" shows [U], since the archive is deleted after
  unpacking.
- **Did not match** is amber with an icon (After 9). The Wi-Fi question shows once per Wi-Fi (After 2,
  M10). Partial files expire after 14 days and show under Storage. The data-used line is on the model's
  page.
- **Added:** the versions card, a 360 dp frame at font scale 2.0 (After 15), letter pins, the Data Saver
  claim marked unverified, and acceptance checks that fail on today's build.

## 2. Before (as built, verified at `c04a100`)

Paths under `apps/android/app/src/main/kotlin/example/hq/spike/voice/`.

- `VoiceSession.kt:21-23` phases `NeedsModels("Voice needs its speech models", "Get")`,
  `Downloading("Downloading speech models", "Wait")`, `Preparing("Getting ready", "Wait")`.
- `VoiceSession.kt:65` `NeedsModels -> download()`; `:104` `download()` runs `downloadThenListen`;
  `:107-121` `downloadThenListen` shows `DOWNLOAD_NOTICE`, and on success calls `hearThenAsk()` (line 121),
  which opens the microphone by itself.
- `VoiceModels.kt:40` `fetch()`, `:58` `fetchOne()` (Range resume, whole-file SHA-256), `:134` `SOURCES`,
  `:138` `57_267_600` bytes, `:152` `DOWNLOAD_MB = bytes / (1024 * 1024)` ("about 54 MB").
- `apps/android/README.md` says "about 55 MB".

### Before frames

- **Before A · The notice and Get.** Sheet label "Voice needs its speech models"; note "Your speech is
  recognised on this phone, so it needs about 54 MB of speech models, downloaded once from the sherpa-onnx
  project on GitHub. The download sends nothing about you. After that, your voice never leaves the phone:
  only the words of your request go to HQ's chat model." (pin A); round button [Get] (pin B).
- **Before B · Downloading.** "Downloading speech models 42%" (pin C), the same note, round button [Wait]
  (pins D, E).

Pins: **A** the size is buried and off by three ("About 54 MB" is `VoiceModels.DOWNLOAD_MB`, bytes divided
by 1,048,576; Android's storage screen counts 57 MB; "sherpa-onnx" is a name the owner does not need
here). **B** Get starts at once, on any network. **C** a percentage, but no MB and no sign of which
network it is using. **D** the button reads "Wait" and does nothing; there is no Cancel. **E** it listens
by itself when the model lands: `downloadThenListen` goes straight to `hearThenAsk`, which can open the
microphone minutes after the owner looked away.

## 3. Decision

- **Use FixDownloads' parts as they are:** the row, the Wi-Fi question, the data ask, the Downloads
  notice, the engine in the hub service. The model gives its name, its pinned byte count, its pinned
  unpacked size and "Understands English".
- **In the sheet, the row takes the round button's place** until the model is on the phone.
- **Never listen by itself after a download.** It says "Speech model ready" and waits for Ask.
- **Spent data cannot be put back (D8):** the ask gives both choices the same weight, Wait first.
- **Settings gets the model's own page** (`hq://settings/speech`), reached from Settings home once
  downloaded and from Phone assistant: what it is, where it came from, data used, and Delete.

Rejected:

- **Offering the download in first run.** 57 MB for a maybe is the cost this finding is about.
- **A smaller model to dodge the question.** The smaller LibriSpeech model lost first words and technical
  terms in the real-voice test; Kroko stays.

## 4. Screens and states (After)

The voice sheet is the bottom sheet over the current app (as built), with the label at the top, a body
line, then the shared download row where the round button sits.

1. **Size and language first.** Title "Speech model needed". Body "To hear you, HQ needs a speech model on
   this phone. It downloads once and sends nothing about you. After that, your voice stays on the phone.
   Only the words of your question go to HQ." Row: "Speech model · 57 MB download · Understands English"
   with [Download · 57 MB] (pin 1). Text button [Not now] (pin 2).
2. **First time on this Wi-Fi (M10).** Over frame 1: "Is this Wi-Fi paid by the MB?" / "Pocket routers
   and phone hotspots often charge for each MB. HQ asks once for each Wi-Fi." / [It's paid] [It's free]
   (pin 3).
3. **On mobile data: the equal ask (D8).** "Download on mobile data?" / "The speech model is 57 MB. Your
   data plan may charge for it." / [Wait for Wi-Fi] [Download · 57 MB] (equal, outlined; pin 3).
4. **Waiting for Wi-Fi** (pin 4). "Speech model needed" / "You can close this. It starts by itself on
   Wi-Fi." Row "Speech model · 57 MB · Waiting for Wi-Fi" (amber with icon) / [Download · 57 MB] /
   [Cancel].
5. **Downloading** (pin 5). Title "Downloading the speech model" / "You can close this. The download keeps
   going." Row "Speech model · Over Wi-Fi · 45% · 26 of 57 MB" with the blue bar, [Pause] [Cancel].
6. **Paused: left Wi-Fi** (pin 6). Row "Speech model · 57 MB · Paused at 26 of 57 MB. It goes on with
   Wi-Fi." / [Download · 31 MB] / [Cancel].
7. **Cut off** (pin 7). Row "Speech model · 57 MB · Stopped at 35 of 57 MB. It starts again when you are
   back online." / [Cancel].
8. **Cancelled: Undo for the timeout** (pin 8). Back to frame 1's row ("To hear you, HQ needs a speech
   model on this phone." / "Speech model · 57 MB download · Understands English" / [Download · 57 MB]),
   snackbar "Download cancelled · Undo".
9. **Did not match** (pin 9). Row "Speech model · 57 MB" / amber with icon "The download did not match.
   Nothing was kept." / [Try again · 57 MB].
10. **Not enough space, before a byte** (pin 10). Dialog "Not enough space" / "The phone is almost full.
    Free up [N] MB and try again." / "This needs [57 + U + 10] MB free: the download, its unpacked copy,
    and 10 MB to spare." / stacked [Not now] · [Open Android settings] (over 18 characters) / "Android
    shows what you can remove."
11. **Ready: waits for a tap** (pin 11). "Speech model ready" / "Tap to ask." / round button [Ask]. The
    microphone stays off.
12. **With the sheet closed** (Android's shade, generic). "Running · Downloads": "AgentHQ · Downloads /
    Downloading the speech model / 45% · 26 of 57 MB / Pause · Cancel". "After Cancel · Downloads" (pin
    8): "Download cancelled / The speech model / Undo". "Done · Updates" (pin 12): "AgentHQ · Updates /
    The speech model is ready. / Hold Home to ask HQ. (only when HQ is the assistant)".
13. **Settings, Speech model** (pin 13). Title "Speech model". Facts: "On this phone · [U] MB", "Language ·
    Understands English", "Comes from · The sherpa-onnx project on GitHub", "Data used · Used about 57 MB
    of mobile data". Note "The download carried nothing about you, and the model never sends anything."
    Ink trigger "Delete speech model". "Not downloaded yet, it reads": "Speech model · 57 MB download ·
    Understands English · [Download · 57 MB]".
14. **Delete: the losing-action dialog** (pin 14). "Delete the speech model?" / "HQ will need to download
    57 MB again to hear you." / stacked [Keep the speech model] on top, [Delete speech model] (red
    `#FF6B6B`, black label) below.
15. **360 dp at font scale 2.0** (pin 16). "Download on mobile data?" / body / [Wait for Wi-Fi] on top,
    [Download · 57 MB] below, both full width.

### Sizes and space, one formula (FixDownloads)

- **Download size:** 57,267,600 bytes (`VoiceModels.SOURCES`), shown as "57 MB": 1 MB = 1,000,000 bytes,
  as Android's storage screen counts.
- **Unpacked size [U]:** the four kept files (encoder, decoder, joiner, tokens), to measure from
  `sherpa-onnx-streaming-zipformer-en-kroko-2025-08-06.tar.bz2` and pin beside the byte count. Not
  measured on this board.
- **Space needed:** 57 MB + [U] MB + 10 MB, checked before any ask.
- **On this phone after the download:** [U] MB. `fetchOne` deletes the archive once it is unpacked, so
  Delete frees [U], not 57.
- **Server HQ owners:** no variant. The model is the phone's own download; a server HQ is not involved.

## 5. Exact copy (FixDownloads' words, {name} = the speech model)

| Where | Words |
|-------|-------|
| Title (was "Voice needs its speech models") | "Speech model needed" |
| Sheet body | "To hear you, HQ needs a speech model on this phone. It downloads once and sends nothing about you. After that, your voice stays on the phone. Only the words of your question go to HQ." |
| Row | "Speech model · 57 MB download · Understands English" |
| Row button | "Download · 57 MB" |
| Dismiss | "Not now" |
| Wi-Fi question | "Is this Wi-Fi paid by the MB?" · "Pocket routers and phone hotspots often charge for each MB. HQ asks once for each Wi-Fi." · "It's paid" · "It's free" |
| Ask, mobile data | "Download on mobile data?" · "The speech model is 57 MB. Your data plan may charge for it." |
| Ask, paid Wi-Fi | "Download on this Wi-Fi?" · "You said this Wi-Fi is paid by the MB. The speech model is 57 MB." |
| Ask, roaming | "Download while roaming?" · "The speech model is 57 MB. Roaming may cost more than at home." |
| Ask buttons (was "Download now (57 MB) · Wait for Wi-Fi") | "Wait for Wi-Fi" · "Download · 57 MB" (equal, outlined) |
| Waiting line | "You can close this. It starts by itself on Wi-Fi." |
| Downloading title (was "Downloading speech models 42%") | "Downloading the speech model" |
| Downloading line (was "You can close this. The download carries on.") | "You can close this. The download keeps going." |
| Row states | "Waiting for Wi-Fi" · "Over Wi-Fi" · "Over mobile data" · "45% · 26 of 57 MB" · "Paused at 26 of 57 MB. It goes on with Wi-Fi." · "Paused at 26 of 57 MB" · "Stopped at 35 of 57 MB. It starts again when you are back online." · "Checking the download…" |
| Row buttons | "Download · 31 MB" · "Pause" · "Resume" · "Cancel" · "Try again · 57 MB" |
| Snackbar and notice | "Download cancelled" · "Undo" |
| No match (amber) | "The download did not match. Nothing was kept." |
| Server failing (amber) | "Couldn't reach the download server. Check your internet and try again." · "35 of 57 MB is kept." |
| No space | "Not enough space" · "The phone is almost full. Free up [N] MB and try again." · "This needs [57 + U + 10] MB free: the download, its unpacked copy, and 10 MB to spare." · "Not now" · "Open Android settings" · "Android shows what you can remove." |
| Ready | "Speech model ready" · "Tap to ask." |
| Notice, running | "Downloading the speech model" · "45% · 26 of 57 MB" · "Pause" · "Cancel" |
| Notice, done (was "Speech model ready · Hold Home to ask HQ.") | "The speech model is ready." · "Hold Home to ask HQ." (only when HQ is the assistant) |
| Page facts | "On this phone" · "[U] MB" · "Language" · "Understands English" · "Comes from" · "The sherpa-onnx project on GitHub" · "Data used" · "Used about {N} MB of mobile data" |
| Page note | "The download carried nothing about you, and the model never sends anything." |
| Delete | "Delete the speech model?" · "HQ will need to download 57 MB again to hear you." · "Keep the speech model" · "Delete speech model" (red) |
| Phone assistant after Delete | "On · speech model not downloaded" |
| TalkBack, bar | "45 percent, 26 of 57 megabytes" |
| Done line (FixDestructive) | "Speech model deleted." |

## 6. Behaviour rules

1. **Size first,** from the pinned byte count, in the shared units. "Understands English" sits under it.
   The same "57 MB" everywhere: sheet, Settings, notice, README.
2. The sheet says what the model is for and that the download sends nothing about the owner. **Not now**
   closes the sheet; nothing is saved.
3. **Download on a free network starts at once.** On a Wi-Fi HQ has not seen, the Wi-Fi question comes
   first, once per Wi-Fi. On mobile data, paid Wi-Fi or roaming, the D8 ask: two equal outlined buttons,
   Wait for Wi-Fi first, the size in the label. Back or a tap outside spends nothing and starts no wait.
4. **Wait for Wi-Fi** is saved across restarts. The hub service starts within seconds of a free network.
   Download · 57 MB from the row still works, with the ask on paid networks. Cancel drops the wait.
5. **Downloading:** the row updates at most four times a second, with Pause and Cancel. Closing the sheet
   does not stop it; the line says so.
6. **Left Wi-Fi** without consent for paid data: pause and show what is left. Consent covers this download
   to the end on any paid network (FixDownloads).
7. **Cut off:** the bytes are kept and it resumes with a Range request when the network is back. If the
   server will not resume, the "Start over?" ask comes on paid networks.
8. **Cancel acts at once,** in the sheet and in the notice alike: "Download cancelled · Undo" for the
   accessibility timeout (10 s on 8 and 9). Undo goes on from the kept bytes. After the timeout the
   partial file is deleted.
9. **Did not match** the pinned size and SHA-256 of the whole file, resumed or not: the file is deleted
   and nothing is unpacked. Amber with an icon. Try again starts over and asks again on paid networks.
10. **Space** is checked before any ask: 57 MB + [U] MB + 10 MB. Not enough shows FixDownloads' dialog;
    "Open Android settings" is over 18 characters, so its buttons stack.
11. **Ready** after "Checking the download…" (hash and unpack). The microphone stays off until Ask.
12. **Sheet closed:** progress on the Downloads channel with Pause · Cancel. When done, one silent "The
    speech model is ready." on Updates; tap opens `hq://settings/speech`. "Hold Home to ask HQ." only when
    `isActiveService` is true.
13. **The Speech model page:** plain facts, and "Used about {N} MB of mobile data" only when any of it
    went over a paid connection. The Settings home row shows only once the model is on the phone (rules
    section 5).
14. **Delete** is a losing action: red "Delete speech model" at the bottom, "Keep the speech model" on
    top. Back or a tap outside keeps it. Then the Settings home row goes, and Phone assistant reads "On ·
    speech model not downloaded".
15. **Partial files** are kept 14 days after the last byte, and listed under Storage as Unfinished
    downloads (FixDownloads).
16. **Font scale 1.5 and up:** the ask stacks, Wait for Wi-Fi on top.

## 7. Android versions

| Version | What differs for this fix |
|---------|---------------------------|
| 8 and 9 | Undo lasts 10 s: `getRecommendedTimeoutMillis` is API 29. `NET_CAPABILITY_NOT_ROAMING` is API 28, so 8.0 and 8.1 read roaming from `TelephonyManager.isNetworkRoaming()`. After Delete, other apps' dictation gets a client error (`ERROR_LANGUAGE_UNAVAILABLE` is API 31). The Downloads and Updates channels exist from 8.0 and are made once at the hub's start. |
| 10 and 11 | Undo follows the phone's accessibility timeout, at least 10 s. Android 11 may mark a 5G plan as free for a while; HQ follows `isActiveNetworkMetered()` and still asks about any Wi-Fi it has not seen. |
| 12 | After Delete, other apps' dictation gets "language unavailable", as before the first download. |
| 13+ | Without notification permission the download still runs; only the row shows it, and "The speech model is ready." is not posted. The next sheet shows Ready. From 14 the owner can swipe the Downloads notice away while it runs. |

Android constraints, as checked on the board:

- All of FixDownloads' constraints apply: network capabilities on the active network, a callback held by
  the hub service, `StorageManager.getAllocatableBytes`, `ACTION_MANAGE_STORAGE`, no WorkManager.
- **The hub service is in the same process** as the sheet (no `android:process` in the manifest), so the
  download moves from `VoiceEngines`' process scope into the shared engine with no new service.
- **Resume already works for this source:** GitHub release downloads redirect to a host that answers
  Range with 206. The finished file, resumed or not, is checked against the pinned size and SHA-256 before
  it is unpacked.
- `noBackupFilesDir` keeps the partial file and the model out of backups and away from cache clearing.
- **Data Saver: not verified.** FixDownloads holds the device check; the row shows its Data Saver line if
  it is needed.
- Android counts the bytes under AgentHQ in its data usage screen, in 1,000,000-byte MB.

## 8. Code touchpoints

| File | Change | Exists |
|------|--------|--------|
| `voice/VoiceModels.kt` | `fetch` (line 40) becomes one item of FixDownloads' engine with its name, pinned bytes, pinned unpacked bytes and the unpack step. `DOWNLOAD_MB` (line 152) goes. `delete()` for Settings. | yes (`delete()` NEW) |
| `voice/VoiceEngines.kt` | `download` and `progress` read the shared engine's state; no Deferred of its own. | yes |
| `voice/VoiceSession.kt` | Phases `NeedsModel`, `Downloading`, `ModelReady` (today `NeedsModels`, `Downloading`, `Preparing` at lines 21-23). `downloadThenListen` (line 107) becomes download, then wait. The "Wait" no-op and `DOWNLOAD_NOTICE` go. | yes |
| `voice/VoiceActivity.kt` | The shared row and asks in the sheet in place of the round button while there is no model. | yes |
| `ui/settings/SettingsScreens.kt` | The Speech model page and row, the delete dialog. | yes |
| `ui/shell/Links.kt` | `"speech"` in `SETTINGS_SCREENS` (line 28). | yes |
| `res/values/strings.xml` | The model strings; the shared ones stay with FixDownloads' keys. | yes |
| `apps/android/README.md` | "about 55 MB" becomes 57 MB and the Wi-Fi rule. | yes |

## 9. Server, release or desktop changes

None. Pin the unpacked size [U] beside the byte count in `VoiceModels.SOURCES` once measured.

## 10. Acceptance checks, Galaxy A32 (each fails on today's build)

1. Wi-Fi off, SIM data on, no model: press, Download · 57 MB. The ask shows two equal outlined buttons.
   Android's data usage for AgentHQ does not move until Download · 57 MB.
2. Join a Wi-Fi the phone has not used with HQ: the Wi-Fi question shows before any byte.
3. Wait for Wi-Fi, close the sheet, join a free Wi-Fi: it starts within 10 s, the Downloads notice shows
   Pause · Cancel, then "The speech model is ready."
4. Cancel from the notice at 40%: it shows "Download cancelled · Undo". Undo within the timeout goes on
   with a 206. Without Undo, the partial file is gone from `no_backup/voice/`.
5. HQ not the assistant: the ready notice has no "Hold Home" line, and its tap opens Settings, Speech
   model.
6. On Wi-Fi, turn Wi-Fi off at about 40%: paused with what is left; no bytes go over mobile data.
7. A proxy serving changed bytes: "The download did not match." in amber; nothing unpacked.
8. Free space under 57 + [U] + 10 MB: "Not enough space" before any network use.
9. After Ready, the status-bar microphone dot does not light until Ask.
10. Settings, Speech model, Delete speech model: Keep is on top and Back keeps it. Delete frees [U] MB in
    Android's storage screen, and the Settings home row is gone.
11. Android 8.1 emulator: Undo lasts 10 s. Android 14 emulator at 360 dp and font scale 2.0: the ask
    stacks with Wait for Wi-Fi on top.

## 11. Check on device

- Measure the unpacked size [U] from the Kroko release (FixPlan wave 2 "the unpacked model size").
- Data Saver with the hub (held by FixDownloads).
- The A32 runs checks 1 to 10.

## 12. Open questions and decisions it depends on

None left on the board. D8 settled which button leads. Where "Ask before big downloads on any network"
lives and how HQ tells one Wi-Fi from another are FixDownloads' questions (N9, N10).

Depends on: D8, D14, N9, N10, rules sections 1, 4, 5, 8; `01-downloads.md`; `../wave-1/03-destructive.md`
(Delete speech model); `../wave-3/01-assistant-setup.md` (`isActiveService`, Phone assistant row).
