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

Nothing readable is kept for credentials. Put the ones the instance needs in a plain file and push
it into the app's private folder; the app encrypts each value under a key in the Android Keystore
the next time it starts `hq`, then overwrites and deletes the file:

```bash
# secrets.in: NAME=value per line; '#' comments; NAME= (empty) removes a stored name
HQ_RELAY__TELEGRAM_TOKEN=...
COPILOT_GITHUB_TOKEN=...
adb exec-in "run-as example.hq.spike sh -c 'cat > files/secrets.in && chmod 600 files/secrets.in'" < secrets.in
adb shell "run-as example.hq.spike kill \$(pidof libhq.so)"   # the supervisor restarts hq and imports
```

Names must be `HQ_*` (any config key, with `__` for nesting) or end in `_API_KEY` or `_TOKEN`.
Fixed settings (vault, config, web token, bind address and port) are applied afterwards, so a
credential cannot change them. `adb logcat -s HQHUB` shows the names imported, never a value.

`hq` starts as standby: the web app only, no relays and no scheduler. Write `active` into
`files/mode` and restart `hq` to run `hq start all` instead. Do that only on one machine at a time:
a second daemon on a copy of the same data would poll the same bots and fire the same schedules.

`check.sh` runs the parsing checks on a plain JDK; set `ANDROID_JAR` to also compile every source.
