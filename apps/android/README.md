# HQ hub app for Android

Packages `hq` (as `libhq.so`) and Caddy (as `libcaddy.so`) with the web UI into one APK. A
foreground service runs both as children of the app and restarts them with backoff. Caddy starts
only when `files/Caddyfile` exists (see `Caddyfile.example`); `hq` reads its settings from
`files/hqhome/config.yaml`.

Build (JDK 17+, Android SDK build-tools 36, platform android-36):

```bash
export JAVA_HOME=<a JDK 17 or newer>
DEBUGGABLE=1 apps/android/build.sh <hq aarch64-linux-android> <apps/hq-web/dist/client> <caddy linux arm64> out.apk
```

`DEBUGGABLE=1` lets `adb shell run-as` place certificates, the Caddyfile and a restored vault in
the app's private directory. Build without it for the version that stays on the phone; installing
it over the debuggable one with `adb install -r` (same signing key) keeps the data and closes
`run-as`. The first build creates a signing key at `apps/android/debug.keystore` (ignored by git), or at
`$KEYSTORE` if set. Keep that file: only the same key can update the installed app.

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
without being read. `adb logcat -s HQHUB` shows the names imported, never a value.

`hq` starts as standby: the web app only, no relays and no scheduler. Write `active` into
`files/mode` and restart `hq` to run `hq start all` instead; the notification then says so. Do that
only on one machine at a time: a second daemon on a copy of the same data would poll the same bots
and fire the same schedules.

`check.sh` runs the parsing and file checks on a plain JDK and bans Java 9+ collection factories and
string methods by name (Android 8 to 10 lack them). Set `ANDROID_JAR` to also compile every source
against that platform's jar. That finds a misspelled Android API but not one added after SDK 26,
the minimum. CI does both.
