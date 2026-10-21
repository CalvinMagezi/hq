#!/usr/bin/env bash
# Builds the hub APK with the SDK command line tools only (no Gradle).
# Usage: build.sh <hq android arm64 binary> <web dist dir> <caddy linux arm64 binary> <out apk>
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
hq_bin=${1:?hq binary for aarch64-linux-android}
web=${2:?built web app (apps/hq-web/dist/client)}
caddy_bin=${3:?caddy binary for linux arm64}
out=${4:?output apk}
SDK=${ANDROID_HOME:-$HOME/Library/Android/sdk}
BT=${BUILD_TOOLS:-$SDK/build-tools/36.0.0}
JAR=$SDK/platforms/android-36/android.jar
JBR=${JAVA_HOME:?set JAVA_HOME to a JDK 17 or newer}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/compiled" "$work/classes" "$work/apk/lib/arm64-v8a" "$work/apk/assets"
"$BT/aapt2" compile --dir "$here/res" -o "$work/compiled/res.zip"
"$BT/aapt2" link -o "$work/base.apk" -I "$JAR" --manifest "$here/AndroidManifest.xml" \
  --min-sdk-version 26 --target-sdk-version "${TARGET_SDK:-35}" ${DEBUGGABLE:+--debug-mode} --version-code 1 --version-name hub "$work/compiled/res.zip"
"$JBR/bin/javac" --release 8 -Xlint:-options -d "$work/classes" -classpath "$JAR" "$here"/src/example/hq/spike/*.java
"$JBR/bin/jar" cf "$work/classes.jar" -C "$work/classes" .
"$BT/d8" --min-api 26 --lib "$JAR" --output "$work" "$work/classes.jar"
cp "$hq_bin" "$work/apk/lib/arm64-v8a/libhq.so"
cp "$caddy_bin" "$work/apk/lib/arm64-v8a/libcaddy.so"
cp -R "$web" "$work/apk/assets/web"
cp "$work/base.apk" "$work/unsigned.apk"
(cd "$work" && zip -q -j unsigned.apk classes.dex && cd apk && zip -q -r ${STORE_LIBS:--0} ../unsigned.apk lib && zip -q -r ../unsigned.apk assets)
"$BT/zipalign" -P 16 -f 4 "$work/unsigned.apk" "$work/aligned.apk"
ks=${KEYSTORE:-$here/debug.keystore}
[ -f "$ks" ] || "$JBR/bin/keytool" -genkeypair -keystore "$ks" -storepass android -keypass android -alias debug \
  -dname "CN=Spike Debug" -keyalg RSA -keysize 2048 -validity 3650 >/dev/null 2>&1
"$BT/apksigner" sign --ks "$ks" --ks-pass pass:android --key-pass pass:android --out "$out" "$work/aligned.apk"
"$BT/apksigner" verify "$out" && ls -l "$out"
