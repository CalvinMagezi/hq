#!/usr/bin/env bash
# Runs the Android-free checks on a plain JDK. The app is compiled with --release 8, so a Java 9+
# API (collection factories, String.repeat, Path.of, ...) fails the build instead of crashing the
# older phones the app supports (Android 8 to 10). With ANDROID_JAR set, every source is compiled
# against that platform's jar, as build.sh does.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
jdk=${JAVA_HOME:+$JAVA_HOME/bin/}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
src=$here/src/example/hq/spike

mkdir "$work/plain" "$work/check"
"${jdk}javac" --release 8 -Xlint:-options -d "$work/plain" "$src/HubConfig.java"
"${jdk}javac" --release 11 -d "$work/check" "$src/HubConfig.java" "$here/check/HubConfigCheck.java"
"${jdk}java" -ea -cp "$work/check" example.hq.spike.HubConfigCheck
if [ -n "${ANDROID_JAR:-}" ]; then
  mkdir "$work/app"
  "${jdk}javac" --release 8 -Xlint:-options -d "$work/app" -classpath "$ANDROID_JAR" "$src"/*.java
  echo "app sources compile against $(basename "$(dirname "$ANDROID_JAR")") at Java 8"
fi
