#!/usr/bin/env bash
# Runs the Android-free checks on a plain JDK. With ANDROID_JAR set it also compiles every source of
# the app against that platform's jar, as build.sh does.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
jdk=${JAVA_HOME:+$JAVA_HOME/bin/}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# The app supports Android 8 to 10, which lack the Java 9+ collection factories and string methods.
# Compiling against a recent platform jar cannot see that, so these are banned by name.
if grep -n -E '(Set|List|Map)\.(of|copyOf|entry)\(|\.(repeat|isBlank|strip)\(' "$here"/src/example/hq/spike/*.java; then
  echo "error: an API that Android 8 to 10 lacks (see above)" >&2
  exit 1
fi

"${jdk}javac" --release 11 -d "$work/check" "$here/src/example/hq/spike/HubConfig.java" "$here/check/HubConfigCheck.java"
"${jdk}java" -ea -cp "$work/check" example.hq.spike.HubConfigCheck
if [ -n "${ANDROID_JAR:-}" ]; then
  "${jdk}javac" --release 11 -d "$work/app" -classpath "$ANDROID_JAR" "$here"/src/example/hq/spike/*.java
  echo "app sources compile against $(basename "$(dirname "$ANDROID_JAR")")"
fi
