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

All four `-P` paths must be absolute and must exist, and they come together: the build stops if one is
missing or wrong, so it cannot produce an APK without hq or signed with the wrong key. Put them in
`~/.gradle/gradle.properties` if you build often; never in the repository.
The APK is `app/build/outputs/apk/debug/app-debug.apk`. It is a debug build on purpose: the debuggable
flag lets `adb shell run-as` place certificates, the Caddyfile and a restored vault in the app's
private directory. The cost is that anyone with `adb` access to an unlocked phone can run as the app, so
keep USB debugging off when you are not using it; a non-debuggable build comes with release work. Release signing, the final package id and an update channel are separate work
(PERSONAL-INBOX-064).

**Do not change the id or the key.** The package id stays `example.hq.spike` and the APK must be signed
with the key the installed app was signed with (`-Phq.keystore`, alias `debug`): only an update with the
same id and the same signature keeps the vault, the Keystore key and the signed-in sessions, and
uninstalling deletes all three. **Keep the key file and back it up off this machine**: if it is lost, the
phone can only be moved to a new key by uninstalling, which deletes its data. Keep `versionCode` the same
or higher. Install with `adb install -r`
only. Without `-Phq.keystore` Gradle signs with its own debug key, which is fine for CI and for a
throwaway emulator but cannot update a phone that already runs the app.

## Name, icon and notifications

The launcher label is `AgentHQ` (the package id stays `example.hq.spike`, so an update keeps the data).
The icon reuses the web app's own artwork: the adaptive icon's foreground is `hq-icon-maskable-512.png`
on the manifest's background colour, with a monochrome layer for themed icons, and the notification
glyph is a white silhouette of the same mark at the five densities, so the launcher icon and the
notification icon are one mark. Regenerate them from `apps/hq-web/public/icons/` if the artwork changes.

HQ's web push needs a browser push service, which a WebView does not have, so the app does its own:
every 20 seconds `Alerts` asks the local `hq` for unread notifications (approvals waiting on you and
`notify_owner` messages) and posts each new one as an Android notification on the `Alerts` channel;
tapping one opens the app on `/notifications`. The first poll after an install only records what is
already unread. Alerts arrive while the app's service is running and `hq` is up, within about 20
seconds; they need no Google services. Web push for browsers and Home Screen apps is a separate
server setting (`web_push.enabled`) and is not needed for this app. Android 13 asks once for the
notification permission; also turn off "Remove permissions if app is unused" for the app, or Android
can revoke it after months without opening it.

## Credentials and the mode flag

Credentials are kept as ciphertext. Put the ones the instance needs in a plain file and push it into
the app's private folder; the app encrypts each value under a key in the Android Keystore the next
time it starts `hq`, then overwrites the file with zeros and deletes it. The web token is sealed the
same way (a token left in plain preferences by an earlier build is moved on first use). Write to a
temporary name and rename, so a restart cannot read half a file. Use two `adb` calls: `adb exec-in`
ends its shell when the input ends, so anything chained after the `cat` does not run:

```bash
# secrets.in: NAME=value per line; '#' comments; NAME= (empty) removes a stored name
#   HQ_RELAY__TELEGRAM_TOKEN=...
#   COPILOT_GITHUB_TOKEN=...
adb exec-in "run-as example.hq.spike sh -c 'cat > files/secrets.tmp'" < secrets.in
adb shell "run-as example.hq.spike sh -c 'chmod 600 files/secrets.tmp && mv files/secrets.tmp files/secrets.in'"
adb shell "run-as example.hq.spike sh -c 'kill \$(pidof libhq.so)'"   # the supervisor restarts hq and imports
```

Only credentials are accepted: a name must end in `_API_KEY` or `_TOKEN`, and `HQ_WEB_AUTH_TOKEN` is
reserved. Switches that `hq` reads from the environment (`HQ_MCP_DEV_NO_AUTH`, `HQ_BIN_PATH`, ...)
and non-secret settings such as chat ids belong in `files/hqhome/config.yaml`. The vault, config
path, web token, bind address and port are applied after the credentials, so a credential cannot
change them. A link, a directory or a file over 64 KiB at `secrets.in` is refused and removed
without being read. A push that stops half way leaves `files/secrets.tmp` holding plaintext; delete it
and push again. Stored names that the rule no longer allows (from an older build) are dropped when
`hq` starts. `adb logcat -s HQHUB` shows the names imported, never a value.

`hq` starts as standby: the web app only, no relays and no scheduler. Write `active` into
`files/mode` and restart `hq` to run `hq start all` instead; the notification then says so, and `adb
logcat -s HQHUB` shows `hq mode`. Do that
only on one machine at a time: a second daemon on a copy of the same data would poll the same bots
and fire the same schedules.

`./gradlew :app:testDebugUnitTest` runs the parsing and file checks (`HubConfigCheck`, wrapped in a JUnit
test). `:app:lintDebug` runs Android Lint's `NewApi` check against `minSdk` 26, so a call to an Android
or Java API added after Android 8 fails the build instead of crashing Android 8 to 10 (APIs that D8
backports, such as `List.of` and `String.repeat`, are fine). CI does both. The service and the Keystore code have no automated
test; the checks cover the logic moved into `HubConfig`.
