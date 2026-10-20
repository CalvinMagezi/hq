#!/usr/bin/env bash
# Runs the Android-free checks on a plain JDK. With ANDROID_JAR set it also compiles every source
# of the app against that platform, which is what the APK build would otherwise find out late.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
jdk=${JAVA_HOME:+$JAVA_HOME/bin/}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
"${jdk}javac" --release 11 -d "$work/check" "$here/src/example/hq/spike/HubConfig.java" "$here/check/HubConfigCheck.java"
"${jdk}java" -ea -cp "$work/check" example.hq.spike.HubConfigCheck
if [ -n "${ANDROID_JAR:-}" ]; then
  "${jdk}javac" --release 11 -d "$work/app" -classpath "$ANDROID_JAR" "$here"/src/example/hq/spike/*.java
  echo "app sources compile against $(basename "$(dirname "$ANDROID_JAR")")"
fi
