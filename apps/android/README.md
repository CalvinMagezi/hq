# AgentHQ app for Android

Packages `hq` (as `libhq.so`) and Caddy (as `libcaddy.so`) with the web UI into one APK. A
foreground service runs both as children of the app and restarts them with backoff. Caddy starts
only when `files/Caddyfile` exists (see `Caddyfile.example`); `hq` reads its settings from
`files/hqhome/config.yaml`.

Build (Android Studio's JDK or any JDK 17+, Android SDK with platform android-36). Gradle wraps itself:

```bash
export JAVA_HOME=<a JDK 17 or newer> ANDROID_HOME=<the Android SDK>
cd apps/android
./gradlew :app:assembleDebug :app:testDebugUnitTest :app:lintDebug      # what CI runs; no hq inside
./gradlew :app:assembleDebug \
  -Phq.bin=<hq aarch64-linux-android> -Phq.caddy=<caddy linux arm64> \
  -Phq.web=<apps/hq-web/dist/client> -Phq.keystore=<the signing key>      # the APK for a phone
```

Build `hq` with `--features tunnel,bridge` (the CI recipe in `.github/workflows/android.yml`): the pairing
screens need `hq bridge run`. Scanning a pairing code uses CameraX and ZXing (both Apache-2.0, no Google
Play services). `-Phq.appId=<id>` builds a copy that installs beside the real app for a phone test with its
own data; it is not for a release, and the default id (`online.agenthq.app`) is what updates the installed app.

The `hq` binary must be built from 2026-10-08 or later: the standby start passes `hq web --supervised`, which keeps the web token out of `hq.log`, and an older binary exits on the unknown flag.

All four `-P` paths must be absolute and must exist, and they come together: the build stops if one is
missing or wrong, so it cannot produce an APK without hq or signed with the wrong key. Put them in
`~/.gradle/gradle.properties` if you build often; never in the repository.
The APK is `app/build/outputs/apk/debug/app-debug.apk`. It is a debug build on purpose: the debuggable
flag lets `adb shell run-as` place certificates, the Caddyfile and a restored vault in the app's
private directory (those `run-as` commands below work on this build only). The cost is that anyone
with `adb` access to an unlocked phone can run as the app, so keep USB debugging off when you are not using it.
A debug build only updates a phone that runs a debug build signed with the same `-Phq.keystore`
(keep that file and back it up); without it Gradle uses its own debug key, which suits CI and a throwaway emulator.

## Release builds and the update channel

The app id is `online.agenthq.app` and is fixed. The code keeps the Kotlin package `example.hq.spike`
(the `namespace`), which is not the app id; nothing reads the package name at run time, the shortcuts
and the file provider follow `applicationId`. A phone that ran the old `example.hq.spike` build keeps
that app beside this one; they do not share data.

A **release** build (`:app:assembleRelease`) is what people install. It is not debuggable, is signed
only with the release key, and needs `-Phq.bin`, `-Phq.caddy` and `-Phq.web` like a phone build.
Gradle refuses to package it unless all three environment variables are set, and
never signs it with a debug key:

```bash
HQ_RELEASE_KEYSTORE=<path to the .jks> HQ_RELEASE_KEYSTORE_PASSWORD=<password> HQ_RELEASE_KEY_ALIAS=<alias> \
  ./gradlew :app:assembleRelease -Phq.bin=... -Phq.caddy=... -Phq.web=... \
  -Phq.versionCode=<a number higher than the last release> -Phq.versionName=<version>
```

**Do not build a release locally.** CI does it: `.github/workflows/release.yml` builds `hq` for Android
(`android.yml`), builds the web app and this APK in the `release` environment from the secrets
`ANDROID_RELEASE_KEYSTORE_B64`, `ANDROID_RELEASE_KEYSTORE_PASSWORD` and `ANDROID_RELEASE_KEY_ALIAS`,
and fails unless `scripts/release/android-check.sh` finds the release certificate and the app id.
`versionCode` is the workflow run number and `versionName` the release version. A second job signs
`android-channel.json` (`{ version, versionCode, url, sha256, minSdk }`) with the release's minisign key and
attaches it, the APK (`agent-hq-android.apk`) and its `.sha256` to the release. The Promote workflow
copies that manifest to `channel-stable`, which is the repository's latest release.

The release certificate's SHA-256, which `apksigner verify --print-certs` must show on every release:

```text
FC:6B:48:A7:9F:DC:3D:EA:90:45:86:4B:7A:09:A0:F9:CC:99:2C:B2:40:66:A3:D6:43:91:FF:41:2A:07:29:5A
```

**The key is permanent.** Only an update signed by the same key as the installed app is accepted, and
uninstalling deletes the vault, the Keystore key and the signed-in sessions. Keep a copy of the keystore
off GitHub and off the build machine; if it is lost, every phone has to uninstall to move to a new key.

### Updating inside the app

Settings, Backup and about, has "Check for updates", and the hub service checks once a day while it
runs. A check only records what it found; the owner taps "Install update", then Android asks to confirm.
The app fetches `releases/latest/download/android-channel.json` and its `.minisig` (stable), or the
`channel-main` release's copy when "Follow every build of main" is on. It trusts the manifest only
when `libhq.so update verify-file` accepts the signature against the release key compiled into `hq`
(no signature code in Kotlin), and trusts the APK only when its SHA-256 equals the one in that signed
manifest and the manifest's URL is a release asset of this repository. A build is newer only by `versionCode`.
Android itself then refuses an APK signed with any other key. The first install needs "Install unknown apps"
turned on for the browser or file manager; the app asks once for its own updates.

## The native shell (in progress)

`ShellActivity` is the Kotlin and Jetpack Compose app described by the mobile design (page 3): the same
two bars on every main screen, five places (Notes, Tasks, a raised Chat button, Helpers, Inbox), Back
from any place to Chat and then out. It is the launcher screen. The WebView app (`MainActivity`) stays as a
fallback that only adb can open: `adb shell am start -n online.agenthq.app/.MainActivity`. Open the shell on
one place with `adb shell am start -n online.agenthq.app/.ShellActivity --es place chat --ei badge 3` (no `-S`:
that force-stops the app and hq with it).

Sizes, colours and type come from the design's own stylesheet, one value for one value (dp are the
design's px): `ui/theme/HqTheme.kt` holds the MTS tokens and type styles, `ui/shell/Icons.kt` the icons
from the same SVG paths, `ui/shell/HqShell.kt` the bars. Clear Sans (Apache 2.0, `licenses/`) is bundled.
`drawable-nodpi/hq_mark.png` is the mark embedded in the design. On a real phone the status and
navigation bar insets replace the design's fixed 28 dp and 16 dp, so the bars can sit a few dp
higher or lower than the board.

## Hey HQ: the phone's assistant

AgentHQ can take Google's place as the phone's assistant: Settings, Apps, Choose default apps, Digital
assistant app, AgentHQ. Holding Home (or the assist gesture) then opens a voice sheet over whatever is on
screen. It does not open over the lock screen (unlock first). It listens, shows what it heard, sends those words to HQ's chat and
reads the reply aloud a sentence at a time. Tap the button to stop listening early, to interrupt the reply,
or to ask a follow-up in the same chat. On a Galaxy A32 the side key opens only the power menu, so hold
Home there.

What stays on the phone: the audio and its recognition (sherpa-onnx streaming Zipformer), the spoken reply
(sherpa-onnx with the LJ Speech Piper voice), and the screen. For HQ the microphone is open only while the
sheet is showing (another app's dictation, below, opens it for that app); audio is read in 100 ms pieces,
decoded and dropped, never written. The assistant turns off
Android's assist data and screenshot, so the screen's text is never copied into the app. What leaves the
phone: the words of the request, to whatever chat model HQ is configured with, exactly as a typed message.

An assistant has to name a recognition service, and some phones then make it the default speech recogniser
(the A32 on One UI 5 kept Google's). So `OnDeviceRecognitionService` is a real recogniser on the same
on-device model, not a stub: any app that reaches it gets working dictation that stays on the phone. Android
lets in only apps that hold the microphone permission. Before the models are downloaded it answers
"language unavailable".

The speech models (about 128 MB) are downloaded once, after the owner taps Get under a notice, from the
sherpa-onnx releases on GitHub. Each archive has a pinned SHA-256 in `voice/VoiceModels.kt`, and only the
named files are unpacked into `no_backup/voice/`. The download sends nothing about the owner. Logcat gets
the sheet's phase changes with a clock (`adb logcat -s HqVoice`), never the words.

Test it beside the real app without touching it: `./gradlew :app:assembleDebug -Phq.appId=online.agenthq.voicetest`,
install with `adb install -r -g`, then `adb shell cmd role add-role-holder android.app.role.ASSISTANT
online.agenthq.voicetest 0` and `adb shell input keyevent 219`. That copy has no hq, so a request ends with
"HQ is not answering", spoken. Note the assistant settings first (`settings get secure assistant`,
`voice_interaction_service`, `voice_recognition_service`): uninstalling an assistant does not fall back to
Google, so put the old holder back with the same `add-role-holder` command.

## Name, icon and notifications

The launcher label is `AgentHQ` (the package id is `online.agenthq.app`, so an update keeps the data).
The icon reuses the web app's own artwork: the adaptive icon's foreground is `hq-icon-maskable-512.png`
on the manifest's background colour, with a monochrome layer for themed icons, and the notification
glyph is a white silhouette of the same mark at the five densities, so the launcher icon and the
notification icon are one mark. Regenerate them from `apps/hq-web/public/icons/` if the artwork changes.

HQ's web push needs a browser push service, which a WebView does not have, so the app does its own:
every 20 seconds `Alerts` asks the local `hq` for unread notifications (approvals waiting on you and
`notify_owner` messages) and posts each new one as an Android notification on the **Needs you** or
**Updates** channel; tapping one opens the matching place in the app. The first poll after an install
only records what is already unread.

If your HQ runs on a server, the app can ask it too: Settings, Notifications, **Your HQ on a server**
takes its address (https only) and a one-time code from Settings, Devices on that HQ, and signs the
phone in like any other device (`ServerHq`; the device token is sealed with the app's Keystore key).
That HQ is then asked once a minute with the ETag of its last answer, so a list that has not changed
costs a bodiless 304 on mobile data. Its questions get the same buttons, and an answer goes back to
the HQ that asked; tapping one opens that HQ's web app.

When HQ could read the dialog a coding agent waits at, the notification says what it asks and the
command or path it is about, and offers **Allow once** (only when the agent has a plain yes-once
answer) and **Decline**. The app does not read the agent's screen or type keys for these: it posts the
choice to `/api/notifications/{id}/answer`, and the server answers that dialog and no other, never with
a "don't ask again" option. Already answered (on another device, or the agent moved on) shows as such.
An older server, or a dialog HQ cannot read, keeps the previous behaviour. Signing the phone out there (Settings, Devices,
or Sign out everywhere) stops it, and the app says so once; Disconnect stops it on the phone. Alerts arrive while the app's service is running and `hq` is up, within about 20
seconds; they need no Google services. Web push for browsers and Home Screen apps is a separate
server setting (`web_push.enabled`) and is not needed for this app. Android 13 asks once for the
notification permission; also turn off "Remove permissions if app is unused" for the app, or Android
can revoke it after months without opening it.

## Credentials and the mode flag

A phone with nothing set up opens a first-run flow in the app (`ui/firstrun/`): an AI service key, optional
Telegram and Discord bot tokens, then Start. Restoring a `.hqbak` backup is the other way in. Nothing
needs `adb`. Each credential is encrypted under a key in the Android Keystore as soon as it is saved
(`SecretStore.put`) and is never written to a plain file, a log line or an Intent extra. A name must end in
`_API_KEY` or `_TOKEN` and follow the same rule as a `secrets.in` line (`HubConfig.entryProblem`), and
`HQ_WEB_AUTH_TOKEN` is reserved. Settings, "Keys on this phone", lists the stored names (never the values) and
adds, replaces or removes one; `hq` restarts to read the change.

The flow saves only which step the person is on (`firstrun` preferences), never a credential, so it resumes
after a kill. It is skipped when a model key is already stored or the app had already started `hq` before the
flow existed, so an upgrade and an `adb`-provisioned build are not asked again. A stored
`HQ_RELAY__TELEGRAM_TOKEN` or `HQ_RELAY__DISCORD_TOKEN` also sets that relay's `*_ENABLED` for the `hq` child
(`HubConfig.relaySwitches`), because a relay stays off until switched on.

"Other OpenAI-compatible" saves a `custom` backend (endpoint, optional model, `credential_env:
CUSTOM_API_KEY`) in `files/hqhome/config.yaml` after a comment marker, and refuses to touch a file that
already has its own top-level `backends:`.

Restore (`ui/firstrun/Restore.kt`) copies the backup and the passphrase or key file into the cache, holds
`hq` stopped (`HubService.holdHq`), runs `libhq.so restore <file> --into files/hqhome/.vault-restoring
--passphrase-file|--key-file <file>` with a cleared environment, and swaps the folder in for `.vault` only on
success. The passphrase is never an argument; its file is mode 0600 and is zeroed and deleted afterwards.

The run mode: `hq` starts as standby (the web app only, no relays and no scheduler) and the Start step writes
`active` into `files/mode` and restarts it to run `hq start all`; the notification then says so. Do that
only on one machine at a time: a second daemon on a copy of the same data would poll the same bots
and fire the same schedules.

### For development builds

`adb` still works, for a phone you drive from a computer. Put the credentials in a plain file and push it
into the app's private folder; the app seals each value the next time it starts `hq`, then overwrites the
file with zeros and deletes it. Write to a temporary name and rename, so a restart cannot read half a file.
Use two `adb` calls: `adb exec-in` ends its shell when the input ends, so anything chained after the
`cat` does not run:

```bash
# secrets.in: NAME=value per line; '#' comments; NAME= (empty) removes a stored name
#   HQ_RELAY__TELEGRAM_TOKEN=...
#   COPILOT_GITHUB_TOKEN=...
adb exec-in "run-as online.agenthq.app sh -c 'cat > files/secrets.tmp'" < secrets.in
adb shell "run-as online.agenthq.app sh -c 'chmod 600 files/secrets.tmp && mv files/secrets.tmp files/secrets.in'"
adb shell "run-as online.agenthq.app sh -c 'kill \$(pidof libhq.so)'"   # the supervisor restarts hq and imports
```

Switches that `hq` reads from the environment (`HQ_MCP_DEV_NO_AUTH`, `HQ_BIN_PATH`, ...) and non-secret
settings such as chat ids belong in `files/hqhome/config.yaml`. The vault, config path, web token, bind
address and port are applied after the credentials, so a credential cannot change them. A link, a directory
or a file over 64 KiB at `secrets.in` is refused and removed without being read. A push that stops half way
leaves `files/secrets.tmp` holding plaintext; delete it and push again. Stored names that the rule no
longer allows (from an older build) are dropped when `hq` starts. `adb logcat -s HQHUB` shows the names
imported and `hq mode`, never a value. To run active from `adb`, write `active` into `files/mode` and
restart `hq`.

`./gradlew :app:testDebugUnitTest` runs the parsing and file checks (`HubConfigCheck`, wrapped in a JUnit
test). `:app:lintDebug` runs Android Lint's `NewApi` check against `minSdk` 26, so a call to an Android
or Java API added after Android 8 fails the build instead of crashing Android 8 to 10 (APIs that D8
backports, such as `List.of` and `String.repeat`, are fine). CI does both. The service and the Keystore code have no automated
test; the checks cover the logic moved into `HubConfig`.
