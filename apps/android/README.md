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
